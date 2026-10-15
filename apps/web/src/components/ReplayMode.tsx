/**
 * Replay mode: while the engine is offline (or the site has used up its
 * free daily requests), the field plays back real past testnet rounds from
 * the site's saved snapshot instead of a dead end: the field filling block
 * by block (depth only, never amounts per tile), the close, the draw, the
 * reveal and the payout. It is labelled as a replay throughout, and the
 * field goes live again by itself when the engine answers.
 *
 * "Verify this round here" runs the checks that need no chain data in this
 * browser; the rest say they need the live engine (no tick for them).
 */

import { usePocket } from "../api/PocketProvider";
import { useEngineComeback } from "../api/health";
import {
  savedBundleAndConfig,
  useReplayIndex,
  useReplayRound,
  type ReplayRound,
} from "../api/replay";
import type { OfflineReport } from "@zeek/pocket";
import { toPocketError } from "@zeek/pocket";
import { useEffect, useState } from "react";
import { Field } from "./field/Field";
import { Icon } from "./Icon";
import { Offline } from "./ui";
import { cacheStage } from "../lib/digPlan";
import { formatZec, zat } from "../lib/money";
import { formatTimeOfDay } from "../lib/time";
import { useMedia } from "../lib/useMedia";
import { prefersReducedMotion } from "../lib/useNow";
import { momentAt, roundLength, type ReplayMoment } from "../lib/replayTimeline";

export type ReplayReason = "offline" | "busy";

export const LIVE_RESUMES = "Live play resumes when the Zeek engine is back.";
export const BUSY_TEXT = "Zeek is very busy, try again shortly.";

export function ReplayMode({ reason }: { reason: ReplayReason }) {
  const index = useReplayIndex();
  const comeback = useEngineComeback();
  const [pick, setPick] = useState(0);
  if (index.isPending) return null;
  const rounds = index.data?.rounds ?? [];
  if (rounds.length === 0) {
    return (
      <Offline
        text={reason === "busy" ? BUSY_TEXT : "Can’t reach the field"}
        onRetry={comeback.checkNow}
      />
    );
  }
  const current = rounds[((pick % rounds.length) + rounds.length) % rounds.length];
  return (
    <section
      className="flex flex-col gap-4 px-4 py-4 lg:px-10"
      data-testid="replay"
      aria-label="Replay"
    >
      <div className="flex flex-wrap items-center gap-3">
        <span className="zk-replay-badge" data-testid="replay-badge">
          <Icon name="refresh" size={16} /> Replay
        </span>
        <p className="text-white font-extrabold" role="status" data-testid="replay-status">
          {reason === "busy" ? `${BUSY_TEXT} ` : ""}
          {LIVE_RESUMES}
        </p>
        <NextCheck nextAt={comeback.nextAt} onCheck={comeback.checkNow} />
      </div>
      {current !== undefined && (
        <ReplayPlayer
          key={current.round}
          round={current.round}
          savedAt={index.data?.saved_at ?? 0}
          onDone={() => {
            setPick((k) => k + 1);
          }}
          onPrev={() => {
            setPick((k) => k - 1);
          }}
        />
      )}
    </section>
  );
}

function NextCheck({ nextAt, onCheck }: { nextAt: number | null; onCheck: () => void }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => {
      setNow(Date.now());
    }, 1000);
    return () => {
      window.clearInterval(id);
    };
  }, []);
  return (
    <span className="flex items-center gap-2 text-fog text-sm">
      {nextAt !== null && `Checking again in ${Math.max(0, Math.ceil((nextAt - now) / 1000))} s`}
      <button
        type="button"
        className="underline text-white"
        onClick={onCheck}
        data-testid="replay-check-now"
      >
        Check now
      </button>
    </span>
  );
}

