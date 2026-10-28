# R1 deploy runbook: a crafted payment never halts the follower

Branch `fix/r1-crafted-tx`, on top of `main` at `7ad4588`:
- `464fd7d`: a crafted payment to the vault is ignored, not a halt.
- `9c6a57b`: every fetched transaction must hash to the txid asked for.

Together they fix audit B finding R1: one crafted payment to the vault halts
the engine and all three keyholders.

## What changes

- An output that decrypts in the compact block (its first 52 bytes) but not
  in the full transaction is logged ("does not decrypt in full; ignored") and
  ignored, like any other non-game note. If it carries value, it is float, as
  every vault note unknown to the game already is. Ingestion continues.
- Anything else that does not match the compact block (another note
  commitment, another value, a missing action, bytes that do not hash to the
  txid) is still an error. So a lightwalletd that alters a real dig cannot
  make it look ignorable. The pool also marks that server failed and tries
  the next one.
- The verifier's Digs check skips the same outputs, and fails on bytes that do
  not hash to their txid.

## What ships

| Service | Today (running) | After this deploy |
| --- | --- | --- |
| engine | `9a7b05605f7f7dc64aea412ccaf9b8ebe897298c35c49e0fd68d62b72eb342d7` (R2 fix, deployment `af7c59fc-62f8-4c36-a1b0-ecbd40eada64`) | `5b9096e3ed3fc1caad63ed1acc4f21ac2a9c2a4eb34a564db99b60213a9abf8c` |
| keyholder-1/2/3 | `d430583070a3920110ce839b691d33b1ef3e1b4c8af39cb774bae5f157b79ef5` (2026-10-23; deployments `daa7df60-31dd-4097-a849-6193d2f4f958`, `4fdcb96c-e136-46fd-913c-a3cef22eff0c`, `baddf8b3-6d7e-4dc5-8a49-1c6cd5d35f05`) | `b85cc20c02ee4d1c7308690fecf456efd0e0cbb548199f9b4564d8f71105bb4e` |
| relay (`zeek-verify`, optional) | unchanged | `ab7b1b461b06567acc0f72506437cfa9ffe889b25b22cbfa8d5ffa95a90edeab` |

The "Today" hashes were read from the running containers on 2026-10-27
(`sha256sum /app/zeek-*`). The new hashes are from `cargo zigbuild --release`
of `9c6a57b` (the build `scripts/railway-deploy.sh` runs). The script prints
them again before it uploads: **stop if they differ**.

**The keyholders still run 2026-10-23 code.** This deploy also brings them:
- `07b7d92`: the quicknet pin. The live `game.json` (`df32bcc9…47a7`) passes it.
- `7ad4588`: R2. Their database gains the empty `settlement_rechecks` table.
- `af753cf`: engine-loop code they do not run.

Nothing changes what they sign: the settlement rules and the verify-then-sign
checks are untouched.

**Not part of the four services:**
- The relay binary (`zeek-verify`) changes too, but the relay runs no round
  checks: redeploying it is optional.
