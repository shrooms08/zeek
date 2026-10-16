# Zeek: developer notes

The full build history and developer reference (the short overview is the
[README](../README.md)).

Zeek is a privacy game built on Zcash. Players send shielded ZEC with a memo to
a vault address to claim tiles on a 5x5 grid; each round a drand beacon picks a
winning tile and the vault pays the winners. The vault is never controlled by a
single key: it is a 2-of-3 FROST threshold key (rerandomized RedPallas, as the
Orchard protocol requires), so no single operator can move player funds.

This repository contains:

* `zeek-vault` (Phase 1): a CLI that generates the FROST keys, derives the
  vault's address, syncs a view-only wallet against Zcash **testnet**, and
  spends from the vault with 2-of-3 FROST signatures applied to a PCZT.
* `zeek-core` (Phase 2): the single source of truth for the game rules. Pure,
  deterministic, no I/O, and it builds for `wasm32-unknown-unknown`, so the
  round engine, every keyholder node and a browser verifier run the same code.
* `zeek-engine` (Phase 3): the round engine. It follows testnet with the
  vault's public incoming viewing key, verifies drand beacons, settles rounds
  with `zeek-core` into signed-later payout plans, and serves the board over
  HTTP and WebSocket. It never holds spend authority.
* `zeek-dig` (Phase 3): the test player tool, one single-key shielded wallet
  per player, which digs by paying the vault with a sealed memo.
* `zeek-keyholder` (Phase 4): the three independent keyholder nodes. They run
  the FROST DKG between themselves, each follows the chain with its own
  lightwalletd and database, recomputes every settlement, and signs a payout
  only after verifying the transaction pays exactly that settlement. The
  engine's coordinator (also Phase 4) builds payout transactions, collects two
  signatures, and broadcasts.

Phase 8 runs Zeek publicly at no cost and prepares mainnet:

* [docs/deployment.md](deployment.md): the public site on Cloudflare
  Pages, the proxy to the engine through a quick tunnel, the launchd
  services, `scripts/zeek-up.sh` / `zeek-down.sh` / `zeek-status.sh`, and
  what to do after a power cut.
* [docs/lightwalletd-servers.md](lightwalletd-servers.md): the
  lightwalletd pool (failover, health, majority block-hash check) and the
  verified servers.
* [docs/mainnet-runbook.md](mainnet-runbook.md): the verified mainnet
  facts, the mainnet vault and readiness run, and how to open mainnet.
* [docs/security-review.md](security-review.md): the independent
  review's findings and what was done about each.

Since NU6.3 ("Ironwood") payments to the vault land in the **Ironwood** pool:
Orchard-shaped notes (same keys, same spend authorization) with version 3 note
plaintexts, carried in the Ironwood bundle of a version 6 transaction. The
legacy Orchard pool is closed to ordinary payments; `zeek-vault` reports it
separately and refuses to spend from it. Testnet is also past NU7, which keeps
the same transaction format; see "Consensus branch" below.

## Layout

