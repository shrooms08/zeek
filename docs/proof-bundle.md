# Zeek proof bundle (formats 1 to 4)

`GET /v1/rounds/{n}/proof` returns everything an independent verifier needs to
recompute round `n`'s settlement with nothing but the `zeek-core` crate. The
reference verifier is `zeek_core::proof::recompute` (re-exported as
`zeek_engine::proof`); `zeek-verify` runs it together with checks against
chain data (see below), and the test suite runs it against every real
artifact under `docs/examples/`.

In formats 1 to 3 the bundle version **is** the rules version the round was
settled under. Format 4 states the rules version in its own field:

* **Version 1** (rules v1): fees are charged per payout output only.
  `params` has no `dig_fee_zat` (it reads as 0) and `settlement` has no
  `dig_fees`.
* **Version 2** (rules v2): every dig note the payout transaction spends is
  also charged `params.dig_fee_zat` (5000 zatoshis, the ZIP 317 marginal
  fee). `settlement.dig_fees` is the sum of those charges and
  `settlement.total_fees` includes them. Rounds settled before the change
  keep their version 1 bundles and stay verifiable.
* **Version 3** (rules v3): fees as in version 2, and `params` carries
  `text_memo_activation_height`. A note mined at or above that height may
  carry its dig memo as ZIP 302 text, `zeek1:` followed by the unpadded
  base64url of the binary memo without its `0xFF` lead byte (142 ASCII
  characters); both encodings decode to the same dig. A text memo mined below
  the activation height is classified exactly as under version 2 (an
  undecodable memo, forfeited). A round is version 3 once any block of its
  window is at or above the activation height; earlier rounds keep their
  versions and their bundles omit the field.

