# Phase 8b results

Measured 2026-10-15 against https://playzeek.pages.dev (testnet), unless
marked local.

## Cloudflare quota: requests per open tab

Measured with `apps/web/e2e/quota.spec.ts` (Chromium). It counts requests
that run Pages Functions (`/api`, `/share`, `/relay` and the live
WebSocket) for 20 minutes visible, then 20 minutes hidden (hidden is
emulated with `document.visibilityState`). The first page load is not
counted in the tab figures. Static files run no Function
(`public/_routes.json`).

| | Before | After |
| --- | --- | --- |
| Visible tab | 13 in 20 min = **39/hour** (6 board + 6 pending refetches, 1 WebSocket) | 3 in 20 min = **9/hour** (1 pending refetch, 2 WebSocket connections) |
| Hidden tab | 5 in 20 min = **15/hour** (the stream stayed open and kept refetching) | 1 in 20 min = **3/hour** (one refetch in the first minute, then the stream closes) |
| One share-link view (first visit) | 9 | 9 |
| One tab all day | about 940 visible, 360 hidden | about 220 visible, under 80 hidden |

- **Target:** under 2,000 requests a day for one tab. Met before and after;
  after the change a tab costs about a quarter as much.
- **A first share view** costs the same 9 (the share page, then the app's
  config, board, proof, bundle, transaction, block, round view and stream).
  Repeat views of share pages and cards, final bundles and stored proofs
  are served from the browser's cache once the new engine binary sends
  their `Cache-Control`.
- **While the engine is offline** a tab makes one health probe, backing off
  from 15 s to 2 minutes: about 30/hour at most. Before, the stream retried
  every 30 s and the board polled every 10 s, about 480/hour.

## Replay mode during a real outage

`scripts/zeek-down.sh` stopped the tunnels, relay and publisher and
unpublished the tunnel addresses; the engine and keyholder daemons kept
running.

- **The site:** `/` 200, `/api/v1/health` 503, `/replay/index.json` 200.
- **The field** showed Replay in Chromium and WebKit, with "Live play
  resumes when the Zeek engine is back." It played round 251 block by
  block (screenshot `public-engine-down-1440.png`).
- **Verify round 251 here**, in both browsers:

  | Part | Result |
  | --- | --- |
  | Beacon: rules, BLS signature, drand's public copy, the draw | pass |
  | Settlement recomputed | pass |
  | Payout openings against the settlement | pass |
  | Beacon round binding, digs, timing, the mined payout | needs the live engine (no tick) |

- **Coming back:** a page was left open in Replay, and `scripts/zeek-up.sh`
  ran 15 s later (it took 32 s). The page switched to the live field by
  itself 46 s after it first showed Replay, without a reload.
- **Share pages** answered 503 during the outage: nothing was in the edge
  cache yet, because the running engine binary does not send the new
  `Cache-Control`. After it is installed, share pages and cards opened in
  the previous day keep loading from each Cloudflare location's cache.

## Saved rounds

`scripts/snapshot-rounds.mjs` saved every settled round with digs, 19 so
far (rounds 0 to 251), about 464 KB of static JSON.
`round-251.proof.json` and `round-245.proof.json` here are the fixtures for
`crates/zeek-verify/tests/offline.rs` and `crates/zeek-engine/tests/cache.rs`.

## What can be verified with the Mac off

No public source of testnet chain data can be read from a browser on this
site:

- testnet.zec.rocks and zec.rocks answer plain gRPC (not grpc-web), send no
  CORS headers, and return 404 to the CORS preflight;
- the CipherScan API answers 500 to other origins and allows only its own.

drand's relays (api.drand.sh, api2, api3, drand.cloudflare.com) send
`Access-Control-Allow-Origin: *`.

## Screenshots

| File | What |
| --- | --- |
| `chest-verified-win-1440.png`, `chest-reveal-1440.png` | The chest on its tile, over the outline and glow (local) |
| `campfire-hover-1440.png` | The camp's hover light (local) |
| `pro-1440.png`, `pro-390.png` | Pro with ALL and +0.001 (local) |
| `copy-phrase-1440.png` | Copy words and the warning; the words blurred before capture (local) |
| `replay-1440.png`, `replay-390.png`, `replay-verify-1440.png` | Replay with the engine blocked, and its offline checks (local) |
| `public-engine-down-1440.png` | Replay on the public site with the tunnels down |

The visual regression baselines are in
`apps/web/e2e/chest.spec.ts-snapshots/`, covering the verified win, the
reveal and both resvg share cards in Chromium and WebKit.
