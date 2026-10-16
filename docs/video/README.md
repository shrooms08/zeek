# Demo video assets

Not part of the app. Every clip is 1920×1080, H.264.

| Clip | What | Made with |
| --- | --- | --- |
| `clips/public-board.mp4` (15 s) | A generic public betting board, for contrast: a 5×5 grid, made-up wallets and amounts scrolling, one wallet followed across rounds. Labelled "Illustration". No real project's names, logos, colours or layout. | `public-board/index.html` (standalone, open with any static server), recorded by `node apps/web/video/record-public-board.mjs` |
| `clips/refusal.mp4`, `clips/refusal.gif` | The rogue-coordinator refusal, as terminal output. | See below |
| `clips/reveal-win.mp4` | The Phase 7b reveal of a win (round 205). | Re-export of `docs/examples/phase7b/reveal-win.webm` (1440×900), scaled to 1080 high and padded with the app's background |
| `clips/dig-landing.mp4` | The Phase 7b landing of a dig from another wallet. | Re-export of `docs/examples/phase7b/dig-landing.webm`, the same way |

## The refusal clip

`node apps/web/video/capture-terminal.mjs refusal` runs real commands and
records their output as it arrives (`terminal/refusal.json`).
`node apps/web/video/record-terminal.mjs refusal` replays that capture in
`terminal/player.html`: commands are typed out, output keeps its timing,
and waits longer than 1.5 s are shortened. Then it exports the mp4 and gif.

What it shows:

1. **The live run's logs.** These are from the Phase 4b run on Zcash testnet
   (round 17, 2026-10-14; `docs/examples/phase4b/rogue-coordinator-refusals.log`).
   - A coordinator built with `--features rogue-demo` redirected a payout.
   - All three keyholders logged
     `REFUSED to sign … reason=output 1: recipient does not match the sealed payout receiver for round 17 (amount matches)`.
   - The engine ended with `not enough keyholders approved the payout: 0 of 2`.
2. **The same checks, run at recording time:** the keyholder's payout
   verification tests against tampered payouts (`cargo test -p zeek-keyholder --test verify_pczt`).

The run was not repeated live for the clip. The live keyholders and the
coordinator's identity now sit in the engine's and keyholders' own protected
directories (`scripts/isolate-services.sh`), and a second coordinator would
compete with the live one for the testnet vault. The log above is the real
output of the one live run.
