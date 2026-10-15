/**
 * Owns the one WebSocket and merges its events into the query cache: board
 * events replace the cached board; round and payout events invalidate what
 * they change (coalesced, since they come in bursts); campfire events
 * refresh the feed. The board itself is never refetched for a round event:
 * the stream pushes it with every block.
 *
 * A tab hidden for a minute pauses the stream (no events, no refetches, no
 * requests); on return it reconnects at once and refreshes what it missed.
 */

import { useQueryClient } from "@tanstack/react-query";
import { createContext, useEffect, useState, type ReactNode } from "react";
import { onVisibilityChange } from "../lib/visibility";
import { ENGINE_URL } from "./engine";
import { LiveClient, streamUrl, type LiveState } from "./live";
import type { Board, LandingDig, LiveEvent } from "./types";

export const LiveContext = createContext<LiveState>("connecting");

/** Applies one event to the cache. Exported for tests. */
export function applyEvent(
  event: LiveEvent,
  cache: {
    setBoard: (board: Board) => void;
    setLanding: (digs: LandingDig[]) => void;
    invalidate: (key: readonly unknown[]) => void;
  },
): void {
  switch (event.event) {
    case "board": {
      const board: Omit<typeof event, "event"> & { event?: string } = { ...event };
      delete board.event;
      cache.setBoard(board);
      return;
    }
    case "round_closed":
    case "reveal":
    case "settled":
    case "paid":
      cache.invalidate(["pending"]);
      cache.invalidate(["rounds"]);
      cache.invalidate(["round", event.round]);
      cache.invalidate(["ledger"]);
      return;
    case "campfire":
      cache.invalidate(["campfire"]);
      return;
    case "landing":
      cache.setLanding(event.digs);
      return;
  }
}

/** How long a hidden tab keeps the stream open (quick tab switches). */
export const HIDDEN_GRACE_MS = 60_000;
/** Round and payout events come in bursts: their refetches wait this long. */
const COALESCE_MS = 1_500;

let current: LiveClient | undefined;

/** Reconnects the stream now (the engine answered again). */
export function resumeLive(): void {
  current?.resume();
}

export function LiveProvider({ children }: { children: ReactNode }) {
  const client = useQueryClient();
  const [state, setState] = useState<LiveState>("connecting");
  useEffect(() => {
    const pending = new Map<string, readonly unknown[]>();
    let flush: number | undefined;
    const invalidate = (key: readonly unknown[]) => {
      pending.set(JSON.stringify(key), key);
      if (flush !== undefined) return;
      flush = window.setTimeout(() => {
        flush = undefined;
        for (const k of pending.values()) void client.invalidateQueries({ queryKey: k });
        pending.clear();
      }, COALESCE_MS);
    };
    const live = new LiveClient({
      url: streamUrl(ENGINE_URL),
      onState: setState,
      onEvent: (event) => {
        applyEvent(event, {
          // A board event carries no landing digs: keep the last known set.
          setBoard: (board) =>
            client.setQueryData<Board>(["board"], (old) => ({
              ...board,
              landing: board.landing ?? old?.landing ?? [],
            })),
          setLanding: (digs) =>
            client.setQueryData<Board>(["board"], (old) =>
              old === undefined ? old : { ...old, landing: digs },
            ),
          invalidate,
        });
      },
    });
    current = live;
    live.start();
    let hideTimer: number | undefined;
    let paused = false;
    const off = onVisibilityChange((hidden) => {
      window.clearTimeout(hideTimer);
      if (hidden) {
        hideTimer = window.setTimeout(() => {
          paused = true;
          live.pause();
        }, HIDDEN_GRACE_MS);
      } else if (paused) {
        paused = false;
        live.resume();
        // Round events missed while paused (the board comes with the stream).
        for (const key of [["pending"], ["rounds"], ["ledger"], ["campfire"]] as const) {
          void client.invalidateQueries({ queryKey: key });
        }
      }
    });
    return () => {
      off();
      window.clearTimeout(hideTimer);
      window.clearTimeout(flush);
      live.stop();
      if (current === live) current = undefined;
    };
  }, [client]);
  return <LiveContext.Provider value={state}>{children}</LiveContext.Provider>;
}
