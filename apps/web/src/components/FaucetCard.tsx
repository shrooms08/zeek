/**
 * "Get free test coins" (testnet only): how to fill a new Pocket from the
 * Valar Group testnet faucet, then "waiting for coins" until they arrive.
 *
 * The faucet's page has no way to receive an address from a link, so the
 * card copies the address first and says so. While waiting, the wallet
 * re-syncs every 30 s; the wait ends when the balance (spendable plus
 * arriving) rises above where it was when the faucet was opened. The wait is
 * remembered in this browser only.
 */

import { useEffect, useRef, useState } from "react";
import { Icon } from "./Icon";
import { CopyButton } from "./ui";

/** The Valar Group testnet faucet: 0.125 TAZ per address per 24 hours. */
export const FAUCET_URL = "https://faucet.testnet.valargroup.dev/";

const WAIT_KEY = "zeek-awaiting-coins";
const RESYNC_MS = 30_000;

interface Wait {
  /** Balance (zatoshis, as a string) when the faucet was opened. */
  from: string;
  since: number;
}

function loadWait(): Wait | null {
  try {
    const raw = window.localStorage.getItem(WAIT_KEY);
    if (raw === null) return null;
    const parsed = JSON.parse(raw) as Partial<Wait>;
    return typeof parsed.from === "string" && typeof parsed.since === "number"
      ? { from: parsed.from, since: parsed.since }
      : null;
  } catch {
    return null;
  }
}

function saveWait(wait: Wait | null) {
  try {
    if (wait === null) window.localStorage.removeItem(WAIT_KEY);
    else window.localStorage.setItem(WAIT_KEY, JSON.stringify(wait));
  } catch {
    // Storage blocked: the wait lasts for this page only.
  }
}

export function FaucetCard({
  address,
  total,
  sync,
}: {
  address: string;
  /** Spendable plus arriving, in zatoshis; undefined until known. */
  total: bigint | undefined;
  sync: () => Promise<void>;
}) {
  const [wait, setWait] = useState<Wait | null>(loadWait);
  const [arrived, setArrived] = useState(false);
  // The latest sync, without restarting the timer on every render.
  const syncRef = useRef(sync);
  syncRef.current = sync;

  useEffect(() => {
    if (wait === null || total === undefined) return;
    if (total > BigInt(wait.from)) {
      saveWait(null);
      setWait(null);
      setArrived(true);
    }
  }, [wait, total]);

  useEffect(() => {
    if (wait === null) return;
    const id = window.setInterval(() => void syncRef.current(), RESYNC_MS);
    return () => {
      window.clearInterval(id);
    };
  }, [wait]);

  if (wait === null && !arrived && (total === undefined || total > 0n)) {
    // A funded Pocket does not need the faucet.
    return null;
  }

  if (arrived) {
    return (
      <section className="zk-stat p-5" aria-label="Test coins" data-testid="faucet-arrived">
        <div className="label flex items-center gap-2">
          <Icon name="check" size={18} /> Test coins arrived
        </div>
        <p className="text-mist mt-2">You can dig now.</p>
      </section>
    );
  }
  if (wait !== null) {
    return (
      <section
        className="zk-stat p-5"
        aria-label="Test coins"
        aria-live="polite"
        data-testid="faucet-waiting"
      >
        <div className="label flex items-center gap-2">
          <span className="zk-hourglass inline-flex">
            <Icon name="hourglass" size={18} />
          </span>
          Waiting for coins
        </div>
        <p className="text-mist mt-2">
          Checking every 30 seconds. Faucet coins show up as “arriving” once the faucet sends them,
          and can be spent once they confirm, a few minutes later.
        </p>
        <button
          type="button"
          className="underline text-fog text-sm mt-2"
          onClick={() => {
            saveWait(null);
            setWait(null);
          }}
        >
          Stop waiting
        </button>
      </section>
    );
  }
  return (
    <section
      className="zk-stat p-5 flex flex-col gap-3"
      aria-label="Get free test coins"
      data-testid="faucet"
    >
      <div className="label flex items-center gap-2">
        <Icon name="coin" size={18} /> Get free test coins
      </div>
      <p className="text-mist">
        This is the Zcash testnet: its coins (TAZ) have no value. The Valar Group faucet gives 0.125
        TAZ per address each day, enough for about a hundred digs.
      </p>
      <ol className="text-mist list-decimal pl-5 flex flex-col gap-1">
        <li>Copy your Pocket address (the faucet page cannot receive it from a link).</li>
        <li>Open the faucet, paste the address and claim.</li>
        <li>Come back here: the Pocket watches for the coins.</li>
      </ol>
      <div className="flex flex-wrap gap-3">
        <CopyButton text={address} label="Copy my address" />
        <a
          className="btn btn-primary flex items-center gap-2"
          href={FAUCET_URL}
          target="_blank"
          rel="noopener noreferrer"
          onClick={() => {
            const next = { from: (total ?? 0n).toString(), since: Date.now() };
            saveWait(next);
            setWait(next);
          }}
          data-testid="faucet-link"
        >
          <Icon name="link" size={18} /> Open the faucet
        </a>
      </div>
    </section>
  );
}
