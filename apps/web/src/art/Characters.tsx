/**
 * Standalone ink pieces: <Zeb>, <Mine>, <Chest>, <Campfire>. Each sits in
 * a box of the given width, keeps its aspect ratio, boils when `live`, and
 * carries a screen-reader label (or is hidden when decorative).
 */

import { useCallback, type CSSProperties, type ReactNode } from "react";
import { BoilStack, InkDefs, LAYER_STYLE, Markup, useInkPrefix } from "./InkSvg";
import { GROUND } from "./ink";
import {
  chest,
  fireBase,
  fireFlames,
  fireGlow,
  flower,
  mine,
  tuft,
  zeb,
  type ZebPose,
} from "./props";

function Box({
  width,
  ratio,
  label,
  children,
  className,
  style,
}: {
  width: number | string;
  ratio: number;
  label?: string | undefined;
  children: ReactNode;
  className?: string | undefined;
  style?: CSSProperties | undefined;
}) {
  return (
    <div
      className={`relative shrink-0 ${className ?? ""}`}
      style={{ width, aspectRatio: String(ratio), ...style }}
      role={label === undefined ? undefined : "img"}
      aria-label={label}
      aria-hidden={label === undefined ? true : undefined}
    >
      {children}
    </div>
  );
}

const POSE_WORDS: Record<ZebPose, string> = {
  idle: "waiting with a lantern",
  cheer: "cheering",
  shrug: "shrugging",
  peek: "peeking out of a hole",
  chest: "holding a treasure chest",
  campfire: "sitting by the campfire",
};

export function Zeb({
  pose,
  width = 120,
  live = true,
  gold = false,
  label,
  className,
}: {
  pose: ZebPose;
  width?: number | string;
  live?: boolean;
  gold?: boolean;
  /** Overrides the default "Zeb the zebra miner, <pose>" (null hides it). */
  label?: string | null;
  className?: string | undefined;
}) {
  const render = useCallback((p: string) => zeb(pose, p, gold), [pose, gold]);
  return (
    <Box
      width={width}
      ratio={120 / 140}
      label={label === null ? undefined : (label ?? `Zeb the zebra miner, ${POSE_WORDS[pose]}`)}
      className={className}
    >
      <BoilStack viewBox="0 0 120 140" render={render} live={live} amp={0.55} />
    </Box>
  );
}

export function Mine({
  stage,
  erupt = false,
  width = 170,
  live = true,
  className,
}: {
  stage: 1 | 2 | 3 | 4 | 5;
  erupt?: boolean;
  width?: number | string;
  live?: boolean;
  className?: string | undefined;
}) {
  const render = useCallback((p: string) => mine(stage, erupt, true, p), [stage, erupt]);
  return (
    <Box
      width={width}
      ratio={170 / 175}
      label={`Deep Cache mine entrance, stage ${stage}${erupt ? ", breaking open" : ""}`}
      className={className}
    >
      <BoilStack viewBox="-85 -140 170 175" render={render} live={live} amp={0.6} />
    </Box>
  );
}

export function Chest({
  open,
  width = 120,
  label = "Treasure chest",
  className,
}: {
  open: boolean;
  width?: number | string;
  label?: string | null;
  className?: string | undefined;
}) {
  const p = useInkPrefix();
  return (
    <Box
      width={width}
      ratio={120 / 112}
      label={label === null ? undefined : label}
      className={className}
    >
      <svg viewBox="0 -16 120 112" style={LAYER_STYLE} overflow="visible" aria-hidden="true">
        <InkDefs p={p} />
        <Markup html={chest(open, true)} data-chest-art="" />
      </svg>
    </Box>
  );
}

export function Campfire({
  withZeb = true,
  width = 360,
  live = true,
  className,
  onPoke,
  sparks = 0,
}: {
  withZeb?: boolean;
  width?: number | string;
  live?: boolean;
  className?: string | undefined;
  /** Poke the fire (a burst of sparks). */
  onPoke?: () => void;
  /** Increments to replay the spark burst. */
  sparks?: number;
}) {
  const stones = useCallback(
    (p: string) =>
      `<ellipse cx="190" cy="196" rx="170" ry="26" fill="${GROUND}"/>${tuft(40, 200)}${tuft(320, 196)}${flower(70, 212)}${flower(300, 210)}` +
      fireGlow(236, 184, p) +
      fireBase(236, 184) +
      (withZeb ? `<g transform="translate(36 70)">${zeb("campfire", p, false)}</g>` : ""),
    [withZeb],
  );
  return (
    <Box
      width={width}
      ratio={360 / 230}
      label={withZeb ? "Zeb sitting by the campfire" : "A campfire"}
      className={className}
    >
      <BoilStack viewBox="0 0 360 230" render={stones} live={live} amp={0.6} />
      <svg viewBox="0 0 360 230" style={LAYER_STYLE} overflow="visible" aria-hidden="true">
        <Markup html={fireFlames(236, 184)} />
        {sparks > 0 && <SparkBurst key={sparks} cx={236} cy={150} />}
      </svg>
      {onPoke !== undefined && (
        <button
          type="button"
          className="zk-prop-button absolute"
          style={{ left: "52%", top: "45%", width: "20%", height: "40%" }}
          onClick={onPoke}
          aria-label="Poke the campfire"
        />
      )}
    </Box>
  );
}

/** A one-shot burst of ink sparks (CSS animated, removed with its key). */
export function SparkBurst({ cx, cy }: { cx: number; cy: number }) {
  return (
    <g className="zk-sparkburst" aria-hidden="true">
      {Array.from({ length: 9 }, (_, k) => {
        const a = -Math.PI / 2 + ((k - 4) * Math.PI) / 9;
        const dx = Math.cos(a) * 46;
        const dy = Math.sin(a) * 46;
        return (
          <path
            key={k}
            d={`M${cx} ${cy - 3} v6 M${cx - 3} ${cy} h6`}
            stroke="#FFFFFF"
            strokeWidth={2}
            style={{ "--dx": `${dx.toFixed(1)}px`, "--dy": `${dy.toFixed(1)}px` } as CSSProperties}
          />
        );
      })}
    </g>
  );
}
