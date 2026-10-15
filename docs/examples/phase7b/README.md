# Phase 7b results (Zcash testnet, Ironwood)

The live suite ran against the redesigned app (`apps/web/e2e/live/*`) on
2026-10-15, with the live engine, the 2-of-3 keyholders and the new mempool
watcher.

## Live rounds

| Step | Round | Result |
| --- | --- | --- |
| 01 Pocket unlock and sync | | The Pocket (funded in Phase 7a) unlocked and synced; its history shows the funding memo |
| 02 Pocket dig, 2 tiles with a message | 202 | txid b7ab47bfc34a2cadee1c392522ff988b4df0efd20827753956746b6dd9b0776a. The field showed the dig **landing** on its tiles before it was mined, then the deeper holes. The message reached Campfire as R202 · TILE 7 19. |
| 06 Other wallet (bob, `zeek-dig pay-uri`) | 205 | The waiting row showed **"seen, landing" 0.3 s** after `pay-uri` returned from broadcasting, then confirmed once mined |
| 03 Reveal | 202 | Treasure tile 13, no winner; the result card matched the settlement |
| 04 Past rounds and Check a round | 202 | Every applicable check passed; cross-check "unavailable" with no second source, "pass" with the second relay |
| 05 Guaranteed win | 205 | Details below |

Step 05 in detail:

- **Dig:** 25 tiles in 5 batched payments.
  - 46bb808af7d362469a365110599033a642fc1d9be9acab0b1c2f716ca2529f41
  - 05ed62357b233919ed3a6cb3273686db3362396fb356d60c5864f1a31e210e8c
  - e7dcaa3ed99424ab3048851ff5f9c9eddd22fc198768d61d7ec586d97433cd86
  - 603b73ddda3e4b36d794d6366282712a3e293063e40f5bc16e16f8d066527792
  - acf114e0c1e897de366c688aaa66847efda67ab9df267a62aa3a759f2b56ce82
- **Settlement:** treasure tile 4. The pot was 0.0247: the Pocket's 25 tiles plus bob's tile, minus 26 dig fees.
- **Payout:** 0.023415 to the Pocket.
  - txid 13c08974a451017d169c66826c5260e86804a61954f5e3990204cabf3c5af4c1
- **Proof:** id dbd7adfbf2bdf06a966bac96953caac2.
  - Its share URL redirected to the Verified win page.
  - `/share/win/{id}` serves the OG and Twitter meta and a 1200×675 PNG.

## Performance (Chromium, 4× CPU throttle)

`perf.json` is written by `e2e/perf.spec.ts`.

| Scenario | fps | p95 frame | Worst frame | Long tasks > 50 ms |
| --- | --- | --- | --- | --- |
| Idle field, 10 s (island, boiling lines, fireflies) | 60 | 16.7 ms | 16.7 ms | 0 |
| Dig landing, 12 s (fixture: one tile one stage deeper) | 59.7 | 16.7 ms | 50 ms | 0 |
| Full win reveal, 5 s (round 188) | 59.4 | 16.7 ms | 50 ms | 0 |

The dig-landing scenario needs a depth to grow on demand, so it replays the
board with one tile one stage deeper. This is a labelled test fixture.

Bundle sizes:

- **Initial JS:** 165.5 KB gzipped (React, router, query, Framer Motion, the
  art kit and the field).
- **CSS:** 23.3 KB gzipped.
- **Lazy:** every other screen is its own chunk of 1.4–5.4 KB gzipped. The
  Pocket wasm (2.7 MB gzipped) loads only when a wallet feature starts.

## Files here

- `compare/`: each design frame beside the live app, at 1440 and 390. The
  Deep Cache reveal is labelled FIXTURE: no Deep Cache round has happened on
  testnet yet (1 in 625 per round).
- `field-interactions.webm`, `dig-landing.webm`, `reveal-win.webm`: screen
  captures for the demo.
- `logo-options.png`: the three "eek" slants, large and at 32 px, on dark and
  light.
- `x-avatar.png`
- `live/`: screenshots taken during the live steps.
