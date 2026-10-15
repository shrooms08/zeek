/**
 * <TimeLeft>: the round clock. In the last minute the panel's border burns
 * like a fuse (one second per step, a spark at the burning end); each of
 * the last ten seconds the numeral pulses; at 0:00 an ink "CLOSED" stamp
 * lands. Reduced motion: the border fills per second without the spark,
 * no pulse, and the stamp fades in.
 */

import { motion, useReducedMotion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { formatClock } from "../lib/time";
import { EASE } from "../motion/tokens";
import { Icon } from "./Icon";

export function TimeLeft({
  secondsLeft,
  open,
  note,
  stale,
  compact = false,
  onTick,
}: {
  secondsLeft: number;
  open: boolean;
  note?: string | undefined;
  stale?: boolean;
  compact?: boolean;
  /** Each second of the last ten (sound cue). */
  onTick?: () => void;
}) {
  const reduce = useReducedMotion() === true;
  const left = Math.max(0, Math.floor(secondsLeft));
  const lastMinute = open && left <= 60;
  const burnt = lastMinute ? (60 - left) / 60 : 0;
  const pulse = open && left <= 10 && left > 0;
  const prev = useRef(left);
  useEffect(() => {
    if (pulse && left !== prev.current) onTick?.();
    prev.current = left;
  }, [left, pulse, onTick]);
  // The stamp shows once the clock reaches zero or the round is closed.
  const [stamped, setStamped] = useState(!open);
  useEffect(() => {
    if (!open || left === 0) setStamped(true);
    else setStamped(false);
  }, [open, left]);

  return (
    <section
      className={`zk-timer relative ${compact ? "p-3" : "px-6 py-5"}`}
      aria-label="Time left"
      data-testid="time-left"
      data-last-minute={lastMinute}
    >
      {lastMinute && <Fuse burnt={burnt} reduce={reduce} />}
      <div className="label flex items-center gap-2">
        <Icon name="hourglass" size={compact ? 16 : 22} />
        Time left
        {stale === true && (
          <span className="text-fog normal-case tracking-normal font-normal">· reconnecting</span>
        )}
      </div>
      <motion.div
        key={pulse ? left : "steady"}
        className="numeral origin-left"
        style={{ fontSize: compact ? 40 : "clamp(48px, 5vw, 68px)", lineHeight: 0.95 }}
        initial={pulse && !reduce ? { scale: 1.12 } : false}
        animate={{ scale: 1 }}
        transition={{ duration: 0.16, ease: EASE.out }}
        aria-live={left <= 10 ? "polite" : "off"}
      >
        {open ? formatClock(left) : "0:00"}
      </motion.div>
      {note !== undefined && !compact && <p className="text-fog text-sm mt-1">{note}</p>}
      {stamped && (
        <motion.div
          className="zk-stamp absolute"
          initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 1.9, rotate: -10 }}
          animate={
            reduce ? { opacity: 1 } : { opacity: 1, scale: [1.9, 0.94, 1], rotate: [-10, -6, -6] }
          }
          transition={{ duration: reduce ? 0.15 : 0.3, ease: EASE.in }}
          data-testid="closed-stamp"
        >
          Closed
        </motion.div>
      )}
    </section>
  );
}

/**
 * The burning border, drawn in the panel's own pixels (measured), over the
 * white 3 px border: the burnt part (from the top-left, clockwise) turns
 * Ash, one second per step, via stroke-dashoffset; the spark rides the end.
 */
function Fuse({ burnt, reduce }: { burnt: number; reduce: boolean }) {
  const ref = useRef<SVGSVGElement>(null);
  const [box, setBox] = useState<{ w: number; h: number } | null>(null);
  useEffect(() => {
    const el = ref.current?.parentElement;
    if (el === null || el === undefined) return;
    const measure = () => {
      setBox({ w: el.clientWidth + 6, h: el.clientHeight + 6 });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => {
      ro.disconnect();
    };
  }, []);
  const w = box?.w ?? 0;
  const h = box?.h ?? 0;
  const per = 2 * (w + h - 6);
  const along = burnt * per;
  const iw = w - 3;
  const ih = h - 3;
  const [x, y] =
    along < iw
      ? [1.5 + along, 1.5]
      : along < iw + ih
        ? [1.5 + iw, 1.5 + (along - iw)]
        : along < 2 * iw + ih
          ? [1.5 + iw - (along - iw - ih), 1.5 + ih]
          : [1.5, 1.5 + ih - (along - 2 * iw - ih)];
  return (
    <svg
      ref={ref}
      className="absolute pointer-events-none"
      style={{ left: -3, top: -3, width: w, height: h, overflow: "visible" }}
      aria-hidden="true"
    >
      {box !== null && (
        <>
          <rect
            x={1.5}
            y={1.5}
            width={iw}
            height={ih}
            fill="none"
            stroke="#6B6B6B"
            strokeWidth={3}
            pathLength={1}
            strokeDasharray="1 1"
            style={{
              strokeDashoffset: 1 - burnt,
              transition: reduce ? "none" : "stroke-dashoffset 1s linear",
            }}
          />
          {!reduce && burnt > 0 && burnt < 1 && (
            <g
              style={{ transform: `translate(${x}px, ${y}px)`, transition: "transform 1s linear" }}
            >
              <circle r={7} fill="#6B6B6B" opacity={0.6} />
              <circle r={3} fill="#FFFFFF" />
            </g>
          )}
        </>
      )}
    </svg>
  );
}
