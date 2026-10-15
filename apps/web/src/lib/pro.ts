/** Pro mode's amount helpers (bigint zatoshis). */

import { digTotal, TILE_COUNT } from "./digPlan";
import type { Zat } from "./money";

/** Every tile, in order. */
export const ALL_TILES: readonly number[] = Array.from({ length: TILE_COUNT }, (_, i) => i);

/** The quick-add steps: +0.001, +0.01, +0.1 ZEC per tile. */
export const QUICK_ADD: readonly Zat[] = [100_000n, 1_000_000n, 10_000_000n];

/**
 * The largest amount per tile the balance covers for `tiles` (fees
 * included), or null when even the minimum does not fit.
 */
export function maxPerTile(
  spendable: Zat,
  tiles: readonly number[],
  separate: boolean,
  minDig: Zat,
): Zat | null {
  if (tiles.length === 0) return null;
  const { fee } = digTotal(tiles, 0n, separate);
  const each = (spendable - fee) / BigInt(tiles.length);
  return each >= minDig ? each : null;
}
