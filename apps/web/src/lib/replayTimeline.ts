/**
 * Where a replayed round is at a given moment: the field fills block by
 * block (compressed time), the round closes, the reveal plays with the
 * design's stage timings, then the result shows before the next round.
 */

import { STAGE_MS } from "../machines/reveal";

/** Playback time per block of the round's window. */
export const BLOCK_MS = 450;
export const INTRO_MS = 1_500;
export const CLOSED_MS = 1_200;
export const RESULT_MS = 6_000;

export type ReplayPhase = "intro" | "filling" | "closed" | "draw" | "shake" | "burst" | "result";

export interface ReplayFrameInput {
  window_start: number;
  window_end: number;
  frames: { height: number; stages: number[] }[];
}

export interface ReplayMoment {
  phase: ReplayPhase;
  /** Depth stages to draw now. */
  stages: number[];
  /** The block being played (filling), else null. */
  height: number | null;
  /** The round has played out. */
  done: boolean;
}

const ZERO = Array.from({ length: 25 }, () => 0);

/** Total playback length of a round. */
export function roundLength(r: ReplayFrameInput): number {
  const blocks = r.window_end - r.window_start;
  return (
    INTRO_MS +
    blocks * BLOCK_MS +
    CLOSED_MS +
    STAGE_MS.draw +
    STAGE_MS.shake +
    STAGE_MS.burst +
    RESULT_MS
  );
}

export function momentAt(r: ReplayFrameInput, elapsed: number): ReplayMoment {
  const blocks = r.window_end - r.window_start;
  const fill = INTRO_MS + blocks * BLOCK_MS;
  const last = r.frames.at(-1)?.stages ?? ZERO;
  if (elapsed < INTRO_MS) return { phase: "intro", stages: ZERO, height: null, done: false };
  if (elapsed < fill) {
    const height = r.window_start + Math.floor((elapsed - INTRO_MS) / BLOCK_MS);
    const shown = r.frames.filter((f) => f.height <= height).at(-1)?.stages ?? ZERO;
    return { phase: "filling", stages: shown, height, done: false };
  }
  let t = elapsed - fill;
  const steps: [ReplayPhase, number][] = [
    ["closed", CLOSED_MS],
    ["draw", STAGE_MS.draw],
    ["shake", STAGE_MS.shake],
    ["burst", STAGE_MS.burst],
    ["result", RESULT_MS],
  ];
  for (const [phase, ms] of steps) {
    if (t < ms) return { phase, stages: last, height: null, done: false };
    t -= ms;
  }
  return { phase: "result", stages: last, height: null, done: true };
}