- The browser verifier (the Pocket's wasm) gains the same skip and txid check
  in `check_digs`. Until the wasm is rebuilt and the site redeployed, a round
  holding a crafted payment fails "Digs" in the browser's Verify, although the
  engine and keyholders settle it correctly. Do that web deploy after the four
  services.

## Why the order does not affect safety

- Old and new code classify every output that exists on chain today the same
  way. Nothing on chain triggers R1: 313 of 313 vault notes since genesis are
  stored by the engine, and the logs show no undecryptable output.
- The new code changes behavior only for a crafted output, by ignoring it
  instead of halting, and for a server serving bytes that do not hash to their
  txid, by trying another server instead of using them.
- So a mixed fleet (some services old, some new) agrees on every round unless a
  crafted payment lands mid-deploy. In that case only the old services stop:
  finish the deploy.

## Order

Deploy one service at a time, so two keyholders can always sign:

1. keyholder-1
2. keyholder-2
3. keyholder-3
4. engine (last: its restart pauses the board and payouts for about a minute)

## Before the first service, and again before each one

All checks are read-only. Stop and wait if any check fails.

```sh
E=https://engine-production-445c.up.railway.app
# 1. No payout in flight: both lists empty, shortfall 0.
curl -s $E/v1/vault/ledger | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["in_progress_rounds"], d["unpaid_rounds"], d["shortfall"])'
# 2. Nothing signed but not mined (expect no output).
(cd data/railway && railway ssh --service engine -- sqlite3 -readonly /data/data/zeek-engine.sqlite \
  "SELECT round, state FROM payouts p WHERE state = 'signing' OR NOT EXISTS (SELECT 1 FROM payout_openings o WHERE o.round = p.round);")
# 3. At least 4 minutes (10 blocks) before the open round closes.
curl -s $E/v1/board | python3 -c 'import json,sys,time; d=json.load(sys.stdin); print(d["status"], d["blocks_remaining"], int(d["estimated_close_time"]-time.time()), "s")'
# 4. No reveal pending: no beacon time within the last 2 or next 6 minutes.
curl -s $E/v1/rounds/pending | python3 -c 'import json,sys,time; print([p["round"] for p in json.load(sys.stdin)["rounds"] if -120 <= p["beacon_time"]-time.time() <= 360])'
```

**Disk:** the build needs room. Check `df -h /` and keep at least 15 GB free
(`target/` grows).

## Rollback copy, made before each service is deployed

Each container holds its whole deployed folder, with its `Dockerfile` and
`SHA256SUMS`, in `/app`. Copy it out exactly, read-only, and check the binary
hash:

**`data/rollback-engine/` from the R2 deploy is stale:** it holds the older
engine `0b0bbd23…`, not the running `9a7b0560…`. Move it aside first (do not
delete it): `mv data/rollback-engine data/rollback-engine.r2`. Do the same for
any other `data/rollback-$SVC/` that already exists, so no old file lingers in
the new copy.

```sh
SVC=keyholder-1   # then keyholder-2, keyholder-3, engine
mkdir -p "data/rollback-$SVC/stage"
# The pipe runs inside the container (sh -c), so the archive crosses ssh as text.
(cd data/railway && railway ssh --service "$SVC" -- sh -c 'tar -C /app -cf - . | base64 -w0' 2>/dev/null) \
  | grep -v '^Using SSH key' | tr -d '\r\n' | base64 -d | tar -C "data/rollback-$SVC/stage" -xf -
(cd "data/rollback-$SVC/stage" && shasum -a 256 -c SHA256SUMS)   # every file: OK
shasum -a 256 "data/rollback-$SVC/stage/zeek-"*   # must equal the "Today" hash above
```

`data/` is gitignored. The copies hold only the binary and public config: the
secrets live on the volumes, which a deploy never touches. (Tested on
keyholder-1 on 2026-10-27: every file in `SHA256SUMS` OK, binary `d4305830…`.)

**One-command rollback** for a service:

```sh
(cd data/railway && railway up "../rollback-$SVC/stage" --path-as-root --no-gitignore --service "$SVC" --detach)
```

The dashboard's Rollback on the "Today" deployment ID also works while Railway
still keeps that image.

## Deploy one service

```sh
scripts/railway-deploy.sh keyholder-1   # builds (no-op if built) and uploads only that service
```

Check the printed hash against the table, then watch the deployment:

```sh
(cd data/railway && railway deployment list --service keyholder-1 --json | head -c 300)
```

It should reach `SUCCESS`.

## Verify each service

**keyholder-N:**
1. The running binary is the new one:
   `railway ssh --service keyholder-N -- sha256sum /app/zeek-keyholder` shows
   `b85cc20c…bb4e`.
2. Its logs show `ingested` lines and no `ERROR`:
   `railway logs --service keyholder-N -n 80`.
3. The engine sees it: `curl -s $E/v1/keyholders/status` lists it as reachable.
4. Only then go on to the next keyholder. After keyholder-3, all three are
   reachable.

**engine (last):**
1. `curl -s https://playzeek.pages.dev/api/v1/health` shows `ok:true`, with
   `uptime_secs` reset.
2. `railway ssh --service engine -- sha256sum /app/zeek-engine` shows
   `5b9096e3…bf8c`.
3. Its logs show `ingested` and no `ERROR`.
4. The board's round counter advances.
5. The next pending round settles normally ("settlement plan stored").
6. `/v1/vault/ledger` shows shortfall 0.
7. https://playzeek.pages.dev shows the live field (not Replay).

**If a check fails:** roll that service back at once with its one command, and
leave the others as they are. A mixed fleet is safe, as explained above.

## After the four services

- Optional: rebuild the wasm (`pnpm build:wasm`) and redeploy the site
  (`scripts/deploy-web.sh`), so the browser verifier skips crafted outputs too.
- Run `cargo run -p zeek-verify --features cli -- round <recent round>
  --engine https://playzeek.pages.dev/api --lightwalletd https://testnet.zec.rocks:443`.
  All five checks should pass.