```
crates/zeek-core    game rules library (no network, no filesystem, wasm-capable)
  src/params.rs     GameParams per network, drand quicknet chain info
  src/rounds.rs     height <-> round, round window, round status
  src/memo.rs       dig memo (ZIP 302 0xFF data) and the payout receiver
  src/seal.rs       HPKE sealing of payout receivers (X25519 / HKDF-SHA256 / ChaCha20Poly1305)
  src/payment.rs    ZIP 321 payment URI for a dig
  src/beacon.rs     drand quicknet verification, round/time math, round binding
  src/timing.rs     the timing rule that voids unsafe rounds (argument in the docs)
  src/random.rs     unbiased treasure-tile and Deep Cache draws
  src/digs.rs       valid / refund / forfeit classification
  src/settle.rs     settlement with the conservation invariant
  src/board.rs      public board views
  tests/            property tests, real drand quicknet beacons as fixtures
crates/zeek-engine  round engine library + `zeek-engine` binary
  src/chain.rs      lightwalletd compact-block follower with reorg rollback
  src/decrypt.rs    trial decryption with the external-scope IVK (Ironwood + Orchard)
  src/beacon.rs     multi-relay drand fetcher (first verifiable answer wins)
  src/processor.rs  status -> timing rule -> beacon -> classify -> settle (zeek-core)
  src/db.rs         SQLite (blocks, notes, beacons, settlement plans)
  src/api.rs        axum REST + WebSocket
  src/proof.rs      proof bundle + zeek-core-only recomputation (docs/proof-bundle.md)
  src/relay.rs      chain relay for browser wallets (compact blocks, tree state, tx, broadcast)
crates/zeek-dig     `zeek-dig` test player tool (also `send`, `pay-uri`, `address --fresh`)
crates/zeek-verify  independent round verifier and prove-a-win (native + wasm; CLI for any lightwalletd)
crates/zeek-pocket  browser wallet core, Rust -> wasm32 (keys, encrypted state, scanning, digs)
packages/zeek-pocket  TypeScript API over the wasm core in a Web Worker; any-wallet ZIP 321 requests
apps/harness        Vite DEV HARNESS: Pocket, any-wallet QR, live board (docs/browser-wallet.md)
config/             committed public game configs (no secrets)
docs/               proof bundle format and real artifacts of settled rounds
crates/zeek-vault   library + `zeek-vault` CLI (testnet only)
  src/frost.rs      FROST keygen and rerandomized signing (reddsa RedPallas ciphersuite)
  src/keys.rs       FROST group key -> Orchard FVK (ak, nk, rivk) -> UFVK -> address
  src/store.rs      ./.secrets/keyholder-N.json and ./.secrets/vault.json
  src/wallet.rs     view-only zcash_client_sqlite wallet + sync; Ironwood/Orchard balances
  src/send.rs       proposal -> V6 PCZT -> proof -> FROST signatures -> broadcast
  tests/offline_pczt.rs   builds, proves and FROST-signs V6 PCZTs with no network
  tests/network.rs        ignored tests against a live testnet lightwalletd
vendor/zcash_protocol   zcash_protocol 0.10.6 plus the NU7 testnet constants (see below)
```

## Setup

Requires a stable Rust toolchain (1.88 or newer).

```sh
cp .env.example .env        # sets ZEEK_LIGHTWALLETD_URL and the data/secrets dirs
cargo build --release
```

Key material is written to `./.secrets/` and the wallet database to `./data/`.
Both are gitignored. Keyholder files are mode `0600`.

## Commands

```sh
zeek-vault keygen --threshold 2 --signers 3 [--force]
zeek-vault address
zeek-vault sync [--birthday <height>]
zeek-vault balance
zeek-vault send --to <testnet UA> --amount <ZEC> --signers 1,3 [--memo <text>]
```

* `keygen` runs trusted-dealer FROST key generation (production will use DKG)
  and writes one file per keyholder plus the public `vault.json`. It refuses to
  overwrite existing files unless `--force` is given.
* `address` prints the vault's unified address (Orchard receiver only; this is
  how the Ironwood pool is addressed) and its unified full viewing key.
* `sync` imports the viewing key as a view-only account on first run (birthday
  defaults to the current chain tip; pass `--birthday` if the address was
  funded earlier) and scans to the tip.
* `balance` reports spendable and pending Ironwood value, any legacy Orchard
  value, and lists every note with its pool and confirmation depth. Received
  notes need 10 confirmations before they are spendable, change needs 3.
* `send` proposes a ZIP 317 transaction spending Ironwood notes only, builds a
  V6 PCZT, proves it, runs the FROST rounds with exactly the selected
  keyholders for every spend that still needs a signature (real spends and any
  wallet-controlled zero-value spend the builder pairs with change in an
  Orchard-pool bundle; protocol padding dummies are pre-signed by the IO
  finalizer), verifies each signature against the action's randomized key,
  checks the extracted transaction is V6 on the expected consensus branch,
  and broadcasts it. If the vault's funds are only in the closed Orchard pool,
  or are not yet confirmed, it explains why and stops without building a
  transaction.

## Consensus branch

Zcash testnet activated NU7 (consensus branch `0x77190AD9`) at height
4,465,026. The published `zcash_protocol 0.10.x` crates only know NU7 as a
placeholder behind the `zcash_unstable="nu7"` cfg, and the newer line that
fixes this drops the FROST-capable `reddsa` ciphersuite. The workspace
therefore sets that cfg in `.cargo/config.toml` and patches `zcash_protocol`
with `vendor/zcash_protocol`, the unmodified 0.10.6 source plus the two NU7
constants (branch ID and testnet activation height) that
`zcash_protocol 0.11.0-pre.0` ships. See `vendor/zcash_protocol/ZEEK-PATCH.md`.
If you export `RUSTFLAGS` yourself it must include `--cfg zcash_unstable="nu7"`.

## How the FROST key becomes an Orchard key

