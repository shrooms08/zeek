# zcash_protocol 0.10.6 with the NU7 testnet constants

This directory is the published `zcash_protocol 0.10.6` crate source (copied from
the cargo registry, unmodified apart from the edits below and the removal of
cargo's packaging metadata files). It is applied to the workspace through
`[patch.crates-io]` so that every other pinned librustzcash crate keeps using
the API line they were built against.

Why: Zcash testnet activated NU7 at height 4,465,026 with consensus branch ID
`0x77190AD9` (ZIP 259). Version 0.10.6 only exposes NU7 behind the
`--cfg zcash_unstable="nu7"` flag, with the placeholder branch ID `0xFFFFFFFF`
and no testnet activation height, so transactions built against it carry the
NU6.3 branch ID and are rejected by the network. `zcash_protocol 0.11.0-pre.0`
fixes both values but is a semver break the rest of the stack (orchard 0.15 /
reddsa 0.5 FROST ciphersuite) cannot follow yet.

Edits, all in `src/consensus.rs`, all under `#[cfg(zcash_unstable = "nu7")]`:

1. `TestNetwork::activation_height(NetworkUpgrade::Nu7)` returns
   `Some(BlockHeight(4_465_026))` instead of `None`.
2. `BranchId::try_from(0x7719_0ad9)` yields `BranchId::Nu7` (was `0xffff_ffff`).
3. `u32::from(BranchId::Nu7)` is `0x7719_0ad9` (was `0xffff_ffff`).

Mainnet has no NU7 activation height, so mainnet behaviour is unchanged.
Remove this directory and the patch entry once the workspace moves to a
librustzcash line that ships these constants together with a FROST-capable
RedPallas ciphersuite.
