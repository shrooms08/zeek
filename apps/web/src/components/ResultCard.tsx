/**
 * <ResultCard>: found the treasure (Zeb cheers, gold count-up) / not this
 * time (Zeb shrugs) / Deep Cache broke open (gold headline, count-up) /
 * voided / watched. Rises 48 px in 320 ms; reduced motion fades in 150 ms.
 */

import { motion, useReducedMotion } from "framer-motion";
import { Link } from "react-router-dom";
import { Zeb } from "../art/Characters";
import { formatZec } from "../lib/money";
import type { Outcome } from "../machines/reveal";
import { CountUp } from "../motion/CountUp";
import { EASE } from "../motion/tokens";
import { Icon } from "./Icon";

export interface ResultCardProps {
  round: number;
  outcome: Outcome;
  treasureTile: number | null;
  myTiles: number[];
  share: bigint;
  prize: bigint;
  newCache: bigint;
  cachePaid: bigint;
  canProve: boolean;
  winners: number;
}

export function ResultCard(p: ResultCardProps) {
  const reduce = useReducedMotion() === true;
  const tile = p.treasureTile === null ? "" : `tile ${p.treasureTile + 1}`;
  const mine = p.myTiles.map((t) => t + 1);
  const won = p.outcome === "win" || p.outcome === "cache-win";
  const pose = won ? "cheer" : p.outcome === "not-this-time" ? "shrug" : "idle";
  return (
    <motion.section
      className="flex flex-col items-center text-center gap-4"
      initial={reduce ? { opacity: 0 } : { opacity: 0, y: 48 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: reduce ? 0.15 : 0.32, ease: EASE.out }}
      aria-live="polite"
      data-testid="result-card"
      data-outcome={p.outcome}
    >
      <div className="label text-fog">
        Round {p.round}
        {p.treasureTile !== null ? ` · Treasure: ${tile}` : ""}
      </div>
      {p.outcome !== "cache-win" && <Zeb pose={pose} width={120} />}
      {won ? (
        <>
          <h2 className="zk-title" style={{ color: "var(--color-gold)" }}>
            {p.outcome === "cache-win" ? "Deep Cache broke open" : "You found the treasure"}
          </h2>
          <div className="zk-amount justify-center" data-testid="result-share">
            <Icon name="chest" size={34} />
            <span
              className="numeral"
              style={{ fontSize: "clamp(48px, 7vw, 84px)", color: "var(--color-gold)" }}
            >
              <CountUp
                value={p.share}
                from={0n}
                sign
                duration={p.outcome === "cache-win" ? 1400 : 900}
                delay={p.outcome === "cache-win" ? 900 : 200}
              />
            </span>
            <span className="zk-unit">ZEC</span>
          </div>
          <p className="text-mist">
            Your share of {tile}
            {p.outcome === "cache-win"
              ? `, including ${formatZec(p.cachePaid)} from the Deep Cache`
              : ""}{" "}
            · arriving privately
          </p>
          <div className="grid grid-cols-2 gap-3 w-full max-w-[420px]">
            {p.canProve ? (
              <Link
                className="btn btn-primary flex items-center justify-center gap-2"
                to={`/prove/${p.round}`}
                data-testid="prove-it"
              >
                <Icon name="share" size={18} /> Prove it
              </Link>
            ) : (
              <span />
            )}
            <Link className="btn flex items-center justify-center" to="/">
              Dig again
            </Link>
          </div>
        </>
      ) : p.outcome === "not-this-time" ? (
        <>
          <h2 className="zk-title text-white">Not this time</h2>
          <p className="text-mist">
            The treasure was in <strong className="text-white">{tile}</strong>.<br />
            You dug {mine.length === 1 ? "tile" : "tiles"} {mine.join(", ")}.
          </p>
          <p className="zk-note flex items-center gap-3">
            <Icon name="mine" size={20} />
            {p.winners === 0
              ? "Nobody dug it: the prize went to the Deep Cache"
              : "Part of the pot went to the Deep Cache"}{" "}
            · now {formatZec(p.newCache)}
          </p>
          <Link
            className="zk-dig flex items-center justify-center gap-3 w-full max-w-[420px]"
            style={{ fontSize: 22, minHeight: 60 }}
            to="/"
          >
            <Icon name="shovel" size={22} /> Dig in round {p.round + 1}
          </Link>
        </>
      ) : p.outcome === "voided" ? (
        <>
          <h2 className="zk-title text-white">Round voided</h2>
          <p className="text-mist">
            It closed too late to be safe from the draw, so it doesn’t count. Every dig is refunded
            minus the fees.
          </p>
          <Link className="btn btn-primary" to="/">
            Back to the field
          </Link>
        </>
      ) : (
        <>
          <h2 className="zk-title text-white">The treasure was in {tile}</h2>
          <p className="text-mist">
            {p.winners > 0
              ? `${p.winners} ${p.winners === 1 ? "dig" : "digs"} on it shared ${formatZec(p.prize)} ZEC.`
              : `Nobody dug it. The prize went to the Deep Cache, now ${formatZec(p.newCache)}.`}
          </p>
          <Link className="btn btn-primary" to="/">
            Back to the field
          </Link>
        </>
      )}
    </motion.section>
  );
}
