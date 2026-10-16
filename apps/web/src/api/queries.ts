/** TanStack Query hooks over the engine API. */

import { useQuery } from "@tanstack/react-query";
import { useContext } from "react";
import { spacingSecs } from "../lib/readiness";
import { engine, EngineError } from "./engine";
import { LiveContext } from "./LiveProvider";

/**
 * Polls only while the WebSocket is down for an ordinary reason: never while
 * it is open (it pushes changes) or paused (the tab is hidden), and never
 * while the engine is offline or the site is busy, when Replay mode's one
 * health probe decides when to come back. Every poll on the public site is
 * one request of its free daily allowance.
 */
function useFallbackInterval(ms: number) {
  const live = useContext(LiveContext);
  return (query: { state: { error: unknown } }): number | false => {
    if (live === "open" || live === "paused") return false;
    const e = query.state.error;
    if (e instanceof EngineError && (e.kind === "offline" || e.kind === "busy")) return false;
    return ms;
  };
}

export const keys = {
  board: ["board"] as const,
  config: ["config"] as const,
  pending: ["pending"] as const,
  round: (n: number) => ["round", n] as const,
  rounds: ["rounds"] as const,
  ledger: ["ledger"] as const,
  keyholders: ["keyholders"] as const,
  status: ["status"] as const,
  campfire: (hidden: boolean) => ["campfire", hidden] as const,
};

export function useBoard() {
  return useQuery({
    queryKey: keys.board,
    queryFn: engine.board,
    refetchInterval: useFallbackInterval(10_000),
  });
}

export function useConfig() {
  return useQuery({ queryKey: keys.config, queryFn: engine.config, staleTime: Infinity });
}

export function usePending() {
  return useQuery({
    queryKey: keys.pending,
    queryFn: engine.pending,
    refetchInterval: useFallbackInterval(15_000),
  });
}

export function useRound(n: number | undefined) {
  return useQuery({
    queryKey: keys.round(n ?? -1),
    queryFn: () => engine.round(n ?? 0),
    enabled: n !== undefined && Number.isInteger(n) && n >= 0,
    refetchInterval: useFallbackInterval(15_000),
  });
}

export function useRounds() {
  return useQuery({
    queryKey: keys.rounds,
    queryFn: () => engine.rounds(),
    refetchInterval: useFallbackInterval(30_000),
  });
}

export function useLedger() {
  return useQuery({
    queryKey: keys.ledger,
    queryFn: engine.ledger,
    refetchInterval: useFallbackInterval(30_000),
  });
}

export function useKeyholders() {
  // Reachability comes from the engine's probes (once a minute); refresh it
  // at that pace regardless of the stream.
  return useQuery({
    queryKey: keys.keyholders,
    queryFn: engine.keyholders,
    refetchInterval: 60_000,
  });
}

export function useCampfire(includeHidden: boolean) {
  return useQuery({
    queryKey: keys.campfire(includeHidden),
    queryFn: () => engine.campfire(includeHidden),
    refetchInterval: useFallbackInterval(20_000),
  });
}

/** Seconds per block to estimate waits with: the recent rate, else the target. */
export function useBlockSpacing(): number {
  const board = useBoard();
  const config = useConfig();
  return spacingSecs(board.data?.recent_spacing_secs, config.data?.params.target_spacing_secs);
}
