/** Exact ZEC/zatoshi conversions for form inputs (no floating point). */

const COIN = 100_000_000n;

/** Parses a decimal ZEC amount with at most 8 fractional digits. */
export function parseZec(text: string): number {
  const trimmed = text.trim();
  const match = /^(\d+)(?:\.(\d{1,8}))?$/.exec(trimmed);
  if (match === null) {
    throw new Error(`not a ZEC amount: "${text}"`);
  }
  const whole = BigInt(match[1] ?? "0");
  const fraction = BigInt((match[2] ?? "").padEnd(8, "0"));
  const zat = whole * COIN + fraction;
  if (zat === 0n || zat > 21_000_000n * COIN) {
    throw new Error(`amount out of range: "${text}"`);
  }
  return Number(zat);
}

/** Formats zatoshis as ZEC with 8 decimals. */
export function formatZec(zat: number): string {
  const value = BigInt(zat);
  const fraction = (value % COIN).toString().padStart(8, "0");
  return `${(value / COIN).toString()}.${fraction}`;
}

/** Parses tiles like "3", "0,4,9" or "0-24". */
export function parseTiles(text: string): number[] {
  const tiles: number[] = [];
  for (const part of text
    .split(",")
    .map((p) => p.trim())
    .filter((p) => p.length > 0)) {
    const range = /^(\d+)\s*-\s*(\d+)$/.exec(part);
    if (range !== null) {
      const from = Number(range[1]);
      const to = Number(range[2]);
      for (let t = from; t <= to; t += 1) {
        tiles.push(t);
      }
    } else if (/^\d+$/.test(part)) {
      tiles.push(Number(part));
    } else {
      throw new Error(`not a tile: "${part}"`);
    }
  }
  if (tiles.length === 0 || tiles.some((t) => t > 24) || new Set(tiles).size !== tiles.length) {
    throw new Error("tiles must be distinct numbers from 0 to 24");
  }
  return tiles;
}
