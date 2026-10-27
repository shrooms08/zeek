/** My wallet (Zeek Pocket). Lives in this browser. */

import { FitAmount } from "../components/FitAmount";
import { qrSvg, toPocketError, type HistoryEntry } from "@zeek/pocket";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { usePocket } from "../api/PocketProvider";
import { usePocketBalance, usePocketHistory } from "../api/pocketQueries";
import { useBlockSpacing, useConfig } from "../api/queries";
import { FaucetCard } from "../components/FaucetCard";
import { motion, useReducedMotion } from "framer-motion";
import { Zeb } from "../art/Characters";
import { Icon, type IconName } from "../components/Icon";
import { CopyButton, Loading, Panel, Screen } from "../components/ui";
import { CountUp } from "../motion/CountUp";
import { EASE } from "../motion/tokens";
import { checkPayoutAddress, shortAddress } from "../lib/address";
import { formatZec, parseZec, toNumber, zat } from "../lib/money";
import { browserClearEnv, clearIfUnchanged, CLEAR_AFTER_MS } from "../lib/clipboard";
import { readyIn } from "../lib/readiness";
import { checkPositions, passphraseStrength } from "../lib/passphrase";
import { formatTimeOfDay } from "../lib/time";
import { useNow } from "../lib/useNow";
import { canTryUnlock } from "../machines/wallet";
import { autoDigRunning } from "../state/autoDigFlag";
import { asWithdrawal, withdrawing } from "../state/pocketActivity";
import { MAYBE_SENT_HOLD_MS, useHold } from "../lib/useHold";

function random(): number {
  const b = new Uint32Array(1);
  crypto.getRandomValues(b);
  return (b[0] ?? 0) / 0x1_0000_0000;
}

const STRENGTH_TEXT = {
  "too-short": "Too short: at least 8 characters.",
  ok: "OK. Longer is stronger.",
  strong: "Strong. 4 words or 12+ letters.",
} as const;

/** Four segments: too short fills one, OK two, strong all four. */
function StrengthMeter({ strength }: { strength: ReturnType<typeof passphraseStrength> | null }) {
  const filled = strength === null ? 0 : strength === "too-short" ? 1 : strength === "ok" ? 2 : 4;
  return (
    <span className="grid grid-cols-4 gap-1.5" aria-hidden="true">
      {[0, 1, 2, 3].map((k) => (
        <span
          key={k}
          className="h-1.5"
          style={{ background: k < filled ? "var(--color-white)" : "var(--color-slate)" }}
        />
      ))}
    </span>
  );
}

function CreateCard() {
  const { pocket, dispatch } = usePocket();
  const [show, setShow] = useState(false);
  const [pass, setPass] = useState("");
  const [again, setAgain] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [words, setWords] = useState<string[] | null>(null);
  const strength = passphraseStrength(pass);
  if (words !== null)
    return (
      <BackupWords
        words={words}
        onDone={() => {
          setWords(null);
          dispatch({ type: "backed-up" });
        }}
      />
    );
  return (
    <Panel label="Make your Pocket" testId="wallet-create" className="max-w-[560px]">
      <Zeb pose="idle" width={96} className="mb-2" label={null} />
      <h2 className="zk-title text-white mb-2">Make your Pocket</h2>
      <p className="text-mist mb-4">
        A small private wallet in this browser, just for Zeek. Pick a passphrase to lock it.
      </p>
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (strength === "too-short" || pass !== again) return;
          setBusy(true);
          dispatch({ type: "create" });
          void (async () => {
            try {
              const p = await pocket();
              await p.create(pass);
              const phrase = await p.exportSeed(pass);
              dispatch({ type: "created" });
              setWords(phrase.split(" "));
            } catch (err) {
              dispatch({ type: "failed", message: toPocketError(err).message });
              setError(toPocketError(err).message);
            } finally {
              setBusy(false);
            }
          })();
        }}
      >
        <label className="flex flex-col gap-1">
          <span className="label">Passphrase</span>
          <span className="relative block">
            <input
              type={show ? "text" : "password"}
              className="field-input pr-14"
              value={pass}
              onChange={(e) => {
                setPass(e.target.value);
              }}
              autoComplete="new-password"
              data-testid="create-pass"
            />
            <button
              type="button"
              className="absolute right-2 top-1/2 -translate-y-1/2 p-2 text-white"
              onClick={() => {
                setShow((v) => !v);
              }}
              aria-label={show ? "Hide passphrase" : "Show passphrase"}
              aria-pressed={show}
            >
              <Icon name="eye" size={20} />
            </button>
          </span>
          <StrengthMeter strength={pass.length === 0 ? null : strength} />
          <span className="text-fog text-sm">
            {pass.length > 0 ? STRENGTH_TEXT[strength] : "Strong: 4 words or 12+ letters."}
          </span>
        </label>
        <label className="flex flex-col gap-1">
          <span className="label">Again</span>
          <input
            type="password"
            className="field-input"
            value={again}
            onChange={(e) => {
              setAgain(e.target.value);
            }}
            autoComplete="new-password"
            data-testid="create-again"
          />
          {again.length > 0 && again !== pass && (
            <span className="text-white text-sm">They don’t match.</span>
          )}
        </label>
        {error !== null && (
          <p className="text-white" role="alert">
            {error}
          </p>
        )}
        <button
          type="submit"
          className="btn btn-primary"
          disabled={busy || strength === "too-short" || pass !== again}
          data-testid="create-submit"
        >
          {busy ? "Making your Pocket…" : "Create Pocket"}
        </button>
      </form>
    </Panel>
  );
}

