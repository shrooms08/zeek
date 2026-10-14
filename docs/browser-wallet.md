# Playing from a browser

Two ways to dig from a browser, both on Zcash testnet:

1. **Any wallet.** The page builds a ZIP 321 payment request (and its QR
   code) that any Zcash wallet can pay. The player pastes a payout address
   from their own wallet; it is sealed into each dig memo, written as ZIP 302
   *text* (`zeek1:` + base64url, rules v3) because many wallets only send
   text memos.
2. **Zeek Pocket.** A shielded wallet that runs entirely in the browser:
   Rust compiled to WebAssembly (`crates/zeek-pocket`) inside a Web Worker,
   wrapped by a typed TypeScript API (`packages/zeek-pocket`).

`apps/harness` is a plain **dev harness** for both, plus a live board. It is
not the product UI.

## Zeek Pocket

### What runs where

| Piece | Where | Holds |
|---|---|---|
| `PocketCore` (wasm) | the wallet Web Worker | the seed, spending key, notes, commitment tree, while unlocked |
| `worker.ts` | the same worker | the relay client, IndexedDB access, the sync loop |
| `Pocket` (TypeScript) | the page | nothing secret: it only posts calls to the worker |
| Engine relay | `zeek-engine` (`/v1/chain/*`, `/v1/tx/*`) | nothing of the wallet's: it forwards lightwalletd calls |

Keys: a BIP 39 phrase of 24 words (256 bits, generated with Web Crypto), the
ZIP 32 Orchard account 0 of coin type 1 (testnet), Orchard-only unified
addresses. Ironwood notes use the same Orchard keys and receivers.
Diversifier index 0 is the receive address; every dig seals a fresh address
at the next unused index (1, 2, 3, ...), recorded before the transaction
leaves the wallet so it is never reused. Change goes to the internal-scope
address.

Sync: from the birthday (the tree state of the block the wallet was created
at), the worker downloads every compact block from the relay in batches of
200 and the core scans them with `zcash_client_backend::scanning::scan_block`
(trial decryption of Ironwood and Orchard outputs with the external and
internal IVKs). Spends are detected by matching the block's nullifiers. The
Ironwood commitments go into a shard tree with a checkpoint per block, which
keeps witnesses for the wallet's notes. A block whose parent hash does not
match is a reorg: the wallet rewinds (up to 100 blocks) and rescans.

Transactions: notes are chosen largest first; the fee is ZIP 317's
`5000 · max(2, spends, outputs)`; received notes need 10 confirmations and the
wallet's own change 3 (ZIP 315). The anchor is the checkpoint 3 blocks below
the target. The transaction is built with the PCZT roles, proven with the
Orchard-protocol circuit (8 threads), signed and extracted (which verifies it)
inside the worker, then broadcast through the relay. A dig is refused when
the round has fewer than 4 blocks left (the 3-block margin of `zeek-dig`).

### Threat model

Protected:

* **The seed never leaves the browser.** It is never sent anywhere, never
  logged, and stored only encrypted. Only an explicit export, which re-checks
  the passphrase, returns the phrase to the page.
* **A stolen device or copied browser profile** yields only ciphertext. The
  seed needs the passphrase through Argon2id (64 MiB, 3 passes), so guessing
  costs about a second of work per try on a laptop; the wallet state is
  encrypted under a key derived from the seed.
* **The relay** learns which block ranges the wallet scans and, among decoys,
  which transactions it fetches (see below); it never learns which notes are
  the wallet's, its balance, or its addresses. It can refuse service or serve
  stale blocks, but cannot forge notes: the wallet checks the block hash
  chain, and every note it accepts decrypts with the wallet's own key.
* **The engine and keyholders** see the wallet's dig receivers only when they
  open a winning payout, and each dig uses a different receiver.

Not protected:

* **The page's own code.** Anyone who can run script on the page's origin
  (a compromised server or build, a malicious extension, an XSS bug) can call
  the wallet while it is unlocked, or capture the passphrase as it is typed.
  The dev harness is served by a local dev server and has no defenses beyond
  that.
* **A weak passphrase.** The minimum is 8 characters; Argon2id slows guessing
  but cannot save a short or common passphrase.
* **Network-level observers** see the client's address talking to the relay,
  as with any light wallet.
* **Losing the phrase and the browser data** loses the funds. This is a
  testnet wallet.

### Storage format

IndexedDB database `zeek-pocket`, object store `wallet`, two keys:

* `seedEnvelope` (JSON):

  ```json
  { "version": 1, "kdf": "argon2id", "m_cost_kib": 65536, "t_cost": 3, "p_cost": 1,
    "salt": "<16 bytes hex>", "nonce": "<24 bytes hex>", "ciphertext": "<hex>" }
  ```

  The ciphertext is XChaCha20-Poly1305 over the 32 bytes of BIP 39 entropy,
  key = Argon2id(passphrase, salt), associated data `zeek-pocket/seed/v1`.
* `state` (bytes): `nonce (24) || XChaCha20-Poly1305(postcard(WalletState))`,
  key = HKDF-SHA256(salt `zeek-pocket`, IKM = seed entropy,
  info `zeek-pocket/state/v1`), associated data `zeek-pocket/state/v1`. The
  state holds the last scanned block and the 100 before it, the Ironwood
  shard tree (shards, cap, checkpoints), received notes with their positions,
  nullifiers and memos, transactions the wallet built, the next unused
  diversifier index, and a pool of recent txids used as decoys
  (`crates/zeek-pocket/src/store.rs`).

