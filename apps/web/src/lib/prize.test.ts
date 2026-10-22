import { describe, expect, it } from "vitest";
import type { LandingDig } from "../api/types";
import { livePrize, prizeOfPot } from "./prize";

const RULES = { rake_bps: 500, dig_fee_zat: 5_000 };

function dig(id: string, value: number, round = 7): LandingDig {
  return { id, round, tile: 3, value_zat: value, memo_sha256: "00", first_seen: 1 };
}

/** A board with `values` mined in round 7. */
function board(values: number[], landing: LandingDig[]) {
  const pot = values.reduce((s, v) => s + v - 5_000, 0);
  return {
    round: 7,
    digs: values.length,
    gross_dug_zat: values.reduce((s, v) => s + v, 0),
    prize_pool_zat: Number(prizeOfPot(BigInt(pot), 500n)),
    landing,
  };
}

describe("live prize", () => {
  it("adds this round's landing digs, the rake taken over the whole pot", () => {
    // Mined 400,000: pot 395,000, prize 375,250. Landing 1,000,000 adds
    // 995,000: pot 1,390,000, prize 1,320,500.
    const b = board([400_000], [dig("a", 1_000_000)]);
    expect(livePrize(b, RULES)).toEqual({ total: 1_320_500n, landing: 945_250n });
  });

  it("counts a dig once when the list repeats it", () => {
    const once = livePrize(board([], [dig("a", 1_000_000)]), RULES);
    const twice = livePrize(board([], [dig("a", 1_000_000), dig("a", 1_000_000)]), RULES);
    expect(twice).toEqual(once);
  });

  it("never counts a mined dig twice: once mined it is on the board, not landing", () => {
    // Before: landing. After the block: mined, and the engine has left it
    // out of the landing list. The total does not jump.
    const landing = livePrize(board([400_000], [dig("a", 1_000_000)]), RULES);
    const mined = livePrize(board([400_000, 1_000_000], []), RULES);
    expect(mined).toEqual({ total: landing.total, landing: 0n });
  });

  it("drops a landing dig that left the mempool unmined", () => {
    const before = livePrize(board([400_000], [dig("a", 1_000_000)]), RULES);
    const after = livePrize(board([400_000], []), RULES);
    expect(before.landing > 0n).toBe(true);
    expect(after).toEqual({ total: 375_250n, landing: 0n });
  });

  it("counts only the current round", () => {
    const b = board([400_000], [dig("a", 1_000_000, 6), dig("b", 1_000_000, 8)]);
    expect(livePrize(b, RULES)).toEqual({ total: 375_250n, landing: 0n });
  });

  it("shows the mined prize alone when it cannot follow the rule", () => {
    expect(livePrize(board([400_000], [dig("a", 1_000_000)]), { rake_bps: 500 })).toEqual({
      total: 375_250n,
      landing: 0n,
    });
    const odd = { ...board([400_000], [dig("a", 1_000_000)]), prize_pool_zat: 1 };
    expect(livePrize(odd, RULES)).toEqual({ total: 1n, landing: 0n });
  });
});
