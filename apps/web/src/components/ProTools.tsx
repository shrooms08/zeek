/**
 * Pro mode on the dig panel: pick many tiles (ALL, Clear), quick amounts
 * (+0.001, +0.01, +0.1, MAX of the Pocket's spendable balance minus fees),
 * the exact total, auto-dig for N rounds (Zeek Pocket only), and this
 * browser's own dig history.
 *
 * Pro shows more about you, never more about anyone else: nothing here
 * reads other players' digs or per-tile totals. "Your digs" comes only from
 * this browser's own record (src/state/myDigs.ts) and the public result of
 * each round (the treasure tile).
 */

import { toPocketError } from "@zeek/pocket";
import { useState } from "react";
import { Link } from "react-router-dom";
import { useRounds } from "../api/queries";
import { digTotal } from "../lib/digPlan";
import type { DigMode } from "../lib/digMode";
import { formatZec, type Zat } from "../lib/money";
import { ALL_TILES, maxPerTile, QUICK_ADD } from "../lib/pro";
import { stopMessage } from "../lib/autoDig";
import { useAutoDig } from "../state/autoDig";
import { loadMyDigs } from "../state/myDigs";
import { Icon } from "./Icon";
import { Modal } from "./ui";

export const PRO_PROMISE = "Pro shows more about you. It never shows more about anyone else.";

export function ModeSwitch({
  mode,
  onChange,
}: {
  mode: DigMode;
  onChange: (mode: DigMode) => void;
}) {
  return (
    <div className="zk-mode flex" role="radiogroup" aria-label="Dig panel mode">
      {(["lite", "pro"] as const).map((m) => (
        <button
          key={m}
          type="button"
          role="radio"
          aria-checked={mode === m}
          className={`zk-mode-option ${mode === m ? "zk-mode-on" : ""}`}
          onClick={() => {
            onChange(m);
          }}
          data-testid={`mode-${m}`}
        >
          {m === "lite" ? "Lite" : "Pro"}
        </button>
      ))}
    </div>
  );
}

export interface ProToolsProps {
  tiles: number[];
  amountEach: Zat;
  minDig: Zat;
  separate: boolean;
  /** The Pocket's spendable balance; undefined unless it is unlocked. */
  spendable: Zat | undefined;
  closed: boolean;
  /** One round's length in ms (for auto-dig's hidden-tab rule). */
  roundMs: number;
  compact: boolean;
  onTiles: (tiles: number[]) => void;
  onAmount: (amount: Zat) => void;
}

