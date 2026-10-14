/** Passphrase strength hint (design: "Strong. 4 words or 12+ letters."). */

export type Strength = "too-short" | "ok" | "strong";

export function passphraseStrength(text: string): Strength {
  const words = text
    .trim()
    .split(/\s+/)
    .filter((w) => w.length > 0).length;
  const letters = Array.from(new Intl.Segmenter().segment(text)).length;
  if (letters < 8) return "too-short";
  if (letters >= 12 || words >= 4) return "strong";
  return "ok";
}

/** Three distinct word positions (1-based) to check after backup. */
export function checkPositions(random: () => number, count = 24): number[] {
  const out = new Set<number>();
  while (out.size < 3) out.add(1 + Math.floor(random() * count));
  return [...out].sort((a, b) => a - b);
}
