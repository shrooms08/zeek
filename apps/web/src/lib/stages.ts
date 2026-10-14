/** Depth stages from exact tile totals (same buckets as the engine board). */

export function stagesFromTotals(totals: readonly number[]): number[] {
  const bounds = [400n, 800n, 1_400n, 2_200n];
  const total = totals.reduce((s, v) => s + BigInt(v), 0n);
  return totals.map((v) => {
    if (v === 0 || total === 0n) return 0;
    const bps = (BigInt(v) * 10_000n) / total;
    return 1 + bounds.filter((b) => bps >= b).length;
  });
}

/**
 * Approximate stages from the engine's relative depths (0–100, relative to
 * the deepest tile) when exact totals are not public yet (a closed,
 * unsettled round).
 */
export function stagesFromDepths(depths: readonly number[]): number[] {
  return stagesFromTotals(depths);
}