/** Pending best-effort clear of a copied phrase (one at a time). */
let clearTimer: number | undefined;

/**
 * Copies the recovery phrase, with the warning next to it. After 60 s the
 * clipboard is cleared again if it still holds the phrase (best effort; the
 * timer outlives this screen on purpose). The phrase is never logged,
 * stored or sent.
 */
function CopyPhrase({ words }: { words: string[] }) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  useEffect(() => {
    if (state !== "copied") return;
    const id = window.setTimeout(() => {
      setState("idle");
    }, 2_000);
    return () => {
      window.clearTimeout(id);
    };
  }, [state]);
  const copy = () => {
    const phrase = words.join(" ");
    navigator.clipboard.writeText(phrase).then(
      () => {
        setState("copied");
        if (clearTimer !== undefined) window.clearTimeout(clearTimer);
        clearTimer = window.setTimeout(() => {
          clearTimer = undefined;
          void clearIfUnchanged(browserClearEnv(), phrase);
        }, CLEAR_AFTER_MS);
      },
      () => {
        setState("failed");
      },
    );
  };
  return (
    <div className="flex flex-col sm:flex-row sm:items-center gap-3 mb-4">
      <button
        type="button"
        className="btn flex items-center justify-center gap-2 shrink-0"
        onClick={copy}
        data-testid="copy-phrase"
      >
        {state === "copied" ? (
          <Icon name="checkink" size={16} draw strokeWidth={3} />
        ) : (
          <Icon name="copy" size={16} />
        )}
        <span aria-live="polite">{state === "copied" ? "Copied" : "Copy words"}</span>
      </button>
      <p className="text-fog text-sm" data-testid="copy-phrase-warning">
        {state === "failed"
          ? "Couldn’t copy here. Write them down instead."
          : "Anyone who gets these words gets your coins. Clipboards can be read by other apps."}
      </p>
    </div>
  );
}

