# Zeek proof bundle (versions 1 and 2)

`GET /v1/rounds/{n}/proof` returns everything an independent verifier needs to
recompute round `n`'s settlement with nothing but the `zeek-core` crate. The
engine's `zeek_engine::proof::recompute` is the reference verifier and is run
in the test suite against real artifacts under `docs/examples/`.

The bundle version is the **rules version** the round was settled under:

* **Version 1** (rules v1): fees are charged per payout output only.
  `params` has no `dig_fee_zat` (it reads as 0) and `settlement` has no
  `dig_fees`.
* **Version 2** (rules v2): every dig note the payout transaction spends is
  also charged `params.dig_fee_zat` (5000 zatoshis, the ZIP 317 marginal
  fee). `settlement.dig_fees` is the sum of those charges and
  `settlement.total_fees` includes them. Rounds settled before the change
  keep their version 1 bundles and stay verifiable; `recompute` rejects a
  bundle whose version disagrees with `params.rules_version()`.

## Fields

| Field | Type | Meaning |
|---|---|---|
| `version` | integer | Bundle format and rules version, `1` or `2`. |
| `params` | `GameParams` | The game parameters the round was played under (network, genesis height, blocks per round, block spacing, rake, Cache odds, minimum dig, output fee, dig fee from version 2, drand chain, reveal delay, MTP window, clock skew). |
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
| `settlement` | `Settlement` | The engine's settlement: pot (after dig fees from version 2), rake, distributable, treasure tile total, winners, Cache paid, payouts (sealed receiver, memo round/tile, gross, output fee, net, reason), previous/new Cache, Cache inflow, total fees (dig fees plus output fees), total paid, forfeited, dig fees (version 2). |

## Verification procedure

1. `round_window(params, round)` must equal `window`; every note height must lie inside it.
2. `beacon_round_for(params, start_block_time)` must equal `drand_round`, and `beacon_time(params, drand_round)` must equal `beacon_time`.
3. `check_window_close(params, beacon_time, post_close_block_times)` must equal `timing`.
4. If the verdict is unsafe the outcome must be `voided`. Otherwise `beacon.round` must equal `drand_round`, `verify_beacon(params.drand, beacon)` must succeed and yield `randomness`, and `treasure_tile(round, randomness)` / `cache_triggers(params, round, randomness)` must reproduce `outcome`.
5. `classify_round(params, round, voided, notes)` then `settle(params, round, classified, previous_cache, outcome)` must reproduce `settlement` exactly (byte-for-byte as canonical JSON).
6. (version 2) `payout_inputs(params, classified, outcome)` lists the notes the payout transaction must spend; the audit (`zeek-engine audit`) checks the broadcast transaction spent exactly those notes, paid exactly the settlement's net amounts, and paid a fee no higher than `settlement.total_fees`.

What the bundle does not prove on its own: that `notes` is complete and that the block timestamps are the canonical chain's. A verifier with the public UIVK and a lightwalletd can rescan the window to check both; the engine stores block hashes for that purpose.