Locking saves the state and drops the keys; unlocking decrypts both.

### Benchmark (headless Chromium, Apple silicon, 8 threads)

| | |
|---|---|
| wasm module | 7.9 MB (benchmark build), 9.1 MB with the wallet, 2.1 MB gzipped |
| proving key build | 5.6 to 6.8 s, once per session |
| proof, 1 spend + 2 outputs | 3.7 to 4.7 s |
| proof, 1 spend + 26 outputs (a 25-tile dig) | 38 s (47 s in the live 25-tile dig, 2 spends) |
| wasm memory after both proofs | 654 MB |

## Relay privacy trade-offs

The relay (`crates/zeek-engine/src/relay.rs`) serves the five lightwalletd
calls a light wallet needs over plain HTTP.

* **Compact blocks reveal nothing about the wallet.** It downloads every
  compact block of its range and decrypts locally; the relay cannot tell
  which outputs, if any, are the wallet's. It does learn the range, which
  roughly reveals the wallet's birthday and when it was last online.
* **Fetching a full transaction reveals interest in it.** Memos are not in
  compact blocks, so to show "Zeek round 74: treasure" the wallet must fetch
  that transaction. It hides each wanted txid among 3 decoys, transactions
  with shielded actions from nearby heights that it saw while scanning, and
  fetches the whole batch in random order. A relay that correlates the
  batch's timing with the wallet's other activity still narrows it down;
  decoys raise the cost, they do not remove the leak. Memos of the wallet's
  own change are never fetched.
* **Broadcasting reveals the transaction's origin** (the client address that
  posted it) to the relay, like any lightwalletd.
* **The relay logs nothing request-specific**: no bodies, txids, heights or
  client addresses. Rate limiting keeps a per-address token bucket in memory
  only. `GET /v1/relay/stats` exposes aggregate counters.
* **Limits**: 500 blocks per request, 256 KiB per broadcast body, 600
  requests per minute per address by default (`ZEEK_RELAY_RATE_PER_MIN`).

## Running the harness

```sh
# once: toolchains for the threaded wasm build
rustup toolchain install nightly --component rust-src --target wasm32-unknown-unknown
rustup component add llvm-tools --toolchain nightly
cargo install wasm-bindgen-cli --version 0.2.128 --locked
corepack enable && pnpm install

pnpm build:wasm                     # crates/zeek-pocket -> packages/zeek-pocket/wasm
cargo build --release
./target/release/zeek-engine        # with ZEEK_ENGINE_CORS=http://localhost:5173 in .env
pnpm --filter harness dev           # http://localhost:5173 (COOP/COEP headers set)
```

`VITE_ZEEK_ENGINE_URL` points the harness at another engine (default
`http://127.0.0.1:8787`). `apps/harness/e2e/drive.mjs` drives the same page
from Playwright (`node e2e/drive.mjs create|sync|dig|anywallet ...`).

## Phase 6 additions

### Batched digs

`pocket.dig({ tiles, amountEachZat, message?, maxTilesPerTx = 6, separate })`
splits a selection into sequential transactions of at most `maxTilesPerTx`
tiles (`separate`: one tile each). Each batch syncs to the tip, is proven
and broadcast in turn, and reports progress (`sync`, `proving`, `broadcast`,
`sent` with its txid). Each batch spends different notes, so a second batch
needs a second confirmed note. If a later batch fails, the error has code
`partial_dig` and `partial = { sent: [{txid, tiles}], unsent: [tiles] }`:
sent batches are never resent. A batch also stops if the round changes
between batches (the rest would land in the next round).

### Campfire (rules v4)

`dig({ ..., message })` attaches a message to every dig of the call;
`shout({ message, amountZat })` sends a shout (at least 0.0005 ZEC; its value
goes to the Deep Cache and is never refunded). Any wallet:
`buildDigRequest({ ..., message })` and `buildShoutRequest({ message,
amountZat })` return ZIP 321 URIs with `zeek1:` text memos. The engine
serves the feed at `GET /v1/campfire` and announces new messages over the
WebSocket (`subscribeCampfire`).

### Browser verifier and second source

`pocket.verifyRound({ round, secondSource? })` runs the five checks of
`zeek-verify` in the worker (wasm), fetching blocks and transactions through
the Zeek relay, then compares the block hashes of every block it used with
the second source. Sources:

| Network | Source | Works from a browser |
|---|---|---|
| mainnet | Blockchair, `api.blockchair.com/zcash/dashboards/block/{h}` | yes (`Access-Control-Allow-Origin: *`) |
| testnet | none public (Blockchair has no testnet; CipherScan's testnet API answered HTTP 500; public testnet lightwalletd servers offer no gRPC-web) | — |
| any | `zeek-verify relay --lightwalletd <url>` run by anyone | yes, with `--cors` |

Without a configured source the result says **cross-check unavailable**; it
never passes silently.

### Win proofs

`pocket.createWinProof(round, shareBase)` proves in the worker that a
treasure payout of `round` reached this wallet, submits it to the engine
(which verifies it before storing) and returns its id and a share URL
(`?winproof=<id>`), which the harness verifies on load. See
`docs/prove-a-win.md`.
