/** Check a round: five checks ticking in this browser, plus the second-source cross-check. */

import type { SecondSource } from "@zeek/pocket";
import { toPocketError } from "@zeek/pocket";
import { useEffect, useReducer, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { ENGINE_URL } from "../api/engine";
import { usePocket } from "../api/PocketProvider";
import { useConfig, useRounds } from "../api/queries";
import { CheckRow, CopyButton, Screen } from "../components/ui";
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
    <Screen title="Check a round" testId="check-screen">
      <p className="label">Don’t trust us. Check.</p>
      <form
        className="flex gap-2 items-end"
        onSubmit={(e) => {
          e.preventDefault();
          void run();
        }}
      >
        <label className="flex flex-col gap-1 flex-1">
          <span className="label">Round</span>
          <input
            className="field-input numeral"
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
          className="btn btn-primary"
          disabled={state.kind === "running" || state.kind === "revealing"}
          data-testid="check-run"
        >
          Check
        </button>
      </form>
      <details>
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
      <ol data-testid="check-rows">
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
        <p className="text-white" role="alert">
          The check couldn’t run: {state.message}
        </p>
      )}
      {cross !== undefined && (
        <section className="panel p-4" data-testid="cross-check" data-status={cross.status}>
          <div className="label">
            Second source ·{" "}
            {cross.status === "pass"
              ? "matches"
              : cross.status === "fail"
                ? "DISAGREES"
                : "unavailable"}
          </div>
          <p className="text-mist">{cross.detail}</p>
          <p className="text-fog text-sm">Source: {cross.source}</p>
        </section>
      )}
      {finished !== null && (
        <p className="text-white" data-testid="check-summary">
          {finished.report.checks.every((c) => c.status !== "fail")
            ? "Every check that applies passed."
            : "A check failed. The keyholders would already have refused a wrong payout."}
        </p>
      )}
      <section className="panel p-4 flex flex-col gap-2">
        <a
          className="underline text-white"
          href={`${ENGINE_URL}/v1/rounds/${round}/proof`}
          target="_blank"
          rel="noreferrer"
        >
          See the raw proof
        </a>
        <span className="text-fog text-sm">
          Runs in your browser against the chain, using the public viewing key:
        </span>
        {config.data !== undefined && (
          <div className="flex items-center gap-2">
            <span className="font-mono text-xs break-all text-fog">
              {config.data.uivk.slice(0, 24)}…{config.data.uivk.slice(-10)}
            </span>
            <CopyButton text={config.data.uivk} />
          </div>
        )}
      </section>
    </Screen>
  );
}
