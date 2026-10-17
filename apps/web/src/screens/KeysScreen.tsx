/** Who holds the keys (design 11): the 2-of-3 keyholders, refusals, the vault and the viewing key. */

import { FitAmount } from "../components/FitAmount";
import { useBoard, useConfig, useKeyholders, useLedger } from "../api/queries";
import type { KeyholderStatus } from "../api/types";
import { Icon } from "../components/Icon";
import { CopyButton, Loading, Offline } from "../components/ui";
import { formatZec, zat } from "../lib/money";
import { formatAgo } from "../lib/time";
import { useNow } from "../lib/useNow";
import { CountUp } from "../motion/CountUp";

function KeyholderCard({ k, now }: { k: KeyholderStatus; now: number }) {
  const online = k.reachable === true;
  const state = k.reachable === null ? "Not checked yet" : online ? "Online" : "Offline";
  return (
    <li
      className={`zk-keycard flex flex-col gap-2 p-6 ${online ? "" : "zk-keycard-off"}`}
      data-testid="keyholder-card"
    >
      <div className="flex items-center justify-between">
        <Icon name="key" size={28} />
        <span className={`zk-chip flex items-center gap-1.5 ${online ? "zk-chip-fill" : ""}`}>
          <Icon name={online ? "check" : "offline"} size={14} strokeWidth={2.5} />
          {state}
        </span>
      </div>
      <h3 className="font-black text-white" style={{ fontSize: 28, fontStretch: "112%" }}>
        {k.label}
      </h3>
      <p className="text-mist">Run by the {k.operator}</p>
      <p className="font-mono text-sm text-fog">
        {k.reachable === false
          ? `last seen ${k.last_seen === null ? "never" : formatAgo(k.last_seen, now)} · payouts still work`
          : `approved ${k.approved} · refused ${k.refused}`}
        {k.last_check !== null ? ` · checked ${formatAgo(k.last_check, now)}` : ""}
      </p>
    </li>
  );
}

