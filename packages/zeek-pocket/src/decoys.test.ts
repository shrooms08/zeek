import { describe, expect, it } from "vitest";
import { planTransactionFetches, type TxRef } from "./decoys.js";

function seeded(seed: number): () => number {
  let state = seed;
  return () => {
    state = (state * 1_103_515_245 + 12_345) % 2 ** 31;
    return state / 2 ** 31;
  };
}

const tx = (n: number, height: number): TxRef => ({
  txid: n.toString(16).padStart(64, "0"),
  height,
});

describe("planTransactionFetches", () => {
  const pool = Array.from({ length: 20 }, (_, i) => tx(100 + i, 1_000 + i * 10));

  it("fetches every wanted txid among decoys without repeats", () => {
    const wanted = [tx(1, 1_050), tx(2, 1_150)];
    const plan = planTransactionFetches(wanted, [...pool, wanted[0] as TxRef], 3, seeded(7));
    expect(
      plan
        .filter((p) => p.wanted)
        .map((p) => p.txid)
        .sort(),
    ).toEqual(wanted.map((w) => w.txid).sort());
    expect(plan.filter((p) => !p.wanted)).toHaveLength(6);
    expect(new Set(plan.map((p) => p.txid)).size).toBe(plan.length);
  });

  it("prefers decoys from nearby heights", () => {
    const plan = planTransactionFetches([tx(1, 1_100)], pool, 2, seeded(3));
    for (const decoy of plan.filter((p) => !p.wanted)) {
      expect(Math.abs(decoy.height - 1_100)).toBeLessThanOrEqual(10);
    }
  });

  it("shuffles so the wanted txid is not always first", () => {
    const positions = new Set<number>();
    for (let seed = 1; seed < 40; seed += 1) {
      const plan = planTransactionFetches([tx(1, 1_100)], pool, 3, seeded(seed));
      positions.add(plan.findIndex((p) => p.wanted));
    }
    expect(positions.size).toBeGreaterThan(2);
  });

  it("works with an empty decoy pool", () => {
    expect(planTransactionFetches([tx(1, 5)], [], 3, seeded(1))).toEqual([
      { ...tx(1, 5), wanted: true },
    ]);
  });
});
