/** Check a round: five checks ticking in this browser, plus the second-source cross-check. */

import type { SecondSource } from "@zeek/pocket";
import { toPocketError } from "@zeek/pocket";
import { useEffect, useReducer, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { ENGINE_URL } from "../api/engine";
import { usePocket } from "../api/PocketProvider";
import { useConfig, useRounds } from "../api/queries";
import { Icon } from "../components/Icon";
import { CheckRow, CopyButton } from "../components/ui";
import { checkReducer, visibleRows } from "../machines/check";

/** A second source from the build (VITE_ZEEK_SECOND_SOURCE), if configured. */
const CONFIGURED_SECOND: string =
  (import.meta.env.VITE_ZEEK_SECOND_SOURCE as string | undefined) ?? "";

export default function CheckScreen() {
  const param = useParams().round;
  const navigate = useNavigate();
  const rounds = useRounds();
  const config = useConfig();
  const { pocket } = usePocket();
  const [state, dispatch] = useReducer(checkReducer, { kind: "idle" });
  const [round, setRound] = useState(param ?? "");
  const [second, setSecond] = useState(CONFIGURED_SECOND);

  useEffect(() => {
    if (param === undefined && round === "" && rounds.data?.rounds[0] !== undefined)
      setRound(String(rounds.data.rounds[0].round));
  }, [param, round, rounds.data]);

  useEffect(() => {
    if (state.kind !== "revealing") return;
    const id = setTimeout(() => {
      dispatch({ type: "tick" });
    }, 600);
    return () => {
      clearTimeout(id);
    };
  }, [state]);

  const run = async () => {
    const n = Number(round);
    if (!Number.isInteger(n) || n < 0) return;
    if (param !== String(n)) void navigate(`/check/${n}`, { replace: true });
    dispatch({ type: "start" });
    try {
      const p = await pocket();
      const secondSource: SecondSource | undefined =
        second.trim() === "" ? undefined : { kind: "relay", url: second.trim() };
      const result = await p.verifyRound(
        secondSource === undefined ? { round: n } : { round: n, secondSource },
        (progress) => {
          dispatch({ type: "progress", progress });
        },
      );
      dispatch({ type: "result", result });
    } catch (error) {
      dispatch({ type: "failed", message: toPocketError(error).message });
    }
  };

  const rows = visibleRows(state);
  const finished = state.kind === "done" ? state.result : null;
  const cross = finished?.crossCheck;
  return (
    <main
      className="mx-auto w-full max-w-[800px] px-4 sm:px-6 py-8 sm:py-12 flex flex-col gap-6"
      data-testid="check-screen"
    >
      <div>
        <div className="label text-fog mb-2">Check a round</div>
        <h1 className="zk-headline" style={{ margin: 0 }}>
          Don’t trust us. Check.
        </h1>
      </div>
      <form
        className="flex gap-3 items-stretch"
        onSubmit={(e) => {
          e.preventDefault();
          void run();
        }}
      >
        <label className="flex flex-1 items-center gap-3 border-[3px] border-white px-4">
          <span className="text-fog">Round</span>
          <input
            className="numeral bg-transparent w-full min-h-[56px] outline-none"
            style={{ fontSize: 22, border: "none" }}
            inputMode="numeric"
            value={round}
            onChange={(e) => {
              setRound(e.target.value);
            }}
            data-testid="check-round"
          />
        </label>
        <button
          type="submit"
          className="btn btn-primary px-10"
          style={{ fontSize: 20 }}
          disabled={state.kind === "running" || state.kind === "revealing"}
          data-testid="check-run"
        >
          Check
        </button>
      </form>
      <ol className="zk-checklist" data-testid="check-rows">
        {rows.map((r) => (
          <CheckRow
            key={r.id}
            id={r.id}
            title={r.title}
            text={r.text}
            state={r.state}
            detail={r.detail}
            testId={`check-${r.id}`}
          />
        ))}
      </ol>
      {state.kind === "error" && (
        <p className="text-white flex items-center gap-2" role="alert">
          <Icon name="warning" size={20} /> The check couldn’t run: {state.message}
        </p>
      )}
      {finished !== null && (
        <p
          className="text-white text-lg font-bold flex items-center gap-2"
          data-testid="check-summary"
        >
          <Icon
            name={finished.report.checks.every((c) => c.status !== "fail") ? "shield" : "warning"}
            size={22}
          />
          {finished.report.checks.every((c) => c.status !== "fail")
            ? "Every check that applies passed."
            : "A check failed. The keyholders would already have refused a wrong payout."}
        </p>
      )}
      {cross !== undefined && (
        <section className="zk-stat p-4" data-testid="cross-check" data-status={cross.status}>
          <div className="label flex items-center gap-2">
            <Icon name="globe" size={16} />
            Second source ·{" "}
            {cross.status === "pass"
              ? "matches"
              : cross.status === "fail"
                ? "DISAGREES"
                : "unavailable"}
          </div>
          <p className="text-mist mt-1">{cross.detail}</p>
          <p className="text-fog text-sm">Source: {cross.source}</p>
        </section>
      )}
      <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm">
        <a
          className="flex items-center gap-2 text-white underline"
          href={`${ENGINE_URL}/v1/rounds/${round}/proof`}
          target="_blank"
          rel="noreferrer"
        >
          <Icon name="link" size={16} /> See the raw proof
        </a>
        <span className="flex items-center gap-2 text-fog">
          <Icon name="key" size={16} /> Uses the public viewing key
          {config.data !== undefined && (
            <span className="font-mono text-xs">
              {config.data.uivk.slice(0, 16)}…{config.data.uivk.slice(-8)}
            </span>
          )}
        </span>
        {config.data !== undefined && <CopyButton text={config.data.uivk} label="Copy key" />}
      </div>
      <details className="text-mist">
        <summary className="cursor-pointer text-white">Second source</summary>
        <label className="flex flex-col gap-1 mt-2">
          <span className="text-fog text-sm">
            A relay that reads another lightwalletd (anyone can run `zeek-verify relay`). Testnet
            has no public one, so without it the cross-check says “unavailable”.
          </span>
          <input
            className="field-input font-mono text-sm"
            value={second}
            onChange={(e) => {
              setSecond(e.target.value);
            }}
            placeholder="https://…"
            data-testid="check-second"
          />
        </label>
      </details>
      <section className="zk-warn flex gap-3">
        <Icon name="warning" size={22} />
        <div>
          <div className="text-white font-bold">If a check fails</div>
          <p className="text-mist text-sm">
            It turns into a dashed ✕ with the exact mismatch. The keyholders check every payout
            against the same rules, and would already have refused it.
          </p>
        </div>
      </section>
    </main>
  );
}
