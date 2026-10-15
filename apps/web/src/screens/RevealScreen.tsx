/**
 * Reveal (design 5A–5D, Motion 9a–9e), one timeline on the reveal machine:
 *   draw   0–860     the world dims; the draw falls as a star
 *   shake  860–1700  cracks race across the field; tiles tremble outward
 *   burst  1700–2600 the treasure tile splits; light bursts (gold)
 *   result 2600      the card rises (Deep Cache: the mine erupts, gems arc)
 * About 4.5 s with the count-up (Deep Cache ≈ 6.5 s). Reduced motion goes
 * straight to the result. Tile pins never cover the treasure tile.
 */

import { motion, useReducedMotion } from "framer-motion";
import { useEffect, useMemo, useReducer, useState } from "react";
import { useParams } from "react-router-dom";
import { Mine } from "../art/Characters";
import { usePending, useRound } from "../api/queries";
import { Field } from "../components/field/Field";
import { Icon } from "../components/Icon";
import { ResultCard } from "../components/ResultCard";
import { Loading, Offline, Screen } from "../components/ui";
import { cacheStage } from "../lib/digPlan";
import { formatZec, zat } from "../lib/money";
import { stagesFromDepths, stagesFromTotals } from "../lib/stages";
import { formatClock } from "../lib/time";
import { useMedia } from "../lib/useMedia";
import { prefersReducedMotion, useNow } from "../lib/useNow";
import {
  estimatedShare,
  initialReveal,
  outcomeFor,
  revealReducer,
  STAGE_MS,
} from "../machines/reveal";
import { CountUp } from "../motion/CountUp";
import { EASE } from "../motion/tokens";
import { play } from "../sound/sound";
import { digsInRound, loadMyDigs, markRevealSeen } from "../state/myDigs";