function BackupWords({ words, onDone }: { words: string[]; onDone: () => void }) {
  const [stage, setStage] = useState<"show" | "check">("show");
  const [positions] = useState(() => checkPositions(random, words.length));
  const [answers, setAnswers] = useState<string[]>(["", "", ""]);
  const [wrong, setWrong] = useState(false);
  if (stage === "show") {
    return (
      <Panel label="Back up 24 words" testId="wallet-backup" className="max-w-[640px]">
        <h2 className="text-heading font-extrabold text-white mb-2">
          Write these {words.length} words on paper
        </h2>
        <ul className="text-mist mb-4 list-disc pl-5">
          <li>On paper, in order.</li>
          <li>No photos, no screenshots.</li>
          <li>Anyone with them can spend your ZEC.</li>
        </ul>
        <ol className="grid grid-cols-2 sm:grid-cols-3 gap-2 mb-4" data-testid="backup-words">
          {words.map((w, i) => (
            <li key={i} className="flex gap-2 border-2 border-white px-3 py-2">
              <span className="text-fog w-6 text-right">{i + 1}</span>
              <span className="font-mono text-white">{w}</span>
            </li>
          ))}
        </ol>
        <CopyPhrase words={words} />
        <button
          type="button"
          className="btn btn-primary w-full"
          onClick={() => {
            setStage("check");
          }}
          data-testid="backup-written"
        >
          I wrote them down · check 3 words
        </button>
      </Panel>
    );
  }
  return (
    <Panel label="Check 3 words" testId="wallet-check">
      <h2 className="text-heading font-extrabold text-white mb-4">Quick check</h2>
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          const ok = positions.every(
            (pos, i) => (answers[i] ?? "").trim().toLowerCase() === words[pos - 1],
          );
          if (ok) onDone();
          else setWrong(true);
        }}
      >
        {positions.map((pos, i) => (
          <label key={pos} className="flex flex-col gap-1">
            <span className="label">Word {pos}</span>
            <input
              className="field-input font-mono"
              autoComplete="off"
              value={answers[i]}
              onChange={(e) => {
                const next = [...answers];
                next[i] = e.target.value;
                setAnswers(next);
              }}
              data-testid={`check-word-${pos}`}
            />
          </label>
        ))}
        {wrong && (
          <p className="text-white" role="alert">
            Not quite. Look at your paper again.
          </p>
        )}
        <div className="flex gap-2">
          <button
            type="button"
            className="btn"
            onClick={() => {
              setStage("show");
            }}
          >
            Show the words
          </button>
          <button type="submit" className="btn btn-primary flex-1" data-testid="check-words">
            Check
          </button>
        </div>
      </form>
    </Panel>
  );
}

function UnlockCard() {
  const { pocket, wallet, dispatch } = usePocket();
  const [pass, setPass] = useState("");
  const [restore, setRestore] = useState(false);
  const now = useNow();
  if (restore)
    return (
      <RestoreCard
        onCancel={() => {
          setRestore(false);
        }}
      />
    );
  const can = canTryUnlock(wallet, now);
  const error = wallet.kind === "locked" ? wallet.error : null;
  return (
    <Panel label="Unlock Pocket" testId="wallet-unlock" className="max-w-[560px]">
      <Zeb pose="peek" width={96} className="mb-2" label={null} />
      <h2 className="zk-title text-white mb-4 flex items-center gap-3">
        <Icon name="lock" size={28} /> Unlock Pocket
      </h2>
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (!can) return;
          dispatch({ type: "unlock" });
          void (async () => {
            try {
              await (await pocket()).unlock(pass);
              setPass("");
              dispatch({ type: "unlock-ok" });
            } catch (err) {
              const ex = toPocketError(err);
              dispatch({
                type: "unlock-failed",
                message: ex.code === "bad_passphrase" ? "That passphrase didn’t work." : ex.message,
                now: Math.floor(Date.now() / 1000),
              });
            }
          })();
        }}
      >
        <input
          type="password"
          className="field-input"
          value={pass}
          onChange={(e) => {
            setPass(e.target.value);
          }}
          autoComplete="current-password"
          aria-label="Passphrase"
          data-testid="unlock-pass"
        />
        {error !== null && (
          <p className="text-white" role="alert">
            {error}
            {!can && wallet.kind === "locked" ? ` (${wallet.lockedUntil - now} s)` : ""}
          </p>
        )}
        <button
          type="submit"
          className="btn btn-primary"
          disabled={!can || wallet.kind === "unlocking"}
          data-testid="unlock-submit"
        >
          {wallet.kind === "unlocking" ? "Unlocking…" : "Unlock"}
        </button>
        <button
          type="button"
          className="underline text-white text-left"
          onClick={() => {
            setRestore(true);
          }}
        >
          Forgot it? Restore with 24 words
        </button>
      </form>
    </Panel>
  );
}

