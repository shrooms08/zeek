# Phase 8 measured results (testnet public site, mainnet read-only)

Measured 2026-10-15 on the one Mac that runs Zeek. The engine and the three
keyholders run as LaunchDaemons under their own users
(`scripts/isolate-services.sh`), and the site is https://playzeek.pages.dev.
The screenshots in `live/` come from the live suite run against that public
URL (`ZEEK_E2E_APP=https://playzeek.pages.dev`).

## API reads while the chain loop is busy

### Stall test

`crates/zeek-engine/tests/stall.rs` holds the store's writer for a 10.6 s
stall while it reads the API.

| Reads during the stall | p50 | p95 | max |
| --- | --- | --- | --- |
| `/v1/board` and `/v1/rounds` | 0.07 ms | 0.8 ms | 9.5 ms |

The test fails above 50 ms.

### During live payouts

A probe read the engine in a loop with a 250 ms pause between passes. Every
pass read it locally (`127.0.0.1:8787`), and every fourth pass also read it
through the public proxy and tunnel. All responses were 200.

| Payout | Window (UTC) | local `/v1/board` p50 / p95 / max | public `/v1/board` p95 |
| --- | --- | --- | --- |
| Round 247 (18 inputs), old binary | 16:06:28–16:06:44 | 3.3 / 251.5 / 1,919.9 ms | 655 ms |
| Round 251 (25 inputs), chain loop on its own runtime | 16:47:05–16:47:23 | 2.9 / **9.4** / **12.6** ms | 635 ms |

- **Round 247's slow reads.** They fell between "settlement plan stored" and
  "payout transaction planned". In that interval, wallet scanning and
  transaction building ran on the same tokio workers as the API.
- **The fix (commit 7a25e43).** The chain loop now runs on its own two-worker
  runtime. In the five minutes around round 251's payout, local board reads
  had p95 8.5 ms and max 22.8 ms (418 samples).
- **Public reads** cost about 450 ms at p50. Almost all of that is the trip
  through Cloudflare and the quick tunnel, since the engine itself answers in
  milliseconds.

## lightwalletd failover

- **A server that fails fast** (dead port, or a server that closes the
  connection) is skipped, and the next server in the preference order answers
  within about 1 s.
- **A server that accepts the connection and never answers** costs one 45 s
  request timeout and is then cooled down for 30 s. Starting against one, the
  first ingest came 93 s in: the startup probe plus one call, 45 s each.
- **Block hashes** are checked against up to two other servers, and a 1-vs-1
  disagreement is never trusted. A forged hash from one server was outvoted in
  `crates/zeek-vault/tests/network.rs`.
- **On mainnet,** all 8 servers in `docs/lightwalletd-servers.md` were
  healthy. During the readiness run, zec.0xrpc.io failed and the pool moved
  on by itself.

## Mainnet readiness (read-only; digging off, vault address not published)

The mainnet vault was created inside the separated users
(`sudo scripts/isolate-services.sh --mainnet-dkg`), with genesis height
3,518,735.

| Check | Result |
| --- | --- |
| Rounds settled | 0–2, all with nothing to pay |
| Round 0 beacon | drand round 33090937; timing safe with a 1,053 s margin |
| `zeek-verify` against na.zec.rocks | every applicable check passes |
| Block hashes vs Blockchair (earlier read-only run, old vault) | 62 of 62 match |

## Public live suite (testnet)

| Step | Round | Result |
| --- | --- | --- |
| 01 create and fund a Pocket | — | alice-public sent 0.06, txid 39e1fd20934471229c86bea8a20da8cf3ea16b61e1c254d79fab48d0452f3c02 |
| 02 dig with a message | 244 | tiles 6 and 18, txid 3ceb82d6fc8b94fbbd74a7ae53804feb132e6747e007cfd49a10b5eeb3eee21d; landing seen; Campfire shows the message |
| 03 reveal | 244 | result card matches the settlement |
| 04 check a round | 244 | passes with the site's own `/relay` as the second source |
| 05 25 tiles and a win | 247 | the dig stopped after 4 of 5 batches, and the 24 tiles sent included the treasure; the Pocket proved the win (see below) |
| 06 another wallet | 245 | tile 16; landing seen 1.8 s after broadcast |

### Round 247 batches

- 55cfcca40247c05b31da71cd3f263cd30058da9f1e2e6ad493bedb954e47798a
- f7a457ada133dd8c4894ed8957777bab399fa56bb64e1eded55305d2c1d7fea6
- 8df49e4da580005bb4ea05a9b6658931395b5e71a1afdd4d9e8529944003469c
- 8871337adc9c909adc4bc3dd1d54a6259fff1771d8c2d73de8c47e251fbf3ed9

The treasure was tile 8 (index 7), which the Pocket had dug.

- Payout: 8c23a700473c3c19ae7daa80833e2eefd9de53cd5be7aabdac8cfbab44bca9c5
- Signed by keyholders 1 and 2; all three approved.

The rest of step 05 ran against the public URL with this round as the win:

- The Pocket's history shows the treasure.
- Prove-a-win made proof 78ea3288c9ef4fea6c07ec9eeb26c9e0, which the engine
  verified before storing.
- `/share/win/78ea3288c9ef4fea6c07ec9eeb26c9e0` serves OG and Twitter
  `summary_large_image` tags whose image is on the pages origin.
- The Verified win page shows +0.016195 ZEC with all six checks passing
  (`live/verified-win-1440.png`).

### Round 251 (latency re-measure)

Zeek's own test wallets dug all 25 tiles at 0.001 each:

- alice, tiles 0–21: d36724b21ea8d58faf2e2eb5b9c5fb16ba7c6e4dccdc4920ffdeaf3ad4ee7774
- bob, tiles 22–24: 8fda34f6971880e052252860d7ef7d018ace923caf5bb3f9331d698c9387930c

The treasure was tile 9 (index 8), alice's.

| Pot | Rake | Paid net |
| --- | --- | --- |
| 0.02375 | 0.0011875 | 0.0225125 |

Payout: 2bcda19e216a952c1eca44e92d69513efa9e43e8d04120183d9726c44187a51e.

## Live stream and restarts

- **WebSocket soak through the public proxy:** 20 minutes on one connection,
  no disconnects, 49 events (39 board, 5 landing, 2 reveal, 2 settled,
  1 campfire).
- **`scripts/zeek-down.sh`** stops everything else (the system daemons keep
  running) and removes both tunnel addresses from KV. Meanwhile the site loads
  and every engine and relay route answers 503 `offline`.
  - Share pages briefly passed through Cloudflare's own 530 while an edge
    still remembered the old tunnel. Fixed in 5ac0189.
- **`scripts/zeek-up.sh`** started new quick tunnels and published them. These
  all returned 200 at the new hostnames:
  - `/share/round/251` (og tags on the pages origin)
  - `/share/round/251.png`
  - `/share/round/247`
  - `/api/v1/board` and `/api/v1/rounds`
  - `/relay/v1/chain/tip`
  - The live stream
