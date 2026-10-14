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

| Path | What |
| --- | --- |
| `src/styles/index.css` | Design tokens in Tailwind 4's `@theme`: colours, type, spacing, borders, shadow, easings. Also the base component classes. |
| `src/lib/` | Pure logic: money (bigint zatoshis), dig plan and fees, address checks, time, passphrase, depth stages |
| `src/api/` | Engine client, TanStack Query hooks, one live WebSocket (`LiveProvider`), Pocket provider |
| `src/machines/` | The explicit state machines (see below) |
| `src/components/` | Design-system components: `BigStat`, `DigButton`, `AmountPicker`, `Panel`, `Modal`, `StepList`, `BatchProgress`, `QrCard`, `CheckRow`, `MessageRow`, `NetBadge`, `RevealLine`, `MenuRow`, `ResultCard`, `Field`/`Tile` |
| `src/screens/` | One file per screen, lazily loaded |
| `e2e/` | Playwright: `smoke.spec.ts`, `live/0N-*.spec.ts`, `screenshots.spec.ts` |

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

- It reconnects with exponential backoff: 1 s doubling to a 30 s cap, with
  ±25% jitter.
- `board` events write straight into the query cache.
- Round and Campfire events invalidate the affected queries.
- While the socket is down, the top bar shows "Reconnecting…" and queries
  fall back to polling.

Every number on screen comes from the engine or the Pocket.

## Money

Amounts are `bigint` zatoshis everywhere. They are parsed from text without
floats, and formatting truncates. Main screens show up to 4 decimals;
details (totals, fees, payouts, history) show all 8.

## Privacy

- The seed never leaves the Pocket worker. The UI shows the 24 words only on
  the backup screen. Screenshots blur them.
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
