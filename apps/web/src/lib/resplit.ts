/**
 * Re-splitting the change after a dig: when a dig's change has confirmed and
 * the Pocket is down to one coin big enough to dig with, the Pocket splits
 * it again ("Getting your coins ready"), so the next digs in a round need
 * not wait for change. The size and fee rule is the deposit split's own
 * (crates/zeek-pocket `plan_split`: parts of at least 0.02 ZEC, 2 to 4 of
 * them, fee at most 1 %), so `plan` decides whether it is worth it; this
 * decides when. Never while a dig, a withdrawal or an auto-dig is under way.
 */

import type { SplitPlan } from "@zeek/pocket";

/** A coin a dig can use on its own (crates/zeek-pocket `SPLIT_PART_MIN_ZAT`). */
export const SPLIT_PART_MIN_ZAT = 2_000_000;

export interface ResplitState {
  /** A dig (or the Dig screen), a withdrawal or an auto-dig is under way. */
  busy: boolean;
  /** Value still confirming (a dig's change among it). */
  pendingZat: number;
  /** Values of the spendable notes. */
  spendableNotes: readonly number[];
  /** The split the Pocket would make now, or null. */
  plan: SplitPlan | null;
}

export type ResplitDecision =
  | { kind: "split"; plan: SplitPlan }
  /** Ask again later. */
  | { kind: "wait"; reason: "busy" | "confirming" }
  /** Nothing to do for this dig. */
  | { kind: "none"; reason: "several-coins" | "no-coin" | "not-worth-it" };

export function resplitDecision(s: ResplitState): ResplitDecision {
  if (s.busy) return { kind: "wait", reason: "busy" };
  // Only once the change is in: until then it is not one coin yet.
  if (s.pendingZat > 0) return { kind: "wait", reason: "confirming" };
  const usable = s.spendableNotes.filter((v) => v >= SPLIT_PART_MIN_ZAT).length;
  if (usable >= 2) return { kind: "none", reason: "several-coins" };
  if (usable === 0) return { kind: "none", reason: "no-coin" };
  if (s.plan === null) return { kind: "none", reason: "not-worth-it" };
  return { kind: "split", plan: s.plan };
}
