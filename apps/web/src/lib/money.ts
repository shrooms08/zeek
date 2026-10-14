/**
 * Money: zatoshi as bigint everywhere inside the app; ZEC strings only at
 * the edge. No floating point ever touches an amount.
 */

export const ZAT_PER_ZEC = 100_000_000n;

/** A zatoshi amount. */
export type Zat = bigint;

/** From an engine JSON number (always an integer number of zatoshis). */
export function zat(value: number | bigint): Zat {
  if (typeof value === "bigint") return value;
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError(`not a zatoshi amount: ${value}`);
  }
  return BigInt(value);
}

/** To a JSON number for the Pocket API (amounts stay far below 2^53). */
export function toNumber(value: Zat): number {
  if (value > BigInt(Number.MAX_SAFE_INTEGER) || value < 0n) {
    throw new RangeError("amount out of range");
  }
  return Number(value);
}

export interface FormatOptions {
  /** Most fractional digits shown (main screens 4, details 8). */
  decimals?: number;
  /** Keep trailing zeros up to `decimals` (fixed-width tables). */
  pad?: boolean;
  /** Prefix positive amounts with "+". */
  sign?: boolean;
}

/**
 * Formats zatoshis as ZEC. Truncates (never rounds up), so a displayed
 * amount is never more than the real one.
 */
export function formatZec(value: Zat, options: FormatOptions = {}): string {
  const decimals = Math.max(0, Math.min(8, options.decimals ?? 4));
  const negative = value < 0n;
  const abs = negative ? -value : value;
  const whole = abs / ZAT_PER_ZEC;
  let frac = (abs % ZAT_PER_ZEC).toString().padStart(8, "0").slice(0, decimals);
  if (!options.pad) frac = frac.replace(/0+$/, "");
  const body = frac.length > 0 ? `${whole.toString()}.${frac}` : whole.toString();
  const sign = negative ? "−" : options.sign === true && abs > 0n ? "+" : "";
  return `${sign}${body}`;
}

/** Parses a ZEC decimal ("0.05", "1", ".1") exactly; at most 8 decimals. */
export function parseZec(text: string): Zat {
  const trimmed = text.trim();
  const match = /^(\d*)(?:\.(\d{0,8}))?$/.exec(trimmed);
  if (match === null || trimmed === "" || trimmed === ".") {
    throw new RangeError(`not a ZEC amount: "${text}"`);
  }
  const whole = BigInt(match[1] === "" || match[1] === undefined ? "0" : match[1]);
  const frac = BigInt((match[2] ?? "").padEnd(8, "0"));
  return whole * ZAT_PER_ZEC + frac;
}

/** The design's amount presets (ZEC per tile). */
export const PRESETS: readonly Zat[] = [1_000_000n, 5_000_000n, 10_000_000n];
/** Step of the − and + buttons. */
export const AMOUNT_STEP: Zat = 1_000_000n;
