# Prove a win

A winner can prove **"the treasure payout of round N, output k, was received
at an address I control"** without revealing their wallet, any other
address, their balance or their history. The proof is about 6.3 kB, takes
about 20 s to create in the browser and a second or two to verify, and can
be published (`POST /v1/proofs`) and shared as a link.

Implementation: `crates/zeek-verify/src/win.rs` (create and verify, native
and wasm); Pocket: `Pocket.createWinProof(round, shareBase)`; CLI:
`zeek-dig prove-win <player> --round N [--submit]`, `zeek-verify win <id or
file>`.

## What is already public

The round's proof bundle (format 4) opens payout `k`: its recipient address
`A`, value and `rseed` (see `docs/proof-bundle.md`). Anyone recomputes the
output's note commitment in the mined payout transaction, so "output `k`
pays `v` to `A`" is checkable by anyone. What remains is **control of `A`**.

## Construction

This follows ZIP 304 (Sapling address signatures), which proves control of a
Sapling address with a spend proof of a made-up note in a one-leaf tree,
adapted to the Orchard-protocol circuit that the Ironwood pool uses.

1. The prover makes up a note `n* = (A, v = 1, rho*, rseed*)` with random
   `rho*` and `rseed*`; it never exists on chain.
2. The anchor is the root of a depth-32 Orchard note commitment tree whose
   only leaf is `cmx(n*)` at position 0, every other node empty. The
   verifier recomputes it from `A`, `rho*` and `rseed*`.
3. The prover builds a **one-action Ironwood bundle** spending `n*` with no
   output (the action's output side is a fabricated zero-value note to `A`),
   so the bundle's value balance is **+1**, proves it with the
   Orchard-protocol Halo2 circuit, and signs it (spend authorization and
   binding signature) over

   ```text
   BLAKE2b-256(personal "ZeekWinProofV1__",
               "zeek-win-proof/v1" ‖ network tag ‖ round (u32 BE) ‖ payout txid
               ‖ payout index k (u32 BE) ‖ recipient A (43 bytes) ‖ nonce (32 bytes))
   ```

   in place of a transaction sighash.

### Why not a zero-value note

The prompt for this feature proposed spending a zero-value note, because the
Orchard circuit skips the Merkle check for those. The circuit does skip it:
`orchard 0.15.5 src/circuit.rs`, gate "Orchard circuit checks", constraint
"Either v_old = 0, or root = anchor" (protocol spec § 4.17.4, Action
statement, Merkle path validity). That is exactly why a zero-value spend
proves nothing about `A`: the spent note's address is not a public input, and
it is bound to the statement only through its commitment, which only the
Merkle check ties to the verifier's anchor. A zero-value spend would prove
control of *some* key, anybody's. Spending a 1-zatoshi made-up note keeps
the Merkle check on, and the +1 value balance, enforced by the binding
signature (§ 4.14), guarantees that a spend of non-zero value exists, so the
check is not skipped.

## What a valid proof establishes

Verification checks, in order (each failure names its step):

1. **payout**: the proof's round and network are the bundle's; payout `k` is
   a treasure payout (`Win`); the proof's opening is the bundle's opening of
   payout `k`; the proof names the round's payout transaction.
2. **payout transaction**: the transaction is in the compact block at its
   height; the opening recomputes the `cmx` of its action (`rho` = the
   action's nullifier).
3. **address binding**: the anchor is the one-leaf root of `cmx(A, 1, rho*,
   rseed*)`.
4. **proof**: the bundle has exactly one action, spends enabled, value
   balance +1, and its Halo2 proof verifies with the Orchard-protocol
   (post-NU 6.3) verifying key.
5. **signature**: the spend authorization signature verifies under the
   action's `rk`, and the binding signature under the bundle's binding key,
   both over the message above.

Soundness argument:

* Value balance +1 (binding signature) means some spent note has value at
  least 1, so for it the circuit enforces root = anchor; the anchor's tree
  has one leaf, `cmx(n*)`; so the spent note **is** `n*`, at address `A`
  (collision resistance of the Sinsemilla note commitment and the Merkle
  hash, spec § 5.4.8.4, § 5.4.1.9).
* Address integrity in the circuit: `pk_d = [ivk] g_d` with `ivk =
  Commit^ivk_rivk(ak, nk)` (spec § 4.17.4), so the prover knows `ak`, `nk`,
  `rivk` of a full viewing key that `A` belongs to.
* Spend authority: the circuit proves `rk = ak + [α] G`, and a signature
  under `rk` proves knowledge of `rsk = ask + α`, hence of `ask` (spec § 4.15,
  RedPallas, § 5.4.7).
* The message binds network, round, payout transaction, output index,
  recipient and a fresh nonce: the proof cannot be moved to another round,
  output or address (each tamper fails at "payout" or "signature"), and two
  proofs of the same win are unlinkable.

Together: **whoever made this proof could spend notes sent to the address
that received round N's treasure payout k.**

## What it does not prove

* That the prover still holds the payout (they may have spent it).
* That the prover is the person who dug (whoever holds the spending key can
  prove; giving someone the key gives them this ability).
* Anything about the prover's other addresses, balance or history: the
  proof reveals only `A` (already public in the bundle), the nullifier of
  the made-up note (random, never on chain), a randomized `rk` and a value
  commitment.

## Privacy of publishing a proof

Publishing adds one fact to what the bundle already says: *someone chose to
claim this win*. If the winner shares the link under their name, they link
their name to that one payout address; since every dig uses a fresh
address, that reveals nothing about their other digs. The payout note's own
nullifier is not revealed, so spending the winnings later stays unlinkable.
