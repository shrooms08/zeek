/** My wallet (Zeek Pocket). Lives in this browser. */

import { qrSvg, toPocketError, type HistoryEntry } from "@zeek/pocket";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { usePocket } from "../api/PocketProvider";
import { usePocketBalance, usePocketHistory } from "../api/pocketQueries";
import { CopyButton, Loading, Panel, Screen } from "../components/ui";
import { checkPayoutAddress, shortAddress } from "../lib/address";
import { formatZec, parseZec, toNumber, zat } from "../lib/money";
import { checkPositions, passphraseStrength } from "../lib/passphrase";
import { formatTimeOfDay } from "../lib/time";
import { useNow } from "../lib/useNow";
import { canTryUnlock } from "../machines/wallet";

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

function CreateCard() {
  const { pocket, dispatch } = usePocket();
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
    <Panel label="Make your Pocket" testId="wallet-create">
      <h2 className="text-heading font-extrabold text-white mb-2">Make your Pocket</h2>
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
          <input
            type="password"
            className="field-input"
            value={pass}
            onChange={(e) => {
              setPass(e.target.value);
            }}
            autoComplete="new-password"
            data-testid="create-pass"
          />
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

function BackupWords({ words, onDone }: { words: string[]; onDone: () => void }) {
  const [stage, setStage] = useState<"show" | "check">("show");
  const [positions] = useState(() => checkPositions(random, words.length));
  const [answers, setAnswers] = useState<string[]>(["", "", ""]);
  const [wrong, setWrong] = useState(false);
  if (stage === "show") {
    return (
      <Panel label="Back up 24 words" testId="wallet-backup">
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
            <li key={i} className="flex gap-2 border-2 border-slate p-2">
              <span className="text-fog w-6 text-right">{i + 1}</span>
              <span className="font-mono text-white">{w}</span>
            </li>
          ))}
        </ol>
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
    <Panel label="Unlock Pocket" testId="wallet-unlock">
      <h2 className="text-heading font-extrabold text-white mb-4">Unlock Pocket</h2>
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

function RestoreCard({ onCancel }: { onCancel: () => void }) {
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
              await p.forget();
              await p.restore(phrase.trim().toLowerCase(), Number(birthday), pass);
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

function historyLabel(e: HistoryEntry): { label: string; amount: bigint; time: string } {
  if (e.type === "received") {
    return { label: e.memo ?? "Received", amount: zat(e.value_zat), time: `block ${e.height}` };
  }
  const k = e.kind;
  if ("dig" in k) {
    const n = k.dig.tiles.length;
    return {
      label: `Zeek round ${k.dig.round}: dig · ${n} ${n === 1 ? "tile" : "tiles"}`,
      amount: -(BigInt(k.dig.amount_each_zat) * BigInt(n) + zat(e.fee_zat)),
      time: e.status === "mined" ? `block ${e.mined_height ?? e.target_height}` : e.status,
    };
  }
  if ("shout" in k)
    return {
      label: `Zeek campfire: shout`,
      amount: -(zat(k.shout.amount_zat) + zat(e.fee_zat)),
      time: e.status,
    };
  return {
    label: k.send.memo === null ? "Withdrawn" : `Withdrawn · ${k.send.memo}`,
    amount: -(zat(k.send.amount_zat) + zat(e.fee_zat)),
    time: e.status,
  };
}

function WithdrawCard({ spendable, onDone }: { spendable: bigint; onDone: () => void }) {
  const { pocket } = usePocket();
  const [to, setTo] = useState("");
  const [amount, setAmount] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
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
              const sent = await (
                await pocket()
              ).send({ to: to.trim(), amountZat: toNumber(value) });
              setResult(`Sent privately. ${sent.txid}`);
              onDone();
            } catch (err) {
              setResult(toPocketError(err).message);
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
          disabled={busy || value === null || !verdict.ok || value + fee > spendable}
        >
          {busy ? "Sending…" : "Withdraw privately"}
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
  const [view, setView] = useState<"home" | "add" | "withdraw" | "backup">("home");
  const [backupWords, setBackupWords] = useState<string[] | null>(null);
  const [backupPass, setBackupPass] = useState("");
  const [syncError, setSyncError] = useState<string | null>(null);
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
  return (
    <div className="flex flex-col gap-4" data-testid="wallet-home">
      <Panel label="Balance">
        <div className="label">Balance</div>
        <div
          className="numeral"
          style={{ fontSize: "clamp(44px, 8vw, 96px)" }}
          data-testid="wallet-balance"
        >
          {b === undefined ? "…" : formatZec(zat(b.spendable_zat))}{" "}
          <span className="label">ZEC</span>
        </div>
        {b !== undefined && b.pending_zat > 0 && (
          <p className="text-fog">
            {formatZec(zat(b.pending_zat), { sign: true })} arriving · confirming
          </p>
        )}
        <p className="text-fog text-sm" role="status" data-testid="wallet-sync">
          {wallet.kind === "syncing"
            ? `Syncing… block ${wallet.scanned} of ${wallet.tip}`
            : (syncError ?? `Synced to block ${b?.scanned_height ?? "…"}`)}
        </p>
        <div className="flex gap-2 mt-3 flex-wrap">
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => {
              setView(view === "add" ? "home" : "add");
            }}
            data-testid="wallet-add"
          >
            Add
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => {
              setView(view === "withdraw" ? "home" : "withdraw");
            }}
          >
            Withdraw
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => void sync()}
            disabled={wallet.kind === "syncing"}
          >
            Sync
          </button>
        </div>
      </Panel>
      {view === "add" && address.data !== undefined && (
        <Panel label="Add ZEC here" testId="wallet-deposit">
          <div className="label mb-2">Add ZEC here</div>
          <div
            className="bg-white p-2 max-w-[220px]"
            role="img"
            aria-label="QR code of your deposit address"
            dangerouslySetInnerHTML={{ __html: qr.data ?? "" }}
          />
          <p className="font-mono text-sm break-all text-mist my-2" data-testid="deposit-address">
            {address.data}
          </p>
          <CopyButton text={address.data} />
          <p className="text-fog text-sm mt-2">
            Only you see this address. Send testnet ZEC (TAZ) to it from any wallet.
          </p>
        </Panel>
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
      <Panel label="History">
        <div className="label mb-2">History</div>
        {history.data === undefined ? (
          <p className="text-fog">Loading…</p>
        ) : history.data.length === 0 ? (
          <p className="text-fog">Nothing yet. Add ZEC to start.</p>
        ) : (
          <ul data-testid="wallet-history">
            {history.data.map((e, i) => {
              const h = historyLabel(e);
              return (
                <li
                  key={i}
                  className="flex justify-between gap-3 border-b hair py-2"
                  data-testid="history-row"
                >
                  <div>
                    <div className="text-white">{h.label}</div>
                    <div className="text-fog text-sm">{h.time}</div>
                  </div>
                  <span
                    className="numeral"
                    style={{ color: h.amount > 0n ? "var(--color-gold)" : undefined }}
                  >
                    {formatZec(h.amount, { decimals: 8, sign: true })}
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </Panel>
      <div className="flex gap-4 flex-wrap text-sm">
        <button
          type="button"
          className="underline text-white"
          onClick={() => {
            setView("backup");
          }}
        >
          Back up again
        </button>
        <button
          type="button"
          className="underline text-white"
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
        <button
          type="button"
          className="underline text-white"
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
      <p className="text-fog text-xs">Synced at {formatTimeOfDay(Math.floor(Date.now() / 1000))}</p>
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
    <Screen title="My wallet" testId="wallet-screen">
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
