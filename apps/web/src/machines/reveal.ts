/**
 * The reveal sequence, driven by the real round result: closed (waiting
 * for the draw) → draw → shake → burst → result. With reduced motion the
 * sequence goes straight from closed to result (cross-fade).
 */

export type RevealStage = "closed" | "draw" | "shake" | "burst" | "result";

export interface RevealResult {
  round: number;
  treasureTile: number | null;
  cacheTriggered: boolean;
  voided: boolean;
}

export type RevealState = {
  stage: RevealStage;
  result: RevealResult | null;
  reducedMotion: boolean;
};

export type RevealEvent =
  { type: "settled"; result: RevealResult } | { type: "advance" } | { type: "skip" };

/**
 * How long each stage plays before advancing (ms), from the design's reveal
 * timeline (Zeek Motion 9a–9d): dim + star 0–860, cracks + tremble
 * 860–1700, split + light burst 1700–2600, result card at 2600.
 */
export const STAGE_MS: Record<Exclude<RevealStage, "closed" | "result">, number> = {
  draw: 860,
  shake: 840,
  burst: 900,
};

export function initialReveal(reducedMotion: boolean): RevealState {
  return { stage: "closed", result: null, reducedMotion };
}

const NEXT: Record<RevealStage, RevealStage> = {
  closed: "draw",
  draw: "shake",
  shake: "burst",
  burst: "result",
  result: "result",
};

export function revealReducer(state: RevealState, event: RevealEvent): RevealState {
  switch (event.type) {
    case "settled":
      if (state.result !== null) return state;
      // A voided round has no draw to show.
      if (state.reducedMotion || event.result.voided)
        return { ...state, stage: "result", result: event.result };
      return { ...state, stage: "draw", result: event.result };
    case "advance":
      if (state.result === null) return state;
      return { ...state, stage: NEXT[state.stage] };
    case "skip":
      return state.result === null ? state : { ...state, stage: "result" };
  }
}

/** What the result card says for a player who dug `myTiles` (0-based). */
export type Outcome = "win" | "cache-win" | "not-this-time" | "watched" | "voided";

export function outcomeFor(result: RevealResult, myTiles: readonly number[]): Outcome {
  if (result.voided) return "voided";
  if (myTiles.length === 0) return "watched";
  if (result.treasureTile !== null && myTiles.includes(result.treasureTile)) {
    return result.cacheTriggered ? "cache-win" : "win";
  }
  return "not-this-time";
}

/**
 * The sound a result plays, only on the player's own round: coin then win
 * when they found the treasure, miss for every other outcome (missed,
 * nobody found it, voided and refunded). A round only watched is silent.
 */
export type ResultSound = "win" | "miss";

export function resultSound(outcome: Outcome, own: boolean): ResultSound | null {
  if (!own) return null;
  switch (outcome) {
    case "win":
    case "cache-win":
      return "win";
    case "watched":
      return null;
    case "not-this-time":
    case "voided":
      return "miss";
  }
}

/**
 * The player's share of a drawn round: their amount on the treasure tile
 * over the tile's total, of the distributable prize plus the Cache paid
 * (truncated, as the rules do). Zatoshis.
 */
export function estimatedShare(
  myOnTreasure: bigint,
  tileTotal: bigint,
  distributable: bigint,
  cachePaid: bigint,
): bigint {
  if (tileTotal === 0n || myOnTreasure === 0n) return 0n;
  return ((distributable + cachePaid) * myOnTreasure) / tileTotal;
}
