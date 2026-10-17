/**
 * The background re-split (src/lib/resplit.ts): after a dig from the Pocket,
 * every RESPLIT_CHECK_MS while the Pocket is unlocked and nothing else is
 * under way, sync and ask whether the change should be split again. One
 * check at a time; it stops looking once it split or found nothing to do.
 */

import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { usePocket } from "../api/PocketProvider";
import { resplitDecision } from "../lib/resplit";
import { autoDigRunning } from "./autoDigFlag";
import { disarmResplit, resplitArmed, withdrawing } from "./pocketActivity";

/** Change confirms in about three blocks; no need to look more often. */
const RESPLIT_CHECK_MS = 45_000;

/** A dig (the Dig screen is open), an auto-dig or a withdrawal. */
function busy(): boolean {
  return window.location.pathname.startsWith("/dig/") || autoDigRunning() || withdrawing();
}

export function ResplitScheduler() {
  const { pocket, wallet } = usePocket();
  const client = useQueryClient();
  const unlocked = wallet.kind === "unlocked";
  useEffect(() => {
    if (!unlocked) return;
    let checking = false;
    const check = async () => {
      if (checking || !resplitArmed() || busy()) return;
      checking = true;
      try {
        const p = await pocket();
        const balance = await p.sync();
        const decision = resplitDecision({
          // Asked again after the sync: a dig may have started meanwhile.
          busy: busy(),
          pendingZat: balance.pending_zat,
          spendableNotes: balance.spendable_notes,
          plan: await p.splitPlan(),
        });
        if (decision.kind === "wait") return;
        if (decision.kind === "split") {
          if (busy()) return;
          await p.prepareCoins();
          void client.invalidateQueries({ queryKey: ["pocket"] });
        }
        disarmResplit();
      } catch {
        // A failed sync or split is tried again at the next check; a dig
        // never depends on it.
      } finally {
        checking = false;
      }
    };
    const id = window.setInterval(() => void check(), RESPLIT_CHECK_MS);
    return () => {
      window.clearInterval(id);
    };
  }, [unlocked, pocket, client]);
  return null;
}
