/**
 * A hold on an action until a moment (ms since epoch): the seconds left,
 * ticking down, and a way to start a new hold. After a broadcast that may
 * have reached the chain, retrying is held for a while so a second payment
 * is not sent before the first one shows up in History.
 */

import { useEffect, useState } from "react";

/** How long a retry is held after a broadcast that may have been sent. */
export const MAYBE_SENT_HOLD_MS = 120_000;

export function useHold(
  initialUntil = 0,
): [secondsLeft: number, holdUntil: (until: number) => void] {
  const [until, setUntil] = useState(initialUntil);
  const [now, setNow] = useState(() => Date.now());
  const left = Math.max(0, Math.ceil((until - now) / 1000));
  useEffect(() => {
    if (left === 0) return;
    const id = window.setInterval(() => {
      setNow(Date.now());
    }, 1000);
    return () => {
      window.clearInterval(id);
    };
  }, [left]);
  const holdUntil = (next: number) => {
    setNow(Date.now());
    setUntil(next);
  };
  return [left, holdUntil];
}
