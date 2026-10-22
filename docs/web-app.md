# Zeek web app (apps/web)

Phase 7a builds the web app's foundation. Every user flow works end to end
against the live testnet engine, with minimal Mono styling. Phase 7b
restyles and animates the same components without changing how they work.

## Run it

```sh
pnpm build:wasm                  # Zeek Pocket wasm (threads), once
pnpm --filter web dev            # http://localhost:5174
pnpm --filter web build && pnpm --filter web preview   # http://localhost:4174
```

`VITE_ZEEK_ENGINE_URL` (default `http://127.0.0.1:8787`) selects the engine.
`VITE_ZEEK_SECOND_SOURCE` optionally pre-fills an independent relay for
Check a round.

The page must be cross-origin isolated: Pocket proves with WebAssembly
threads, which need `SharedArrayBuffer`. `vite dev` and `vite preview` send
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp`. Production hosting sends the
same headers through `public/_headers`. Fonts are self-hosted
(fontsource), so no third-party asset needs CORP. The engine's share PNGs
send `Cross-Origin-Resource-Policy: cross-origin`. The smoke e2e test checks
`crossOriginIsolated`, every response and every font face.

## Layout

| Path                   | What                                                                                                                                                                                                                   |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/styles/index.css` | Design tokens in Tailwind 4's `@theme`: colours, type, spacing, borders, shadow, easings. Also the base component classes.                                                                                             |
| `src/lib/`             | Pure logic: money (bigint zatoshis), dig plan and fees, address checks, time, passphrase, depth stages                                                                                                                 |
| `src/api/`             | Engine client, TanStack Query hooks, one live WebSocket (`LiveProvider`), Pocket provider                                                                                                                              |
| `src/machines/`        | The explicit state machines (see below)                                                                                                                                                                                |
| `src/components/`      | Design-system components: `BigStat`, `DigButton`, `AmountPicker`, `Panel`, `Modal`, `StepList`, `BatchProgress`, `QrCard`, `CheckRow`, `MessageRow`, `NetBadge`, `RevealLine`, `MenuRow`, `ResultCard`, `Field`/`Tile` |
| `src/screens/`         | One file per screen, lazily loaded                                                                                                                                                                                     |
| `e2e/`                 | Playwright: `smoke.spec.ts`, `live/0N-*.spec.ts`, `screenshots.spec.ts`                                                                                                                                                |

## State machines

Each flow with more than two states is a typed reducer: a discriminated
union of states, a union of events, and a pure transition function. The
files are `machines/dig.ts`, `reveal.ts`, `wallet.ts`, `check.ts` and
`otherWallet.ts`.

**Why reducers and not XState:**

- They are plain TypeScript, so the compiler checks that every state
  handles every event.
- They are unit-tested without rendering (`machines.test.ts`).
- They add no runtime dependency or DSL that 7b would have to learn.

The flows are small: the largest has six states. Side effects (Pocket
calls, timers) stay in the screen and dispatch events, so the machine stays
pure.

- **Dig:** `idle → confirm → sealing → done | partial | error`.
  - The `sealing` state tracks the planned batches, the current batch, the
    step (packing, sealing privately, sending) and a wait. The wait is
    either the random privacy gap between separate payments or the previous
    batch's change confirming.
  - `partial` lists exactly which batches were sent (never resent) and which
    tiles were not.
- **Reveal:** `closed → draw → shake → burst → result`, timed by the design's
  motion tokens. With reduced motion it skips straight to the result.
- **Wallet:** `unknown → none → creating → backup`, or
  `locked ↔ unlocking → unlocked ↔ syncing`. Unlocking is throttled: after 3
  failed tries it waits 30 s.
- **Check:** runs the five verifier checks, then reveals them one by one
  about 600 ms apart, as the design does.
- **Other wallet:** `address → scan → waiting → confirmed`. It watches the
  round's memo digests (SHA-256 of each 512-byte memo) for the exact memos
  it built.
  - It can only see a payment once it is mined, because there is no mempool
    watch.
  - Payments mined after the round closes are marked late (refunded).

## Live data

The app opens one WebSocket to `/v1/stream`:

- It reconnects with exponential backoff: 1 s doubling to 30 s, then 1, 2
  and 5 minutes while the engine stays away, with ±25% jitter.
- `board` events write straight into the query cache; the board is never
  refetched for a round event (the stream pushes it every block).
- Round and Campfire events invalidate the affected queries, coalesced over
  1.5 s (they come in bursts).
- A tab hidden for a minute closes the stream; on return it reconnects at
  once and refreshes what it missed. Nothing polls while hidden, and
  nothing refetches on focus.
- While the socket is down, the top bar shows "Reconnecting…" and queries
  fall back to polling, except while the engine is offline (see Replay).

