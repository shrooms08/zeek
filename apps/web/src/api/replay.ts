/**
 * Replay mode's saved rounds: static files the deploy wrote into the site
 * (scripts/snapshot-rounds.mjs), copied from the engine. Real rounds only;
 * served without the engine and without Pages Functions.
 */

import { useQuery } from "@tanstack/react-query";

export interface ReplayIndex {
  saved_at: number;
  network: "test" | "main";
  rounds: {
    round: number;
    status: string;
    treasure_tile: number | null;
    closed_time: number | null;
  }[];
}

/** `zeek_verify::replay::Replay`: depth stages per block, never amounts per tile. */
export interface ReplayRound {
  round: number;
  window_start: number;
  window_end: number;
  start_block_time: number;
  frames: { height: number; landed: number[]; stages: number[] }[];
  outcome:
    { outcome: "drawn"; treasure_tile: number; cache_triggered: boolean } | { outcome: "voided" };
  pot: number;
  winners: number;
  total_paid: number;
  previous_cache: number;
  new_cache: number;
  cache_paid: number;
  drand_round: number;
  payout_txid: string | null;
}

/** A saved file, or null when this build has none (the site answers with its page). */
async function saved(path: string): Promise<Response | null> {
  const response = await fetch(`/replay/${path}`);
  const json = (response.headers.get("content-type") ?? "").includes("application/json");
  return response.ok && json ? response : null;
}

export function useReplayIndex() {
  return useQuery({
    queryKey: ["replay", "index"],
    queryFn: async () => {
      const r = await saved("index.json");
      return r === null ? null : ((await r.json()) as ReplayIndex);
    },
    staleTime: Infinity,
    retry: false,
  });
}

export function useReplayRound(round: number | undefined) {
  return useQuery({
    queryKey: ["replay", "round", round],
    queryFn: async () => {
      const r = await saved(`rounds/${round ?? 0}.replay.json`);
      if (r === null) throw new Error(`round ${round ?? 0} is not saved`);
      return (await r.json()) as ReplayRound;
    },
    enabled: round !== undefined,
    staleTime: Infinity,
  });
}

/** The saved proof bundle and public config, as text (for the offline checks). */
export async function savedBundleAndConfig(
  round: number,
): Promise<{ bundle: string; config: string }> {
  const [b, c] = await Promise.all([saved(`rounds/${round}.proof.json`), saved("config.json")]);
  if (b === null || c === null) throw new Error(`round ${round} is not saved in this build`);
  return { bundle: await b.text(), config: await c.text() };
}