An Orchard full viewing key is `(ak, nk, rivk)`. `ak` is the FROST group
verifying key. The protocol requires `ak`'s encoding to have an even
y-coordinate; `reddsa`'s RedPallas ciphersuite negates every share after key
generation when that is not the case, and `zeek-vault` re-checks the sign bit
before building the viewing key. `nk` and `rivk` are derived from a throw-away
random spending key and then kept only inside the viewing key.

Each Orchard-protocol spend (Ironwood or Orchard pool) is signed with
`rsk = ask + alpha`, where `alpha` is chosen by the PCZT constructor. The FROST
coordinator feeds `alpha` in as the rerandomized-FROST randomizer, so the
aggregated signature verifies under `rk = ak + [alpha] G`, exactly what the
transaction commits to. Before applying a signature the vault checks that the
action's `rk` equals its own group key randomized by that `alpha`, so it can
never sign a spend it does not control.

## Running the engine and playing

```sh
zeek-vault sealing-keygen            # once: .secrets/payout-sealing.json
zeek-vault address                   # vault address, UFVK and the public UIVK
# config/game.testnet.json holds the GameParams, vault address, sealing public
# key, UIVK and genesis height (notes below it are ignored by the game).
cargo run --release -p zeek-engine   # API on 127.0.0.1:8787 (see .env.example)

zeek-dig init alice && zeek-dig sync alice && zeek-dig balance alice
zeek-dig dig alice --tile 3 --amount 0.005 --engine http://127.0.0.1:8787
```

API: `GET /v1/config`, `/v1/board`, `/v1/rounds/pending`, `/v1/rounds/{n}`,
`/v1/rounds/{n}/proof`, `/v1/health`, WebSocket `/v1/stream`. Sealed payout
receivers are the only receiver form the engine stores or serves.

Dig detection uses the vault's **external-scope** unified incoming viewing
key: payments to the vault's address are visible, the vault's own change
(internal scope) is not, so change can never be mistaken for a dig. The pinned
`zcash_client_sqlite` can only import full viewing keys, so the engine scans
compact blocks itself and completes each hit from the full transaction to
read the memo.

## Keyholders and payouts (Phase 4)

```sh
# identities (one per node, one for the coordinator); public keys go into config/keyholder-N.json and .env
zeek-keyholder identity-keygen --secrets-dir .secrets/keyholder-1
zeek-keyholder identity-keygen --secrets-dir .secrets/coordinator
# DKG: start all three with the same fresh 32-byte hex session id
zeek-keyholder serve --config config/keyholder-1.json --dkg <session>   # ports 9101-9103
zeek-keyholder address --config config/keyholder-1.json               # all three print the same address
# copy .secrets/keyholder-1/payout-sealing.json to the other two nodes and to .secrets/ (engine)
# then the engine with ZEEK_KEYHOLDERS set (see .env.example) pays settlements automatically
curl http://127.0.0.1:8787/v1/vault/ledger
# rebuild both vaults' history from chain data and reconcile it with the engine
zeek-engine audit > docs/examples/phase4b/audit.txt
```

Keyholders refuse with the exact item that failed, for example
`output 0: recipient does not match the sealed payout receiver for round 7 (amount matches)`,
`output 2: amount 2,370,000 expected 2,365,000`,
`input 3: note is not a dig of rounds [7]`, or
`fee 15,000 exceeds the fees collected by the rules for rounds [7] (10,000)`.

The vault's `ak` is the DKG group key; reddsa's RedPallas ciphersuite negates
every share when the group key's y-coordinate is odd (`post_dkg`), and each
node re-checks it. `nk` and `rivk` are derived from a transcript of every
participant's round-1 package, so all nodes derive the same viewing key and
address and abort otherwise. Every message between nodes and coordinator is
an ed25519-signed envelope bound to a session; a coordinator that tampers
with a payout is refused by every keyholder with the reason logged. The
`rogue-demo` cargo feature exists only to demonstrate that refusal.

## Playing from a browser (Phase 5)

Any Zcash wallet can dig by paying a ZIP 321 request whose memos use the text
encoding of rules v3; Zeek Pocket is a shielded wallet that runs entirely in
the browser. Architecture, threat model, storage format, relay privacy
trade-offs and how to run the harness: [docs/browser-wallet.md](browser-wallet.md).

## Privacy: who can learn what

