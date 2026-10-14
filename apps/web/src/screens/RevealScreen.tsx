/** Reveal: closed → the draw arrives → ground shakes → light bursts → result. */

import { motion } from "framer-motion";
import { useEffect, useMemo, useReducer } from "react";
import { useParams } from "react-router-dom";
import { usePending, useRound } from "../api/queries";
import { Field } from "../components/field/Field";
import { ResultCard } from "../components/ResultCard";
import { Loading, Offline, Screen } from "../components/ui";
import { zat } from "../lib/money";
import { stagesFromDepths, stagesFromTotals } from "../lib/stages";
import { formatClock } from "../lib/time";
import { prefersReducedMotion, useNow } from "../lib/useNow";
import {
  estimatedShare,
  initialReveal,
  outcomeFor,
  revealReducer,
  STAGE_MS,
} from "../machines/reveal";
import { digsInRound, loadMyDigs } from "../state/myDigs";

export default function RevealScreen() {
  const n = Number(useParams().round);
  const round = useRound(n);
  const pending = usePending();
  const now = useNow();
  const [state, dispatch] = useReducer(revealReducer, prefersReducedMotion(), initialReveal);

  const settled = round.data?.settled ?? null;
  useEffect(() => {
    if (settled === null) return;
    const outcome = settled.outcome;
    dispatch({
      type: "settled",
      result: {
        round: n,
        treasureTile: outcome.outcome === "drawn" ? outcome.treasure_tile : null,
        cacheTriggered: outcome.outcome === "drawn" && outcome.cache_triggered,
        voided: outcome.outcome === "voided",
      },
    });
  }, [settled, n]);

  useEffect(() => {
    if (state.stage === "closed" || state.stage === "result") return;
    const id = setTimeout(() => {
      dispatch({ type: "advance" });
    }, STAGE_MS[state.stage]);
    return () => {
      clearTimeout(id);
    };
  }, [state.stage]);

  const mine = useMemo(() => digsInRound(n, loadMyDigs()), [n]);
  if (!Number.isInteger(n) || n < 0) return <Offline text="No such round" />;
  if (round.isPending) return <Loading text="Finding the round…" />;
  if (round.isError) return <Offline onRetry={() => void round.refetch()} />;

  const r = round.data;
  const stages =
    settled === null ? stagesFromDepths(r.depths) : stagesFromTotals(settled.tile_totals);
  const treasure = state.result?.treasureTile ?? null;
  const showTreasure = state.stage === "burst" || state.stage === "result";
  const beacon = pending.data?.find((p) => p.round === n);
  const myTiles = [...mine.keys()].sort((a, b) => a - b);
  const result = state.result;

  let share = 0n;
  if (settled !== null && treasure !== null) {
    share = estimatedShare(
      mine.get(treasure) ?? 0n,
      zat(settled.tile_totals[treasure] ?? 0),
      zat(settled.distributable),
      zat(settled.cache_paid),
    );
  }

  return (
    <Screen title={`Round ${n}`} testId="reveal-screen" wide>
      <div className="grid gap-6 lg:grid-cols-[2fr_1fr] items-start">
        <motion.div
          animate={state.stage === "shake" ? { x: [0, -4, 4, -4, 4, -4, 0] } : { x: 0 }}
          transition={{ duration: 0.7, ease: [0.6, 0, 0.2, 1] }}
          data-stage={state.stage}
          data-testid="reveal-field"
        >
          <Field
            stages={stages}
            selected={myTiles}
            closed
            treasure={showTreasure ? treasure : null}
            burst={state.stage === "burst" ? 2 : state.stage === "result" ? 3 : 0}
            label={`Round ${n} field`}
          />
        </motion.div>
        <div className="flex flex-col gap-4" aria-live="polite">
          {state.stage === "closed" && (
            <section className="panel p-4" data-testid="reveal-waiting">
              <div className="label">Round {n} · closed</div>
              <p className="text-white text-heading font-extrabold">
                {beacon !== undefined && beacon.beacon_time > now
                  ? `Reveal in ${formatClock(beacon.beacon_time - now)}`
                  : "Waiting for the draw…"}
              </p>
              <p className="text-fog">
                Holes stay; no new digs. The draw comes from a public random beacon nobody can
                steer.
              </p>
            </section>
          )}
          {state.stage === "draw" && settled !== null && (
            <section className="panel p-4" data-testid="reveal-draw">
              <div className="label">The draw arrives</div>
              <p className="font-mono text-mist">drand round {settled.drand_round}</p>
              <p className="text-white">Nobody can steer this.</p>
            </section>
          )}
          {(state.stage === "shake" || state.stage === "burst") && (
            <section className="panel p-4">
              <div className="label">
                {state.stage === "shake"
                  ? "The ground shakes"
                  : `Light from tile ${(treasure ?? 0) + 1}`}
              </div>
            </section>
          )}
          {state.stage === "result" && result !== null && settled !== null && (
            <ResultCard
              round={n}
              outcome={outcomeFor(result, myTiles)}
              treasureTile={result.treasureTile}
              myTiles={myTiles}
              share={share}
              prize={zat(settled.distributable)}
              newCache={zat(settled.new_cache)}
              cachePaid={zat(settled.cache_paid)}
              winners={settled.winners}
              canProve={loadMyDigs().some((d) => d.round === n && d.via === "pocket")}
            />
          )}
          {state.stage !== "result" && state.result !== null && (
            <button
              type="button"
              className="btn"
              onClick={() => {
                dispatch({ type: "skip" });
              }}
            >
              Skip to the result
            </button>
          )}
        </div>
      </div>
    </Screen>
  );
}
