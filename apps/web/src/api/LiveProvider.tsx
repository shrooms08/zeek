/**
 * Owns the one WebSocket and merges its events into the query cache: board
 * events replace the cached board; round and payout events invalidate what
 * they change; campfire events refresh the feed.
 */

import { useQueryClient } from "@tanstack/react-query";
import { createContext, useEffect, useState, type ReactNode } from "react";
import { ENGINE_URL } from "./engine";
import { LiveClient, streamUrl, type LiveState } from "./live";
import type { Board, LiveEvent } from "./types";

export const LiveContext = createContext<LiveState>("connecting");

/** Applies one event to the cache. Exported for tests. */
export function applyEvent(
  event: LiveEvent,
  cache: {
    setBoard: (board: Board) => void;
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
          setBoard: (board) => client.setQueryData(["board"], board),
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