export function RestoreCard({ onCancel }: { onCancel: () => void }) {
  const { pocket, dispatch, refresh } = usePocket();
  const [phrase, setPhrase] = useState("");
  const [birthday, setBirthday] = useState("");
  const [pass, setPass] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <Panel label="Restore" testId="wallet-restore">
      <h2 className="text-heading font-extrabold text-white mb-2">Restore with 24 words</h2>
      <p className="text-fog mb-3">This replaces the Pocket in this browser.</p>
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          setBusy(true);
          void (async () => {
            try {
              const p = await pocket();
              // Replaces the Pocket here only once the restored one is built:
              // a wrong word or birthday leaves it as it was.
              await p.restore(phrase.trim().toLowerCase(), Number(birthday), pass, true);
              setPhrase("");
              dispatch({ type: "unlock-ok" });
              await refresh();
            } catch (err) {
              setError(toPocketError(err).message);
            } finally {
              setBusy(false);
            }
          })();
        }}
      >
        <textarea
          className="field-input font-mono"
          rows={4}
          value={phrase}
          onChange={(e) => {
            setPhrase(e.target.value);
          }}
          aria-label="24 words"
          autoComplete="off"
        />
        <input
          className="field-input"
          inputMode="numeric"
          value={birthday}
          onChange={(e) => {
            setBirthday(e.target.value);
          }}
          placeholder="Block height it was made at (or earlier)"
          aria-label="Birthday height"
        />
        <input
          type="password"
          className="field-input"
          value={pass}
          onChange={(e) => {
            setPass(e.target.value);
          }}
          placeholder="New passphrase"
          aria-label="New passphrase"
        />
        {error !== null && (
          <p className="text-white" role="alert">
            {error}
          </p>
        )}
        <div className="flex gap-2">
          <button type="button" className="btn" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="submit"
            className="btn btn-primary flex-1"
            disabled={busy || passphraseStrength(pass) === "too-short"}
          >
            Restore
          </button>
        </div>
      </form>
    </Panel>
  );
}

function historyLabel(e: HistoryEntry): {
  label: string;
  amount: bigint;
  time: string;
  icon: IconName;
} {
  if (e.type === "received") {
    const treasure = e.memo !== null && /treasure/i.test(e.memo);
    return {
      label: e.memo ?? "Received",
      amount: zat(e.value_zat),
      time: `block ${e.height}`,
      icon: treasure ? "chest" : "down2",
    };
  }
  const k = e.kind;
  if ("dig" in k) {
    const n = k.dig.tiles.length;
    return {
      icon: "shovel",
      label: `Zeek round ${k.dig.round}: dig · ${n} ${n === 1 ? "tile" : "tiles"}`,
      amount: -(BigInt(k.dig.amount_each_zat) * BigInt(n) + zat(e.fee_zat)),
      time: e.status === "mined" ? `block ${e.mined_height ?? e.target_height}` : e.status,
    };
  }
  if ("shout" in k)
    return {
      icon: "shout",
      label: `Zeek campfire: shout`,
      amount: -(zat(k.shout.amount_zat) + zat(e.fee_zat)),
      time: e.status,
    };
  if ("split" in k)
    return {
      icon: "split",
      label: `Getting your coins ready · ${k.split.parts} parts, so several digs can go in one round`,
      amount: -zat(e.fee_zat),
      time: e.status === "mined" ? `block ${e.mined_height ?? e.target_height}` : e.status,
    };
  return {
    icon: "up2",
    label: k.send.memo === null ? "Withdrawn" : `Withdrawn · ${k.send.memo}`,
    amount: -(zat(k.send.amount_zat) + zat(e.fee_zat)),
    time: e.status,
  };
}

