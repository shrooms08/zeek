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

## License

Licensed under either of Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
or MIT license ([LICENSE-MIT](LICENSE-MIT)) at your option.
