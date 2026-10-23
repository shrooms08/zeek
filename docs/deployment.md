# Public deployment

Since 2026-10-23 the testnet runs on **Railway** (Hobby plan, EU West /
Amsterdam). The site stays on Cloudflare Pages (free plan). Mainnet stays
offline on the operator's Mac; nothing of it was moved. The Mac setup below
remains as the fallback (see [Rollback](#rollback-to-the-mac)).

```
browser ── https://playzeek.pages.dev ───────────────────────────────────────┐
           static app (dist/_headers from vite.config.ts: COOP/COEP, CSP)    │
           calls the engine and the relay directly (CORS, WebSocket):        │
             https://engine-production-445c.up.railway.app   /v1/*, stream   │
             https://relay-production-9648.up.railway.app    /v1/chain/*     │
           Pages Functions /share/* (edge-cached, stable links), and /api/*, │
           /relay/* for older cached copies of the app → the same services  │
                   ▼                                                         │
Railway project zeek-testnet (one operator, the Zeek team)                   │
  engine       public domain; volume engine-data (/data)                     │
  relay        public domain; no volume (it keeps no state)                  │
  keyholder-1  no domain: private network only, keyholder-1.railway.internal │
  keyholder-2  "                                                             │
  keyholder-3  "                                                             │
─────────────────────────────────────────────────────────────────────────────┘
```

## Services

| Service | Reachable | State | Runs |
| --- | --- | --- | --- |
| engine | public HTTPS (direct mode), its local API on 127.0.0.1 inside the container | `engine-data`: coordinator identity, vault file, payout sealing key, proxy secret, engine and vault-wallet databases | `zeek-engine` |
| relay | public HTTPS | none | `zeek-verify relay` |
| keyholder-1..3 | Railway's private network only (port 910N) | `keyholder-N-data`: that keyholder's key share, chain and signing databases | `zeek-keyholder serve` |

- **Separate services, one operator.** Each runs in its own container with its own volume, but all five are in one Railway project under one account (2FA on). They are not separate machines or separate operators.
- **No secret in a variable.** Service variables hold only public settings (`scripts/railway-setup.sh`). Secrets live in the volumes, put there once by the cut-over (below).
- **Never starts empty.** `deploy/railway/entrypoint.sh` waits for `/data/.imported`; then the binary runs as uid 10001 with no capabilities (`setpriv`; Railway mounts volumes as root).
- **DKG is off** on the hosted keyholders (`"dkg": false`): their DKG round-2 packages would travel signed but not encrypted (security review M5). No key ceremony ever runs on Railway.
- **Images:** the binaries are built on the Mac for Linux (`cargo zigbuild`), and `scripts/railway-deploy.sh` prints each one's full sha256. Railway's builder only copies them and checks `SHA256SUMS`, so the running binary is exactly the printed one.

## Rate limits behind Railway's edge

The engine and the relay count each client by the address in
`ZEEK_CLIENT_IP_HEADER=X-Real-IP`. Measured with a header-echo service:
Railway's edge rewrites `X-Forwarded-For` as `client, edge proxy` (the last
entry is a different proxy almost every request, so counting by it limits
nothing) and overwrites `X-Real-IP` with the client's address whatever the
client sent. With `X-Real-IP`, 400 requests carrying forged values were
still limited by the real address (62 × 429). Requests from the site's own
proxy still carry the proxy secret and name their client.

## Commands

| What | Command |
| --- | --- |
| One-time Railway setup (project, services, volumes, variables, domains) | `scripts/railway-setup.sh` |
| Build for Linux and deploy services | `scripts/railway-deploy.sh [SERVICE...]` |
| Deploy the site (Railway mode when `.zeek-deploy` names `ENGINE_ORIGIN`) | `scripts/deploy-web.sh` |
| What is running, and whether the vault covers the Cache | `scripts/zeek-status.sh` |
| Daily engine database backup to this Mac (kept 7) | `scripts/railway-backup.sh --install` |
| Logs | `(cd data/railway && railway logs --service SERVICE)` |
| Stop a stateful service without losing reach (`railway ssh` still works) | `railway variable set ZEEK_HOLD=1 --service SERVICE` (delete it to resume) |

## The cut-over (2026-10-23)

1. `scripts/zeek-down.sh` at a round boundary (11:40 UTC): nothing owed or
   being signed; the engine and all three keyholders at block 4532311.
2. `sudo scripts/railway-export.sh` (the operator): stopped the four testnet
   daemons and disabled them at boot (a stale copy started by a reboot would
   pay rounds a second time), then exported each service's `secrets/` and
   `data/` with a manifest of every file's sha256:

   | Export | Files | sha256 of the tar |
   | --- | --- | --- |
   | engine | 8 | `57419511bc1a60fda2e9cc55f29c007d9726cdfb335c8997b1cba507526dd966` |
   | keyholder-1 | 9 | `7b72fd1854f5826646dd2978712a1bf0b4ada956e972bbc5c583fc7857432de7` |
   | keyholder-2 | 8 | `42ae4c68ed8fe30459d01b800dae564ba3c23aec78a6e7383815f39a61fb3304` |
   | keyholder-3 | 8 | `4a85bb9fa8954b8abd9170eee5a3c4b80938f8c08461a3927d8ff61b87fee3ee` |

3. `scripts/railway-migrate.sh`: each tar to its own volume over Railway's
   API, checked in the container against its sha256 and every file against
   the manifest, then the local copy deleted. The Mac's originals stay in
   `/Library/Application Support/Zeek/testnet` as the backup.
4. Every service resumed from its stored height (4532312) without a rescan;
   3 of 3 keyholders reachable from the engine; the vault ledger unchanged.
5. The site was switched to Railway mode after the operator approved.

Only `/Library/Application Support/Zeek/testnet` was read; the mainnet
folders and keys were never touched.

## Backups

Railway's Hobby plan has no volume backups.

- **Key shares** never change: the Mac's copies are their backup.
- **The engine's databases** are copied daily (`scripts/railway-backup.sh`):
  `sqlite3 .backup` in the container (consistent while the engine runs) with
  an integrity check, downloaded to `data/backups/railway-engine/STAMP`
  (yours, mode 600) and checked against `SHA256SUMS`; the newest 7 are kept.
  Database files only, no secrets.

## The vault ledger

`GET /v1/vault/ledger` reads the vault wallet, which the engine syncs every
5 minutes (and before each payout). Before 2026-10-23 it synced only at
payouts, so between them the ledger set a stale vault balance against a
Cache counted to the tip: on 2026-10-23 it showed a 0.02405 ZEC
"shortfall" that the chain audit (`zeek-engine audit`) proved was 0 (vault
23,727,750 = Cache 20,032,750 + float 3,695,000; the wallet was read at
block 4528705, before 6,100,000 zat of digs). The status page's **Vault**
row and `scripts/zeek-status.sh` warn whenever the vault would not cover
the Deep Cache and what is owed, or when the reading is more than 30 blocks
old.

## Cost and limits

Measured on Railway (2026-10-23, an hour after the cut-over): engine 63 MB,
keyholders 26–39 MB each, relay 2 MB of memory; CPU about 0.003 vCPU in
total. At Railway's prices (RAM $10/GB-month, vCPU $20/month, volumes
$0.15/GB-month, egress $0.05/GB) that is about $1.70 a month, inside the
$5 the Hobby plan includes. The workspace has an email alert at $5 and a
**hard limit at $10** (Railway's minimum): above it Railway stops every
service. The limit covers the whole workspace, other projects included.

## Uptime and Replay

Railway had a major outage from 2026-10-21 07:24 UTC (Hobby deploys paused
until 2026-10-22 21:59 UTC); running services were not the problem then, but
it is a single provider. When the engine does not answer, the site plays
back saved rounds (Replay), and share links keep loading from the edge
cache. The status page shows each service and the vault.

## Rollback to the Mac

Two engines with the same vault must never run at once, and the Mac's
copies must never start after Railway has paid a round (they would pay it
again).

- **Before Railway paid any round:** remove the Railway deployments, then
  `sudo scripts/railway-restore.sh --railway-stopped` (re-enables and starts
  the four daemons), `scripts/zeek-up.sh`, and deploy the site in Mac mode
  (remove `ENGINE_ORIGIN`/`RELAY_ORIGIN` from `apps/web/.zeek-deploy` and the
  `[vars]` from `apps/web/wrangler.toml`, then `scripts/deploy-web.sh`).
- **After it did:** `scripts/railway-pull.sh` holds each service and
  downloads its state with checksums; then
  `sudo scripts/railway-restore.sh --railway-stopped --from data/rollback`
  sets the Mac's frozen copies aside, installs Railway's state into the
  protected folders and starts the daemons; then as above.

Both paths were dry-run on a throwaway Railway service (import, hold, pack,
download; checksums and a byte-identical round trip).

## Free-tier limits that still apply (Cloudflare)

With the app calling Railway directly, only `/share/*` (and older cached
copies of the app) run Pages Functions, so the 100,000 Functions requests a
day no longer limit play. Static files are free.

# On the Mac (fallback)

How the testnet ran from Phase 8 until the cut-over, and how it runs again
after a rollback.

```
browser ── https://<project>.pages.dev ─────────────────────────────────────┐
           static app (public/_headers: COOP/COEP, CSP)                     │
           Pages Functions  /api/* → engine   /share/* → engine             │
                            /relay/* → second-source relay                  │
           (functions/_lib/proxy.ts: route allow-list, proxy secret,        │
            client address, own COOP/COEP/CORP, WebSocket passthrough)      │
                   │ reads the current tunnel hosts from Workers KV         │
                   ▼                                                        │
https://<random>.trycloudflare.com  (cloudflared quick tunnels, this Mac)   │
                   ▼                                                        │
engine public listener 127.0.0.1:8788 (refuses requests without the proxy   │
secret; rate limits per client); the engine runs as its own macOS user      │
relay 127.0.0.1:8790 (read-only chain routes)                               │
keyholders 127.0.0.1:9101-9103: localhost only, never tunneled; each runs   │
as its own macOS user (scripts/isolate-services.sh)                        │
──────────────────────────────────────────────────────────────────────────── ┘
```

### Why links stay stable

A quick tunnel gets a new random hostname every time `cloudflared` starts.
Nothing public points at it:

- Every public link (the app, `/share/...` cards, verified-win links) is on
  the Pages origin.
- The Pages Functions read the current tunnel hostnames from Workers KV
  (keys `engine` and `relay`).
- The `dev.zeek.publish` agent writes them to KV whenever they change, for
  example after a crash restart or a reboot.
- A KV change can take up to a minute to reach every Cloudflare location, so
  for that minute after a restart the site may report the engine offline.
- The share pages' meta tags use `ZEEK_PUBLIC_URL`, which `zeek-up.sh` sets
  to the Pages origin.

### Free-tier limits that apply

Verified on 2026-10-15 in Cloudflare's documentation.

| Service | Limit | Effect |
| --- | --- | --- |
| Pages | 500 builds/month; 20,000 files, 25 MiB each | One deploy per release |
| Pages Functions (Workers Free) | 100,000 requests/day (reset at midnight UTC), 10 ms CPU per request | Every `/api`, `/share`, `/relay` call counts, edge-cache hits included; static files are free (`public/_routes.json` keeps them off Functions). A live WebSocket is one request; its messages are free. |
| Workers KV | 100,000 reads/day, 1,000 writes/day, 1 write/s per key, about 60 s propagation | The proxy caches reads for 10 s per isolate. The publisher writes only when a hostname changes. |
| Quick tunnels | 200 requests in flight, no SSE, no uptime guarantee, new hostname per start | Fine for a demo; a named tunnel would need a domain |

#### Staying inside the quota

What one open tab costs is measured in `docs/examples/phase8b/README.md`.

- **The app** asks for little: one live stream, no refetch on focus, the
  board pushed by the stream, round-event refetches coalesced, the stream
  closed after a minute in a hidden tab, no polling while the engine is
  offline (one health probe backing off to 2 minutes).
- **Cacheable answers** (share pages and cards, final proof bundles, stored
  win proofs) carry `Cache-Control: public`, so browsers and unfurlers
  keep them, and the proxy keeps them in Cloudflare's edge cache. The edge
  cache spares the Mac and keeps those links working while it is off; a
  hit still counts as a Functions request.
- **When the allowance is used up** the app says "Zeek is very busy, try
  again shortly." and shows Replay mode. That needs the project's
  **Settings → Runtime → Fail open / closed** set to **Fail open**: static
  files keep being served (fail closed would serve an error page instead
  of the app).

### Commands

| What | Command |
| --- | --- |
| One-time Cloudflare setup (after `wrangler login`) | `scripts/cloudflare-setup.sh` |
| One-time: engine and keyholders as their own users | `sudo scripts/isolate-services.sh` |
| Install a rebuilt engine or keyholder binary | `sudo scripts/isolate-services.sh --update-binary NAME=SHA256` (zeek-up prints the command) |
| Deploy the web app | `scripts/deploy-web.sh` |
| Start everything and check it end to end | `scripts/zeek-up.sh` |
| Stop everything except the daemons | `scripts/zeek-down.sh` |
| Stop the engine and keyholder daemons too | `scripts/zeek-down.sh --daemons` |
| What is running | `scripts/zeek-status.sh` |
| Optional demo digger | `scripts/demo-digger.sh on`, then `scripts/zeek-up.sh` |

`zeek-up.sh` is safe to run again at any time.

1. It rebuilds what changed.
2. It (re)installs the launchd agents and restarts them: caffeinate, the
   relay, two tunnels, the publisher and, before `isolate-services.sh`, the
   engine and the keyholders. After it, those two run as system daemons
   under their own users; zeek-up only checks them and tells you when a
   rebuilt binary is waiting to be installed.
3. It waits for every local health check, for both tunnel hostnames to be
   published, and for the public site to reach the engine, the relay and the
   live stream.

### Logs

- Engine and keyholders: `/Library/Logs/Zeek/testnet-engine/` and
  `/Library/Logs/Zeek/testnet-keyholder-N/`, rotated daily and kept 14 days
  (before `isolate-services.sh`: `data/logs/engine/` and
  `data/logs/keyholder-N/`).
- Tunnels, relay, publisher, caffeinate: `data/logs/*.log`. The publisher
  truncates these when they pass 10 MB.

### After a power cut or reboot

- **The engine and the keyholders** run as system daemons and start at
  boot.
- **Everything else** (the tunnels, the publisher, the relay, caffeinate)
  consists of agents of your user, and starts when you log in.
  1. Turn the Mac on and log in. FileVault prevents automatic login.
  2. Run `scripts/zeek-status.sh`. Every service should be `running`, and
     the public checks `ok` within about two minutes.
  3. If something is not, run `scripts/zeek-up.sh`.
- To have the Mac power on by itself after a power failure, run
  `sudo pmset -a autorestart 1` once.
- While the services run, caffeinate stops idle and system sleep on mains
  power. On battery the Mac may still sleep.

### Uptime during judging

#### No automatic macOS update restarts

An automatic macOS update restarts the Mac, and everything but the two
daemons then waits for a login. For the judging period:

1. Open **System Settings → General → Software Update**.
2. Click the **ⓘ** next to **Automatic updates**.
3. Turn off **Install macOS updates** and **Install Security Responses and
   system files**. (Downloading updates may stay on; installing is what
   restarts.)
4. Click **Done**.

Check from Terminal (both should print `0`; a missing key means the
default, which is on):

```sh
defaults read /Library/Preferences/com.apple.SoftwareUpdate AutomaticallyInstallMacOSUpdates
defaults read /Library/Preferences/com.apple.SoftwareUpdate CriticalUpdateInstall
```

Turn them back on after judging.

#### Services return after a reboot and one login

1. Restart the Mac (Apple menu → Restart) and log in to your account.
2. Wait two minutes: the daemons start at boot, the agents (tunnels,
   relay, publisher, caffeinate) at login, and the publisher writes the
   new tunnel addresses to KV.
3. Run `scripts/zeek-status.sh`. Every service should be `running` and the
   public checks `ok`; links on the site keep working because they never
   name a tunnel.
4. If something is not, run `scripts/zeek-up.sh`.

`sudo pmset -a autorestart 1` makes the Mac power on by itself after a
power cut; it still needs the one login (FileVault).

#### What the site does while the Mac is off

- **Works:** the site and every screen load (static files). The field shows
  Replay mode: the last settled rounds saved at the last deploy play back,
  clearly labelled, and each can be checked in the browser as far as no
  chain data is needed (the beacon against drand's public relays, the
  settlement recompute, the payout openings). Share pages and cards that
  were opened recently in a Cloudflare location keep loading there from its
  edge cache.
- **Does not work:** anything live. No digs, no new rounds, no Campfire, no
  Pocket creation, sync or sending (the Pocket reads the chain through the
  engine's relay), no checks that need chain data, no status of the
  services other than "offline".
- **Coming back:** when the engine answers again, open pages switch from
  Replay to live by themselves within about two minutes.

### The public status page

`/status` shows, refreshed every 30 s:

- the site, the engine (network, last block and its age, catching up), the
  relay;
- each keyholder: reachable, or unknown when the engine has not checked it
  for 5 minutes;
- each lightwalletd server.

When the engine or the tunnel is down, the proxy answers 503 or 502 with
"offline", and the page says the engine does not answer.

### Who can read what

`scripts/isolate-services.sh` (run once with sudo) gives the engine and each
keyholder its own macOS user.

- **Each keyholder's directory** holds its key share, chain database and
  config. Only that keyholder's user can read it.
- **The engine's directory** holds the coordinator identity, the vault's
  public file, the payout sealing key, the vault wallet and the proxy
  secret. Only the engine's user can read it.
- **The engine's user cannot:**
  - read any key share;
  - write this repository;
  - use your sudo rights or your Cloudflare login.

The daemons run root-owned copies of the binaries in
`/Library/Application Support/Zeek/bin`. The script only creates and changes
its own folders (`/Library/Application Support/Zeek/...` and
`/Library/Logs/Zeek/...`) and its own plists; it never changes a system
directory. It can be re-run after a failure at any step.

The script refuses to run with uncommitted changes. It refuses symbolic
links in anything it moves, and it never replaces an installed binary or
config without the binary's SHA-256 or `--replace-config`.
