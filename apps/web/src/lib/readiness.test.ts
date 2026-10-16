import { describe, expect, it } from "vitest";
import { fitsInRound, readyIn, roundAcceptsDigs, shortfall, spacingSecs } from "./readiness";

describe("readiness", () => {
  it("uses the recent block rate, else the target spacing", () => {
    expect(spacingSecs(31, 25)).toBe(31);
    expect(spacingSecs(null, 25)).toBe(25);
    expect(spacingSecs(undefined, undefined)).toBe(25);
  });

  it("says when in words", () => {
    expect(readyIn(1, 25)).toBe("in under a minute");
    expect(readyIn(3, 25)).toBe("in about 1 min");
    expect(readyIn(10, 31)).toBe("in about 5 min");
  });

  it("tells confirming change apart from a real shortfall", () => {
    // 0.0103 needed; nothing spendable, 0.12 confirming in 3 blocks.
    expect(shortfall(1_030_000n, 0n, 12_000_000n, 3)).toEqual({ kind: "confirming", blocks: 3 });
    expect(shortfall(1_030_000n, 2_000_000n, 0n, null)).toEqual({ kind: "enough" });
    expect(shortfall(1_030_000n, 500_000n, 100_000n, 2)).toEqual({ kind: "short" });
    expect(shortfall(1_030_000n, 0n, 0n, null)).toEqual({ kind: "short" });
  });

  it("queues only if the dig can still start in time", () => {
    expect(fitsInRound(3, 10)).toBe(true);
    expect(fitsInRound(3, 7)).toBe(true);
    expect(fitsInRound(3, 6)).toBe(false);
    expect(fitsInRound(10, 12)).toBe(false);
  });
});

describe("whether a round still takes digs", () => {
  it("needs the round open with at least 4 blocks left", () => {
    expect(roundAcceptsDigs({ status: "open", blocks_remaining: 12 })).toBe(true);
    expect(roundAcceptsDigs({ status: "open", blocks_remaining: 4 })).toBe(true);
    expect(roundAcceptsDigs({ status: "open", blocks_remaining: 3 })).toBe(false);
    expect(roundAcceptsDigs({ status: "open", blocks_remaining: 0 })).toBe(false);
    expect(roundAcceptsDigs({ status: "closed", blocks_remaining: 20 })).toBe(false);
  });
});