Digs are shielded payments; the only public data is the vault's chain
footprint. Every dig seals a **fresh diversified address** of the player's
wallet (one new diversifier index per tile, allocated and persisted by the
player's wallet), so no receiver is ever reused.

| Party | Sees | Cannot see |
|---|---|---|
| Public observer with the UIVK (anyone; the key is in `config/game.testnet.json`) | Every dig's value, round and tile, which digs share a transaction, the vault's payout transactions and their fees (value balance), the number of payout outputs | Who dug (sender identity is shielded), payout recipients and amounts (outputs are encrypted to the recipients), the vault's change (internal scope) |
| Engine operator (runs the coordinator, holds the sealing key) | Everything above, plus the opened payout receivers of the payouts it builds and the exact amounts paid | Which player a receiver belongs to; with fresh diversified receivers it cannot link two digs of the same player, even when both win |
| Keyholders (open the sealed receivers of the payouts they sign) | The payout receivers and amounts of the rounds they sign, and the vault's own notes | Whose receivers they are, and that two payouts went to the same wallet: every dig's receiver is a distinct diversified address |
| Players | Their own digs and payouts (their wallet decrypts them) | Other players' receivers or amounts beyond the public board |

Only the payouts of a round are opened, by the coordinator and by each
keyholder, in memory; nothing opened is logged, stored or served. The API only
ever exposes sealed receiver bytes.

## Game rules (zeek-core)

Each round is `blocks_per_round` blocks (24 x 25 s on testnet, 8 x 75 s on
mainnet, about ten minutes). A dig is a shielded payment of at least
0.001 ZEC to the vault whose memo is `0xFF "ZEEK" 1 round tile` followed by the
digger's payout receiver sealed with HPKE to the vault's sealing key, bound to
the round and tile. When the window closes, the drand quicknet round fixed at
the round's start reveals the treasure tile (rejection-sampled, no modulo
bias). Diggers on that tile split the pot pro rata after a 5% rake into the
Deep Cache; an empty treasure tile rolls the pot into the Cache; the Cache
breaks open with 1-in-625 odds. Fees follow ZIP 317 exactly (rules v2): every
dig note the payout transaction spends is charged a dig fee of 5000 zatoshis
before it joins the pot, and every payout or refund is one shielded output
charged the 5000-zatoshi marginal fee. The payout transaction spends exactly
the round's charged notes (plus the fewest operator-float notes when a Cache
payout exceeds them), so its ZIP 317 fee, `5000 · max(2, spends, outputs)`,
never exceeds the fees collected; see `crates/zeek-core/src/settle.rs`.
Undecodable memos and digs under the minimum are forfeited to the Cache; digs
for the wrong round, outside the window or in a voided round are refunded
minus both fees, or forfeited whole when they cannot cover them. Rounds
settled before rules v2 keep their recorded results (proof bundle version 1).
From `text_memo_activation_height` (rules v3) a dig memo may also be sent as
ZIP 302 text, `zeek1:` followed by the unpadded base64url of the binary memo
without its `0xFF` byte; both encodings decode to the same dig, and a text
memo before activation is classified exactly as before. The board's
`prize_pool_zat` is what winners would split now (valid digs minus dig fees,
minus rake); `gross_dug_zat` is the plain total dug.
From `campfire.activation_height` (rules v4, **Campfire**) a dig may carry a
public message and a **shout** (a message only, at least 0.0005 ZEC) feeds
the Deep Cache; see `docs/proof-bundle.md`.

## Verifying rounds and wins

`zeek-verify round <n> --lightwalletd <any lightwalletd>` checks a round
against chain data (beacon, completeness of digs, timing, settlement, payout
transaction); `zeek-verify win <id>` checks a win proof
([docs/prove-a-win.md](prove-a-win.md)). The browser verifier in the
harness runs the same checks.
A round is voided when
the median timestamp of the 11 blocks after its window, plus the assumed
honest clock skew, is not before the beacon's publication time (see the
"Timing safety argument" in `crates/zeek-core/src/timing.rs`).

## Tests

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build -p zeek-core --target wasm32-unknown-unknown   # rustup target add wasm32-unknown-unknown
# browser wallet (see docs/browser-wallet.md for toolchains)
pnpm build:wasm && pnpm lint && pnpm typecheck && pnpm test && pnpm build
# network tests (need ZEEK_LIGHTWALLETD_URL):
cargo test -p zeek-vault --test network -- --ignored --skip funded_vault_send
```

## License

Licensed under either of Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE))
or MIT license ([LICENSE-MIT](../LICENSE-MIT)) at your option.
