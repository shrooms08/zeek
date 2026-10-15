/**
 * Pro auto-dig: the Pocket digs the same selection once per new round, for
 * N rounds, while the tab stays open. The person confirms once (with their
 * passphrase); after that the run stops on its own, before any dig that
 * would break a rule:
 *
 * - the balance would not cover the next round's dig (dig + fees);
 * - the next dig would pass the total spend cap;
 * - the tab has been hidden for a whole round or longer;
 * - any error (a failed or part-way dig, a failed balance or board read);
 * - the Stop button (no further dig starts; a payment already being sent
 *   finishes, since a broadcast cannot be taken back).
 *
 * The controller is plain logic over injected effects (clock, board,
 * balance, dig, visibility), so every stop is unit-tested.
 */

/** What the controller needs from the outside world. */
export interface AutoDigDeps {
  now: () => number;
  sleep: (ms: number) => Promise<void>;
  /** The engine's current round. */
  board: () => Promise<{ round: number; status: string; blocks_remaining: number }>;
  /** The Pocket's spendable balance (after a sync). */
  spendable: () => Promise<bigint>;
  /** Digs the selection in the current round; resolves with that round. */
  dig: () => Promise<{ round: number; txids: string[] }>;
  /** When the tab was hidden (ms), or null while it is visible. */
  hiddenSince: () => number | null;
}

export interface AutoDigPlan {
  /** Rounds to dig, counting the first. */
  rounds: number;
  /** One round's cost: the digs plus all fees. */
  perRound: bigint;
  /** The most the whole run may spend. */
  cap: bigint;
  /** One round's length (ms), for the hidden-tab rule. */
  roundMs: number;
}

export type StopReason =
  | { kind: "finished" }
  | { kind: "stopped" }
  | { kind: "balance"; spendable: bigint }
  | { kind: "cap" }
  | { kind: "hidden" }
  | { kind: "error"; message: string };

export interface AutoDigProgress {
  done: number;
  spent: bigint;
  /** Rounds dug, in order. */
  rounds: number[];
  /** True while a dig is being sent. */
  sending: boolean;
}

/** Fewest blocks left in a round for a dig to start (it must land in time). */
export const MIN_BLOCKS_LEFT = 4;
/** How often the board is read while waiting for the next round. */
export const POLL_MS = 15_000;

export function stopMessage(reason: StopReason): string {
  switch (reason.kind) {
    case "finished":
      return "Done: every round you asked for is dug.";
    case "stopped":
      return "Stopped. No more digs will start.";
    case "balance":
      return "Stopped: your Pocket would not cover the next round.";
    case "cap":
      return "Stopped: the next round would pass your spend cap.";
    case "hidden":
      return "Stopped: this tab was in the background for more than a round.";
    case "error":
      return `Stopped after an error: ${reason.message}`;
  }
}

/**
 * Runs the plan. `onProgress` reports each change; `shouldStop` is the Stop
 * button (checked before every step). Resolves with why it ended.
 */
export async function runAutoDig(
  plan: AutoDigPlan,
  deps: AutoDigDeps,
  onProgress: (p: AutoDigProgress) => void,
  shouldStop: () => boolean,
): Promise<{ reason: StopReason; progress: AutoDigProgress }> {
  const progress: AutoDigProgress = { done: 0, spent: 0n, rounds: [], sending: false };
  const end = (reason: StopReason) => ({ reason, progress: { ...progress, sending: false } });
  let last = -1;
  try {
    while (progress.done < plan.rounds) {
      if (shouldStop()) return end({ kind: "stopped" });
      const hidden = deps.hiddenSince();
      if (hidden !== null && deps.now() - hidden >= plan.roundMs) return end({ kind: "hidden" });
      const board = await deps.board();
      if (shouldStop()) return end({ kind: "stopped" });
      const ready =
        board.status === "open" && board.round > last && board.blocks_remaining >= MIN_BLOCKS_LEFT;
      if (!ready) {
        await deps.sleep(POLL_MS);
        continue;
      }
      if (progress.spent + plan.perRound > plan.cap) return end({ kind: "cap" });
      const spendable = await deps.spendable();
      if (spendable < plan.perRound) return end({ kind: "balance", spendable });
      if (shouldStop()) return end({ kind: "stopped" });
      progress.sending = true;
      onProgress({ ...progress });
      const result = await deps.dig();
      progress.sending = false;
      progress.done += 1;
      progress.spent += plan.perRound;
      progress.rounds.push(result.round);
      last = result.round;
      onProgress({ ...progress });
    }
    return end({ kind: "finished" });
  } catch (error) {
    return end({
      kind: "error",
      message: error instanceof Error ? error.message : "the dig failed",
    });
  }
}
