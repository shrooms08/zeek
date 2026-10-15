/**
 * <CountUp>: a zatoshi amount that counts to its new value (400 ms, ease-out
 * cubic) and optionally pulses once when it grows. Money stays integral: the
 * animation steps through whole zatoshis and always lands on the exact
 * value. Reduced motion jumps straight to the value.
 */

import { animate, useMotionValue, useReducedMotion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { formatZec, type Zat } from "../lib/money";
import { DUR, EASE } from "./tokens";

export function CountUp({
  value,
  decimals = 4,
  sign = false,
  duration = DUR.count,
  delay = 0,
  from,
  onGrow,
}: {
  value: Zat;
  decimals?: number;
  sign?: boolean;
  duration?: number;
  delay?: number;
  /** Start from this value on first render (e.g. 0 for a win count-up). */
  from?: Zat;
  /** Called when the value grows (for a pulse). */
  onGrow?: () => void;
}) {
  const reduce = useReducedMotion() === true;
  const start = from ?? value;
  const mv = useMotionValue(Number(start));
  const [shown, setShown] = useState<Zat>(start);
  const last = useRef<Zat>(start);

  useEffect(() => {
    const before = last.current;
    last.current = value;
    if (before === value) return;
    if (value > before) onGrow?.();
    if (reduce) {
      mv.set(Number(value));
      setShown(value);
      return;
    }
    const controls = animate(mv, Number(value), {
      duration: duration / 1000,
      delay: delay / 1000,
      ease: [0.33, 1, 0.68, 1],
      onUpdate: (v) => {
        setShown(BigInt(Math.round(v)));
      },
      onComplete: () => {
        setShown(value);
      },
    });
    return () => {
      controls.stop();
    };
  }, [value, reduce, mv, duration, delay, onGrow]);

  return (
    <span style={{ fontVariantNumeric: "tabular-nums" }}>
      {formatZec(shown, { decimals, sign })}
    </span>
  );
}

/** Pulse keyframes for a value that grew (scale 1 → 1.08 → 1, 420 ms). */
export const PULSE: {
  animate: { scale: number[] };
  transition: { duration: number; ease: readonly [number, number, number, number] };
} = {
  animate: { scale: [1, 1.08, 1] },
  transition: { duration: DUR.pulse / 1000, ease: EASE.out },
};
