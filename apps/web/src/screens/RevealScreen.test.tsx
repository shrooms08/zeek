import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** The sound player, recorded. */
const played = vi.hoisted(() => ({ results: [] as string[], effects: [] as string[] }));
vi.mock("../sound/sound", () => ({
  sound: {
    play: (name: string) => played.effects.push(name),
    playResult: (kind: string) => {
      played.results.push(kind);
      return () => undefined;
    },
    startLoop: () => undefined,
    stopLoop: () => undefined,
    releaseDuck: () => undefined,
    duckMusic: () => undefined,
    releaseMusic: () => undefined,
  },
}));

/** Round 5, settled: tile 7 (0-based) held the treasure, or the round was voided. */
const round = vi.hoisted(() => ({ voided: false }));
vi.mock("../api/queries", () => ({
  useRound: () => ({
    isPending: false,
    isError: false,
    data: {
      round: 5,
      depths: Array<number>(25).fill(0),
      settled: {
        outcome: round.voided
          ? { outcome: "voided" }
          : { outcome: "drawn", treasure_tile: 7, cache_triggered: false },
        tile_totals: Array<number>(25).fill(100_000),
        drand_round: 1,
        distributable: 2_000_000,
        winners: 1,
        previous_cache: 0,
        new_cache: 100_000,
        cache_paid: 0,
      },
    },
  }),
  usePending: () => ({ data: [] }),
  useBoard: () => ({ data: undefined }),
  useConfig: () => ({ data: undefined }),
}));

const { default: RevealScreen } = await import("./RevealScreen");

/** Lets the sequence run: each stage's timer is set once the last one has rendered. */
function run(ms: number) {
  for (let t = 0; t < ms; t += 250) {
    act(() => {
      vi.advanceTimersByTime(250);
    });
  }
}

function dug(tiles: number[]) {
  localStorage.setItem(
    "zeek.myDigs.v1",
    JSON.stringify([{ round: 5, at: 1, tiles, amountEach: "100000", via: "pocket" }]),
  );
}

function open() {
  return render(
    <MemoryRouter initialEntries={["/reveal/5"]}>
      <Routes>
        <Route path="/reveal/:round" element={<RevealScreen />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("the reveal's result sound", () => {
  beforeEach(() => {
    localStorage.clear();
    played.results = [];
    played.effects = [];
    round.voided = false;
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: false,
      media: query,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      addListener: () => undefined,
      removeListener: () => undefined,
    }));
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
    vi.useFakeTimers();
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("Skip to the result: the miss plays exactly once", () => {
    dug([2]);
    open();
    fireEvent.click(screen.getByText("Skip to the result"));
    expect(screen.getByTestId("result-card").getAttribute("data-outcome")).toBe("not-this-time");
    expect(played.results).toEqual(["miss"]);
    // The sequence's own timers and re-renders do not repeat it.
    run(5_000);
    expect(played.results).toEqual(["miss"]);
  });

  it("the whole sequence: draw, rays, then the win once", () => {
    dug([7]);
    open();
    run(5_000);
    expect(screen.getByTestId("result-card").getAttribute("data-outcome")).toBe("win");
    expect(played.effects).toEqual(["rumble", "crack", "draw"]);
    expect(played.results).toEqual(["win"]);
  });

  it("a voided round the player dug in: miss", () => {
    round.voided = true;
    dug([2]);
    open();
    expect(screen.getByTestId("result-card").getAttribute("data-outcome")).toBe("voided");
    expect(played.results).toEqual(["miss"]);
  });

  it("a round only watched is silent", () => {
    open();
    run(5_000);
    expect(screen.getByTestId("result-card").getAttribute("data-outcome")).toBe("watched");
    expect(played.results).toEqual([]);
    expect(played.effects).toEqual([]);
  });
});
