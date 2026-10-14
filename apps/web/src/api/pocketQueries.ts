/** Pocket data as queries (enabled only while the wallet is unlocked). */

import type { Balance, HistoryEntry } from "@zeek/pocket";
import { useQuery } from "@tanstack/react-query";
import { usePocket } from "./PocketProvider";

export function usePocketBalance() {
  const { pocket, wallet } = usePocket();
  const unlocked = wallet.kind === "unlocked" || wallet.kind === "syncing";
  return useQuery<Balance>({
    queryKey: ["pocket", "balance"],
    queryFn: async () => (await pocket()).getBalance(),
    enabled: unlocked,
    refetchInterval: 15_000,
  });
}

export function usePocketHistory() {
  const { pocket, wallet } = usePocket();
  const unlocked = wallet.kind === "unlocked" || wallet.kind === "syncing";
  return useQuery<HistoryEntry[]>({
    queryKey: ["pocket", "history"],
    queryFn: async () => (await pocket()).history(),
    enabled: unlocked,
  });
}
