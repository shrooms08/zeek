import { describe, expect, it } from "vitest";
import { checkReducer, visibleRows } from "./check";
import { digReducer, initialDig } from "./dig";
import {
  withLanding,
  initialOtherWallet,
  memoDigest,
  otherWalletReducer,
  paymentStatus,
  type PaymentModel,
} from "./otherWallet";
import { estimatedShare, initialReveal, outcomeFor, resultSound, revealReducer } from "./reveal";
import { canTryUnlock, initialWallet, walletReducer } from "./wallet";

describe("wallet machine", () => {
  it("follows the lifecycle and throttles wrong passphrases", () => {
    let s = walletReducer(initialWallet, { type: "status", exists: false, unlocked: false });
    expect(s.kind).toBe("none");
    s = walletReducer(walletReducer(walletReducer(s, { type: "create" }), { type: "created" }), {
      type: "backed-up",
    });
    expect(s.kind).toBe("unlocked");
    s = walletReducer(s, { type: "sync", fraction: 0.5, scanned: 10, tip: 20 });
    expect(s.kind).toBe("syncing");
    s = walletReducer(s, { type: "synced" });
    s = walletReducer(s, { type: "lock" });
    expect(s.kind).toBe("locked");
    for (let i = 0; i < 2; i += 1) {
      s = walletReducer(walletReducer(s, { type: "unlock" }), {
        type: "unlock-failed",
        message: "No.",
        now: 100,
      });
    }
    expect(s.kind === "locked" && s.error).toContain("1 try before a 30 s wait");
    s = walletReducer(walletReducer(s, { type: "unlock" }), {
      type: "unlock-failed",
      message: "No.",
      now: 100,
    });
    expect(canTryUnlock(s, 110)).toBe(false);
    expect(canTryUnlock(s, 130)).toBe(true);
  });
});

describe("dig machine", () => {
  it("tracks batches and reports partial failure without losing sent batches", () => {
    let s = digReducer(initialDig, { type: "review" });
    s = digReducer(s, {
      type: "start",
      batches: [
        [0, 1, 2, 3, 4, 5],
        [6, 7],
      ],
    });
    s = digReducer(s, {
      type: "progress",
      progress: { phase: "proving", batch: 1, batches: 2, tiles: [0, 1, 2, 3, 4, 5] },
    });
    expect(s.kind === "sealing" && s.step).toBe(1);
    s = digReducer(s, {
      type: "progress",
      progress: { phase: "sent", batch: 1, batches: 2, tiles: [0, 1, 2, 3, 4, 5], txid: "aa" },
    });
    s = digReducer(s, {
      type: "progress",
      progress: {
        phase: "waiting",
        batch: 2,
        batches: 2,
        tiles: [6, 7],
        waitingFor: "change",
        seconds: 20,
      },
    });
    expect(s.kind === "sealing" && s.waiting).toEqual({ for: "change", seconds: 20 });
    expect(s.kind === "sealing" && s.current).toBe(1);
    const partial = digReducer(s, {
      type: "failed",
      code: "partial_dig",
      message: "stopped",
      partial: { sent: [{ txid: "aa", tiles: [0, 1, 2, 3, 4, 5] }], unsent: [6, 7] },
    });
    expect(partial).toEqual({
      kind: "partial",
      sent: [{ txid: "aa", tiles: [0, 1, 2, 3, 4, 5] }],
      unsent: [6, 7],
      message: "stopped",
    });
    expect(digReducer(s, { type: "cancel" }).kind).toBe("sealing");
  });
});

describe("reveal machine", () => {
  const result = { round: 9, treasureTile: 12, cacheTriggered: false, voided: false };
  it("plays the sequence, or jumps to the result with reduced motion", () => {
    let s = revealReducer(initialReveal(false), { type: "settled", result });
    expect(s.stage).toBe("draw");
    for (const stage of ["shake", "burst", "result"]) {
      s = revealReducer(s, { type: "advance" });
      expect(s.stage).toBe(stage);
    }
    expect(revealReducer(initialReveal(true), { type: "settled", result }).stage).toBe("result");
    expect(
      revealReducer(initialReveal(false), { type: "settled", result: { ...result, voided: true } })
        .stage,
    ).toBe("result");
  });

  it("judges outcomes and shares like the rules", () => {
    expect(outcomeFor(result, [12, 3])).toBe("win");
    expect(outcomeFor({ ...result, cacheTriggered: true }, [12])).toBe("cache-win");
    expect(outcomeFor(result, [3])).toBe("not-this-time");
    expect(outcomeFor(result, [])).toBe("watched");
    // Round 74: the Pocket dug all 25 tiles at 0.001; tile 13 total 0.001; prize 2,256,250.
    expect(estimatedShare(100_000n, 100_000n, 2_256_250n, 0n)).toBe(2_256_250n);
    expect(estimatedShare(1n, 3n, 10n, 0n)).toBe(3n);
  });
});

