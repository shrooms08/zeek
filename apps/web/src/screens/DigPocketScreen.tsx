/** Dig with Zeek Pocket: confirm → sealing (per batch) → done / partial / error. */

import { toPocketError } from "@zeek/pocket";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useReducer } from "react";
import { Link, useNavigate } from "react-router-dom";
import { usePocket } from "../api/PocketProvider";
import { usePocketBalance } from "../api/pocketQueries";
import { useBoard, useConfig } from "../api/queries";
import { Zeb } from "../art/Characters";
import { Icon } from "../components/Icon";
import { SealRing } from "../components/SealRing";
import { BatchProgress, Loading, Modal, StepList } from "../components/ui";
import { useFieldFx } from "../state/fieldFx";
import { digTotal, MARGINAL_FEE, planPayments, paymentFee } from "../lib/digPlan";
import { formatZec, toNumber, zat } from "../lib/money";
import { digReducer, SEAL_STEPS } from "../machines/dig";
import { useDraft } from "../state/draft";
import { recordMyDig } from "../state/myDigs";
import { play } from "../sound/sound";

/** Most tiles affordable from `spendable` at `amountEach`. */
function affordable(spendable: bigint, amountEach: bigint, separate: boolean): number {
  for (let n = 25; n > 0; n -= 1) {
    if (
      digTotal(
        Array.from({ length: n }, (_, i) => i),
        amountEach,
        separate,
      ).total <= spendable
    )
      return n;
  }
  return 0;
}

