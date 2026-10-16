import { describe, expect, it } from "vitest";
import {
  autoPick,
  cacheStage,
  digTotal,
  notesCoverPlan,
  paymentFee,
  planPayments,
} from "./digPlan";

describe("dig planning", () => {
  it("splits into payments of up to 6, or one each", () => {
    const twelve = Array.from({ length: 12 }, (_, i) => i);
    expect(planPayments(twelve, false).map((p) => p.length)).toEqual([6, 6]);
    expect(planPayments([1, 2, 3, 4, 5, 6, 7, 8], false).map((p) => p.length)).toEqual([6, 2]);
    expect(planPayments([4, 9], true)).toEqual([[4], [9]]);
  });

  it("matches the ZIP 317 fee of a Pocket dig", () => {
    // Live: 2 tiles paid 15,000; 6 tiles 35,000; 25 tiles in one tx 130,000.
    expect(paymentFee(2)).toBe(15_000n);
    expect(paymentFee(6)).toBe(35_000n);
    expect(paymentFee(25)).toBe(130_000n);
    expect(paymentFee(1)).toBe(10_000n);
    const t = digTotal([1, 2, 3], 5_000_000n, false);
    expect(t.dug).toBe(15_000_000n);
    expect(t.fee).toBe(20_000n);
    expect(t.total).toBe(15_020_000n);
    expect(digTotal([1, 2, 3], 5_000_000n, true).fee).toBe(30_000n);
  });

  it("auto-picks the shallowest tiles, at least 3", () => {
    const stages = Array.from({ length: 25 }, (_, i) => (i < 5 ? 0 : 3));
    const picked = autoPick(stages, 1, () => 0.5);
    expect(picked).toHaveLength(3);
    expect(picked.every((t) => t < 5)).toBe(true);
    expect(autoPick(stages, 4, () => 0.5, [0, 1]).every((t) => t !== 0 && t !== 1)).toBe(true);
  });

  it("buckets the Deep Cache", () => {
    expect(cacheStage(100_000_000n)).toBe(1);
    expect(cacheStage(200_000_000n)).toBe(2);
    expect(cacheStage(6_000_000_000n)).toBe(5);
  });
});

describe("notes for a plan of separate payments", () => {
  const plan2 = planPayments([3, 9], true);
  it("needs one note per payment sent seconds apart", () => {
    // One note (last dig's change): the second payment would have to wait.
    expect(notesCoverPlan(plan2, 100_000n, [5_000_000n])).toEqual({
      enough: false,
      payments: 2,
      notes: 1,
    });
    expect(notesCoverPlan(plan2, 100_000n, [3_000_000n, 2_000_000n]).enough).toBe(true);
  });

  it("a note too small for its payment does not count", () => {
    // 0.001 + 0.0001 fee per payment; the second note covers only 0.0005.
    expect(notesCoverPlan(plan2, 100_000n, [5_000_000n, 50_000n]).enough).toBe(false);
  });

  it("one payment can combine notes (fee grows with inputs)", () => {
    expect(notesCoverPlan([[1, 2, 3]], 1_000_000n, [2_000_000n, 1_100_000n]).enough).toBe(true);
    expect(notesCoverPlan([[1, 2, 3]], 1_000_000n, [2_000_000n, 1_010_000n]).enough).toBe(false);
  });
});
