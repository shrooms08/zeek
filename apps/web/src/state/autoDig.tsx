/**
 * The Pro auto-dig run, kept at app level so moving between screens does
 * not stop it (closing or leaving the tab does). One run at a time.
 */

import { toPocketError } from "@zeek/pocket";
import { useQueryClient } from "@tanstack/react-query";
import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { engine } from "../api/engine";
import { usePocket } from "../api/PocketProvider";
import {
  runAutoDig,
  type AutoDigPlan,
  type AutoDigProgress,
  type StopReason,
} from "../lib/autoDig";
import { toNumber, zat, type Zat } from "../lib/money";
import { hiddenSince } from "../lib/visibility";
import { setAutoDigRunning } from "./autoDigFlag";
import { armResplit } from "./pocketActivity";
import { recordMyDig } from "./myDigs";

export interface AutoDigSelection {
  tiles: number[];
  amountEach: Zat;
  separate: boolean;
}

export type AutoDigStatus =
  | { kind: "idle" }
  | { kind: "running"; plan: AutoDigPlan; selection: AutoDigSelection; progress: AutoDigProgress }
  | {
      kind: "ended";
      plan: AutoDigPlan;
      selection: AutoDigSelection;
      progress: AutoDigProgress;
      reason: StopReason;
    };

interface AutoDigApi {
  status: AutoDigStatus;
  /**
   * Checks the passphrase, then starts the run in the background. Rejects
   * (bad passphrase) without starting.
   */
  start: (plan: AutoDigPlan, selection: AutoDigSelection, passphrase: string) => Promise<void>;
  stop: () => void;
  dismiss: () => void;
}

const AutoDigContext = createContext<AutoDigApi | undefined>(undefined);

export function AutoDigProvider({ children }: { children: ReactNode }) {
  const { pocket } = usePocket();
  const client = useQueryClient();
  const [status, setStatus] = useState<AutoDigStatus>({ kind: "idle" });
  const stopRef = useRef(false);

  const start = useCallback(
    async (plan: AutoDigPlan, selection: AutoDigSelection, passphrase: string) => {
      const p = await pocket();
      await p.confirmPassphrase(passphrase);
      stopRef.current = false;
      const empty: AutoDigProgress = { done: 0, spent: 0n, rounds: [], sending: false };
      setStatus({ kind: "running", plan, selection, progress: empty });
      setAutoDigRunning(true);
      const tiles = [...selection.tiles];
      // The run goes on in the background; start resolves once confirmed.
      void runAutoDig(
        plan,
        {
          now: () => Date.now(),
          sleep: (ms) =>
            new Promise((resolve) => {
              window.setTimeout(resolve, ms);
            }),
          board: () => engine.board(),
          spendable: async () => zat((await p.sync()).spendable_zat),
          dig: async () => {
            // The round the dig goes into, for the record if it stops part-way.
            const round = (await engine.board()).round;
            try {
              const result = await p.dig({
                tiles,
                amountEachZat: toNumber(selection.amountEach),
                separate: selection.separate,
              });
              recordMyDig({
                round: result.round,
                tiles,
                amountEach: selection.amountEach.toString(),
                via: "pocket",
                txids: result.batches.map((b) => b.txid),
                at: Date.now(),
              });
              void client.invalidateQueries({ queryKey: ["pocket"] });
              armResplit();
              return { round: result.round, txids: result.batches.map((b) => b.txid) };
            } catch (error) {
              const e = toPocketError(error);
              if (e.partial !== undefined) {
                recordMyDig({
                  round,
                  tiles: e.partial.sent.flatMap((s) => s.tiles),
                  amountEach: selection.amountEach.toString(),
                  via: "pocket",
                  txids: e.partial.sent.map((s) => s.txid),
                  at: Date.now(),
                });
              }
              throw new Error(e.message);
            }
          },
          hiddenSince,
        },
        (progress) => {
          setStatus({ kind: "running", plan, selection, progress });
        },
        () => stopRef.current,
      ).then(({ reason, progress }) => {
        setAutoDigRunning(false);
        setStatus({ kind: "ended", plan, selection, progress, reason });
      });
    },
    [pocket, client],
  );

  const value = useMemo<AutoDigApi>(
    () => ({
      status,
      start,
      stop: () => {
        stopRef.current = true;
      },
      dismiss: () => {
        setStatus((s) => (s.kind === "ended" ? { kind: "idle" } : s));
      },
    }),
    [status, start],
  );
  return <AutoDigContext.Provider value={value}>{children}</AutoDigContext.Provider>;
}

export function useAutoDig(): AutoDigApi {
  const value = useContext(AutoDigContext);
  if (value === undefined) throw new Error("useAutoDig outside AutoDigProvider");
  return value;
}
