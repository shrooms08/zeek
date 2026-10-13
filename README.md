# Zeek

Zeek is a privacy game built on Zcash. Players send shielded ZEC with a memo to
a vault address to claim tiles on a 5x5 grid; each round a drand beacon picks a
winning tile and the vault pays the winners. The vault is never controlled by a
single key: it is a 2-of-3 FROST threshold key (rerandomized RedPallas, as the
Orchard protocol requires), so no single operator can move player funds.

This repository currently contains the Phase 1 vault spike: `zeek-vault`, a
CLI that generates the FROST keys, derives the vault's address, syncs a
view-only wallet against Zcash **testnet**, and spends from the vault with
2-of-3 FROST signatures applied to a PCZT.

Since NU6.3 ("Ironwood") payments to the vault land in the **Ironwood** pool:
Orchard-shaped notes (same keys, same spend authorization) with version 3 note
plaintexts, carried in the Ironwood bundle of a version 6 transaction. The
legacy Orchard pool is closed to ordinary payments; `zeek-vault` reports it
separately and refuses to spend from it. Testnet is also past NU7, which keeps
the same transaction format; see "Consensus branch" below.

## Layout

```
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

## Tests

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
# network tests (need ZEEK_LIGHTWALLETD_URL):
cargo test -p zeek-vault --test network -- --ignored --skip funded_vault_send
```

## License

Licensed under either of Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
or MIT license ([LICENSE-MIT](LICENSE-MIT)) at your option.
