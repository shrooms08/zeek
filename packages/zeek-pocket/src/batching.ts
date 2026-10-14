/** Splitting a dig into sequential transactions. */

import { PocketError } from "./errors.js";

/** Default tiles per transaction: a 6-tile dig proves in about 15 s. */
export const DEFAULT_MAX_TILES_PER_TX = 6;

/**
 * Splits `tiles` into batches of at most `maxTilesPerTx` (or one tile each
 * when `separate`), keeping the order the player chose.
 */
export function planBatches(
  tiles: readonly number[],
  maxTilesPerTx: number = DEFAULT_MAX_TILES_PER_TX,
  separate = false,
): number[][] {
  if (tiles.length === 0) {
    throw new PocketError("invalid_request", "choose at least one tile");
  }
  if (new Set(tiles).size !== tiles.length) {
    throw new PocketError("invalid_request", "tiles must be distinct");
  }
  const size = separate ? 1 : Math.floor(maxTilesPerTx);
  if (!Number.isFinite(size) || size < 1 || size > 25) {
    throw new PocketError("invalid_request", "maxTilesPerTx must be between 1 and 25");
  }
  const batches: number[][] = [];
  for (let i = 0; i < tiles.length; i += size) {
    batches.push(tiles.slice(i, i + size));
  }
  return batches;
}

/** The message a partial dig reports: what was sent, what was not. */
export function describePartial(
  sent: readonly { txid: string; tiles: readonly number[] }[],
  unsent: readonly number[],
  cause: string,
): string {
  const done =
    sent.length === 0
      ? "nothing was sent"
      : sent.map((b) => `tiles ${b.tiles.join(",")} in ${b.txid}`).join("; ");
  return `dig stopped: ${cause}. Sent: ${done}. Not sent: tiles ${unsent.join(",")} (send them again in a new dig; sent batches are never resent).`;
}
