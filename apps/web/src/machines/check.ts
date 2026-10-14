/**
 * Check a round: five rows, each pending → checking → pass | fail (or not
 * available), plus the second-source cross-check. Rows tick from the
 * verifier's progress, then results are revealed one by one.
 */

import type { RoundVerification, VerifyProgress } from "@zeek/pocket";

export type RowState = "pending" | "checking" | "pass" | "fail" | "na";

export interface CheckRowModel {
  id: "beacon" | "digs" | "timing" | "settlement" | "payout";
  title: string;
  state: RowState;
  text: string;
  detail: string[];
}

export type CheckState =
  | { kind: "idle" }
  | { kind: "running"; rows: CheckRowModel[]; step: VerifyProgress["step"] }
  | { kind: "revealing"; rows: CheckRowModel[]; shown: number; result: RoundVerification }
  | { kind: "done"; rows: CheckRowModel[]; result: RoundVerification }
  | { kind: "error"; message: string };

export type CheckEvent =
  | { type: "start" }
  | { type: "progress"; progress: VerifyProgress }
  | { type: "result"; result: RoundVerification }
  | { type: "tick" }
  | { type: "failed"; message: string };

/** The design's plain-language titles. */
export const TITLES: Record<CheckRowModel["id"], string> = {
  beacon: "The draw is genuine",
  digs: "Every dig is on the chain",
  timing: "No late digs",
  settlement: "The math matches",
  payout: "The payout matches",
};

const ORDER: CheckRowModel["id"][] = ["beacon", "digs", "timing", "settlement", "payout"];

function freshRows(): CheckRowModel[] {
  return ORDER.map((id) => ({
    id,
    title: TITLES[id],
    state: "pending",
    text: "Waiting for the step above.",
    detail: [],
  }));
}

/** Which rows are being worked on at each verifier step. */
const CHECKING: Record<VerifyProgress["step"], number> = {
  bundle: 0,
  blocks: 1,
  scan: 1,
  transactions: 2,
  checks: 4,
  "cross-check": 5,
  done: 5,
};

export function checkReducer(state: CheckState, event: CheckEvent): CheckState {
  switch (event.type) {
    case "start":
      return {
        kind: "running",
        rows: freshRows().map((r, i) =>
          i === 0 ? { ...r, state: "checking", text: "Fetching the round…" } : r,
        ),
        step: "bundle",
      };
    case "progress": {
      if (state.kind !== "running") return state;
      const upTo = CHECKING[event.progress.step];
      return {
        kind: "running",
        step: event.progress.step,
        rows: state.rows.map((r, i) =>
          i <= upTo ? { ...r, state: "checking", text: event.progress.detail } : r,
        ),
      };
    }
    case "result": {
      if (state.kind !== "running") return state;
      const rows = ORDER.map((id) => {
        const c = event.result.report.checks.find((x) => x.id === id);
        const resultState: RowState =
          c === undefined
            ? "na"
            : c.status === "pass"
              ? "pass"
              : c.status === "fail"
                ? "fail"
                : "na";
        return {
          id,
          title: TITLES[id],
          state: resultState,
          text: c?.summary ?? "",
          detail: c?.detail ?? [],
        };
      });
      return { kind: "revealing", rows, shown: 0, result: event.result };
    }
    case "tick":
      if (state.kind !== "revealing") return state;
      if (state.shown + 2 >= state.rows.length)
        return { kind: "done", rows: state.rows, result: state.result };
      return { ...state, shown: state.shown + 1 };
    case "failed":
      return { kind: "error", message: event.message };
  }
}

/** Rows as displayed while revealing: later rows still "checking". */
export function visibleRows(state: CheckState): CheckRowModel[] {
  switch (state.kind) {
    case "idle":
      return freshRows();
    case "running":
    case "done":
      return state.rows;
    case "revealing":
      return state.rows.map((r, i) =>
        i <= state.shown ? r : { ...r, state: "checking", text: "Checking…", detail: [] },
      );
    case "error":
      return freshRows();
  }
}
