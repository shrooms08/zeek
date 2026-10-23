/**
 * Public status: is each part of Zeek working right now? Every row says what
 * was checked and when, and a part that cannot be reached says so rather
 * than showing old numbers as current.
 */

import { useQuery } from "@tanstack/react-query";
import { engine, EngineError, SECOND_SOURCE } from "../api/engine";
import { keys } from "../api/queries";
import type { ChainServerStatus, KeyholderStatus } from "../api/types";
import { Icon } from "../components/Icon";
import { Screen } from "../components/ui";
import { formatAgo } from "../lib/time";
import { vaultHealth } from "../lib/vaultHealth";
import { useNow } from "../lib/useNow";

/** A keyholder check older than this is reported as unknown. */
const STALE_CHECK_SECS = 5 * 60;
/** The engine is "behind" when the chain is this many blocks ahead of it. */
const BEHIND_BLOCKS = 3;

type Health = "ok" | "down" | "unknown" | "warn";

function Row({
  health,
  title,
  detail,
  testId,
}: {
  health: Health;
  title: string;
  detail: string;
  testId?: string;
}) {
  const word = { ok: "Working", down: "Down", unknown: "Unknown", warn: "Degraded" }[health];
  return (
    <li
      className="flex items-start justify-between gap-4 px-5 py-4 border-b hair"
      data-testid={testId}
      data-health={health}
    >
      <div className="min-w-0">
        <div className="text-white font-bold [overflow-wrap:anywhere]">{title}</div>
        <div className="text-fog text-sm [overflow-wrap:anywhere]">{detail}</div>
      </div>
      <span
        className={`zk-chip flex shrink-0 items-center gap-1.5 ${health === "ok" ? "zk-chip-fill" : ""}`}
      >
        <Icon
          name={health === "ok" ? "check" : health === "down" ? "offline" : "warning"}
          size={14}
          strokeWidth={2.5}
        />
        {word}
      </span>
    </li>
  );
}

function keyholderRow(k: KeyholderStatus, now: number) {
  if (k.last_check === null || now - k.last_check > STALE_CHECK_SECS) {
    return {
      health: "unknown" as const,
      detail:
        k.last_check === null
          ? "Not checked yet."
          : `Last checked ${formatAgo(k.last_check, now)}; the engine has not checked since.`,
    };
  }
  if (k.reachable === true) {
    return {
      health: "ok" as const,
      detail: `Answered ${formatAgo(k.last_seen ?? k.last_check, now)}.`,
    };
  }
  return {
    health: "down" as const,
    detail:
      k.last_seen === null
        ? "Does not answer."
        : `Does not answer; last answered ${formatAgo(k.last_seen, now)}.`,
  };
}