On the public site every request through `/api` is one of the free plan's
100,000 Pages Functions requests a day, so the app asks for as little as it
can (measured in `docs/examples/phase8b/README.md`).

## Replay mode

When the engine does not answer (or the site's free requests are used up,
"Zeek is very busy, try again shortly."), the field plays back real past
rounds instead of a dead end, labelled **Replay**, with "Live play resumes
when the Zeek engine is back."

- **Data:** `scripts/snapshot-rounds.mjs` (run by `deploy-web.sh`) saves
  the last settled rounds into the build as static files: each proof
  bundle and its replay from `zeek-verify replay` (depth stages per block,
  the draw and the payout; never amounts per tile).
- **Playback:** the field fills block by block, the round closes, the
  reveal plays with the usual stage timings, then the result and payout.
  With reduced motion it shows each round's end state.
- **Coming back:** one probe of `/v1/health`, every 15 s doubling to 2
  minutes and paused while hidden; when it answers, the field goes live.
- **Verify this round here** runs `zeek_verify::offline` in the browser:

  | Check                                        | Without the engine                                                                         |
  | -------------------------------------------- | ------------------------------------------------------------------------------------------ |
  | Beacon                                       | rules, BLS signature, drand's own public copy (its public relays allow browsers), the draw |
  | Beacon round bound by the first block's time | needs the live engine                                                                      |
  | Digs, Timing                                 | need the live engine                                                                       |
  | Settlement                                   | recomputed in full                                                                         |
  | Payout                                       | openings against the settlement; the mined transaction needs the live engine               |

  A part that needs chain data shows "Needs the live engine" and never a
  tick. No public testnet chain source can be read from a browser on this
  site: lightwalletd speaks gRPC without grpc-web or CORS, and CipherScan's
  API only allows its own origin.

Every number on screen comes from the engine or the Pocket.

## Money

Amounts are `bigint` zatoshis everywhere. They are parsed from text without
floats, and formatting truncates. Main screens show up to 4 decimals;
details (totals, fees, payouts, history) show all 8.

## Lite and Pro

The dig panel has a Lite / Pro switch, remembered in this browser, with
one line under it: Lite "One tile, quick dig.", Pro "Many tiles, auto-dig,
your history." The rules are in `src/lib/digMode.ts`.

- **Lite** (the default) digs one tile. Tapping another tile moves the
  pick, Auto-pick picks one quiet tile, and "Dig separately" is hidden: one
  tile is one payment. The amount presets and the Campfire message stay.
  Switching from Pro with several tiles keeps the first one and says "Lite
  digs one tile. Switch to Pro for more."
- **Pro** picks many tiles and adds ALL and Clear; Dig separately;
  +0.001, +0.01, +0.1 and MAX (the Pocket's
  spendable balance minus fees, over the picked tiles); the exact total
  line; and **Your digs**, this browser's own record with each round's
  public result and a Verify link. "Pro shows more about you. It never
  shows more about anyone else."
- **Auto-dig** (Zeek Pocket only): after the passphrase, the Pocket digs
  the same selection once in each new round for N rounds while the tab is
  open, with a counter, the spend cap and a Stop button. It stops before a
  round the balance would not cover, before passing the cap, after a whole
  round hidden, and on any error (`src/lib/autoDig.ts`). Other wallets get
  Pro's selection and the payment request, not auto-dig.

## Privacy

- The seed never leaves the Pocket worker. The UI shows the 24 words only on
  the backup screen. Screenshots blur them. The backup screen's Copy words
  button puts them on the clipboard with the warning "Anyone who gets these
  words gets your coins. Clipboards can be read by other apps."; after 60 s
  the clipboard is cleared if it still holds them (best effort, never a
  permission prompt).
- No screen shows an address except the user's own deposit address, on
  their own wallet screen.
- The payout address pasted for another wallet is sealed into memos and is
  never shown back.
- Campfire rows show round, tile or Shout, time and text.
- Nothing is logged to the console.

## Accessibility

- **Field:**
  - The field is one tab stop (`role="application"`).
  - Keys: arrows move, Space/Enter selects, A auto-picks, D digs, Esc
    clears.
  - A live region reads e.g. "Tile 7, dug a lot, selected".
  - A screen-reader list describes all 25 tiles.
  - Depth is never shown by colour alone: hole size, wall height and floor
    all change.
- **General:**
  - Focus is always visible.
  - Modals take focus and close with Esc.
  - Reduced motion removes shake and particles.
  - Hit targets are at least 44 px.

## How 7b applies the design

7b changes the look and leaves behaviour alone:

- Components keep their names, props and `data-testid`s. 7b replaces their
  markup and styles: the procedural illustration set, Zeb, the mine and
  pictogram bubbles.
- The Field's SVG already has the named layers (ground, shadow, tiles,
  props, effects, overlay). 7b draws into `props` and `effects` and
  replaces `Tile`'s simple hole with the hatched illustration per depth
  stage.