export default function DigPocketScreen() {
  const draft = useDraft();
  const board = useBoard();
  const config = useConfig();
  const { pocket, wallet, refresh } = usePocket();
  const balance = usePocketBalance();
  const navigate = useNavigate();
  const client = useQueryClient();
  const [state, dispatch] = useReducer(digReducer, { kind: "confirm" });
  const close = () => void navigate("/");
  const fx = useFieldFx();
  const sealing = state.kind === "sealing";
  const { setSealing, launch } = fx;
  useEffect(() => {
    setSealing(sealing);
    return () => {
      setSealing(false);
    };
  }, [sealing, setSealing]);
  // Sealed: ink flies from the seal into your tiles on the field behind.
  const doneTiles = state.kind === "done" ? state.sent.flatMap((b) => b.tiles) : null;
  const doneKey = doneTiles?.join(",") ?? "";
  useEffect(() => {
    if (doneKey === "") return;
    launch(doneKey.split(",").map(Number));
    play("thunk");
  }, [doneKey, launch]);
  // Opened directly (or after a reload): ask the Pocket where it stands.
  useEffect(() => {
    if (wallet.kind === "unknown") void refresh();
  }, [wallet.kind, refresh]);

  if (wallet.kind === "unknown") return <Loading text="Opening your Pocket…" />;
  if (wallet.kind !== "unlocked" && wallet.kind !== "syncing") {
    return (
      <Modal title="Unlock your Pocket first" onClose={close}>
        <Link className="btn btn-primary w-full block text-center" to="/wallet?then=dig">
          Go to My wallet
        </Link>
      </Modal>
    );
  }
  const tiles = draft.tiles;
  const minDig = zat(config.data?.params.min_dig_zat ?? 100_000);
  const amountEach = draft.amountEach < minDig ? minDig : draft.amountEach;
  const total = digTotal(tiles, amountEach, draft.separate);
  const round = board.data?.round;
  const spendable = balance.data === undefined ? undefined : zat(balance.data.spendable_zat);
  const message = draft.message.trim();

  const start = async () => {
    dispatch({ type: "start", batches: planPayments(tiles, draft.separate) });
    try {
      const p = await pocket();
      const result = await p.dig(
        {
          tiles,
          amountEachZat: toNumber(amountEach),
          separate: draft.separate,
          ...(message.length > 0 ? { message } : {}),
        },
        (progress) => {
          dispatch({ type: "progress", progress });
        },
      );
      const sent = result.batches.map((b) => ({ txid: b.txid, tiles: b.tiles }));
      recordMyDig({
        round: result.round,
        tiles,
        amountEach: amountEach.toString(),
        via: "pocket",
        txids: sent.map((s) => s.txid),
        at: Date.now(),
      });
      dispatch({ type: "done", round: result.round, sent });
      draft.clear();
      draft.setMessage("");
      void client.invalidateQueries({ queryKey: ["pocket"] });
      void client.invalidateQueries({ queryKey: ["board"] });
    } catch (error) {
      const e = toPocketError(error);
      if (e.partial !== undefined && round !== undefined) {
        const sentTiles = e.partial.sent.flatMap((s) => s.tiles);
        recordMyDig({
          round,
          tiles: sentTiles,
          amountEach: amountEach.toString(),
          via: "pocket",
          txids: e.partial.sent.map((s) => s.txid),
          at: Date.now(),
        });
      }
      dispatch({
        type: "failed",
        code: e.code,
        message: e.message,
        ...(e.partial === undefined ? {} : { partial: e.partial }),
      });
    }
  };

  switch (state.kind) {
    case "idle":
    case "confirm": {
      const short = spendable !== undefined && spendable < total.total;
      if (short) {
        const fits = affordable(spendable, amountEach, draft.separate);
        return (
          <Modal title="Not enough in your Pocket" onClose={close} testId="dig-short">
            <div className="flex flex-col items-center gap-3 mb-5 text-center">
              <Zeb pose="shrug" width={96} label={null} />
              <p className="text-mist">
                You have <strong className="text-white">{formatZec(spendable)}</strong> spendable ·
                this dig needs{" "}
                <strong className="text-white">{formatZec(total.total, { decimals: 8 })}</strong>
                {balance.data !== undefined && balance.data.pending_zat > 0
                  ? ` (${formatZec(zat(balance.data.pending_zat))} more is still confirming)`
                  : ""}
              </p>
            </div>
            <div className="flex flex-col gap-2">
              <Link
                className="zk-dig flex items-center justify-center gap-2"
                style={{ fontSize: 22, minHeight: 60 }}
                to="/wallet"
              >
                <Icon name="plus" size={20} /> Add ZEC
              </Link>
              {fits > 0 && (
                <button
                  type="button"
                  className="btn"
                  onClick={() => {
                    draft.setTiles(tiles.slice(0, fits));
                  }}
                >
                  Dig {fits} {fits === 1 ? "tile" : "tiles"} instead
                </button>
              )}
              <Link className="btn text-center" to="/dig/wallet">
                Pay from another wallet instead
              </Link>
            </div>
          </Modal>
        );
      }
      return (
        <Modal title="Check your dig" onClose={close} testId="dig-confirm">
          <div className="grid grid-cols-2 gap-3 mb-4">
            <div className="zk-stat p-3">
              <div className="label flex items-center gap-2">
                <Icon name="tile" size={16} /> Tiles
              </div>
              <div className="numeral" style={{ fontSize: 44, lineHeight: 1 }}>
                {tiles.length}
              </div>
            </div>
            <div className="zk-stat p-3">
              <div className="label flex items-center gap-2">
                <Icon name="coin" size={16} /> Each
              </div>
              <div className="numeral" style={{ fontSize: 44, lineHeight: 1 }}>
                {formatZec(amountEach)}
              </div>
            </div>
          </div>
          <dl className="flex flex-col gap-1.5 text-sm mb-3">
            <div className="flex justify-between">
              <dt className="text-fog">
                Dig · {tiles.length} × {formatZec(amountEach, { decimals: 8 })}
              </dt>
              <dd className="font-mono text-white">{formatZec(total.dug, { decimals: 8 })}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-fog">
                Fee · {formatZec(MARGINAL_FEE, { decimals: 8 })} per tile + per payment
              </dt>
              <dd className="font-mono text-white">{formatZec(total.fee, { decimals: 8 })}</dd>
            </div>
          </dl>
          <div className="flex items-baseline justify-between border-t hair pt-3 mb-3">
            <span className="text-white font-extrabold text-xl">Total</span>
            <span className="numeral" style={{ fontSize: 26 }} data-testid="confirm-total">
              {formatZec(total.total, { decimals: 8 })} ZEC
            </span>
          </div>
          {total.payments.length > 1 && (
            <p className="text-fog text-sm mb-2">
              {total.payments.length} payments: {total.payments.map((p) => p.length).join(" + ")}{" "}
              tiles
              {draft.separate ? ", sent 2–20 s apart" : ""}.
            </p>
          )}
          {message.length > 0 && <p className="text-fog text-sm mb-2">Message: “{message}”</p>}
          <p className="text-fog mb-5 text-sm flex items-center gap-2">
            <Icon name="wallet" size={16} />
            From Zeek Pocket · {spendable === undefined ? "…" : formatZec(spendable)} ZEC · round{" "}
            {round ?? "…"}
          </p>
          <button
            type="button"
            className="zk-dig flex items-center justify-center gap-3"
            style={{ fontSize: 24, minHeight: 64 }}
            disabled={tiles.length === 0 || spendable === undefined}
            onClick={() => void start()}
            data-testid="dig-privately"
          >
            <Icon name="shovel" size={24} /> Dig privately
          </button>
          <p className="text-fog text-xs mt-3">
            The game also takes 0.00005 per dig from the prize at the end of the round.
          </p>
        </Modal>
      );
    }
    case "sealing": {
      const fraction = (state.current + (state.step + 1) / 3) / Math.max(1, state.batches.length);
      return (
        <Modal title="Sealing your dig" testId="dig-sealing">
          <div className="label text-fog mb-4">Round {round ?? "…"}</div>
          <div className="flex justify-center mb-6">
            <SealRing fraction={Math.min(0.97, fraction)} />
          </div>
          <BatchProgress
            batches={state.batches}
            current={state.current}
            fill={(state.step + 1) / 3}
          />
          <div className="my-5">
            <StepList steps={SEAL_STEPS} current={state.step} />
          </div>
          {state.waiting !== null && (
            <p className="text-white" role="status" data-testid="dig-waiting">
              {state.waiting.for === "privacy-gap"
                ? `Waiting ${state.waiting.seconds} s before the next payment, so they can’t be tied together.`
                : `Waiting for your last payment’s change to confirm (${state.waiting.seconds} s so far).`}
            </p>
          )}
          {state.sent.length > 0 && (
            <p className="text-fog text-sm">
              Sent: {state.sent.length} of {state.batches.length}
            </p>
          )}
          <p className="text-fog text-sm mt-3">
            Keep this open. Sealing happens on your device; nobody else sees it.
          </p>
        </Modal>
      );
    }
    case "done":
      return (
        <Modal
          title={`Your dig is in round ${state.round}`}
          onClose={close}
          testId="dig-done"
          dim={false}
        >
          <p className="label text-fog mb-2 flex items-center gap-2">
            <Icon name="checkink" size={18} draw strokeWidth={3} /> In the ground
          </p>
          <p className="text-white mb-1">
            {state.sent.flatMap((s) => s.tiles).length} tiles ·{" "}
            {formatZec(total.total, { decimals: 8 })} ZEC
          </p>
          <p className="text-fog mb-4">The holes got a little deeper. Only you know why.</p>
          <details className="mb-4 text-fog text-sm">
            <summary className="cursor-pointer">Transactions</summary>
            <ul className="font-mono text-xs mt-2 break-all" data-testid="dig-txids">
              {state.sent.map((s) => (
                <li key={s.txid}>{s.txid}</li>
              ))}
            </ul>
          </details>
          <div className="flex flex-col gap-2">
            <button type="button" className="btn btn-primary" onClick={close}>
              Back to the field
            </button>
            <Link className="btn text-center flex items-center justify-center gap-2" to="/campfire">
              <Icon name="fire" size={18} /> Say something at the campfire
            </Link>
          </div>
        </Modal>
      );
    case "partial":
      return (
        <Modal title="Part of your dig went in" onClose={close} testId="dig-partial">
          <p className="text-mist mb-3">{state.message}</p>
          <p className="text-white flex items-start gap-2">
            <Icon name="checkink" size={18} strokeWidth={3} />
            In the round: tiles{" "}
            {state.sent
              .flatMap((s) => s.tiles)
              .map((t) => t + 1)
              .join(", ")}
          </p>
          <p className="text-white mb-5 flex items-start gap-2">
            <Icon name="xink" size={18} strokeWidth={3} />
            Not sent: tiles {state.unsent.map((t) => t + 1).join(", ")}
          </p>
          <button
            type="button"
            className="btn btn-primary w-full"
            onClick={() => {
              draft.setTiles(state.unsent);
              dispatch({ type: "review" });
              void navigate("/");
            }}
          >
            Pick the rest again
          </button>
        </Modal>
      );
    case "error":
      return (
        <Modal title="That dig didn’t go through" onClose={close} testId="dig-error">
          <div className="flex justify-center mb-3">
            <Zeb pose="shrug" width={88} label={null} />
          </div>
          <p className="text-mist mb-2">{state.message}</p>
          <p className="text-fog text-sm mb-5">Nothing was sent.</p>
          <button
            type="button"
            className="btn btn-primary w-full"
            onClick={() => {
              dispatch({ type: "review" });
            }}
          >
            Try again
          </button>
        </Modal>
      );
  }
  return null;
}

export { paymentFee };
