import { describe, expect, it } from "vitest";
import { isUnclearBroadcast, PocketError, toPocketError } from "./errors.js";

describe("toPocketError", () => {
  it("keeps the code, message and reorg height of wasm errors", () => {
    const e = toPocketError({ code: "reorg", message: "the chain reorganised", height: 42 });
    expect(e).toBeInstanceOf(PocketError);
    expect(e.code).toBe("reorg");
    expect(e.height).toBe(42);
    expect(PocketError.fromData(e.toData())).toEqual(e);
  });

  it("maps unknown codes to internal and fetch TypeErrors to network", () => {
    expect(toPocketError({ code: "nope", message: "x" }).code).toBe("internal");
    expect(toPocketError(new TypeError("Failed to fetch")).code).toBe("network");
    expect(toPocketError("boom").message).toBe("boom");
    const same = new PocketError("locked", "locked");
    expect(toPocketError(same)).toBe(same);
  });
});

describe("a failed broadcast: refused, or maybe sent", () => {
  const cases: [string, PocketError, boolean][] = [
    [
      "the network dropped",
      new PocketError("network", "cannot reach the engine: Failed to fetch"),
      true,
    ],
    ["the relay timed out", new PocketError("relay", "502 Bad Gateway"), true],
    [
      "a second node already has it",
      new PocketError("broadcast_rejected", "transaction already in mempool"),
      true,
    ],
    ["a duplicate", new PocketError("broadcast_rejected", "txn-already-known"), true],
    [
      "its notes are spent by another of ours",
      new PocketError("broadcast_rejected", "bad-txns-orchard-duplicate-nullifier"),
      true,
    ],
    ["the node refused it", new PocketError("broadcast_rejected", "tx-expiring-soon"), false],
    ["it never left this browser", new PocketError("internal", "invalid hex"), false],
  ];
  for (const [name, error, unclear] of cases) {
    it(`${name}: ${unclear ? "maybe sent" : "not sent"}`, () => {
      expect(isUnclearBroadcast(error)).toBe(unclear);
    });
  }

  it("keeps the maybe-sent code across the worker boundary", () => {
    const e = toPocketError({ code: "broadcast_unknown", message: "check History" });
    expect(e.code).toBe("broadcast_unknown");
  });
});
