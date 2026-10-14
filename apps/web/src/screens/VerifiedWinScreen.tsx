/** The public page a share link opens. Verified in this browser. */

import { useQuery } from "@tanstack/react-query";
import { Link, useParams } from "react-router-dom";
import { usePocket } from "../api/PocketProvider";
import { useRound } from "../api/queries";
import { Loading, Offline, Screen } from "../components/ui";
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
  if (verdict.isPending) return <Loading text="Checking the proof in your browser…" />;
  if (verdict.isError)
    return (
      <Offline
        text={`Couldn’t check this proof: ${verdict.error.message}`}
        onRetry={() => void verdict.refetch()}
      />
    );
  const v = verdict.data;
  const payout = round.data?.settled?.payouts[v.payoutIndex];
  return (
    <Screen title={v.valid ? "Verified win" : "Not verified"} testId="win-screen">
      <div className="panel p-6 flex flex-col gap-3">
        <span
          className="label px-2 py-1 border-2 self-start"
          style={{ borderColor: v.valid ? "var(--color-gold)" : "var(--color-white)" }}
          data-testid="win-verdict"
        >
          {v.valid ? "Verified" : "Not verified"}
        </span>
        <h2 className="text-title font-black text-white">
          {v.valid
            ? `Someone found the treasure in round ${v.round}`
            : `This proof doesn’t check out`}
        </h2>
        {v.valid && payout !== undefined && (
          <div
            className="numeral text-num-m"
            style={{ color: "var(--color-gold)" }}
            data-testid="win-amount"
          >
            +{formatZec(zat(payout.net), { decimals: 8 })} <span className="label">ZEC</span>
          </div>
        )}
        {!v.valid && v.reason !== null && (
          <p className="text-white" role="alert">
            {v.reason}
          </p>
        )}
        <ul className="flex flex-col gap-1 text-mist">
          {v.valid && payout !== undefined && (
            <li>Made by a wallet that dug tile {payout.memo_tile + 1}</li>
          )}
          {v.steps.map((s) => (
            <li key={s.name} title={s.detail}>
              {s.passed ? "✓" : "✕"} {s.name}
            </li>
          ))}
        </ul>
        <p className="text-fog">Doesn’t say who. Never will.</p>
        <div className="flex gap-2 flex-wrap">
          <Link className="btn" to={`/check/${v.round}`}>
            Check round {v.round}
          </Link>
          <Link className="btn btn-primary" to="/">
            Play Zeek
          </Link>
        </div>
      </div>
    </Screen>
  );
}
