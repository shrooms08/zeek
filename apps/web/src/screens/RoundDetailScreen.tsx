/** One past round: per-tile amounts, treasure, payouts (amounts only), check link. */

import { Link, useParams } from "react-router-dom";
import { ENGINE_URL } from "../api/engine";
import { useRound, useRounds } from "../api/queries";
import { CopyButton, Loading, Offline, Screen } from "../components/ui";
import { formatZec, zat } from "../lib/money";
import { formatTimeOfDay } from "../lib/time";
import { STATUS_LABEL } from "./PastRoundsScreen";

export default function RoundDetailScreen() {
  const n = Number(useParams().round);
  const round = useRound(n);
  const list = useRounds();
  if (round.isPending) return <Loading text="Loading the round…" />;
  if (round.isError) return <Offline onRetry={() => void round.refetch()} />;
  const s = round.data.settled;
  if (s === null) {
    return (
      <Screen title={`Round ${n}`} back="/rounds">
        <p className="text-mist">This round isn’t settled yet. {round.data.digs} digs so far.</p>
        <Link className="btn" to={`/reveal/${n}`}>
          Watch the reveal
        </Link>
      </Screen>
    );
  }
  const summary = list.data?.rounds.find((r) => r.round === n);
  const treasure = s.treasure_tile;
  const wins = s.payouts.filter((p) => p.reason.reason === "win");
  const refunds = s.payouts.filter((p) => p.reason.reason !== "win");
  const digFees = s.dig_fees;
  const digCount = digFees / 5_000;
  return (
    <Screen title={`Round ${n}`} back="/rounds" testId="round-detail" wide>
      <div className="flex flex-wrap gap-3 items-center">
        {summary !== undefined && (
          <span className="label px-2 py-1 border-2 border-white" data-testid="round-status">
            {STATUS_LABEL[summary.status]}
          </span>
        )}
        <span className="text-fog">
          {summary?.closed_time != null ? `Closed ${formatTimeOfDay(summary.closed_time)} · ` : ""}
          revealed {formatTimeOfDay(s.beacon_time)}
        </span>
      </div>
      <section aria-label="Dug per tile" className="panel p-4">
        <div className="label mb-3">Dug per tile · ZEC</div>
        <ol className="grid grid-cols-5 gap-1" data-testid="tile-amounts">
          {s.tile_totals.map((v, i) => (
            <li
              key={i}
              className="p-2 border-2 flex flex-col"
              style={{ borderColor: treasure === i ? "var(--color-gold)" : "var(--color-slate)" }}
              aria-label={`Tile ${i + 1}: ${formatZec(zat(v), { decimals: 8 })} ZEC${treasure === i ? ", the treasure" : ""}`}
            >
              <span className="text-fog text-xs">{i + 1}</span>
              <span
                className="numeral text-sm"
                style={{ color: treasure === i ? "var(--color-gold)" : undefined }}
              >
                {formatZec(zat(v))}
              </span>
            </li>
          ))}
        </ol>
      </section>
      <div className="grid gap-4 sm:grid-cols-2">
        <section className="panel p-4">
          <div className="label">Treasure</div>
          <div
            className="numeral"
            style={{ fontSize: 48, color: treasure === null ? undefined : "var(--color-gold)" }}
            data-testid="detail-treasure"
          >
            {treasure === null ? "—" : `Tile ${treasure + 1}`}
          </div>
        </section>
        <section className="panel p-4">
          <div className="label">Prize</div>
          <div className="numeral" style={{ fontSize: 48, color: "var(--color-gold)" }}>
            {formatZec(zat(s.distributable))}
          </div>
        </section>
      </div>
      <section className="panel p-4" aria-label="Payouts">
        <div className="label mb-2">
          Payouts · {wins.length} {wins.length === 1 ? "dig" : "digs"} of{" "}
          {treasure === null ? "—" : `tile ${treasure + 1}`}
        </div>
        <ul className="flex flex-col" data-testid="payout-list">
          {wins.map((p, i) => {
            const share =
              p.reason.reason === "win" && s.distributable > 0
                ? Math.floor((p.reason.treasure_share * 100) / s.distributable)
                : 0;
            return (
              <li key={i} className="flex justify-between border-b hair py-2">
                <span className="text-mist">{share}% of the prize</span>
                <span className="numeral">{formatZec(zat(p.net), { decimals: 8 })}</span>
              </li>
            );
          })}
          {refunds.map((p, i) => (
            <li key={`r${i}`} className="flex justify-between border-b hair py-2">
              <span className="text-mist">Refund</span>
              <span className="numeral">{formatZec(zat(p.net), { decimals: 8 })}</span>
            </li>
          ))}
          <li className="flex justify-between border-b hair py-2">
            <span className="text-mist">
              To Deep Cache ·{" "}
              {s.rake === 0 ? "rake" : `${Math.round((s.rake * 100) / Math.max(1, s.pot))}%`}
            </span>
            <span className="numeral">{formatZec(zat(s.rake), { decimals: 8 })}</span>
          </li>
          <li className="flex justify-between border-b hair py-2">
            <span className="text-mist">
              Dig fees · {digCount} {digCount === 1 ? "dig" : "digs"}
            </span>
            <span className="numeral">{formatZec(zat(digFees), { decimals: 8 })}</span>
          </li>
          {(s.shouts ?? 0) > 0 && (
            <li className="flex justify-between border-b hair py-2">
              <span className="text-mist">Shouts to the Deep Cache</span>
              <span className="numeral">
                {formatZec(zat((s.shouts ?? 0) - (s.shout_fees ?? 0)), { decimals: 8 })}
              </span>
            </li>
          )}
        </ul>
        {s.payout?.txid != null && (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <span className="label">Payout</span>
            <span className="font-mono text-xs break-all text-fog" data-testid="payout-txid">
              {s.payout.txid}
            </span>
            <CopyButton text={s.payout.txid} label="Copy txid" />
          </div>
        )}
      </section>
      <div className="flex gap-2 flex-wrap">
        <Link className="btn btn-primary" to={`/check/${n}`} data-testid="check-this-round">
          Check this round
        </Link>
        <a
          className="btn"
          href={`${ENGINE_URL}/v1/rounds/${n}/proof`}
          target="_blank"
          rel="noreferrer"
        >
          Raw proof
        </a>
      </div>
    </Screen>
  );
}
