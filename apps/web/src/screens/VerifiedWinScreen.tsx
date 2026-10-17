/** The public page a share link opens. Verified in this browser. */

import { FitAmount } from "../components/FitAmount";
import { useQuery } from "@tanstack/react-query";
import { Link, useParams } from "react-router-dom";
import { usePocket } from "../api/PocketProvider";
import { useRound } from "../api/queries";
import { motion, useReducedMotion } from "framer-motion";
import { Field } from "../components/field/Field";
import { Icon } from "../components/Icon";
import { Loading, Offline } from "../components/ui";
import { stagesFromTotals } from "../lib/stages";
import { EASE } from "../motion/tokens";
import { formatZec, zat } from "../lib/money";

export default function VerifiedWinScreen() {
  const id = useParams().id ?? "";
  const { pocket } = usePocket();
  const verdict = useQuery({
    queryKey: ["win", id],
    queryFn: async () => (await pocket()).verifyWinProof(id),
    staleTime: Infinity,
    retry: 1,
  });
  const round = useRound(verdict.data?.round);
  const reduce = useReducedMotion() === true;
  if (verdict.isPending) return <Loading text="Checking the proof in your browser…" />;
  if (verdict.isError)
    return (
      <Offline
        text={`Couldn’t check this proof: ${verdict.error.message}`}
        onRetry={() => void verdict.refetch()}
      />
    );
  const v = verdict.data;
  const settled = round.data?.settled ?? null;
  const payout = settled?.payouts[v.payoutIndex];
  return (
    <main
      className="mx-auto w-full max-w-[1300px] px-4 sm:px-8 py-8 sm:py-12 grid gap-8 lg:grid-cols-[1fr_1fr] items-center"
      data-testid="win-screen"
    >
      <section className="flex flex-col gap-4 min-w-0">
        <motion.span
          className={`zk-verified self-start ${v.valid ? "" : "zk-verified-no"}`}
          initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 1.9, rotate: -10 }}
          animate={
            reduce ? { opacity: 1 } : { opacity: 1, scale: [1.9, 0.94, 1], rotate: [-10, -4, -4] }
          }
          transition={{ duration: reduce ? 0.15 : 0.3, delay: 0.35, ease: EASE.in }}
          data-testid="win-verdict"
        >
          {v.valid ? "Verified" : "Not verified"}
        </motion.span>
        <div className="label text-fog">{v.valid ? "Verified win" : "Proof check"}</div>
        <h1 className="zk-headline" style={{ margin: 0, fontSize: "clamp(40px, 5.6vw, 72px)" }}>
          {v.valid
            ? `Someone found the treasure in round ${v.round}`
            : "This proof doesn’t check out"}
        </h1>
        {v.valid && payout !== undefined && (
          <div className="flex items-center gap-2 min-w-0" data-testid="win-amount">
            <Icon name="chest" size={30} />
            {/* Full precision: the size shrinks to fit, never a digit. */}
            <div className="flex-1 min-w-0">
              <FitAmount size={76} min={16} unit="ZEC" unitScale={0.32} color="var(--color-gold)">
                +{formatZec(zat(payout.net), { decimals: 8 })}
              </FitAmount>
            </div>
          </div>
        )}
        {!v.valid && v.reason !== null && (
          <p className="text-white" role="alert">
            {v.reason}
          </p>
        )}
        <ul className="flex flex-col gap-2 text-white">
          {v.valid && payout !== undefined && (
            <li className="flex items-center gap-3">
              <Icon name="checkink" size={20} strokeWidth={3} /> Made by a wallet that dug tile{" "}
              {payout.memo_tile + 1}
            </li>
          )}
          {v.steps.map((s) => (
            <li key={s.name} title={s.detail} className="flex items-center gap-3 text-mist">
              <Icon name={s.passed ? "checkink" : "xink"} size={20} strokeWidth={3} /> {s.name}
            </li>
          ))}
        </ul>
        <p className="text-fog">Doesn’t say who. Never will.</p>
        <div className="flex gap-3 flex-wrap">
          <Link className="btn flex items-center gap-2" to={`/check/${v.round}`}>
            <Icon name="shield" size={18} /> Check round {v.round}
          </Link>
          <Link className="btn btn-primary" to="/">
            Play Zeek
          </Link>
        </div>
      </section>
      {settled !== null && settled.treasure_tile !== null && (
        <div aria-hidden="true" className="min-w-0">
          <Field
            stages={stagesFromTotals(settled.tile_totals)}
            selected={[]}
            treasure={settled.treasure_tile}
            burst={3}
            closed
            world={false}
            interactive={false}
            reveal={{ gold: true }}
            decorative
          />
        </div>
      )}
    </main>
  );
}
