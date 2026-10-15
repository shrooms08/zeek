# Public deployment (zero cost)

The public site is a Cloudflare Pages project on the free plan. The engine,
the three keyholders and the second-source relay run on the operator's Mac.
No payment card is needed anywhere.

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

## Why links stay stable

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

## Free-tier limits that apply

Verified on 2026-10-15 in Cloudflare's documentation.

| Service | Limit | Effect |
| --- | --- | --- |
| Pages | 500 builds/month; 20,000 files, 25 MiB each | One deploy per release |
| Pages Functions (Workers Free) | 100,000 requests/day, 10 ms CPU per request | Every `/api`, `/share`, `/relay` call counts; static files are free. A live WebSocket is one request. |
| Workers KV | 100,000 reads/day, 1,000 writes/day, 1 write/s per key, about 60 s propagation | The proxy caches reads for 10 s per isolate. The publisher writes only when a hostname changes. |
| Quick tunnels | 200 requests in flight, no SSE, no uptime guarantee, new hostname per start | Fine for a demo; a named tunnel would need a domain |

## Commands

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

## Logs

- Engine and keyholders: `/Library/Logs/Zeek/testnet-engine/` and
  `/Library/Logs/Zeek/testnet-keyholder-N/`, rotated daily and kept 14 days
  (before `isolate-services.sh`: `data/logs/engine/` and
  `data/logs/keyholder-N/`).
- Tunnels, relay, publisher, caffeinate: `data/logs/*.log`. The publisher
  truncates these when they pass 10 MB.

## After a power cut or reboot

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

## The public status page

`/status` shows, refreshed every 15 s:

- the site, the engine (network, last block and its age, catching up), the
  relay;
- each keyholder: reachable, or unknown when the engine has not checked it
  for 5 minutes;
- each lightwalletd server.

When the engine or the tunnel is down, the proxy answers 503 or 502 with
"offline", and the page says the engine does not answer.

## Who can read what

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

The script refuses to run with uncommitted changes. It refuses symbolic
links in anything it moves, and it never replaces an installed binary or
config without the binary's SHA-256 or `--replace-config`.