export function WithdrawCard({ spendable, onDone }: { spendable: bigint; onDone: () => void }) {
  const { pocket } = usePocket();
  const [to, setTo] = useState("");
  const [amount, setAmount] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  // A broadcast that may have been sent holds the next one (no double withdrawal).
  const [held, holdUntil] = useHold();
  const fee = 10_000n; // ZIP 317: one payment output + change, one input.
  let value: bigint | null = null;
  try {
    value = parseZec(amount);
  } catch {
    value = null;
  }
  const verdict = checkPayoutAddress(to, "test");
  return (
    <Panel label="Withdraw" testId="wallet-withdraw">
      <h2 className="text-heading font-extrabold text-white mb-3">Withdraw</h2>
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (value === null || !verdict.ok) return;
          setBusy(true);
          void (async () => {
            try {
              const p = await pocket();
              const sent = await asWithdrawal(() =>
                p.send({ to: to.trim(), amountZat: toNumber(value), passphrase }),
              );
              setPassphrase("");
              setResult(`Sent privately. ${sent.txid}`);
              onDone();
            } catch (err) {
              const e = toPocketError(err);
              if (e.code === "broadcast_unknown") holdUntil(Date.now() + MAYBE_SENT_HOLD_MS);
              setResult(e.message);
            } finally {
              setBusy(false);
            }
          })();
        }}
      >
        <label className="flex flex-col gap-1">
          <span className="label">To</span>
          <input
            className="field-input font-mono text-sm"
            value={to}
            onChange={(e) => {
              setTo(e.target.value);
            }}
            placeholder="utest1…"
          />
          {to.length > 0 && !verdict.ok && (
            <span className="text-white text-sm">{verdict.message}</span>
          )}
          {verdict.ok && <span className="text-fog text-sm">Private address</span>}
        </label>
        <label className="flex flex-col gap-1">
          <span className="label">Amount</span>
          <div className="flex gap-2">
            <input
              className="field-input numeral"
              inputMode="decimal"
              value={amount}
              onChange={(e) => {
                setAmount(e.target.value);
              }}
            />
            <button
              type="button"
              className="btn"
              onClick={() => {
                setAmount(spendable > fee ? formatZec(spendable - fee, { decimals: 8 }) : "0");
              }}
            >
              Max
            </button>
          </div>
        </label>
        <label className="flex flex-col gap-1">
          <span className="label">Passphrase</span>
          <input
            className="field-input"
            type="password"
            autoComplete="current-password"
            value={passphrase}
            onChange={(e) => {
              setPassphrase(e.target.value);
            }}
            data-testid="withdraw-passphrase"
          />
          <span className="text-fog text-sm">Asked again before coins leave your Pocket.</span>
        </label>
        <div className="flex justify-between text-fog">
          <span>Network fee</span>
          <span className="font-mono">{formatZec(fee, { decimals: 8 })}</span>
        </div>
        <div className="flex justify-between text-white">
          <span>Leaves your Pocket</span>
          <span className="numeral">
            {value === null ? "—" : formatZec(value + fee, { decimals: 8 })}
          </span>
        </div>
        {result !== null && (
          <p className="text-white break-all" role="status">
            {result}
          </p>
        )}
        <button
          type="submit"
          className="btn btn-primary"
          disabled={
            busy ||
            held > 0 ||
            value === null ||
            !verdict.ok ||
            value + fee > spendable ||
            passphrase === ""
          }
        >
          {busy ? "Sending…" : held > 0 ? `Withdraw again in ${held} s` : "Withdraw"}
        </button>
      </form>
    </Panel>
  );
}

