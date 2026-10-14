import { describe, expect, it } from "vitest";
import { formatZec, parseTiles, parseZec } from "./amounts.js";

describe("amounts", () => {
  it("parses and formats ZEC exactly", () => {
    expect(parseZec("0.001")).toBe(100_000);
    expect(parseZec("1")).toBe(100_000_000);
    expect(parseZec("0.00000001")).toBe(1);
    expect(formatZec(2_251_250)).toBe("0.02251250");
    expect(() => parseZec("0.000000001")).toThrow();
    expect(() => parseZec("0")).toThrow();
    expect(() => parseZec("1e3")).toThrow();
  });

  it("parses tile lists and ranges", () => {
    expect(parseTiles("3")).toEqual([3]);
    expect(parseTiles("0, 4,9")).toEqual([0, 4, 9]);
    expect(parseTiles("0-24")).toHaveLength(25);
    expect(() => parseTiles("25")).toThrow();
    expect(() => parseTiles("1,1")).toThrow();
    expect(() => parseTiles("")).toThrow();
  });
});
