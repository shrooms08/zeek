import { describe, expect, it } from "vitest";
import { stagesFromTotals } from "./stages";

describe("stagesFromTotals", () => {
  it("matches the engine's buckets", () => {
    const totals = Array.from({ length: 25 }, () => 0);
    [3, 40, 80, 140, 220, 517].forEach((v, i) => {
      totals[i] = v;
    });
    expect(stagesFromTotals(totals).slice(0, 7)).toEqual([1, 2, 3, 4, 5, 5, 0]);
    expect(stagesFromTotals(Array.from({ length: 25 }, () => 0))).toEqual(
      Array.from({ length: 25 }, () => 0),
    );
  });
});
