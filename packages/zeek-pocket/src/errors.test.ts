import { describe, expect, it } from "vitest";
import { PocketError, toPocketError } from "./errors.js";

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
