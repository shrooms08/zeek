# The reveal wait: measured margin and decision

Measured 2026-10-16 with `node scripts/reveal-margin.mjs` against the
testnet engine. That covers every settled round it has seen: rounds 0 to 335
(336 rounds), from round 0's start at block 4,499,347.

## The rule being sized

The reveal comes from the drand round bound to each round, published at the
first drand time at or after:

```
start + round length (24 × 25 s = 600 s) + reveal delay (900 s)
```

A round only counts if the chain proves it closed before that beacon:

```
median time of the 11 blocks after the window + 300 s clock-skew allowance < beacon time
```

Otherwise it is voided and refunded (`crates/zeek-core/src/timing.rs`).
Lowering the reveal delay would shorten the wait one for one.

## How late the chain ran

All values are in seconds and come from the engine's own record of each
round's blocks (its proof bundles).

| Measure | min | median | p90 | p99 | max |
| --- | --- | --- | --- | --- | --- |
| Round duration (scheduled 600) | 329 | 591 | 750 | 880 | 1,025 |
| Round start vs the schedule (drift) | −707 | −83 | 579 | 928 | 1,001 |
| Median time of the 11 blocks after close, minus the scheduled close | −108 | 117 | 287 | 490 | **539** |
| Smallest reveal delay this round would have passed with | 193 | 418 | 588 | 791 | **840** |
| Reveal margin it actually had (with 900) | 61 | 486 | 635 | 695 | 710 |
| From the actual close to when the reveal could settle | 475 | 910 | 1,037 | 1,123 | 1,171 |

- **Voided rounds:** none in 336.
- **Smallest reveal delay that would have voided no round:** 840 s (the
  current delay is 900 s).
- **Worst lateness:** the median time after close ran 539 s behind the
  scheduled close. One round got through with only 61 s to spare.

## Decision: keep 15 minutes

The bar for lowering the delay was zero historical voids at the new value
**and** a budget of at least twice the worst observed lateness.

| | Budget for lateness (delay − 300 s skew) | Zero voids in history |
| --- | --- | --- |
| Current, 900 s | 600 s | yes |
| Needed by the bar | ≥ 2 × 539 = 1,078 s | — |
| Lowest without voids, 840 s | 540 s | yes, by 1 s |

The data does not support lowering the margin: the current one is already
below twice the worst lateness, and 840 s would leave a single second of
room. Testnet keeps the 900 s reveal delay, and mainnet parameters are
untouched. The closed-round screen now says why the wait exists ("Why the
wait: …"), shows progress to the reveal, offers the open round
("While you wait: dig round N"), and can notify the player when the reveal
lands.

The worst lateness comes from the testnet's uneven block times: one window
took 1,025 s instead of 600. If anything, these numbers argue for watching
the margin, not cutting it.
