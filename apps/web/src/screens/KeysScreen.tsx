/** Who holds the keys: the 2-of-3 keyholders, refusals, the vault and the viewing key. */

import { useBoard, useConfig, useKeyholders, useLedger } from "../api/queries";
import type { KeyholderStatus } from "../api/types";
import { CopyButton, Loading, Offline, Panel, Screen } from "../components/ui";
import { formatZec, zat } from "../lib/money";
import { formatAgo } from "../lib/time";
import { useNow } from "../lib/useNow";

function KeyholderCard({ k, now }: { k: KeyholderStatus; now: number }) {
  const state = k.reachable === null ? "NOT CHECKED YET" : k.reachable ? "ONLINE" : "OFFLINE";
  return (
    <li className="panel p-4 flex flex-col gap-1" data-testid="keyholder-card">
      <span
        className={`label self-start px-2 py-1 border-2 ${k.reachable === true ? "border-white text-white" : "border-dashed border-fog text-fog"}`}
      >
        {state}
      </span>
      <h3 className="text-heading font-extrabold text-white">{k.label}</h3>
      <p className="text-fog">Run by the {k.operator}</p>
      <p className="text-mist text-sm">
        {k.reachable === false
          ? `last seen ${k.last_seen === null ? "never" : formatAgo(k.last_seen, now)} · payouts still work`
          : `approved ${k.approved} · refused ${k.refused}`}
      </p>
      {k.last_check !== null && (
        <p className="text-fog text-xs">checked {formatAgo(k.last_check, now)}</p>
      )}
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
    <Screen title="Who holds the keys" testId="keys-screen" wide>
      <p className="text-mist">
        Three keyholders guard the prize. Any {k.threshold} must sign each payout. Each one checks
        every payout against the rules on its own, and refuses anything wrong. If one is offline,
        payouts still work.
      </p>
      <ul className="grid gap-3 sm:grid-cols-3">
        {k.keyholders.map((h) => (
          <KeyholderCard key={h.identifier} k={h} now={now} />
        ))}
      </ul>
      <p className="text-white" data-testid="keys-honest">
        Today all three are run by the Zeek team. Next: independent operators.
      </p>
      <Panel label="Refused payouts">
        <h3 className="label mb-2">Refused payouts</h3>
        {k.refused_attempts.length === 0 ? (
          <p className="text-fog">None recorded. Refusals are recorded from Phase 7 onward.</p>
        ) : (
          <ul data-testid="refusals">
            {k.refused_attempts.map((r, i) => (
              <li key={i} className="border-b hair py-2">
                <div className="text-white">Round {r.rounds.join(", ")}</div>
                <div className="text-mist">
                  {r.reason}. Refused by {r.refusals}{" "}
                  {r.refusals === 1 ? "keyholder" : "keyholders"}.
                </div>
                <div className="text-fog text-xs">{formatAgo(r.time, now)}</div>
              </li>
            ))}
          </ul>
        )}
      </Panel>
      <Panel label="The vault">
        <h3 className="label mb-2">The vault</h3>
        {ledger.data === undefined ? (
          <p className="text-fog">{ledger.isError ? "Can’t reach the ledger." : "Loading…"}</p>
        ) : (
          <dl className="grid gap-2" data-testid="vault-ledger">
            <div className="flex justify-between">
              <dt>Deep Cache</dt>
              <dd className="numeral" style={{ color: "var(--color-gold)" }}>
                {formatZec(zat(ledger.data.cache))}
              </dd>
            </div>
            <div className="flex justify-between">
              <dt>
                Operator float{" "}
                <span className="text-fog text-sm">pays network fees and refunds</span>
              </dt>
              <dd className="numeral">{formatZec(zat(ledger.data.operator_float))}</dd>
            </div>
            {board.data !== undefined && (
              <div className="flex justify-between">
                <dt>This round’s prize</dt>
                <dd className="numeral" style={{ color: "var(--color-gold)" }}>
                  {formatZec(zat(board.data.prize_pool_zat))}
                </dd>
              </div>
            )}
            <div className="flex justify-between text-fog">
              <dt>Owed, not yet paid</dt>
              <dd className="font-mono">
                {formatZec(zat(ledger.data.unpaid_obligations), { decimals: 8 })}
              </dd>
            </div>
            <div className="flex justify-between text-fog">
              <dt>Vault balance</dt>
              <dd className="font-mono">
                {formatZec(zat(ledger.data.vault_balance), { decimals: 8 })}
              </dd>
            </div>
          </dl>
        )}
      </Panel>
      <Panel label="Public viewing key">
        <h3 className="label mb-2">Public viewing key</h3>
        <p className="text-mist mb-2">
          With this key anyone can watch every dig come in and every payout go out. It can only
          look; it can’t spend, and it doesn’t show who sent anything.
        </p>
        {uivk !== undefined && (
          <div className="flex gap-2 items-center flex-wrap">
            <code className="font-mono text-sm break-all text-white" data-testid="uivk">
              {uivk.slice(0, 16)}…{uivk.slice(-12)}
            </code>
            <CopyButton text={uivk} />
          </div>
        )}
      </Panel>
    </Screen>
  );
}
