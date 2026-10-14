import { describe, expect, it } from "vitest";
import { qrSvg } from "./qr.js";

describe("qrSvg", () => {
  it("renders an SVG for a long ZIP 321 URI", async () => {
    const uri = `zcash:?address=utest1${"q".repeat(100)}&amount=0.001&memo=${"A".repeat(190)}`;
    const svg = await qrSvg(uri);
    expect(svg.startsWith("<svg")).toBe(true);
    expect(svg).toContain("<path");
  });
});
