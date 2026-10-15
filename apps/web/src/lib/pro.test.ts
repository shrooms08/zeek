import { describe, expect, it } from "vitest";
import { digTotal } from "./digPlan";
import { ALL_TILES, maxPerTile, QUICK_ADD } from "./pro";

describe("Pro amounts", () => {
  it("ALL is the 25 tiles in order", () => {
    expect(ALL_TILES).toHaveLength(25);
    expect(ALL_TILES[0]).toBe(0);
    expect(ALL_TILES[24]).toBe(24);
  });

  it("quick adds are 0.001, 0.01 and 0.1 ZEC", () => {
    expect(QUICK_ADD).toEqual([100_000n, 1_000_000n, 10_000_000n]);
  });

  it("MAX spends the balance minus fees, never more", () => {
    const tiles = [0, 1, 2, 3, 4, 5, 6];
    const spendable = 10_000_000n;
    const each = maxPerTile(spendable, tiles, false, 100_000n);
    expect(each).not.toBeNull();
    const total = digTotal(tiles, each ?? 0n, false).total;
    expect(total).toBeLessThanOrEqual(spendable);
    // One zatoshi more per tile would not fit.
    expect(digTotal(tiles, (each ?? 0n) + 1n, false).total).toBeGreaterThan(spendable);
  });

  it("MAX is null when even the minimum does not fit, or nothing is picked", () => {
    expect(maxPerTile(200_000n, [0, 1, 2], false, 100_000n)).toBeNull();
    expect(maxPerTile(10_000_000n, [], false, 100_000n)).toBeNull();
  });

  it("dig separately costs more in fees, so MAX is lower", () => {
    const tiles = [0, 1, 2, 3];
    const together = maxPerTile(5_000_000n, tiles, false, 100_000n) ?? 0n;
    const apart = maxPerTile(5_000_000n, tiles, true, 100_000n) ?? 0n;
    expect(apart).toBeLessThan(together);
  });
});