* **Rules v4** (Campfire, format 4 bundles only): `params.campfire`
  (`activation_height`, `min_shout_zat`). From the activation height a dig
  memo may carry a message (memo version 2: the version 1 layout followed by
  a big-endian `u16` length and UTF-8 text) and a **shout** memo (version 3:
  `0xFF "ZEEK" 3`, length, text; no tile, no receiver) is accepted. Messages
  are at most 275 bytes on a dig (the real limit of a dig's text encoding)
  and 280 on a shout, valid UTF-8 without control characters; an invalid
  message is dropped and the memo still counts. A shout of at least
  `min_shout_zat` adds its value minus one dig fee to the Cache
  (`settlement.shouts`, `settlement.shout_fees`; the reserved dig fee pays
  for spending the note later) and is never refunded; a smaller one is
  forfeited.
* **Format 4**: `rules_version` (1 to 4) and, once the payout transaction is
  mined, `payout_openings`, which let anyone check every payout amount in
  the transaction (check 5 below). Rounds settled under earlier rules are
  served in format 4 too, with the parameters they were settled under;
  bundles saved in formats 1 to 3 stay verifiable for checks 1 to 4.

`recompute` rejects a bundle whose stated rules version disagrees with
`rounds::rules_version(params, round)`.

## Fields

| Field | Type | Meaning |
|---|---|---|
| `version` | integer | Bundle format, `1` to `4` (formats 1 to 3: also the rules version). |
| `rules_version` | integer | (format 4) Rules version of the round, `1` to `4`. |
| `params` | `GameParams` | The game parameters the round was played under (network, genesis height, blocks per round, block spacing, rake, Cache odds, minimum dig, output fee, dig fee from version 2, text-memo activation height from version 3, `campfire` from rules v4, drand chain, reveal delay, MTP window, clock skew). |
| `round` | integer | Round number. |
| `window` | `{start, end}` | Block heights `[start, end)` of the round. |
| `notes` | array | Every note received at the vault's external address inside the window, in chain order: `txid` (display hex), `output_index` (action index), `height`, `value` (zatoshis), `memo` (hex of the 512-byte memo field). Sealed receivers are inside the memos; nothing is opened. |
| `start_block_time` | integer | `nTime` of block `start`; fixes the bound drand round. |
| `post_close_block_times` | array | `nTime` of blocks `end .. end + mtp_window`, in height order; the timing rule's input. |
| `drand_round` | integer | The drand quicknet round bound to this game round. |
| `beacon_time` | integer | Unix time that drand round is published (`genesis_time + (round − 1) · period`). |
| `beacon` | object or null | `{round, signature}` as published by drand; `null` for voided rounds. |
| `randomness` | hex or null | `SHA-256(signature)`, the verified beacon randomness. |
| `timing` | object | The timing verdict (`safe` with `margin_secs`, or `unsafe` with `deficit_secs`). |
| `outcome` | object | `voided`, or `drawn` with `treasure_tile` and `cache_triggered`. |
| `previous_cache` | integer | Deep Cache balance before this round. |
| `settlement` | `Settlement` | The engine's settlement: pot (after dig fees from version 2), rake, distributable, treasure tile total, winners, Cache paid, payouts (sealed receiver, memo round/tile, gross, output fee, net, reason), previous/new Cache, Cache inflow, total fees (dig fees plus output fees), total paid, forfeited, dig fees (version 2), shouts and shout fees (rules v4, omitted when zero). |
| `payout` | object | (when broadcast) The engine's payout record: state, signing session, txid, fee, signers. |
| `payout_openings` | object | (format 4, once mined) `txid`, `mined_height` and per settlement payout: `payout_index`, `action_index` (in the Ironwood bundle), `recipient` (43-byte raw Orchard-protocol address, hex), `value`, `rseed` (hex), `note_version`. `rho` is the nullifier of the same action (protocol spec § 4.7.3), already public. |

## Verification procedure

1. `round_window(params, round)` must equal `window`; every note height must lie inside it.
2. `beacon_round_for(params, start_block_time)` must equal `drand_round`, and `beacon_time(params, drand_round)` must equal `beacon_time`.
3. `check_window_close(params, beacon_time, post_close_block_times)` must equal `timing`.
4. If the verdict is unsafe the outcome must be `voided`. Otherwise `beacon.round` must equal `drand_round`, `verify_beacon(params.drand, beacon)` must succeed and yield `randomness`, and `treasure_tile(round, randomness)` / `cache_triggers(params, round, randomness)` must reproduce `outcome`.
5. `classify_round(params, round, voided, notes)` then `settle(params, round, classified, previous_cache, outcome)` must reproduce `settlement` exactly (byte-for-byte as canonical JSON).
6. (version 2) `payout_inputs(params, classified, outcome)` lists the notes the payout transaction must spend; the audit (`zeek-engine audit`) checks the broadcast transaction spent exactly those notes, paid exactly the settlement's net amounts, and paid a fee no higher than `settlement.total_fees`.

What the bundle does not prove on its own: that `notes` is complete and that the block timestamps are the canonical chain's. `zeek-verify` checks both against chain data from any lightwalletd.

## Independent verification (`zeek-verify`)

`zeek-verify round <n> --engine <url> --lightwalletd <any lightwalletd>`
(and the browser verifier in `@zeek/pocket`) take only the bundle and the
public config (parameters, vault address, UIVK) from Zeek and run five
checks, each with a plain-language verdict and technical detail:

1. **Beacon**: the bundle's parameters agree with the public config; the
   time of the window's first block, read from the chain, binds the drand
   round; the beacon's BLS signature verifies against drand quicknet and
   reproduces the draw.
2. **Digs**: every compact block of the window is trial-decrypted with the
   public UIVK and each hit completed from its full transaction; the set
   (txid, action, height, value, memo) must equal `notes` exactly, so no dig
   is hidden and none invented.
3. **Timing**: the post-close block times are read from the chain and the
   timing verdict recomputed.
4. **Settlement**: `recompute`, compared byte for byte.
5. **Payout** (format 4 with openings; otherwise "not available"): the
   payout txid is in the compact block at `mined_height`; the transaction's
   Ironwood bundle verifies (Halo2 proof with the Orchard-protocol verifying
   key; spend authorization and binding signatures over the shielded
   sighash, which for a transaction without transparent parts equals the
   txid digest, ZIP 244 § S.2); and for every settlement payout the opening
   recomputes the `cmx` of its action (with `rho` = that action's
   nullifier), with the settled net amount.

### What payout openings disclose

Each opening reveals the payout's recipient address, its value and its
`rseed`. The value is already public in the settlement. The recipient is a
fresh diversified address the winner's wallet created for that one dig;
diversified addresses of one wallet are unlinkable without its incoming
viewing key (protocol spec § 4.2.3, diversified addresses), so the opening
links neither two payouts of one player nor a payout to the player's other
addresses. The opening does not reveal the note's nullifier (that needs the
recipient's nullifier key, § 4.16), so a later spend of the payout stays
unlinkable to it. The keyholders already saw these receivers when they
signed; the openings add the recipient for the public, nothing about
identity.
