import { describe, expect, it } from "vitest";
import { POLL_MS, runAutoDig, type AutoDigDeps, type AutoDigPlan } from "./autoDig";

const ROUND_MS = 600_000;
const PER_ROUND = 1_350_000n;

/** A mocked clock, board and wallet: rounds advance as the clock does. */
function world(opts: {
  balance: bigint;
  startRound?: number;
  hiddenAt?: number | null;
  digFails?: number;
  blocksLeft?: number;
}) {
  let clock = 0;
  let balance = opts.balance;
  const digs: number[] = [];
  const round = () => (opts.startRound ?? 100) + Math.floor(clock / ROUND_MS);
  const deps: AutoDigDeps = {
    now: () => clock,
    sleep: (ms) => {
      clock += ms;
      return Promise.resolve();
    },
    board: () =>
      Promise.resolve({
        round: round(),
        status: "open",
        blocks_remaining: opts.blocksLeft ?? 20,
      }),
    spendable: () => Promise.resolve(balance),
    dig: () => {
      if (opts.digFails !== undefined && digs.length === opts.digFails) {
        return Promise.reject(new Error("the relay timed out"));
      }
      balance -= PER_ROUND;
      digs.push(round());
      clock += 90_000; // proving and sending take a while
      return Promise.resolve({ round: round(), txids: ["t"] });
    },
    hiddenSince: () => (opts.hiddenAt === undefined ? null : opts.hiddenAt),
  };
  return { deps, digs, clock: () => clock };
}

const plan = (rounds: number, cap = PER_ROUND * BigInt(rounds)): AutoDigPlan => ({
  rounds,
  perRound: PER_ROUND,
  cap,
  roundMs: ROUND_MS,
});

describe("auto-dig", () => {
  it("digs once per new round, N times, then finishes", async () => {
    const w = world({ balance: 10_000_000n });
    const out = await runAutoDig(
      plan(3),
      w.deps,
      () => undefined,
      () => false,
    );
    expect(out.reason).toEqual({ kind: "finished" });
    expect(w.digs).toEqual([100, 101, 102]);
    expect(out.progress.spent).toBe(PER_ROUND * 3n);
  });

  it("never digs the same round twice (waits for the next one)", async () => {
    const w = world({ balance: 10_000_000n });
    await runAutoDig(
      plan(2),
      w.deps,
      () => undefined,
      () => false,
    );
    expect(new Set(w.digs).size).toBe(w.digs.length);
    expect(w.clock()).toBeGreaterThanOrEqual(ROUND_MS);
  });

  it("stops before a round the balance would not cover", async () => {
    const w = world({ balance: PER_ROUND * 2n + 5n });
    const out = await runAutoDig(
      plan(5),
      w.deps,
      () => undefined,
      () => false,
    );
    expect(out.reason.kind).toBe("balance");
    expect(w.digs).toHaveLength(2);
  });

  it("stops before passing the spend cap", async () => {
    const w = world({ balance: 100_000_000n });
    const out = await runAutoDig(
      plan(5, PER_ROUND * 2n),
      w.deps,
      () => undefined,
      () => false,
    );
    expect(out.reason).toEqual({ kind: "cap" });
    expect(w.digs).toHaveLength(2);
  });

  it("stops when the tab has been hidden for more than one round", async () => {
    const w = world({ balance: 100_000_000n, hiddenAt: 0 });
    // Hidden from the start: the first check still digs (not yet a round).
    const out = await runAutoDig(
      plan(5),
      w.deps,
      () => undefined,
      () => false,
    );
    expect(out.reason).toEqual({ kind: "hidden" });
    expect(w.digs.length).toBeLessThanOrEqual(1);
    expect(w.clock()).toBeLessThanOrEqual(ROUND_MS + POLL_MS + 90_000);
  });

  it("stops on any error, without retrying", async () => {
    const w = world({ balance: 100_000_000n, digFails: 1 });
    const out = await runAutoDig(
      plan(5),
      w.deps,
      () => undefined,
      () => false,
    );
    expect(out.reason).toEqual({ kind: "error", message: "the relay timed out" });
    expect(w.digs).toHaveLength(1);
  });

  it("Stop: no further dig starts", async () => {
    const w = world({ balance: 100_000_000n });
    let stop = false;
    const out = await runAutoDig(
      plan(5),
      w.deps,
      (p) => {
        if (p.done === 1) stop = true;
      },
      () => stop,
    );
    expect(out.reason).toEqual({ kind: "stopped" });
    expect(w.digs).toHaveLength(1);
  });

  it("waits while a round has too few blocks left for a dig to land", async () => {
    const w = world({ balance: 100_000_000n, blocksLeft: 2 });
    let stop = false;
    let polls = 0;
    const deps = {
      ...w.deps,
      sleep: (ms: number) => {
        polls += 1;
        if (polls > 3) stop = true;
        return w.deps.sleep(ms);
      },
    };
    const out = await runAutoDig(
      plan(2),
      deps,
      () => undefined,
      () => stop,
    );
    expect(out.reason).toEqual({ kind: "stopped" });
    expect(w.digs).toHaveLength(0);
  });

  it("reports sending while a dig is in flight", async () => {
    const w = world({ balance: 100_000_000n });
    const seen: boolean[] = [];
    await runAutoDig(
      plan(1),
      w.deps,
      (p) => seen.push(p.sending),
      () => false,
    );
    expect(seen).toEqual([true, false]);
  });
});
