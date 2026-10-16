/**
 * When confirming coins become spendable, in words people can use. The
 * Pocket reports blocks (its own confirmation policy: received coins need
 * 10 confirmations, its own change 3); the engine's board reports how fast
 * blocks have come lately.
 */

import type { Zat } from "./money";

/** Blocks a dig needs left in its round (the Pocket's own refusal margin + 1). */
export const DIG_MARGIN_BLOCKS = 4;

/** Seconds per block to estimate with: the recent rate, else the target. */
export function spacingSecs(recent: number | null | undefined, target: number | undefined): number {
  if (recent !== null && recent !== undefined && recent > 0) return recent;
  return target !== undefined && target > 0 ? target : 25;
}

/** "about 3 min", or "in under a minute". */
export function readyIn(blocks: number, spacing: number): string {
  const secs = Math.max(0, blocks) * spacing;
  if (secs < 60) return "in under a minute";
  return `in about ${Math.round(secs / 60)} min`;
}

/** Seconds until `blocks` more blocks, for countdowns. */
export function readySecs(blocks: number, spacing: number): number {
  return Math.max(0, blocks) * spacing;
}

export type Shortfall =
  | { kind: "enough" }
  /** Spendable alone is short, but confirming coins cover it. */
  | { kind: "confirming"; blocks: number }
  | { kind: "short" };

export function shortfall(
  need: Zat,
  spendable: Zat,
  pending: Zat,
  allReadyInBlocks: number | null,
): Shortfall {
  if (spendable >= need) return { kind: "enough" };
  if (spendable + pending >= need && allReadyInBlocks !== null) {
    return { kind: "confirming", blocks: allReadyInBlocks };
  }
  return { kind: "short" };
}

/**
 * Whether coins ready in `readyBlocks` can still dig in a round with
 * `blocksRemaining` left (the dig must start with DIG_MARGIN_BLOCKS left).
 */
export function fitsInRound(readyBlocks: number, blocksRemaining: number): boolean {
  return blocksRemaining - readyBlocks >= DIG_MARGIN_BLOCKS;
}

/**
 * Whether a dig can still go into this round: it must be open with at least
 * DIG_MARGIN_BLOCKS left (the Pocket refuses with fewer, since a payment
 * sent later may not be mined inside the window).
 */
export function roundAcceptsDigs(board: { status: string; blocks_remaining: number }): boolean {
  return board.status === "open" && board.blocks_remaining >= DIG_MARGIN_BLOCKS;
}
