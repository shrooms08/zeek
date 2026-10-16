import { describe, expect, it } from "vitest";
import { revealProgress } from "./time";

describe("progress to the reveal", () => {
  it("runs from the close (beacon − delay) to the beacon", () => {
    expect(revealProgress(10_000, 900, 9_100)).toBe(0);
    expect(revealProgress(10_000, 900, 9_550)).toBe(50);
    expect(revealProgress(10_000, 900, 10_000)).toBe(100);
    expect(revealProgress(10_000, 900, 12_000)).toBe(100);
    expect(revealProgress(10_000, 900, 8_000)).toBe(0);
  });
});