function serverRow(s: ChainServerStatus, now: number) {
  const host = s.url.replace(/^https?:\/\//, "");
  const seen = s.last_ok === null ? "never answered" : `answered ${formatAgo(s.last_ok, now)}`;
  const tip = s.tip === null ? "" : `, at block ${s.tip.toLocaleString("en-US")}`;
  const latency = s.latency_ms === null ? "" : `, ${s.latency_ms} ms`;
  const outvoted = s.outvoted > 0 ? `; outvoted ${s.outvoted}×` : "";
  return {
    title: host,
    health: s.healthy ? ("ok" as const) : ("down" as const),
    detail: s.healthy
      ? `${seen}${tip}${latency}${outvoted}`
      : `${s.last_error ?? "failing"} (${seen})${outvoted}`,
  };
}

export default function StatusScreen() {
  const now = useNow();
  const status = useQuery({
    queryKey: keys.status,
    queryFn: engine.status,
    // Every 30 s is enough to watch an outage, and spares the free quota.
    refetchInterval: 30_000,
    retry: false,
  });
  const ledger = useQuery({
    queryKey: ["ledger"],
    queryFn: engine.ledger,
    refetchInterval: 60_000,
    retry: false,
  });
  const relay = useQuery({
    queryKey: ["relay-tip"],
    queryFn: async () => {
      const response = await fetch(`${SECOND_SOURCE}/v1/chain/tip`, {
        signal: AbortSignal.timeout(10_000),
      });
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      return (await response.json()) as { height: number };
    },
    enabled: SECOND_SOURCE !== "",
    refetchInterval: 30_000,
    retry: false,
  });

  const s = status.data;
  const engineError =
    status.error instanceof EngineError ? status.error.message : "The engine does not answer.";
  const lastBlockTime = s?.engine.last_block_time ?? null;
  const behind =
    s?.engine.chain_tip != null && s.engine.stored_tip != null
      ? s.engine.chain_tip - s.engine.stored_tip
      : 0;
  const healthyServers = s?.chain_servers.filter((c) => c.healthy).length ?? 0;

  return (
    <Screen title="Status" kicker="Is everything working?" testId="status-screen">
      <p className="text-mist">
        Live checks of each part of Zeek, refreshed every 30 seconds. Zeek’s engine, relay and three
        keyholders run as separate services on Railway, all operated by the Zeek team; the
        keyholders are reachable only on Railway’s private network.
      </p>
      <section className="panel" aria-label="Services">
        <ul>
          <Row health="ok" title="This site" detail="Loaded." testId="status-site" />
          {s === undefined ? (
            <Row
              health={status.isPending ? "unknown" : "down"}
              title="Game engine"
              detail={status.isPending ? "Checking…" : engineError}
              testId="status-engine"
            />
          ) : (
            <Row
              health={behind > BEHIND_BLOCKS ? "warn" : "ok"}
              title="Game engine"
              detail={[
                `${s.network === "main" ? "Mainnet" : "Testnet"}, digs ${s.digging_enabled ? "open" : "closed"}.`,
                s.engine.stored_tip === null
                  ? "No block followed yet."
                  : `Last block ${s.engine.stored_tip.toLocaleString("en-US")}${lastBlockTime === null ? "" : `, mined ${formatAgo(lastBlockTime, now)}`}.`,
                behind > BEHIND_BLOCKS ? `Catching up: ${behind} blocks behind the chain.` : "",
                s.engine.last_settled_round === null
                  ? ""
                  : `Last settled round ${s.engine.last_settled_round}.`,
              ]
                .filter((t) => t !== "")
                .join(" ")}
              testId="status-engine"
            />
          )}
          {ledger.data !== undefined && (
            <Row
              {...vaultHealth(ledger.data, s?.engine.stored_tip ?? null)}
              title="Vault"
              testId="status-vault"
            />
          )}
          {SECOND_SOURCE !== "" && (
            <Row
              health={relay.data !== undefined ? "ok" : relay.isPending ? "unknown" : "down"}
              title="Second-source relay"
              detail={
                relay.data !== undefined
                  ? `At block ${relay.data.height.toLocaleString("en-US")}.`
                  : relay.isPending
                    ? "Checking…"
                    : "Does not answer; round checks show the cross-check as unavailable."
              }
              testId="status-relay"
            />
          )}
        </ul>
      </section>

      {s !== undefined && (
        <section className="panel" aria-label="Keyholders">
          <h2 className="px-5 pt-4 pb-2 text-white font-extrabold">
            Keyholders ({s.threshold} of {s.keyholders.length} sign each payout)
          </h2>
          <ul>
            {s.keyholders.map((k) => {
              const row = keyholderRow(k, now);
              return (
                <Row
                  key={k.identifier}
                  health={row.health}
                  title={k.label}
                  detail={row.detail}
                  testId="status-keyholder"
                />
              );
            })}
          </ul>
        </section>
      )}

      {s !== undefined && (
        <section className="panel" aria-label="Chain servers">
          <h2 className="px-5 pt-4 pb-2 text-white font-extrabold">
            Chain servers ({healthyServers} of {s.chain_servers.length} working)
          </h2>
          {s.chain_servers.length === 1 && (
            <p className="px-5 text-fog text-sm">
              Only one public lightwalletd server serves this network, so blocks cannot be compared
              against a second server.
            </p>
          )}
          <ul>
            {s.chain_servers.map((c) => {
              const row = serverRow(c, now);
              return (
                <Row
                  key={c.url}
                  health={row.health}
                  title={row.title}
                  detail={row.detail}
                  testId="status-server"
                />
              );
            })}
          </ul>
        </section>
      )}
    </Screen>
  );
}