export default function KeysScreen() {
  const keys = useKeyholders();
  const ledger = useLedger();
  const config = useConfig();
  const board = useBoard();
  const now = useNow(10_000);
  if (keys.isPending) return <Loading text="Asking the keyholders…" />;
  if (keys.isError) return <Offline onRetry={() => void keys.refetch()} />;
  const k = keys.data;
  const uivk = config.data?.uivk;
  return (
    <main
      className="mx-auto w-full max-w-[1120px] px-4 sm:px-6 py-8 sm:py-12 flex flex-col gap-6"
      data-testid="keys-screen"
    >
      <h1 className="zk-headline" style={{ margin: 0 }}>
        Who holds the keys
      </h1>
      <p className="text-mist text-lg max-w-[740px]">
        Three keyholders guard the prize. Any {k.threshold} must sign each payout. Each one checks
        every payout against the rules on its own, and refuses anything wrong. If one is offline,
        payouts still work.
      </p>
      <ul className="grid gap-4 sm:grid-cols-3">
        {k.keyholders.map((h) => (
          <KeyholderCard key={h.identifier} k={h} now={now} />
        ))}
      </ul>
      <p
        className="zk-honest flex items-center gap-3 px-5 py-3 text-white"
        data-testid="keys-honest"
      >
        <Icon name="arrow" size={18} />
        The three keyholders are separate programs, each with its own key share under its own system
        account. Today the Zeek team runs all three on one computer. Next: independent operators on
        their own machines.
      </p>
      {config.data?.demo_digs === true && (
        <p
          className="zk-honest flex items-center gap-3 px-5 py-3 text-white"
          data-testid="demo-digs"
        >
          <Icon name="info" size={18} />
          Some digs on testnet come from Zeek’s own test wallets.
        </p>
      )}
      <div className="grid gap-4 lg:grid-cols-2">
        <section className="panel" aria-label="Refused payouts">
          <h2 className="flex items-center gap-3 px-5 py-4 border-b-[3px] border-white text-white font-extrabold text-xl">
            <Icon name="shield" size={22} /> Refused payouts
          </h2>
          {k.refused_attempts.length === 0 ? (
            <p className="text-fog p-5">
              None recorded yet. Every refusal from Phase 7 onward is listed here.
            </p>
          ) : (
            <ul data-testid="refusals">
              {k.refused_attempts.map((r, i) => (
                <li key={i} className="flex gap-4 px-5 py-4 border-b hair">
                  <span className="zk-mark zk-mark-fail" aria-hidden="true">
                    <Icon name="x" size={18} />
                  </span>
                  <div>
                    <div className="text-white font-bold">Round {r.rounds.join(", ")}</div>
                    <div className="text-mist">
                      {r.reason}. Refused by {r.refusals}{" "}
                      {r.refusals === 1 ? "keyholder" : "keyholders"}.
                    </div>
                    <div className="text-fog text-xs">{formatAgo(r.time, now)}</div>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>
        <section className="panel" aria-label="The vault">
          <h2 className="flex items-center gap-3 px-5 py-4 border-b-[3px] border-white text-white font-extrabold text-xl">
            <Icon name="vault" size={22} /> The vault
          </h2>
          {ledger.data === undefined ? (
            <p className="text-fog p-5">
              {ledger.isError ? "Can’t reach the ledger." : "Loading…"}
            </p>
          ) : (
            <dl data-testid="vault-ledger">
              <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 px-5 py-4 border-b hair">
                <dt className="flex items-center gap-3 text-white">
                  <Icon name="mine" size={18} /> Deep Cache
                </dt>
                <dd className="flex-1 min-w-[8rem]">
                  <FitAmount size={28} min={13} align="end">
                    <CountUp value={zat(ledger.data.cache)} />
                  </FitAmount>
                </dd>
              </div>
              <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 px-5 py-4 border-b hair">
                <dt className="flex items-center gap-3 text-white">
                  <Icon name="coin" size={18} />
                  <span>
                    Operator float
                    <span className="block text-fog text-xs">pays network fees and refunds</span>
                  </span>
                </dt>
                <dd className="flex-1 min-w-[8rem]">
                  <FitAmount size={28} min={13} align="end">
                    {formatZec(zat(ledger.data.operator_float))}
                  </FitAmount>
                </dd>
              </div>
              {board.data !== undefined && (
                <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 px-5 py-4 border-b hair">
                  <dt className="flex items-center gap-3 text-white">
                    <Icon name="chest" size={18} /> This round’s prize
                  </dt>
                  <dd className="flex-1 min-w-[8rem]">
                    <FitAmount size={28} min={13} align="end" color="var(--color-gold)">
                      <CountUp value={zat(board.data.prize_pool_zat)} />
                    </FitAmount>
                  </dd>
                </div>
              )}
              {ledger.data.unclaimable > 0 && (
                <div className="flex justify-between px-5 py-3 text-fog text-sm border-b hair">
                  <dt>Unclaimable payouts (malformed dig memos), held in the vault</dt>
                  <dd className="font-mono">
                    {formatZec(zat(ledger.data.unclaimable), { decimals: 8 })}
                  </dd>
                </div>
              )}
              <div className="flex justify-between px-5 py-3 text-fog text-sm">
                <dt>Owed, not yet paid · vault balance</dt>
                <dd className="font-mono">
                  {formatZec(zat(ledger.data.unpaid_obligations), { decimals: 8 })} ·{" "}
                  {formatZec(zat(ledger.data.vault_balance), { decimals: 8 })}
                </dd>
              </div>
            </dl>
          )}
        </section>
      </div>
      <section className="panel p-5 flex flex-col gap-3" aria-label="Public viewing key">
        <h2 className="flex items-center gap-3 text-white font-extrabold text-xl">
          <Icon name="eye" size={22} /> Public viewing key
        </h2>
        <p className="text-mist">
          With this key anyone can watch every dig come in and every payout go out. It can only
          look; it can’t spend, and it doesn’t show who sent anything.
        </p>
        {uivk !== undefined && (
          <div className="flex gap-3 items-stretch">
            <code
              className="flex-1 border-2 border-slate px-3 py-3 font-mono text-sm text-mist truncate"
              data-testid="uivk"
            >
              {uivk.slice(0, 18)}…{uivk.slice(-14)}
            </code>
            <CopyButton text={uivk} />
          </div>
        )}
      </section>
    </main>
  );
}
