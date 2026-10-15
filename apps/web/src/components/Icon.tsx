/**
 * <Icon>: the design's pictograms. The chest is gold (treasure lives there)
 * unless `tone="ink"`. `draw` draws the stroke on once (ink check marks,
 * copy confirmation); with reduced motion it simply appears.
 */

import { motion, useReducedMotion } from "framer-motion";
import { ICONS, type IconName } from "../art/icons";
import { EASE } from "../motion/tokens";

export type { IconName };

export function Icon({
  name,
  size = 20,
  label,
  tone,
  draw = false,
  strokeWidth = 2.25,
}: {
  name: IconName;
  size?: number;
  label?: string;
  tone?: "ink" | "gold";
  draw?: boolean;
  strokeWidth?: number;
}) {
  const reduce = useReducedMotion() === true;
  const hand = name.endsWith("ink");
  const gold = tone === "gold" || (name === "chest" && tone !== "ink");
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke={gold ? "var(--color-gold)" : "currentColor"}
      strokeWidth={strokeWidth}
      strokeLinecap={hand ? "round" : "square"}
      strokeLinejoin={hand ? "round" : "miter"}
      role={label === undefined ? undefined : "img"}
      aria-label={label}
      aria-hidden={label === undefined ? true : undefined}
      focusable="false"
      className="shrink-0"
    >
      {draw && !reduce ? (
        <motion.path
          d={ICONS[name]}
          pathLength={1}
          strokeDasharray="1 1"
          initial={{ strokeDashoffset: 1 }}
          animate={{ strokeDashoffset: 0 }}
          transition={{ duration: 0.25, ease: EASE.out }}
        />
      ) : (
        <path d={ICONS[name]} />
      )}
    </svg>
  );
}
