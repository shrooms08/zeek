# Zeek

Zeek is a privacy game built on Zcash. Players send shielded ZEC with a memo to
a vault Orchard address to claim tiles on a 5x5 grid; each round a drand beacon
picks a winning tile and the vault pays the winners. The vault is never
controlled by a single key: it is a 2-of-3 FROST threshold key (rerandomized
RedPallas, as Orchard requires), so no single operator can move player funds.

This repository currently contains the Phase 1 vault spike: `zeek-vault`, a
CLI that generates the FROST keys, derives the vault's Orchard address, syncs a
view-only wallet against Zcash **testnet**, and spends from the vault with
2-of-3 FROST signatures applied to a PCZT.

## Layout

```
crates/zeek-vault   library + `zeek-vault` CLI (testnet only)
  src/frost.rs      FROST keygen and rerandomized signing (reddsa RedPallas ciphersuite)
  src/keys.rs       FROST group key -> Orchard FVK (ak, nk, rivk) -> UFVK -> address
  src/store.rs      ./.secrets/keyholder-N.json and ./.secrets/vault.json
  src/wallet.rs     view-only zcash_client_sqlite wallet + sync via lightwalletd
  src/send.rs       proposal -> PCZT -> proof -> FROST signatures -> broadcast
  tests/offline_pczt.rs   builds, proves and FROST-signs a PCZT with no network
  tests/network.rs        ignored tests against a live testnet lightwalletd
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
* `address` prints the vault's unified address (Orchard receiver only) and its
  unified full viewing key.
* `sync` imports the viewing key as a view-only account on first run (birthday
  defaults to the current chain tip; pass `--birthday` if the address was
  funded earlier) and scans to the tip.
* `balance` reports spendable and pending Orchard value and lists notes with
  their confirmation depth.
* `send` proposes a ZIP 317 transaction, builds a PCZT, proves it, runs the
  FROST rounds with exactly the selected keyholders for every Orchard spend,
  verifies each signature against the action's randomized key, extracts the
  transaction and broadcasts it.

## How the FROST key becomes an Orchard key

An Orchard full viewing key is `(ak, nk, rivk)`. `ak` is the FROST group
verifying key. The protocol requires `ak`'s encoding to have an even
y-coordinate; `reddsa`'s RedPallas ciphersuite negates every share after key
generation when that is not the case, and `zeek-vault` re-checks the sign bit
before building the viewing key. `nk` and `rivk` are derived from a throw-away
random spending key and then kept only inside the viewing key.

Each Orchard spend is signed with `rsk = ask + alpha`, where `alpha` is chosen
by the PCZT constructor. The FROST coordinator feeds `alpha` in as the
rerandomized-FROST randomizer, so the aggregated signature verifies under
`rk = ak + [alpha] G`, exactly what the transaction commits to.

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