export function ProTools(p: ProToolsProps) {
  const auto = useAutoDig();
  const [rounds, setRounds] = useState(1);
  const [confirming, setConfirming] = useState(false);
  const n = p.tiles.length;
  const total = digTotal(p.tiles, p.amountEach, p.separate);
  const max =
    p.spendable === undefined ? null : maxPerTile(p.spendable, p.tiles, p.separate, p.minDig);
  const pocket = p.spendable !== undefined;
  const running = auto.status.kind === "running";
  return (
    <div className="flex flex-col gap-3" data-testid="pro-tools">
      <div className="flex gap-2">
        <button
          type="button"
          className="btn flex-1"
          onClick={() => {
            p.onTiles([...ALL_TILES]);
          }}
          disabled={p.closed}
          data-testid="pro-all"
        >
          ALL
        </button>
        <button
          type="button"
          className="btn flex-1"
          onClick={() => {
            p.onTiles([]);
          }}
          disabled={n === 0}
          data-testid="pro-clear"
        >
          Clear
        </button>
      </div>
      <div className="grid grid-cols-4 gap-2" role="group" aria-label="Add to the amount per tile">
        {QUICK_ADD.map((step) => (
          <button
            key={step.toString()}
            type="button"
            className="btn zk-preset"
            onClick={() => {
              p.onAmount(p.amountEach + step);
            }}
            data-testid={`pro-add-${formatZec(step)}`}
          >
            +{formatZec(step).replace(/^0/, "")}
          </button>
        ))}
        <button
          type="button"
          className="btn zk-preset"
          onClick={() => {
            if (max !== null) p.onAmount(max);
          }}
          disabled={max === null}
          title={
            pocket
              ? "Your Pocket's spendable balance, minus fees, over the picked tiles"
              : "MAX needs an unlocked Zeek Pocket"
          }
          data-testid="pro-max"
        >
          MAX
        </button>
      </div>
      <p className="font-mono text-xs text-fog" data-testid="pro-total">
        {n} {n === 1 ? "tile" : "tiles"} × {formatZec(p.amountEach, { decimals: 8 })} + dig fees{" "}
        {formatZec(total.fee, { decimals: 8 })} ={" "}
        <span className="text-white">{formatZec(total.total, { decimals: 8 })}</span> ZEC
      </p>
      {pocket ? (
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-2 text-white">
            <span className="label text-fog" style={{ fontSize: 11 }}>
              Rounds
            </span>
            <input
              className="field-input w-20 text-center"
              type="number"
              inputMode="numeric"
              min={1}
              max={50}
              value={rounds}
              onChange={(e) => {
                const v = Math.floor(Number(e.target.value));
                setRounds(Number.isFinite(v) ? Math.max(1, Math.min(50, v)) : 1);
              }}
              disabled={running}
              data-testid="pro-rounds"
            />
          </label>
          <button
            type="button"
            className="btn flex-1"
            disabled={rounds < 2 || n === 0 || p.closed || running}
            onClick={() => {
              setConfirming(true);
            }}
            data-testid="pro-autodig"
          >
            Auto-dig {rounds} rounds
          </button>
        </div>
      ) : (
        <p className="text-fog text-xs">Auto-dig works with Zeek Pocket only.</p>
      )}
      <p className="text-fog text-xs" data-testid="pro-promise">
        {PRO_PROMISE}
      </p>
      {confirming && (
        <AutoDigConfirm
          rounds={rounds}
          perRound={total.total}
          tiles={p.tiles}
          amountEach={p.amountEach}
          separate={p.separate}
          roundMs={p.roundMs}
          onClose={() => {
            setConfirming(false);
          }}
        />
      )}
    </div>
  );
}

function AutoDigConfirm(props: {
  rounds: number;
  perRound: Zat;
  tiles: number[];
  amountEach: Zat;
  separate: boolean;
  roundMs: number;
  onClose: () => void;
}) {
  const auto = useAutoDig();
  const fullCap = props.perRound * BigInt(props.rounds);
  const [pass, setPass] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal title="Auto-dig with your Pocket" onClose={props.onClose} testId="autodig-confirm">
      <p className="text-mist mb-3">
        Your Pocket digs tiles {props.tiles.map((t) => t + 1).join(", ")} at{" "}
        {formatZec(props.amountEach, { decimals: 8 })} each, once in each new round, for{" "}
        {props.rounds} rounds, while this tab stays open.
      </p>
      <dl className="flex flex-col gap-1.5 text-sm mb-3">
        <div className="flex justify-between">
          <dt className="text-fog">Each round (fees included)</dt>
          <dd className="font-mono text-white">{formatZec(props.perRound, { decimals: 8 })}</dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-fog">Spend cap ({props.rounds} rounds)</dt>
          <dd className="font-mono text-white" data-testid="autodig-cap">
            {formatZec(fullCap, { decimals: 8 })}
          </dd>
        </div>
      </dl>
      <p className="text-fog text-sm mb-4">
        It stops by itself if your balance would not cover the next round, if this tab is in the
        background for a whole round, or on any error. Stop works at any time.
      </p>
      <form
        className="flex flex-col gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          const plan = {
            rounds: props.rounds,
            perRound: props.perRound,
            cap: fullCap,
            roundMs: props.roundMs,
          };
          const selection = {
            tiles: props.tiles,
            amountEach: props.amountEach,
            separate: props.separate,
          };
          auto.start(plan, selection, pass).then(
            () => {
              setPass("");
              props.onClose();
            },
            (err: unknown) => {
              setBusy(false);
              setPass("");
              const e = toPocketError(err);
              setError(
                e.code === "bad_passphrase"
                  ? "That passphrase didn’t work."
                  : `Auto-dig could not start: ${e.message}`,
              );
            },
          );
        }}
      >
        <input
          type="password"
          className="field-input"
          value={pass}
          onChange={(e) => {
            setPass(e.target.value);
          }}
          aria-label="Passphrase"
          placeholder="Passphrase"
          autoComplete="current-password"
          data-testid="autodig-passphrase"
        />
        {error !== null && (
          <p className="text-white" role="alert">
            {error}
          </p>
        )}
        <button
          type="submit"
          className="zk-dig flex items-center justify-center gap-2"
          style={{ fontSize: 20, minHeight: 56 }}
          disabled={busy || pass === ""}
          data-testid="autodig-start"
        >
          <Icon name="shovel" size={20} /> Start auto-dig
        </button>
      </form>
    </Modal>
  );
}