export default function RevealScreen() {
  const n = Number(useParams().round);
  const round = useRound(n);
  const pending = usePending();
  const now = useNow();
  const wide = useMedia("(min-width: 640px)");
  const desktop = useMedia("(min-width: 1024px)");
  const reduce = useReducedMotion() === true;
  const [state, dispatch] = useReducer(revealReducer, prefersReducedMotion(), initialReveal);
  // Light burst: a 250 ms glint (frame 1) before the full burst (frame 2).
  const [glint, setGlint] = useState(false);

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

  useEffect(() => {
    if (state.stage === "shake") play("crack");
    if (state.stage === "burst") {
      setGlint(true);
      const id = setTimeout(() => {
        setGlint(false);
      }, 250);
      return () => {
        clearTimeout(id);
      };
    }
    if (state.stage === "result") {
      markRevealSeen(n);
      if (state.result?.cacheTriggered === true) play("rumble");
    }
    return undefined;
  }, [state.stage, state.result, n]);

  const mine = useMemo(() => digsInRound(n, loadMyDigs()), [n]);
  const myTiles = useMemo(() => [...mine.keys()].sort((a, b) => a - b), [mine]);
  const result = state.result;
  const outcome = result === null ? null : outcomeFor(result, myTiles);
  useEffect(() => {
    if (state.stage === "burst" && (outcome === "win" || outcome === "cache-win")) play("chime");
  }, [state.stage, outcome]);

  if (!Number.isInteger(n) || n < 0) return <Offline text="No such round" />;
  if (round.isPending) return <Loading text="Finding the round…" />;
  if (round.isError) return <Offline onRetry={() => void round.refetch()} />;

  const r = round.data;
  const stages =
    settled === null ? stagesFromDepths(r.depths) : stagesFromTotals(settled.tile_totals);
  const treasure = result?.treasureTile ?? null;
  const beacon = pending.data?.find((p) => p.round === n);
  const stage = state.stage;
  const lit = stage === "burst" || stage === "result";
  const burst = stage === "result" ? 3 : stage === "burst" ? (glint ? 1 : 2) : 0;
  const erupt = stage === "result" && result?.cacheTriggered === true;
  const cacheMine = cacheStage(zat(settled?.previous_cache ?? 0));

  let share = 0n;
  if (settled !== null && treasure !== null) {
    share = estimatedShare(
      mine.get(treasure) ?? 0n,
      zat(settled.tile_totals[treasure] ?? 0),
      zat(settled.distributable),
      zat(settled.cache_paid),
    );
  }

  const field = (
    <motion.div
      animate={
        stage === "shake" && !reduce
          ? { x: [0, -4, 4, -3, 3, -2, 2, 0], y: [0, 1, -1, 1, -1, 0, 0, 0] }
          : { x: 0, y: 0 }
      }
      transition={{ duration: 0.7, ease: "linear" }}
      data-stage={stage}
      data-testid="reveal-field"
    >
      <Field
        stages={stages}
        selected={myTiles}
        closed
        treasure={lit ? treasure : null}
        burst={burst}
        label={`Round ${n} field`}
        world={wide}
        mineStage={cacheMine}
        erupt={erupt}
        interactive={false}
        reveal={{
          dim: stage === "draw" || stage === "shake",
          star: stage === "draw",
          cracks: stage === "shake",
          gold: true,
        }}
      />
    </motion.div>
  );

  const waiting = stage === "closed" && (
    <section className="flex flex-col items-center gap-3 text-center" data-testid="reveal-waiting">
      <div className="label text-fog">Round {n} · closed</div>
      <p className="zk-title text-white flex items-center gap-3">
        <span className="zk-hourglass inline-flex">
          <Icon name="hourglass" size={36} />
        </span>
        {beacon !== undefined && beacon.beacon_time > now
          ? `Reveal in ${formatClock(beacon.beacon_time - now)}`
          : "Waiting for the draw…"}
      </p>
      <p className="text-fog max-w-[420px]">
        Holes stay; no new digs. The draw comes from a public random beacon nobody can steer, about
        15 minutes after the round closes.
      </p>
    </section>
  );

  const progress = stage !== "closed" && stage !== "result" && (
    <section
      className="flex flex-col items-center gap-2 text-center"
      aria-live="polite"
      data-testid="reveal-draw"
    >
      <div className="label text-fog">
        {stage === "draw"
          ? "The draw arrives"
          : stage === "shake"
            ? "The ground shakes"
            : `Light from tile ${(treasure ?? 0) + 1}`}
      </div>
      {settled !== null && (
        <p className="font-mono text-fog text-sm">drand round {settled.drand_round}</p>
      )}
      <button
        type="button"
        className="underline text-fog text-sm"
        onClick={() => {
          dispatch({ type: "skip" });
        }}
      >
        Skip to the result
      </button>
    </section>
  );

  const card = stage === "result" && result !== null && settled !== null && outcome !== null && (
    <ResultCard
      round={n}
      outcome={outcome}
      treasureTile={result.treasureTile}
      myTiles={myTiles}
      share={share}
      prize={zat(settled.distributable)}
      newCache={zat(settled.new_cache)}
      cachePaid={zat(settled.cache_paid)}
      winners={settled.winners}
      canProve={loadMyDigs().some((d) => d.round === n && d.via === "pocket")}
    />
  );

  if (erupt && desktop && settled !== null) {
    return (
      <main
        className="mx-auto grid w-full max-w-[1440px] grid-cols-[minmax(380px,1fr)_2fr] items-center gap-6 px-10 py-8"
        data-testid="reveal-screen"
      >
        <motion.div
          initial={{ opacity: 0, y: 24 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.32, ease: EASE.out }}
        >
          <div className="label text-fog">Round {n} · 1 in 625</div>
          <h1 className="zk-headline" style={{ color: "var(--color-gold)" }}>
            Deep Cache broke open
          </h1>
          <div className="zk-amount">
            <Icon name="mine" size={34} />
            <span className="numeral" style={{ fontSize: 96, color: "var(--color-gold)" }}>
              <CountUp value={zat(settled.cache_paid)} from={0n} duration={1400} delay={900} />
            </span>
            <span className="zk-unit">ZEC</span>
          </div>
          <p className="text-mist mt-3">
            On top of the round’s prize. Everyone who dug tile {(treasure ?? 0) + 1} shares it by
            how much they dug.
          </p>
          <div className="mt-6">{card}</div>
        </motion.div>
        {field}
      </main>
    );
  }

  return (
    <Screen title={`Round ${n}`} testId="reveal-screen" wide>
      <div
        className={desktop ? "grid grid-cols-[3fr_2fr] items-center gap-8" : "flex flex-col gap-6"}
      >
        <div className={`relative ${wide ? "" : "pt-24"}`}>
          {field}
          {erupt && !wide && (
            <div className="absolute left-1/2 top-0 -translate-x-1/2">
              <Mine stage={cacheMine} erupt width={180} />
            </div>
          )}
        </div>
        <div className="flex flex-col gap-4">
          {waiting}
          {progress}
          {card}
          {stage === "result" && result?.cacheTriggered === true && settled !== null && (
            <p className="text-fog text-center">
              Deep Cache paid {formatZec(zat(settled.cache_paid))} ZEC into this round.
            </p>
          )}
        </div>
      </div>
    </Screen>
  );
}