function ReplayPlayer({
  round,
  savedAt,
  onDone,
  onPrev,
}: {
  round: number;
  savedAt: number;
  onDone: () => void;
  onPrev: () => void;
}) {
  const data = useReplayRound(round);
  const wide = useMedia("(min-width: 640px)");
  const reduce = prefersReducedMotion();
  const [start] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  const [verifying, setVerifying] = useState(false);
  useEffect(() => {
    if (reduce) return;
    const id = window.setInterval(() => {
      setNow(Date.now());
    }, 100);
    return () => {
      window.clearInterval(id);
    };
  }, [reduce]);
  const r = data.data;
  const length = r === undefined ? 0 : roundLength(r);
  // Reduced motion: no playback, the round's end state at once.
  const elapsed = reduce ? length : now - start;
  const moment = r === undefined ? null : momentAt(r, elapsed);
  useEffect(() => {
    if (moment?.done === true && !reduce && !verifying) onDone();
  }, [moment?.done, reduce, verifying, onDone]);
  if (data.isError) return <p className="text-fog">Round {round} is not saved in this build.</p>;
  if (r === undefined || moment === null) return null;
  const drawn = r.outcome.outcome === "drawn" ? r.outcome : null;
  const phase = moment.phase;
  const revealing =
    phase === "draw" || phase === "shake" || phase === "burst" || phase === "result";
  const burst = phase === "burst" ? 2 : phase === "result" ? 3 : 0;
  return (
    <div className="grid gap-4 lg:grid-cols-[1fr_360px]">
      {/* The reveal's light stays inside the field (it must not cover the status above). */}
      <div className="relative overflow-hidden">
        <Field
          stages={moment.stages}
          selected={[]}
          closed={phase !== "intro" && phase !== "filling"}
          treasure={drawn !== null && revealing ? drawn.treasure_tile : null}
          burst={drawn !== null ? burst : 0}
          reveal={{
            dim: phase === "draw",
            star: phase === "draw",
            cracks: phase === "shake" || phase === "burst",
            gold: true,
          }}
          world={wide}
          mineStage={cacheStage(zat(r.previous_cache))}
          erupt={phase === "result" && drawn?.cache_triggered === true}
          interactive={false}
          decorative
        />
      </div>
      <div className="panel p-5 flex flex-col gap-3 self-start" data-testid="replay-panel">
        <div className="label text-fog">
          Round {r.round} · real testnet round, saved {formatTimeOfDay(savedAt)}
        </div>
        <p
          className="text-white text-lg font-extrabold"
          aria-live="polite"
          data-testid="replay-caption"
        >
          {caption(r, moment)}
        </p>
        <dl className="flex flex-col gap-1 text-sm">
          <div className="flex justify-between">
            <dt className="text-fog">Prize</dt>
            <dd className="font-mono text-white">{formatZec(zat(r.pot), { decimals: 8 })} ZEC</dd>
          </div>
          {phase === "result" && r.payout_txid !== null && (
            <div className="flex justify-between gap-3">
              <dt className="text-fog">Payout</dt>
              <dd className="font-mono text-white break-all text-xs" data-testid="replay-payout">
                {r.payout_txid}
              </dd>
            </div>
          )}
        </dl>
        <div className="flex gap-2">
          <button type="button" className="btn flex-1" onClick={onPrev} data-testid="replay-prev">
            Previous
          </button>
          <button type="button" className="btn flex-1" onClick={onDone} data-testid="replay-next">
            Next round
          </button>
        </div>
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => {
            setVerifying(true);
          }}
          data-testid="replay-verify"
        >
          Verify round {r.round} here
        </button>
        {verifying && <OfflineVerify round={r.round} />}
      </div>
    </div>
  );
}