function Home() {
  const { pocket, wallet, dispatch } = usePocket();
  const client = useQueryClient();
  const balance = usePocketBalance();
  const history = usePocketHistory();
  const config = useConfig();
  const [view, setView] = useState<"home" | "add" | "withdraw" | "backup">("home");
  const [backupWords, setBackupWords] = useState<string[] | null>(null);
  const [backupPass, setBackupPass] = useState("");
  const [syncError, setSyncError] = useState<string | null>(null);
  const reduce = useReducedMotion() === true;
  const address = useQuery({
    queryKey: ["pocket", "address"],
    queryFn: async () => (await pocket()).getAddress(),
  });
  const qr = useQuery({
    queryKey: ["pocket", "address-qr", address.data],
    queryFn: () => qrSvg(address.data ?? ""),
    enabled: address.data !== undefined,
  });

  const sync = async () => {
    setSyncError(null);
    try {
      dispatch({ type: "sync", fraction: 0, scanned: 0, tip: 0 });
      await (
        await pocket()
      ).sync((p) => {
        dispatch({
          type: "sync",
          fraction: p.fraction,
          scanned: p.scannedHeight,
          tip: p.tipHeight,
        });
      });
      dispatch({ type: "synced" });
      // A deposit large enough to dig several times a round is split into a
      // few notes (shown as "Getting your coins ready"); never during a dig,
      // an auto-dig or a withdrawal.
      try {
        if (!autoDigRunning() && !withdrawing()) await (await pocket()).prepareCoins();
      } catch (err) {
        setSyncError(`Could not get your coins ready: ${toPocketError(err).message}`);
      }
    } catch (err) {
      dispatch({ type: "failed", message: toPocketError(err).message });
      setSyncError(toPocketError(err).message);
    } finally {
      void client.invalidateQueries({ queryKey: ["pocket"] });
    }
  };
  // Sync once when the wallet opens.
  const synced = useRef(false);
  useEffect(() => {
    if (synced.current) return;
    synced.current = true;
    void sync();
  });

  const b = balance.data;
  const spacing = useBlockSpacing();
  const spendable = b === undefined ? undefined : zat(b.spendable_zat);
  const confirming = b === undefined ? undefined : zat(b.pending_zat);
  const actions = (
    <div className="grid grid-cols-2 gap-3">
      <button
        type="button"
        className={`btn flex items-center justify-center gap-2 ${view === "add" ? "" : "btn-primary"}`}
        style={{ minHeight: 60, fontSize: 20 }}
        onClick={() => {
          setView(view === "add" ? "home" : "add");
        }}
        aria-expanded={view === "add"}
        data-testid="wallet-add"
      >
        <Icon name="down2" size={22} /> Add
      </button>
      <button
        type="button"
        className="btn flex items-center justify-center gap-2"
        style={{ minHeight: 60, fontSize: 20 }}
        onClick={() => {
          setView(view === "withdraw" ? "home" : "withdraw");
        }}
        aria-expanded={view === "withdraw"}
      >
        <Icon name="up2" size={22} /> Withdraw
      </button>
    </div>
  );
  return (
    <div
      className="grid gap-8 lg:grid-cols-[minmax(0,560px)_minmax(0,480px)]"
      data-testid="wallet-home"
    >
      <div className="flex min-w-0 flex-col gap-5">
        <section className="zk-stat px-6 py-5" aria-label="Balance">
          <div className="label flex items-center gap-2">
            <Icon name="wallet" size={18} /> Balance
          </div>
          <FitAmount size={112} min={20} unit="ZEC" unitScale={0.24} testId="wallet-balance">
            {spendable === undefined || confirming === undefined ? (
              "…"
            ) : (
              <CountUp value={spendable + confirming} />
            )}
          </FitAmount>
          {b !== undefined && spendable !== undefined && confirming !== undefined && (
            <dl className="mt-2 flex flex-col gap-0.5 font-mono text-sm">
              <div className="flex gap-2">
                <dt className="text-fog">Ready to dig</dt>
                <dd className="text-white min-w-0 break-all" data-testid="wallet-spendable">
                  {formatZec(spendable, { decimals: 8 })}
                </dd>
              </div>
              {confirming > 0n && (
                <div className="flex gap-2" data-testid="wallet-confirming">
                  <dt className="text-fog">Confirming</dt>
                  <dd className="text-white">
                    {formatZec(confirming, { decimals: 8 })}
                    {b.all_ready_in_blocks !== null &&
                      ` · ready ${readyIn(b.all_ready_in_blocks, spacing)}`}
                  </dd>
                </div>
              )}
            </dl>
          )}
          <div className="flex items-center justify-between gap-3 mt-2">
            <p className="text-fog text-sm" role="status" data-testid="wallet-sync">
              {wallet.kind === "syncing"
                ? `Syncing… block ${wallet.scanned} of ${wallet.tip}`
                : (syncError ?? `Synced to block ${b?.scanned_height ?? "…"}`)}
            </p>
            <button
              type="button"
              className="flex items-center gap-1.5 text-white underline text-sm"
              onClick={() => void sync()}
              disabled={wallet.kind === "syncing"}
            >
              <Icon name="refresh" size={16} /> Sync
            </button>
          </div>
        </section>
        {actions}
        {config.data?.params.network === "test" && address.data !== undefined && (
          <FaucetCard
            address={address.data}
            spendable={spendable}
            pending={confirming}
            readyInBlocks={b?.ready_in_blocks ?? null}
            spacing={spacing}
            sync={sync}
          />
        )}
        {view === "add" && address.data !== undefined && (
          <section
            className="zk-stat p-5 flex flex-col sm:flex-row gap-5 items-center"
            aria-label="Add ZEC here"
            data-testid="wallet-deposit"
          >
            <div
              className="bg-white p-2 w-[160px] shrink-0"
              role="img"
              aria-label="QR code of your deposit address"
              dangerouslySetInnerHTML={{ __html: qr.data ?? "" }}
            />
            <div className="flex flex-col gap-2 min-w-0">
              <div className="label">Add ZEC here</div>
              <p className="font-mono text-sm break-all text-mist" data-testid="deposit-address">
                {address.data}
              </p>
              <CopyButton text={address.data} />
              <p className="text-fog text-xs">
                Only you see this address. Send testnet ZEC (TAZ) from any wallet.
              </p>
            </div>
          </section>
        )}
        {view === "withdraw" && (
          <WithdrawCard
            spendable={b === undefined ? 0n : zat(b.spendable_zat)}
            onDone={() => void client.invalidateQueries({ queryKey: ["pocket"] })}
          />
        )}
        {view === "backup" && (
          <Panel label="Back up again">
            {backupWords === null ? (
              <form
                className="flex gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  void (async () => {
                    try {
                      setBackupWords((await (await pocket()).exportSeed(backupPass)).split(" "));
                      setBackupPass("");
                    } catch (err) {
                      setSyncError(toPocketError(err).message);
                    }
                  })();
                }}
              >
                <input
                  type="password"
                  className="field-input"
                  value={backupPass}
                  onChange={(e) => {
                    setBackupPass(e.target.value);
                  }}
                  aria-label="Passphrase"
                  placeholder="Passphrase"
                />
                <button type="submit" className="btn btn-primary">
                  Show words
                </button>
              </form>
            ) : (
              <BackupWords
                words={backupWords}
                onDone={() => {
                  setBackupWords(null);
                  setView("home");
                }}
              />
            )}
          </Panel>
        )}
      </div>
      <div className="flex min-w-0 flex-col gap-3">
        <div className="label text-fog">History</div>
        {history.data === undefined ? (
          <p className="text-fog">Loading…</p>
        ) : history.data.length === 0 ? (
          <p className="text-fog">Nothing yet. Add ZEC to start.</p>
        ) : (
          <ul className="zk-rows" data-testid="wallet-history">
            {history.data.map((e, i) => {
              const h = historyLabel(e);
              return (
                <motion.li
                  key={i}
                  className="flex items-center gap-3 px-4 py-3"
                  data-testid="history-row"
                  initial={i === 0 && !reduce ? { opacity: 0, x: 24 } : false}
                  animate={{ opacity: 1, x: 0 }}
                  transition={{ duration: 0.22, ease: EASE.out }}
                >
                  <Icon name={h.icon} size={22} />
                  <div className="flex-1 min-w-0">
                    <div className="text-white font-bold truncate">{h.label}</div>
                    <div className="text-fog text-xs">{h.time}</div>
                  </div>
                  <span className="font-mono text-white">
                    {formatZec(h.amount, { decimals: 8, sign: true })}
                  </span>
                </motion.li>
              );
            })}
          </ul>
        )}
        <div className="flex gap-2 flex-wrap items-center text-sm text-mist">
          <Icon name="pen" size={16} />
          <button
            type="button"
            className="hover:text-white underline"
            onClick={() => {
              setView("backup");
            }}
          >
            Back up again
          </button>
          ·
          <button
            type="button"
            className="hover:text-white underline"
            onClick={() =>
              void (async () => {
                await (await pocket()).lock();
                dispatch({ type: "lock" });
              })()
            }
            data-testid="wallet-lock"
          >
            Lock now
          </button>
          ·
          <button
            type="button"
            className="hover:text-white underline"
            onClick={() => {
              if (
                window.confirm(
                  "Remove this Pocket from this browser? Without your 24 words its ZEC is lost.",
                )
              ) {
                void (async () => {
                  await (await pocket()).forget();
                  dispatch({ type: "forgotten" });
                })();
              }
            }}
          >
            Remove from this browser
          </button>
        </div>
        {address.data !== undefined && (
          <p className="text-fog text-xs">Pocket address {shortAddress(address.data)}</p>
        )}
        <p className="text-fog text-xs">
          Synced at {formatTimeOfDay(Math.floor(Date.now() / 1000))}
        </p>
      </div>
    </div>
  );
}

export default function WalletScreen() {
  const { wallet, refresh } = usePocket();
  const [params] = useSearchParams();
  const navigate = useNavigate();
  useEffect(() => {
    void refresh();
  }, [refresh]);
  useEffect(() => {
    if (params.get("then") === "dig" && wallet.kind === "unlocked") void navigate("/dig/pocket");
  }, [params, wallet.kind, navigate]);
  return (
    <Screen title="My wallet" testId="wallet-screen" wide>
      {wallet.kind === "unknown" ? (
        <Loading text="Opening your Pocket…" />
      ) : wallet.kind === "none" || wallet.kind === "creating" || wallet.kind === "backup" ? (
        <CreateCard />
      ) : wallet.kind === "locked" || wallet.kind === "unlocking" ? (
        <UnlockCard />
      ) : (
        <Home />
      )}
    </Screen>
  );
}
