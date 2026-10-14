import { describe, expect, it } from "vitest";
import { formatZec, parseZec, zat } from "./money";

describe("money", () => {
  it("formats truncating, never rounding up", () => {
    expect(formatZec(2_251_250n)).toBe("0.0225");
    expect(formatZec(2_251_250n, { decimals: 8 })).toBe("0.0225125");
    expect(formatZec(99_999n)).toBe("0.0009");
    expect(formatZec(100_000_000n)).toBe("1");
    expect(formatZec(15_015_000n, { decimals: 5 })).toBe("0.15015");
    expect(formatZec(0n)).toBe("0");
    expect(formatZec(5_000n, { decimals: 4, pad: true })).toBe("0.0000");
    expect(formatZec(41_200_000n, { sign: true })).toBe("+0.412");
    expect(formatZec(-15_015_000n, { decimals: 8 })).toBe("−0.15015");
  });

  it("parses exactly", () => {
    expect(parseZec("0.05")).toBe(5_000_000n);
    expect(parseZec(".1")).toBe(10_000_000n);
    expect(parseZec("1")).toBe(100_000_000n);
    expect(parseZec("0.00000001")).toBe(1n);
    expect(() => parseZec("0.000000001")).toThrow();
    expect(() => parseZec("1e3")).toThrow();
    expect(() => parseZec("")).toThrow();
    expect(() => parseZec("-1")).toThrow();
  });

  it("refuses non-integer engine numbers", () => {
    expect(zat(5)).toBe(5n);
    expect(() => zat(0.5)).toThrow();
  });
});