describe("the result's sound", () => {
  const drawn = { round: 5, treasureTile: 7, cacheTriggered: false, voided: false };
  const cases: [string, Parameters<typeof outcomeFor>[0], number[], "win" | "miss" | null][] = [
    ["found the treasure", drawn, [7], "win"],
    ["found it as the Deep Cache broke open", { ...drawn, cacheTriggered: true }, [7], "win"],
    ["missed (another tile)", drawn, [2], "miss"],
    ["missed while the Deep Cache broke open", { ...drawn, cacheTriggered: true }, [2], "miss"],
    ["voided and refunded", { ...drawn, treasureTile: null, voided: true }, [2], "miss"],
    ["only watched", drawn, [], null],
    ["only watched a voided round", { ...drawn, voided: true }, [], null],
  ];
  for (const [name, result, mine, expected] of cases) {
    it(`${name}: ${expected ?? "silent"}`, () => {
      expect(resultSound(outcomeFor(result, mine), mine.length > 0)).toBe(expected);
    });
  }
});

describe("check machine", () => {
  it("ticks rows and reveals results one by one", () => {
    let s = checkReducer({ kind: "idle" }, { type: "start" });
    s = checkReducer(s, { type: "progress", progress: { step: "scan", detail: "scanning" } });
    expect(
      visibleRows(s)
        .slice(0, 2)
        .every((r) => r.state === "checking"),
    ).toBe(true);
    s = checkReducer(s, {
      type: "result",
      result: {
        source: "relay",
        crossCheck: { status: "unavailable", source: "none", detail: "" },
        report: {
          round: 1,
          bundle_version: 4,
          rules_version: 4,
          checks: [
            { id: "beacon", title: "", status: "pass", summary: "ok", detail: [] },
            { id: "digs", title: "", status: "pass", summary: "ok", detail: [] },
            { id: "timing", title: "", status: "pass", summary: "ok", detail: [] },
            { id: "settlement", title: "", status: "fail", summary: "bad", detail: ["x"] },
            { id: "payout", title: "", status: "not_available", summary: "none", detail: [] },
          ],
        },
      },
    });
    expect(visibleRows(s).map((r) => r.state)).toEqual([
      "pass",
      "checking",
      "checking",
      "checking",
      "checking",
    ]);
    for (let i = 0; i < 4; i += 1) s = checkReducer(s, { type: "tick" });
    expect(s.kind).toBe("done");
    expect(visibleRows(s).map((r) => r.state)).toEqual(["pass", "pass", "pass", "fail", "na"]);
  });
});

describe("other wallet machine", () => {
  it("waits for every payment and flags late ones", async () => {
    const d1 = await memoDigest("zeek1:abc");
    const d2 = await memoDigest("zeek1:def");
    expect(d1).toHaveLength(64);
    const payments: PaymentModel[] = [
      { uri: "zcash:a", tiles: [1], amount: 1n, memoDigests: [d1], qrSvg: "" },
      { uri: "zcash:b", tiles: [2], amount: 1n, memoDigests: [d2], qrSvg: "" },
    ];
    let s = otherWalletReducer(initialOtherWallet, { type: "built", round: 5, payments });
    s = otherWalletReducer(s, { type: "sent" });
    const partial = paymentStatus(payments, [{ height: 10, memo_sha256: d1 }], 20);
    expect(partial).toEqual(["landed", "not-sent"]);
    s = otherWalletReducer(s, { type: "seen", status: partial });
    expect(s.kind).toBe("waiting");
    const all = paymentStatus(
      payments,
      [
        { height: 10, memo_sha256: d1 },
        { height: 21, memo_sha256: d2 },
      ],
      20,
    );
    expect(all).toEqual(["landed", "late"]);
    s = otherWalletReducer(s, { type: "seen", status: all });
    expect(s.kind).toBe("confirmed");
  });

  it("shows mempool payments as landing, and only mined ones confirm", async () => {
    const d1 = await memoDigest("zeek1:abc");
    const d2 = await memoDigest("zeek1:def");
    const payments: PaymentModel[] = [
      { uri: "zcash:a", tiles: [1], amount: 1n, memoDigests: [d1], qrSvg: "" },
      { uri: "zcash:b", tiles: [2], amount: 1n, memoDigests: [d2], qrSvg: "" },
    ];
    const mined = paymentStatus(payments, [], 20);
    expect(withLanding(payments, mined, new Set([d1]))).toEqual(["landing", "not-sent"]);
    let s = otherWalletReducer(initialOtherWallet, { type: "built", round: 5, payments });
    s = otherWalletReducer(s, { type: "sent" });
    s = otherWalletReducer(s, { type: "seen", status: ["landing", "landing"] });
    expect(s.kind).toBe("waiting");
    s = otherWalletReducer(s, { type: "seen", status: ["landed", "landed"] });
    expect(s.kind).toBe("confirmed");
  });
});