- Motion goes into the reveal stages and component transitions, using the
  existing easing tokens and the reveal machine's stage timings.
- The machines, API layer, money and e2e tests don't change. The live suite
  is the regression check for 7b.

## Phase 7b: Zeek Mono applied

### Art

The design's art is generated at runtime by `zeek-art.js` (identical in
all six design files). It is ported to typed modules under `src/art/`:

| Module            | Contents                                                                                            |
| ----------------- | --------------------------------------------------------------------------------------------------- |
| `ink.ts`          | palette, iso projection, seeded RNG, defs, boil variants                                            |
| `field.ts`        | slab, tiles, holes, burst, puff, cracks, star                                                       |
| `props.ts`        | pines, rocks, lamps, rails and cart, chest, mine stages 1–5 and eruption, Zeb's six poses, campfire |
| `world.ts`        | the prop table                                                                                      |
| `icons.ts`        | the 57 pictograms, verbatim                                                                         |
| `logoGeometry.ts` | the Z mark                                                                                          |

React wrappers live in `InkSvg.tsx`, `Characters.tsx` and `Logo.tsx`.

The generators return markup built only from numbers and constants (no
user text). Two scripts reuse them:

- `scripts/share-art.mjs` writes the engine's share-card art.
- `scripts/brand-assets.mjs` writes the logo comparison, favicons, app
  icons and the X avatar.

Fixes from the design file:

- The slab's shadow is a soft iso diamond.
- Prop shadows are 2:1 footprints.
- Non-finite coordinates throw instead of drawing at 0.
- Markup is valid XML, so resvg can render it.
- Pins never cover the treasure tile.
- Units sit on the numeral's baseline.

### Motion architecture

Every animation touches only transform, opacity or stroke-dashoffset.

| Technique                            | Used for                                                                                                                                                                                                  |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CSS keyframes                        | Loops (flicker, blink, sway, nod, flame frames, sparks, fireflies, stars, hourglass, seeking outline) and one-shots (pin drop, puff with steps(6), tremble, burst, chest, star, eruption, prop reactions) |
| `stroke-dashoffset` (`pathLength=1`) | Hole rim, cracks, treasure split, seal ring, fuse, ink check and X, ink flight                                                                                                                            |
| Framer Motion                        | Modals, menu, sheet, result card, count-ups (integer zatoshis), pulses, stamps, note unfold, shout spark, bubbles                                                                                         |
| Line-variant swap                    | Boiling lines                                                                                                                                                                                             |

Boiling lines use three seeded vertex-jitter variants on stacked SVGs,
each its own compositor layer, swapped at 9 fps. This replaces the
design's `feTurbulence` filters, which would re-filter the art every frame.

The reveal is one timeline on the existing reveal machine:

| Stage  | Time         | What happens        |
| ------ | ------------ | ------------------- |
| draw   | 0–860 ms     | dim and star        |
| shake  | 860–1700 ms  | cracks and tremble  |
| burst  | 1700–2600 ms | split, glint, burst |
| result | from 2600 ms | card and count-up   |

Under `prefers-reduced-motion`:

- Loops stop.
- One-shots become instant changes or 150 ms fades.
- There is no shake, and no particles or boil.
- Prop taps become a static highlight.

### Interactive environment

The props are buttons over the island, kept outside the tile diamond:

- **Trees** sway and drop leaves.
- **Lamps** flicker.
- **The cart** rolls along its rails.
- **Rocks** wobble.
- **The mine:** holding it rumbles and shows the Deep Cache.
- **The campfire** opens the Campfire chat (sparks fly first). It takes
  Enter and Space, shows a focus ring, and glows white on hover.
- **Zeb** answers in pictograms; five quick taps make him pop out of a hole.

Tab reaches the props, but the field's arrow keys skip them. They are
disabled during the reveal and while sealing, and they never touch the
chain.

### Sound

Cues are synthesized with Web Audio: thunk, pin, tick, crack, chime, rumble
and crackle. Sound is muted by default, with one toggle in the menu,
remembered in this browser.

### Landing digs

The engine's mempool watcher publishes digs that are seen but not yet mined
(`landing` on `/v1/board` and the `landing` stream event). They are never
counted in the prize or depths.

- **On the field,** a landing dig shows as dust settling on its tile, with a
  LANDING marker.
- **In the other-wallet flow,** a payment shows "seen, landing" as soon as
  the watcher sees it, then "landed" once it is mined.

### Logo

The "eek" slant is one value, `EEK_SLANT` in `src/art/logoGeometry.ts`.
There are three options:

- `"20"`
- `"30"`
- `"diagonal"`: 43.6°, measured from the Z's own diagonal. This is the
  default.

The comparison image is `docs/examples/phase7b/logo-options.png`.
