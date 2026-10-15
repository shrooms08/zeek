import { describe, expect, it } from "vitest";
import { BLOCK_MS, INTRO_MS, momentAt, roundLength } from "./replayTimeline";

const stages = (v: number) => Array.from({ length: 25 }, () => v);
const round = {
  window_start: 1000,
  window_end: 1024,
  frames: [
    { height: 1003, stages: stages(1) },
    { height: 1010, stages: stages(3) },
  ],
};

describe("replay timeline", () => {
  it("starts on an empty field", () => {
    expect(momentAt(round, 0)).toMatchObject({ phase: "intro", stages: stages(0) });
  });

  it("shows each block's stages only once that block is played", () => {
    expect(momentAt(round, INTRO_MS + 2 * BLOCK_MS).stages).toEqual(stages(0));
    expect(momentAt(round, INTRO_MS + 3 * BLOCK_MS)).toMatchObject({
      phase: "filling",
      height: 1003,
      stages: stages(1),
    });
    expect(momentAt(round, INTRO_MS + 12 * BLOCK_MS).stages).toEqual(stages(3));
  });

  it("closes, reveals and ends in order", () => {
    const fill = INTRO_MS + 24 * BLOCK_MS;
    const phases = [fill + 1, fill + 1300, fill + 2100, fill + 3000, fill + 3900].map(
      (t) => momentAt(round, t).phase,
    );
    expect(phases).toEqual(["closed", "draw", "shake", "burst", "result"]);
    expect(momentAt(round, roundLength(round)).done).toBe(true);
    expect(momentAt(round, roundLength(round) - 1).done).toBe(false);
  });

  it("an empty round stays empty", () => {
    const empty = { window_start: 0, window_end: 24, frames: [] };
    expect(momentAt(empty, INTRO_MS + 30 * BLOCK_MS).stages).toEqual(stages(0));
  });
});