function caption(r: ReplayRound, m: ReplayMoment): string {
  switch (m.phase) {
    case "intro":
      return `Round ${r.round} opens`;
    case "filling": {
      const block = (m.height ?? r.window_start) - r.window_start + 1;
      const landed = r.frames.some((f) => f.height === m.height);
      return `Block ${block} of ${r.window_end - r.window_start}${landed ? ": digs land" : ""}`;
    }
    case "closed":
      return `Round ${r.round} is closed`;
    case "draw":
    case "shake":
    case "burst":
      return `The draw: drand round ${r.drand_round}`;
    case "result": {
      if (r.outcome.outcome === "voided") {
        return "The round closed too late, so it was voided and every dig refunded.";
      }
      const tile = r.outcome.treasure_tile + 1;
      if (r.winners === 0)
        return `Treasure in tile ${tile}. Nobody dug it: the prize went to the Deep Cache.`;
      return `Treasure in tile ${tile}. ${r.winners} ${r.winners === 1 ? "winner" : "winners"} paid ${formatZec(zat(r.total_paid), { decimals: 8 })} ZEC.`;
    }
  }
}

const PART_LABEL: Record<string, string> = {
  "beacon.rules": "Beacon: the rules match the published configuration",
  "beacon.signature": "Beacon: the BLS signature (drand quicknet)",
  "beacon.public": "Beacon: the same as drand’s public copy",
  "beacon.draw": "Beacon: the draw follows from it",
  "beacon.binding": "Beacon: the round the rules bind",
  "digs.chain": "Digs complete and correct",
  "timing.chain": "Timing rule",
  "settlement.recompute": "Settlement recomputed",
  "payout.openings": "Payout: openings match the settlement",
  "payout.chain": "Payout: the transaction is mined and valid",
};

/** The checks that need no chain data, run in this browser. */
function OfflineVerify({ round }: { round: number }) {
  const { pocket } = usePocket();
  const [report, setReport] = useState<OfflineReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const gone = new AbortController();
    void (async () => {
      try {
        const { bundle, config } = await savedBundleAndConfig(round);
        const out = await (await pocket()).verifyOffline(bundle, config);
        if (!gone.signal.aborted) setReport(out);
      } catch (e) {
        if (!gone.signal.aborted) setError(toPocketError(e).message);
      }
    })();
    return () => {
      gone.abort();
    };
  }, [round, pocket]);
  if (error !== null)
    return (
      <p className="text-white" role="alert">
        {error}
      </p>
    );
  if (report === null) return <p className="text-fog text-sm">Checking in this browser…</p>;
  const ran = report.parts.filter((p) => p.status !== "needs_chain").length;
  const waiting = report.parts.length - ran;
  return (
    <div className="flex flex-col gap-2" data-testid="offline-verify">
      <p className="text-fog text-sm">
        Checked here, from the saved round and drand’s public relays: {ran}. Needs the live engine:{" "}
        {waiting}.
      </p>
      <ul className="flex flex-col gap-1.5">
        {report.parts.map((p) => (
          <li
            key={`${p.check}-${p.part}`}
            className="flex items-start gap-2 text-sm"
            data-testid="offline-part"
            data-status={p.status}
            data-part={`${p.check}.${p.part}`}
          >
            <span className="w-5 shrink-0 pt-0.5" aria-hidden="true">
              {p.status === "pass" ? (
                <Icon name="checkink" size={16} strokeWidth={3} />
              ) : p.status === "fail" ? (
                <Icon name="xink" size={16} strokeWidth={3} />
              ) : (
                <span className="text-fog">–</span>
              )}
            </span>
            <span className="min-w-0">
              <span className="text-white">
                {PART_LABEL[`${p.check}.${p.part}`] ?? `${p.check} ${p.part}`}
              </span>
              <span className="block text-fog">
                {p.status === "needs_chain"
                  ? `Needs the live engine: ${p.summary.replace(/^Needs the chain: /, "")}`
                  : p.status === "not_available"
                    ? `Not applicable: ${p.summary}`
                    : p.summary}
              </span>
              <span className="sr-only">
                {p.status === "pass"
                  ? "passed"
                  : p.status === "fail"
                    ? "failed"
                    : p.status === "needs_chain"
                      ? "not checked: needs the live engine"
                      : "not applicable"}
              </span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
