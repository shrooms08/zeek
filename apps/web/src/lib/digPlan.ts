/**
 * Planning a dig: batches, fees and totals, and Auto-pick.
 *
 * Fees (ZIP 317): a dig transaction pays 5,000 zatoshis per action, and a
 * Pocket dig of `n` tiles has `n` dig outputs plus one change output, so its
 * network fee is `5,000 · (n + 1)` (with one input, the usual case; at
 * least two actions). Shown to players as "0.00005 per tile + 0.00005 per
 * payment". The game's own dig fee (also 0.00005 per dig) is taken out of
 * the prize at settlement, not paid on top.
 */

import type { Zat } from "./money";

export const MARGINAL_FEE: Zat = 5_000n;
export const MAX_TILES_PER_TX = 6;
export const TILE_COUNT = 25;

/** Splits tiles into payments: up to 6 per payment, or one each. */
export function planPayments(tiles: readonly number[], separate: boolean): number[][] {
  const size = separate ? 1 : MAX_TILES_PER_TX;
  const out: number[][] = [];
  for (let i = 0; i < tiles.length; i += size) out.push(tiles.slice(i, i + size));
  return out;
}

/** Network fee of one payment of `tiles` tiles from the Pocket. */
export function paymentFee(tiles: number): Zat {
  const actions = BigInt(Math.max(2, tiles + 1));
  return MARGINAL_FEE * actions;
}

export interface DigTotal {
  payments: number[][];
  dug: Zat;
  fee: Zat;
  total: Zat;
}

/** What a dig costs from the Pocket. */
export function digTotal(tiles: readonly number[], amountEach: Zat, separate: boolean): DigTotal {
  const payments = planPayments(tiles, separate);
  const dug = amountEach * BigInt(tiles.length);
  const fee = payments.reduce((sum, p) => sum + paymentFee(p.length), 0n);
  return { payments, dug, fee, total: dug + fee };
}

/**
 * Auto-pick: `count` tiles (at least 1; Lite asks for one, Pro for at least
 * 3), favouring the shallowest (lowest stage); ties broken at random with
 * `random`, so different players spread out. Returns tile indices in pick
 * order.
 */
export function autoPick(
  stages: readonly number[],
  count: number,
  random: () => number,
  exclude: readonly number[] = [],
): number[] {
  const want = Math.min(TILE_COUNT - exclude.length, Math.max(1, count));
  const candidates = stages
    .map((stage, tile) => ({ tile, stage, jitter: random() }))
    .filter((c) => !exclude.includes(c.tile))
    .sort((a, b) => a.stage - b.stage || a.jitter - b.jitter);
  return candidates.slice(0, want).map((c) => c.tile);
}

/** Deep Cache stage 1–5 by size (design: crack, timbers, lantern, rails, gate). */
export function cacheStage(cache: Zat): 1 | 2 | 3 | 4 | 5 {
  const zec = cache / 100_000_000n;
  if (zec < 2n) return 1;
  if (zec < 10n) return 2;
  if (zec < 25n) return 3;
  if (zec < 60n) return 4;
  return 5;
}

/** Plain words for a depth stage (screen readers, legends). */
export const DEPTH_WORDS = [
  "untouched",
  "barely dug",
  "dug a little",
  "dug some",
  "dug a lot",
  "dug the most",
] as const;

/**
 * Whether the Pocket's spendable notes can pay every payment of a plan on
 * its own: payments sent seconds apart cannot use an earlier one's change
 * (it needs 3 confirmations), so each needs notes of its own. Notes go to
 * the largest payments first, largest notes first; a payment's fee grows
 * with its inputs (ZIP 317: 5,000 per action, actions = max(2, inputs,
 * outputs)).
 */
export function notesCoverPlan(
  payments: readonly (readonly number[])[],
  amountEach: Zat,
  notes: readonly Zat[],
): { enough: boolean; payments: number; notes: number } {
  const free = [...notes].sort((a, b) => (a > b ? -1 : a < b ? 1 : 0));
  const costs = payments
    .map((p) => ({ tiles: p.length, value: amountEach * BigInt(p.length) }))
    .sort((a, b) => (a.value > b.value ? -1 : a.value < b.value ? 1 : 0));
  for (const pay of costs) {
    let total = 0n;
    let inputs = 0;
    const outputs = pay.tiles + 1;
    const fee = () => MARGINAL_FEE * BigInt(Math.max(2, inputs, outputs));
    while (total < pay.value + fee()) {
      const next = free.shift();
      if (next === undefined)
        return { enough: false, payments: payments.length, notes: notes.length };
      total += next;
      inputs += 1;
    }
  }
  return { enough: true, payments: payments.length, notes: notes.length };
}
