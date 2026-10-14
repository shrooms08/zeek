/** Prove a win: a share card that says a wallet won, never which one. */

import { toPocketError, type WinProofResult } from "@zeek/pocket";
import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { shareUrls } from "../api/engine";
import { usePocket } from "../api/PocketProvider";
import { useRound } from "../api/queries";
import { CopyButton, Loading, Panel, Screen } from "../components/ui";
import { formatZec, zat } from "../lib/money";

type Stage =
  | { kind: "idle" }
  | { kind: "making" }
  | { kind: "done"; proof: WinProofResult }
  | { kind: "error"; message: string };

async function downloadPng(url: string, name: string): Promise<void> {
  // A cross-origin `download` link is ignored, so fetch the bytes first.
  const blob = await (await fetch(url)).blob();
  const href = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = href;
  a.download = name;
  a.click();
  URL.revokeObjectURL(href);
}

export default function ProveWinScreen() {
  const n = Number(useParams().round);
  const { pocket, wallet } = usePocket();
  const round = useRound(n);
  const [stage, setStage] = useState<Stage>({ kind: "idle" });
  const unlocked = wallet.kind === "unlocked" || wallet.kind === "syncing";

  const make = () => {
    setStage({ kind: "making" });
    void (async () => {
      try {
        const proof = await (await pocket()).createWinProof(n, window.location.origin);
        if (!proof.stored)
          throw new Error(proof.verdict.reason ?? "The engine rejected the proof.");
        setStage({ kind: "done", proof });
      } catch (err) {
        setStage({ kind: "error", message: toPocketError(err).message });
      }
    })();
  };

  if (round.isPending) return <Loading text="Loading the round…" />;
  const payout =
    stage.kind === "done" ? round.data?.settled?.payouts[stage.proof.payoutIndex] : undefined;
  return (
    <Screen title="Prove a win" back={`/rounds/${n}`} testId="prove-screen">
      <p className="text-mist">
        Make a card that proves a wallet won round {n}. It shows the amount and the round. It never
        says who.
      </p>
      {!unlocked ? (
        <Panel>
          <p className="text-mist mb-3">Unlock the Pocket that won to make the proof.</p>
          <Link className="btn btn-primary" to="/wallet">
            My wallet
          </Link>
        </Panel>
      ) : stage.kind === "idle" || stage.kind === "error" ? (
        <Panel>
          {stage.kind === "error" && (
            <p className="text-white mb-3" role="alert">
              {stage.message}
            </p>
          )}
          <button
            type="button"
            className="btn btn-primary w-full"
            onClick={make}
            data-testid="prove-create"
          >
            Create proof
          </button>
        </Panel>
      ) : stage.kind === "making" ? (
        <Loading text="Proving in your browser… about a minute" />
      ) : (
        <ProofDone
          id={stage.proof.id}
          round={n}
          amount={payout === undefined ? null : zat(payout.net)}
        />
      )}
    </Screen>
  );
}

function ProofDone({ id, round, amount }: { id: string; round: number; amount: bigint | null }) {
  const urls = shareUrls(id);
  const text = `I found the treasure in Zeek round ${round}${amount === null ? "" : ` · +${formatZec(amount)} ZEC`}. Check it:`;
  const intent = `https://x.com/intent/post?text=${encodeURIComponent(text)}&url=${encodeURIComponent(urls.page)}`;
  return (
    <div className="flex flex-col gap-4" data-testid="prove-done">
      <img
        src={urls.image}
        alt={`Share card: I found the treasure in round ${round}`}
        width={1200}
        height={675}
        className="w-full h-auto border-[3px] border-white"
        data-testid="prove-card"
      />
      <div className="flex flex-wrap gap-2">
        <a className="btn btn-primary" href={intent} target="_blank" rel="noopener noreferrer">
          Post to X
        </a>
        <button
          type="button"
          className="btn"
          onClick={() => void downloadPng(urls.image, `zeek-win-${round}.png`)}
        >
          Download PNG
        </button>
        <CopyButton text={urls.page} label="Copy link" />
        <Link className="btn" to={`/win/${id}`} data-testid="prove-open">
          Open the page
        </Link>
      </div>
      <p className="text-fog text-sm font-mono break-all" data-testid="prove-link">
        {urls.page}
      </p>
    </div>
  );
}
