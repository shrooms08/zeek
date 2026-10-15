/**
 * <SealRing>: "Sealing your dig". An ink ring draws around the Z mark as
 * the payments are proven (stroke-dashoffset, linear; the design budgets
 * about 5 s per payment). It follows real progress and never claims to be
 * done before the last payment is sent. Reduced motion: a static ring.
 */

import { useReducedMotion } from "framer-motion";
import { LOGO_VIEWBOX, zMarkMarkup } from "../art/logoGeometry";

export function SealRing({ fraction, size = 168 }: { fraction: number; size?: number }) {
  const reduce = useReducedMotion() === true;
  const f = Math.max(0, Math.min(1, fraction));
  const [x, y, w, h] = LOGO_VIEWBOX.split(" ").map(Number) as [number, number, number, number];
  return (
    <div
      className="relative"
      style={{ width: size, height: size }}
      role="img"
      aria-label={`Sealing, ${Math.round(f * 100)} percent`}
    >
      <svg viewBox="0 0 200 200" className="absolute inset-0 h-full w-full" aria-hidden="true">
        <circle
          cx={100}
          cy={100}
          r={88}
          fill="none"
          stroke="#3A3A3A"
          strokeWidth={3}
          strokeDasharray="4 6"
        />
        <circle
          cx={100}
          cy={100}
          r={88}
          fill="none"
          stroke="#FFFFFF"
          strokeWidth={7}
          strokeLinecap="round"
          pathLength={1}
          strokeDasharray="1 1"
          transform="rotate(-90 100 100)"
          style={{
            strokeDashoffset: reduce ? 0 : 1 - f,
            transition: reduce ? "none" : "stroke-dashoffset 1.6s linear",
          }}
          data-testid="seal-ring"
        />
      </svg>
      <svg
        viewBox={`${x} ${y} ${w} ${h}`}
        className="absolute"
        style={{ left: "22%", top: "26%", width: "56%", height: "48%" }}
        aria-hidden="true"
        dangerouslySetInnerHTML={{ __html: zMarkMarkup("#FFFFFF", "#0D0D0D") }}
      />
    </div>
  );
}
