/**
 * The Pocket dig flow: idle → confirm → sealing (per batch: packing,
 * sealing privately, sending) → done, or partial (some batches sent, which
 * are never resent) or error.
 */

import type { DigProgress } from "@zeek/pocket";

export interface SentBatch {
  txid: string;
  tiles: number[];
}

export type DigState =
  | { kind: "idle" }
  | { kind: "confirm" }
  | {
      kind: "sealing";
      batches: number[][];
      current: number;
      step: 0 | 1 | 2;
      waiting: { for: "privacy-gap" | "change"; seconds: number } | null;
      sent: SentBatch[];
    }
  | { kind: "done"; round: number; sent: SentBatch[] }
  | { kind: "partial"; sent: SentBatch[]; unsent: number[]; message: string }
  | { kind: "error"; code: string; message: string };

export type DigEvent =
  | { type: "review" }
  | { type: "cancel" }
  | { type: "start"; batches: number[][] }
  | { type: "progress"; progress: DigProgress }
  | { type: "done"; round: number; sent: SentBatch[] }
  | {
      type: "failed";
      code: string;
      message: string;
      partial?: { sent: SentBatch[]; unsent: number[] };
    };

export const initialDig: DigState = { kind: "idle" };

/** The design's three seal steps. */
export const SEAL_STEPS = ["Packing tiles", "Sealing privately", "Sending to the field"] as const;

function stepOf(phase: DigProgress["phase"]): 0 | 1 | 2 {
  switch (phase) {
    case "sync":
    case "waiting":
      return 0;
    case "proving":
      return 1;
    case "broadcast":
    case "sent":
      return 2;
  }
}

export function digReducer(state: DigState, event: DigEvent): DigState {
  switch (event.type) {
    case "review":
      return state.kind === "idle" || state.kind === "error" || state.kind === "done"
        ? { kind: "confirm" }
        : state;
    case "cancel":
      return state.kind === "sealing" ? state : { kind: "idle" };
    case "start":
      return state.kind === "confirm"
        ? { kind: "sealing", batches: event.batches, current: 0, step: 0, waiting: null, sent: [] }
        : state;
    case "progress": {
      if (state.kind !== "sealing") return state;
      const p = event.progress;
      const sent =
        p.phase === "sent" && p.txid !== undefined && !state.sent.some((b) => b.txid === p.txid)
          ? [...state.sent, { txid: p.txid, tiles: p.tiles }]
          : state.sent;
      return {
        ...state,
        current: p.batch - 1,
        step: stepOf(p.phase),
        waiting:
          p.phase === "waiting" && p.waitingFor !== undefined
            ? { for: p.waitingFor, seconds: p.seconds ?? 0 }
            : null,
        sent,
      };
    }
    case "done":
      return { kind: "done", round: event.round, sent: event.sent };
    case "failed":
      if (event.partial !== undefined && event.partial.sent.length > 0) {
        return {
          kind: "partial",
          sent: event.partial.sent,
          unsent: event.partial.unsent,
          message: event.message,
        };
      }
      return { kind: "error", code: event.code, message: event.message };
  }
}
