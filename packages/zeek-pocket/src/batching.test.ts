import { describe, expect, it } from "vitest";
import { describePartial, planBatches, privacyGapMs } from "./batching.js";

describe("planBatches", () => {
  it("splits 12 tiles into two transactions of 6 by default", () => {
    const tiles = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
    expect(planBatches(tiles)).toEqual([
      [0, 1, 2, 3, 4, 5],
      [6, 7, 8, 9, 10, 11],
    ]);
    expect(planBatches(tiles, 5).map((b) => b.length)).toEqual([5, 5, 2]);
    expect(planBatches([3, 9, 1], 6, true)).toEqual([[3], [9], [1]]);
  });

  it("rejects empty, duplicate and nonsensical requests", () => {
    expect(() => planBatches([])).toThrow(/at least one tile/);
    expect(() => planBatches([1, 1])).toThrow(/distinct/);
    expect(() => planBatches([1], 0)).toThrow(/maxTilesPerTx/);
  });

  it("describes exactly what was and was not sent", () => {
    const message = describePartial([{ txid: "ab", tiles: [0, 1] }], [2, 3], "insufficient funds");
    expect(message).toContain("tiles 0,1 in ab");
    expect(message).toContain("Not sent: tiles 2,3");
    expect(message).toContain("never resent");
  });
});

describe("privacyGapMs", () => {
  it("stays within 2 to 20 seconds", () => {
    expect(privacyGapMs(() => 0)).toBe(2_000);
    expect(privacyGapMs(() => 0.999999)).toBeLessThanOrEqual(20_000);
    expect(privacyGapMs(() => 0.5)).toBe(11_000);
  });
});
