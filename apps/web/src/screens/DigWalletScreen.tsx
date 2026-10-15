/** Dig with any Zcash wallet: where should winnings go → scan → waiting → confirmed. */

import { qrSvg, toPocketError } from "@zeek/pocket";
import { useEffect, useReducer, useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { engine } from "../api/engine";
import { usePocket } from "../api/PocketProvider";
import { useBoard, useConfig } from "../api/queries";
import { Zeb } from "../art/Characters";
import { Icon } from "../components/Icon";
import { Modal, QrCard } from "../components/ui";
import { checkPayoutAddress } from "../lib/address";
import { planPayments } from "../lib/digPlan";
import { formatZec, toNumber, zat } from "../lib/money";
import { formatClock } from "../lib/time";
import { useNow } from "../lib/useNow";
import {
  initialOtherWallet,
  memoDigest,
  withLanding,
  otherWalletReducer,
  paymentStatus,
  type PaymentModel,
  type PaymentStatus,
} from "../machines/otherWallet";
import { useDraft } from "../state/draft";
import { recordMyDig } from "../state/myDigs";

export default function DigWalletScreen() {
  const draft = useDraft();
  const board = useBoard();
  const config = useConfig();
  const { pocket } = usePocket();
  const navigate = useNavigate();
  const now = useNow();
  const [state, dispatch] = useReducer(otherWalletReducer, initialOtherWallet);
  const [address, setAddress] = useState("");
  const [building, setBuilding] = useState(false);
  const close = () => void navigate("/");
  const network = config.data?.params.network ?? "test";
  const minDig = zat(config.data?.params.min_dig_zat ?? 100_000);
  const amountEach = draft.amountEach < minDig ? minDig : draft.amountEach;
  const message = draft.message.trim();

  // Waiting: poll the round's memo digests until every payment landed.
  useEffect(() => {
    if (state.kind !== "waiting") return;
    let cancelled = false;
    const poll = async () => {
      try {
        const round = await engine.round(state.round);
        const seen = await engine.digests(state.round);
        if (!cancelled)
          dispatch({ type: "seen", status: paymentStatus(state.payments, seen, round.window.end) });
      } catch {
        // offline: keep waiting
      }
    };
    void poll();
    const id = setInterval(() => void poll(), 10_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [state]);

  const recorded = useRef(false);
  useEffect(() => {
    if (state.kind === "confirmed" && !recorded.current) {
      recorded.current = true;
      recordMyDig({
        round: state.round,
        tiles: state.payments.flatMap((p) => p.tiles),
        amountEach: amountEach.toString(),
        via: "wallet",
        txids: [],
        at: Date.now(),
      });
    }
  }, [state, amountEach]);

  const build = async () => {
    const verdict = checkPayoutAddress(address, network);
    if (!verdict.ok) {
      dispatch({ type: "address-error", message: verdict.message });
      return;
    }
    setBuilding(true);
    try {
      const p = await pocket();
      const groups = planPayments(draft.tiles, draft.separate);
      const payments: PaymentModel[] = [];
      let round = board.data?.round ?? 0;
      for (const tiles of groups) {
        const built = await p.buildDigRequest({
          payoutAddress: address.trim(),
          tiles,
          amountEachZat: toNumber(amountEach),
          ...(message.length > 0 ? { message } : {}),
        });
        round = built.round;
        payments.push({
          uri: built.uri,
          tiles,
          amount: zat(built.totalZat),
          memoDigests: await Promise.all(built.memos.map((m) => memoDigest(m))),
          qrSvg: built.qrSvg.length > 0 ? built.qrSvg : await qrSvg(built.uri),
        });
      }
      dispatch({ type: "built", round, payments });
    } catch (error) {
      dispatch({ type: "address-error", message: toPocketError(error).message });
    } finally {
      setBuilding(false);
    }
  };

  const landingDigests = new Set((board.data?.landing ?? []).map((d) => d.memo_sha256));
  const closesIn =
    board.data === undefined ? null : Math.max(0, board.data.estimated_close_time - now);

  switch (state.kind) {
    case "address":
      if (draft.tiles.length === 0) {
        return (
          <Modal title="Pick tiles first" onClose={close}>
            <Link to="/" className="btn btn-primary w-full block text-center">
              Back to the field
            </Link>
          </Modal>
        );
      }
      return (
        <Modal title="Where should winnings go?" onClose={close} testId="wallet-address">
          <p className="label mb-2">Step 1 of 3</p>
          <label className="flex flex-col gap-2">
            <span className="label">Private address</span>
            <textarea
              className="field-input font-mono text-sm"
              rows={3}
              value={address}
              onChange={(e) => {
                setAddress(e.target.value);
              }}
              placeholder={network === "test" ? "utest1…" : "u1…"}
              data-testid="payout-address"
              aria-invalid={state.error !== null}
              aria-describedby="address-help"
            />
          </label>
          <p
            id="address-help"
            className={state.error === null ? "text-fog text-sm my-2" : "text-white my-2"}
            role={state.error === null ? undefined : "alert"}
          >
            {state.error ??
              "Paste a fresh receive address from your wallet. We only use it to pay you. It’s never shown to anyone."}
          </p>
          <button
            type="button"
            className="btn btn-primary w-full"
            disabled={building}
            onClick={() => void build()}
            data-testid="wallet-next"
          >
            {building ? "Sealing your address…" : "Next"}
          </button>
        </Modal>
      );
    case "scan":
      return (
        <Modal title="Scan with your wallet" onClose={close} testId="wallet-scan">
          <p className="label mb-2">Step 2 of 3 · round {state.round}</p>
          <div className="flex flex-col gap-4">
            {state.payments.map((p, i) => (
              <QrCard
                key={p.uri}
                uri={p.uri}
                qrSvg={p.qrSvg}
                amount={formatZec(p.amount, { decimals: 8 })}
                index={i}
                total={state.payments.length}
                tiles={p.tiles}
              />
            ))}
          </div>
          {state.payments.length > 1 && (
            <p className="text-fog text-sm my-3">
              {draft.separate
                ? "Separate payments make it harder to link your tiles together. Send them in any order."
                : "Big digs go in payments of up to 6 tiles."}
            </p>
          )}
          <button
            type="button"
            className="btn btn-primary w-full mt-4"
            onClick={() => {
              dispatch({ type: "sent" });
            }}
            data-testid="wallet-sent"
          >
            I’ve sent {state.payments.length === 1 ? "it" : "them"}
          </button>
        </Modal>
      );
    case "waiting": {
      const shown = withLanding(state.payments, state.status, landingDigests);
      return (
        <Modal title="Waiting for your dig" onClose={close} testId="wallet-waiting">
          <p className="label text-fog mb-3">Step 3 of 3</p>
          <div className="flex justify-center mb-3">
            <Zeb pose="peek" width={110} label={null} />
          </div>
          <p className="text-mist mb-5 text-center">
            You’ll see it the moment it reaches the network; it counts once mined, usually within a
            minute or two. You can close this: it still counts if it lands before the round ends.
          </p>
          <ul className="zk-rows mb-5">
            {state.payments.map((p, i) => {
              const st = shown[i] ?? "not-sent";
              return (
                <li
                  key={p.uri}
                  className="flex items-center gap-3 px-4 py-3"
                  data-testid="payment-row"
                  data-status={st}
                >
                  <PaymentMark status={st} />
                  <span
                    className={
                      st === "not-sent" ? "text-white font-bold flex-1" : "text-white flex-1"
                    }
                  >
                    Payment {i + 1} ·{" "}
                    {st === "landed"
                      ? "landed"
                      : st === "landing"
                        ? "seen, landing"
                        : st === "late"
                          ? "late (refunded)"
                          : "looking…"}
                  </span>
                  <span className="font-mono text-sm text-fog">
                    {formatZec(p.amount, { decimals: 8 })}
                  </span>
                </li>
              );
            })}
          </ul>
          {closesIn !== null && board.data?.round === state.round && (
            <p className="text-fog text-sm flex items-center justify-center gap-2">
              <Icon name="hourglass" size={16} /> Round closes in {formatClock(closesIn)}
            </p>
          )}
        </Modal>
      );
    }
    case "confirmed": {
      const late = state.status.filter((s) => s === "late").length;
      return (
        <Modal
          title={`Your dig is in round ${state.round}`}
          onClose={close}
          testId="wallet-confirmed"
        >
          <p className="text-white mb-2">
            {state.status.length - late} of {state.payments.length} payments landed ·{" "}
            {formatZec(
              state.payments.reduce((s, p) => s + p.amount, 0n),
              { decimals: 8 },
            )}{" "}
            ZEC
          </p>
          {late > 0 && (
            <p className="text-mist mb-2">
              {late} arrived after the round closed. Late digs never count: they are refunded to
              your winnings address minus the fees (0.0001 ZEC).
            </p>
          )}
          <button
            type="button"
            className="btn btn-primary w-full"
            onClick={() => {
              draft.clear();
              close();
            }}
          >
            Back to the field
          </button>
        </Modal>
      );
    }
  }
}

/** A payment's state mark: ink check, settling dust, ink X, or a spinner. */
function PaymentMark({ status }: { status: PaymentStatus }) {
  if (status === "landed")
    return (
      <span className="zk-step zk-step-done">
        <Icon name="checkink" size={18} draw strokeWidth={3} />
      </span>
    );
  if (status === "landing")
    return (
      <span className="zk-step" aria-hidden="true">
        <Icon name="down2" size={18} />
      </span>
    );
  if (status === "late")
    return (
      <span className="zk-step">
        <Icon name="xink" size={18} strokeWidth={3} />
      </span>
    );
  return <span className="zk-spinner" aria-hidden="true" />;
}
