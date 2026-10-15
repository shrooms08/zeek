/** TanStack Query hooks over the engine API. */

import { useQuery } from "@tanstack/react-query";
import { useContext } from "react";
import { engine } from "./engine";
import { LiveContext } from "./LiveProvider";

/** Polls only while the WebSocket is down (it pushes changes when up). */
function useFallbackInterval(ms: number): number | false {
  return useContext(LiveContext) === "open" ? false : ms;
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
  // Reachability comes from the engine's probes; refresh it regardless.
  return useQuery({
    queryKey: keys.keyholders,
    queryFn: engine.keyholders,
    refetchInterval: 20_000,
  });
}

export function useCampfire(includeHidden: boolean) {
  return useQuery({
    queryKey: keys.campfire(includeHidden),
    queryFn: () => engine.campfire(includeHidden),
    refetchInterval: useFallbackInterval(20_000),
  });
}
