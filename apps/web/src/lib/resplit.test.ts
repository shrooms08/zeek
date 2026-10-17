import type { SplitPlan } from "@zeek/pocket";
import { describe, expect, it } from "vitest";
import { resplitDecision, type ResplitState } from "./resplit";

/** What `plan_split` gives for one 0.2 ZEC coin: 4 parts, fee 0.0002. */
const PLAN: SplitPlan = {
  inputs: 1,
  totalZat: 20_000_000,
  parts: 4,
  partZat: 4_995_000,
  feeZat: 20_000,
};

const afterDig: ResplitState = {
  busy: false,
  pendingZat: 0,
  spendableNotes: [20_000_000],
  plan: PLAN,
};

describe("re-split after a dig", () => {
  it("splits one big coin once the change has confirmed", () => {
    expect(resplitDecision(afterDig)).toEqual({ kind: "split", plan: PLAN });
  });

  it("splits one coin even beside dust too small to dig with", () => {
    expect(resplitDecision({ ...afterDig, spendableNotes: [20_000_000, 50_000] })).toEqual({
      kind: "split",
      plan: PLAN,
    });
  });

  it("waits while a dig, a withdrawal or an auto-dig is under way", () => {
    expect(resplitDecision({ ...afterDig, busy: true })).toEqual({
      kind: "wait",
      reason: "busy",
    });
  });

  it("waits while the change is still confirming", () => {
    expect(resplitDecision({ ...afterDig, pendingZat: 19_000_000, spendableNotes: [] })).toEqual({
      kind: "wait",
      reason: "confirming",
    });
    // Even with one coin spendable already: the change will be a second.
    expect(resplitDecision({ ...afterDig, pendingZat: 3_000_000 })).toEqual({
      kind: "wait",
      reason: "confirming",
    });
  });

  it("busy wins over confirming", () => {
    expect(resplitDecision({ ...afterDig, busy: true, pendingZat: 1 })).toEqual({
      kind: "wait",
      reason: "busy",
    });
  });

  it("does not split when two coins can already dig", () => {
    expect(
      resplitDecision({ ...afterDig, spendableNotes: [10_000_000, 2_000_000], plan: null }),
    ).toEqual({ kind: "none", reason: "several-coins" });
  });

  it("does not gather small coins into a split", () => {
    expect(
      resplitDecision({ ...afterDig, spendableNotes: [1_900_000, 1_900_000, 1_900_000] }),
    ).toEqual({ kind: "none", reason: "no-coin" });
    expect(resplitDecision({ ...afterDig, spendableNotes: [] })).toEqual({
      kind: "none",
      reason: "no-coin",
    });
  });

  it("does not split a coin too small for two parts or the 1 % fee", () => {
    // 0.039 ZEC: two parts of 0.02 do not fit (the Pocket's plan is null).
    expect(resplitDecision({ ...afterDig, spendableNotes: [3_900_000], plan: null })).toEqual({
      kind: "none",
      reason: "not-worth-it",
    });
  });
});
