/**
 * Owns the one WebSocket and merges its events into the query cache: board
 * events replace the cached board; round and payout events invalidate what
 * they change; campfire events refresh the feed.
 */

import { useQueryClient } from "@tanstack/react-query";
import { createContext, useEffect, useState, type ReactNode } from "react";
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
      cache.invalidate(["board"]);
      return;
    case "campfire":
      cache.invalidate(["campfire"]);
      return;
    case "landing":
      cache.setLanding(event.digs);
      return;
  }
}

export function LiveProvider({ children }: { children: ReactNode }) {
  const client = useQueryClient();
  const [state, setState] = useState<LiveState>("connecting");
  useEffect(() => {
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
          invalidate: (key) => void client.invalidateQueries({ queryKey: key }),
        });
      },
    });
    live.start();
    return () => {
      live.stop();
    };
  }, [client]);
  return <LiveContext.Provider value={state}>{children}</LiveContext.Provider>;
}