/** The running (or just ended) auto-dig, with its big Stop button. */
export function AutoDigCard() {
  const auto = useAutoDig();
  const s = auto.status;
  if (s.kind === "idle") return null;
  const { plan, progress } = s;
  return (
    <section
      className="border-2 border-white p-3 flex flex-col gap-2"
      aria-label="Auto-dig"
      data-testid="autodig-card"
    >
      <div className="flex items-baseline justify-between gap-2">
        <span className="label">Auto-dig</span>
        <span className="font-mono text-white" data-testid="autodig-counter">
          {progress.done} / {plan.rounds} rounds
        </span>
      </div>
      <p className="font-mono text-xs text-fog">
        Spent {formatZec(progress.spent, { decimals: 8 })} of cap{" "}
        {formatZec(plan.cap, { decimals: 8 })}
        {progress.rounds.length > 0 ? ` · rounds ${progress.rounds.join(", ")}` : ""}
      </p>
      {s.kind === "running" ? (
        <>
          <p className="text-white text-sm" role="status">
            {progress.sending
              ? "Sending this round’s dig…"
              : "Waiting for the next round. Keep this tab open."}
          </p>
          <button
            type="button"
            className="btn btn-primary w-full"
            style={{ fontSize: 20, minHeight: 56 }}
            onClick={auto.stop}
            data-testid="autodig-stop"
          >
            Stop
          </button>
        </>
      ) : (
        <>
          <p className="text-white text-sm" role="status" data-testid="autodig-ended">
            {stopMessage(s.reason)}
          </p>
          <button type="button" className="btn w-full" onClick={auto.dismiss}>
            OK
          </button>
        </>
      )}
    </section>
  );
}

/** This browser's own digs (local record only), newest first. */
export function YourDigs() {
  const rounds = useRounds();
  const mine = loadMyDigs()
    .filter((d) => d.round >= 0)
    .sort((a, b) => b.at - a.at)
    .slice(0, 12);
  const summary = new Map((rounds.data?.rounds ?? []).map((r) => [r.round, r]));
  return (
    <section className="flex flex-col gap-2" aria-label="Your digs" data-testid="your-digs">
      <div className="label text-fog" style={{ fontSize: 11 }}>
        Your digs (this browser)
      </div>
      {mine.length === 0 ? (
        <p className="text-fog text-sm">No digs from this browser yet.</p>
      ) : (
        <ul className="flex flex-col gap-1 text-sm">
          {mine.map((d) => {
            const r = summary.get(d.round);
            const result =
              r === undefined
                ? "not settled yet"
                : r.treasure_tile === null
                  ? "no draw"
                  : d.tiles.includes(r.treasure_tile)
                    ? `treasure in tile ${r.treasure_tile + 1}: yours`
                    : `treasure in tile ${r.treasure_tile + 1}`;
            return (
              <li
                key={`${d.round}-${d.at}`}
                className="flex flex-wrap items-baseline gap-x-2 border-b hair py-1"
                data-testid="your-dig"
              >
                <span className="font-mono text-white">R{d.round}</span>
                <span className="text-mist">
                  tiles {d.tiles.map((t) => t + 1).join(", ")} ×{" "}
                  {formatZec(BigInt(d.amountEach), { decimals: 8 })}
                </span>
                <span className="text-fog">· {result}</span>
                <Link className="underline text-white ml-auto" to={`/check/${d.round}`}>
                  Verify
                </Link>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
