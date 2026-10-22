import { describe, expect, it } from "vitest";
import { liteTiles, tapTile } from "./digMode";

describe("dig modes", () => {
  it("Lite moves the pick to the tapped tile; tapping it again clears it", () => {
    expect(tapTile("lite", [], 4)).toEqual([4]);
    expect(tapTile("lite", [4], 9)).toEqual([9]);
    expect(tapTile("lite", [9], 9)).toEqual([]);
  });

  it("Pro adds and removes tiles", () => {
    expect(tapTile("pro", [4], 9)).toEqual([4, 9]);
    expect(tapTile("pro", [4, 9], 4)).toEqual([9]);
  });

  it("switching to Lite keeps the first tile and says when others were dropped", () => {
    expect(liteTiles([7, 3, 12])).toEqual({ tiles: [7], trimmed: true });
    expect(liteTiles([7])).toEqual({ tiles: [7], trimmed: false });
    expect(liteTiles([])).toEqual({ tiles: [], trimmed: false });
  });
});
