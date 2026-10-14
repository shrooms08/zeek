import { describe, expect, it } from "vitest";
import { checkPositions, passphraseStrength } from "./passphrase";

describe("passphrase", () => {
  it("rates strength like the design says", () => {
    expect(passphraseStrength("short")).toBe("too-short");
    expect(passphraseStrength("eightchr")).toBe("ok");
    expect(passphraseStrength("twelve chars")).toBe("strong");
    expect(passphraseStrength("a b c d e f g h")).toBe("strong");
  });
  it("picks three distinct positions", () => {
    let i = 0;
    const seq = [0.1, 0.1, 0.5, 0.9];
    const p = checkPositions(() => seq[i++ % seq.length] ?? 0);
    expect(p).toHaveLength(3);
    expect(new Set(p).size).toBe(3);
    expect(p.every((x) => x >= 1 && x <= 24)).toBe(true);
  });
});
