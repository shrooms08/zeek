/** <ResultCard>: found treasure / not this time / Deep Cache broke open / voided. */

import { motion } from "framer-motion";
import { Link } from "react-router-dom";
import { formatZec } from "../lib/money";
import type { Outcome } from "../machines/reveal";

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
  const tile = p.treasureTile === null ? "" : `tile ${p.treasureTile + 1}`;
  const mine = p.myTiles.map((t) => t + 1);
  return (
    <motion.section
      className="panel p-6 flex flex-col gap-3"
      style={{ boxShadow: "var(--shadow-lift)" }}
      initial={{ opacity: 0, y: 16 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: [0.2, 0.9, 0.2, 1] }}
      aria-live="polite"
      data-testid="result-card"
      data-outcome={p.outcome}
    >
      <div className="label">
        Round {p.round}
        {p.treasureTile !== null ? ` · Treasure: ${tile}` : ""}
      </div>
      {p.outcome === "win" || p.outcome === "cache-win" ? (
        <>
          <h2 className="text-title font-black" style={{ color: "var(--color-gold)" }}>
            {p.outcome === "cache-win" ? "Deep Cache broke open" : "You found the treasure"}
          </h2>
          <div
            className="numeral"
            style={{ fontSize: "clamp(48px, 8vw, 96px)", color: "var(--color-gold)" }}
            data-testid="result-share"
          >
            {formatZec(p.share, { sign: true })} <span className="label">ZEC</span>
          </div>
          <p className="text-mist">
            Your share of {tile}
            {p.outcome === "cache-win"
              ? `, including ${formatZec(p.cachePaid)} from the Deep Cache`
              : ""}{" "}
            · arriving privately
          </p>
          <div className="flex gap-2 flex-wrap">
            {p.canProve && (
              <Link className="btn btn-primary" to={`/prove/${p.round}`} data-testid="prove-it">
                Prove it
              </Link>
            )}
            <Link className="btn" to="/">
              Dig again
            </Link>
          </div>
        </>
      ) : p.outcome === "not-this-time" ? (
        <>
          <h2 className="text-title font-black text-white">Not this time</h2>
          <p className="text-mist">The treasure was in {tile}.</p>
          <p className="text-mist">
            You dug {mine.length === 1 ? "tile" : "tiles"} {mine.join(", ")}.
          </p>
          <p className="text-fog">
            {p.winners === 0
              ? "Nobody dug it, so the prize went to the Deep Cache"
              : "Part of every pot goes to the Deep Cache"}{" "}
            · now {formatZec(p.newCache)}
          </p>
          <Link className="btn btn-primary" to="/">
            Dig in round {p.round + 1}
          </Link>
        </>
      ) : p.outcome === "voided" ? (
        <>
          <h2 className="text-title font-black text-white">Round voided</h2>
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
          <h2 className="text-title font-black text-white">The treasure was in {tile}</h2>
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
