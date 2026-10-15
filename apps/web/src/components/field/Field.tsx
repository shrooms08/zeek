/**
 * The field: the island and its props, the 5×5 tiles, and every effect on
 * them, as stacked SVGs sharing one viewBox (bottom to top):
 *
 *   ground   world ground, the slab's soft diamond shadow, slab sides
 *   back     props behind the field (boiling lines), Zeb's camp
 *   tiles    the tiles; overlays (closed, landing, dim, cracks, rings,
 *            split, light burst, chest), pins, dust puffs, the star, ink
 *            flight; and the hit polygons (the one tab stop)
 *   front    lamp light, props in front (boiling lines), fireflies
 *   props    buttons over the props (outside the tile diamond)
 *
 * Keyboard (one tab stop): arrows move, Space/Enter selects, A auto-picks,
 * D opens the dig, Esc clears. A live region reads the focused tile.
 */

import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import {
  CRACKS,
  burst as burstMarkup,
  puff,
  slabShadow,
  slabSides,
  splitPath,
  starPath,
} from "../../art/field";
import { InkDefs, LAYER_STYLE, Markup, useInkPrefix } from "../../art/InkSvg";
import { ctr, f, FIELD_DIAMOND, full, pts, VIEWBOX_FIELD, VIEWBOX_WORLD } from "../../art/ink";
import { chest } from "../../art/props";
import type { LandingDig } from "../../api/types";
import { DEPTH_WORDS } from "../../lib/digPlan";
import { prefersReducedMotion } from "../../lib/useNow";
import { moveCursor } from "./iso";
import { Tile } from "./Tile";
import { PropButtons, usePropReactions, WorldBack, WorldFront, type PropReaction } from "./World";

export interface FieldReveal {
  /** Dim the world (the draw is coming). */
  dim?: boolean;
  /** The draw falls as a star onto the field. */
  star?: boolean;
  /** Cracks race across the field; tiles tremble outward. */
  cracks?: boolean;
  /** Gold light (the real treasure); white in previews. */
  gold?: boolean;
}

export interface FieldProps {
  /** Depth stage 0–5 per tile. */
  stages: readonly number[];
  /** Selected tiles in pick order. */
  selected: readonly number[];
  /** The treasure tile once revealed. */
  treasure?: number | null;
  /** Light burst frame 0–3 (reveal). */
  burst?: 0 | 1 | 2 | 3;
  /** The round is closed: tiles cannot be picked. */
  closed?: boolean;
  onToggle?: (index: number) => void;
  onAutoPick?: () => void;
  onDig?: () => void;
  onClear?: () => void;
  /** Accessible label of the whole field. */
  label?: string;
  /** Draw the island and its props (desktop); false = the slab alone. */
  world?: boolean;
  mineStage?: 1 | 2 | 3 | 4 | 5;
  erupt?: boolean;
  /** The Deep Cache, formatted, for the mine's bubble. */
  cacheLabel?: string;
  /** Props respond to taps (false during the reveal and while sealing). */
  interactive?: boolean;
  /** Digs seen in the mempool for this round: dust settling, "landing". */
  landing?: readonly LandingDig[];
  reveal?: FieldReveal;
  /** Ink flies to these tiles (your dig was sealed); `n` replays it. */
  flight?: { tiles: readonly number[]; n: number } | null;
  onReact?: (reaction: PropReaction) => void;
}

interface PinPlace {
  i: number;
  n: number;
  base: [number, number];
  head: [number, number];
}

/** Inside tile t's diamond (with a margin)? */
function onTile(t: number, x: number, y: number, margin = 6): boolean {
  const [tx, ty] = ctr(t);
  return Math.abs(x - tx) / (40 + margin) + Math.abs(y - ty) / (20 + margin / 2) <= 1;
}

/**
 * Where each pin stands. During the reveal no pin may cover the treasure
 * tile: a pin whose stem or head would cross it leans sideways, away from
 * the treasure; the treasure tile's own pin stands on its back corner.
 */
export function placePins(selected: readonly number[], treasure: number | null): PinPlace[] {
  return selected.map((i, k) => {
    const [cx, cy] = ctr(i);
    if (treasure === null) return { i, n: k + 1, base: [cx, cy], head: [cx, cy - 44] };
    if (i === treasure) return { i, n: k + 1, base: [cx, cy - 20], head: [cx, cy - 64] };
    const crosses = (hx: number, hy: number) => {
      for (let s = 0; s <= 1; s += 0.1) {
        if (onTile(treasure, cx + (hx - cx) * s, cy + (hy - cy) * s)) return true;
      }
      return [
        [hx, hy - 12],
        [hx - 12, hy],
        [hx + 12, hy],
        [hx, hy + 12],
      ].some(([x, y]) => onTile(treasure, x ?? hx, y ?? hy));
    };
    const away = cx >= ctr(treasure)[0] ? 1 : -1;
    for (const [dx, dy] of [
      [0, -44],
      [30 * away, -36],
      [52 * away, -24],
      [64 * away, -8],
    ] as const) {
      if (!crosses(cx + dx, cy + dy))
        return { i, n: k + 1, base: [cx, cy], head: [cx + dx, cy + dy] };
    }
    return { i, n: k + 1, base: [cx, cy], head: [cx + 70 * away, cy] };
  });
}

export function Field(props: FieldProps) {
  const {
    stages,
    selected,
    treasure = null,
    burst = 0,
    closed = false,
    world = true,
    mineStage = 1,
    erupt = false,
    cacheLabel = "",
    interactive = true,
    landing = [],
    reveal = {},
    flight = null,
  } = props;
  const p = useInkPrefix();
  const [hover, setHover] = useState<number | null>(null);
  const [cursor, setCursor] = useState(12);
  const [hasFocus, setHasFocus] = useState(false);
  const liveId = useId();
  const reduce = prefersReducedMotion();
  const viewBox = world ? VIEWBOX_WORLD : VIEWBOX_FIELD;
  const [vx, vy, vw, vh] = viewBox.split(" ").map(Number) as [number, number, number, number];
  const { active, fire } = usePropReactions(props.onReact);

  const toggle = useCallback(
    (i: number) => {
      if (!closed) props.onToggle?.(i);
    },
    [closed, props],
  );

  const landingTiles = useMemo(() => new Set(landing.map((d) => d.tile)), [landing]);
  const describe = (i: number): string => {
    const stage = stages[i] ?? 0;
    const pick = selected.indexOf(i);
    const parts = [`Tile ${i + 1}`, DEPTH_WORDS[stage] ?? "untouched"];
    if (landingTiles.has(i)) parts.push("a dig is landing");
    if (treasure === i) parts.push("the treasure");
    if (pick >= 0) parts.push(`selected, pick ${pick + 1}`);
    return parts.join(", ");
  };

  const onKeyDown = (e: KeyboardEvent<SVGSVGElement>) => {
    if (e.key.startsWith("Arrow")) {
      e.preventDefault();
      setCursor((c) => moveCursor(c, e.key));
    } else if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      toggle(cursor);
    } else if (e.key === "a" || e.key === "A") {
      e.preventDefault();
      props.onAutoPick?.();
    } else if (e.key === "d" || e.key === "D") {
      e.preventDefault();
      props.onDig?.();
    } else if (e.key === "Escape") {
      props.onClear?.();
    }
  };

  // Dust puffs where a tile just got deeper (someone dug).
  const [puffs, setPuffs] = useState<{ i: number; key: number }[]>([]);
  const prevStages = useRef(stages);
  const puffKey = useRef(0);
  useEffect(() => {
    const before = prevStages.current;
    prevStages.current = stages;
    if (reduce) return;
    const grew = stages.map((s, i) => ((before[i] ?? 0) < s ? i : -1)).filter((i) => i >= 0);
    if (grew.length === 0 || grew.length === 25) return;
    setPuffs((list) => [...list, ...grew.map((i) => ({ i, key: ++puffKey.current }))]);
  }, [stages, reduce]);

  const pins = useMemo(
    () => placePins(selected, burst > 0 ? treasure : null),
    [selected, burst, treasure],
  );
  const lit = (i: number): "gold" | "white" | null =>
    treasure === i && burst > 0 ? (reveal.gold === false ? "white" : "gold") : null;
  const tremble = (i: number): number | null => {
    if (reveal.cracks !== true || reduce) return null;
    const [x, y] = ctr(i);
    return Math.round(Math.hypot(x, y - 100) * 2.2);
  };
  const shadow = useMemo(() => slabShadow() + slabSides(p), [p]);
  const sorted = useMemo(
    () =>
      Array.from({ length: 25 }, (_, i) => i).sort(
        (a, b) => (a % 5) + Math.floor(a / 5) - ((b % 5) + Math.floor(b / 5)),
      ),
    [],
  );
  const worldProps = { mineStage, erupt, cacheLabel, interactive, live: !reduce };

  return (
    <div
      className="relative w-full select-none"
      style={{ aspectRatio: `${vw} / ${vh}` }}
      data-world={world}
      data-reveal={reveal.cracks === true ? "cracks" : burst > 0 ? `burst-${burst}` : undefined}
    >
      {world ? <WorldBack {...worldProps} active={active} /> : null}
      <svg viewBox={viewBox} style={LAYER_STYLE} aria-hidden="true">
        <InkDefs p={p} />
        <Markup html={shadow} />
      </svg>
      <svg
        viewBox={viewBox}
        style={{ ...LAYER_STYLE, pointerEvents: "auto" }}
        overflow="visible"
        role="application"
        aria-roledescription="treasure field"
        aria-label={`${props.label ?? "The field: 25 tiles"}. Arrows move, Space selects, A auto-picks, D digs, Escape clears.`}
        aria-describedby={liveId}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onFocus={() => {
          setHasFocus(true);
        }}
        onBlur={() => {
          setHasFocus(false);
        }}
        data-closed={closed}
        data-testid="field"
      >
        <g data-layer="tiles">
          {sorted.map((i) => (
            <Tile
              key={i}
              index={i}
              depth={stages[i] ?? 0}
              p={p}
              lifted={hover === i && !closed && !reduce}
              lit={lit(i)}
              tremble={tremble(i)}
            />
          ))}
        </g>
        <g data-layer="overlay" style={{ pointerEvents: "none" }}>
          {closed && burst === 0 && (
            <polygon
              points={pts(FIELD_DIAMOND)}
              fill="#0D0D0D"
              opacity={0.5}
              className="zk-fade-in"
            />
          )}
          {[...landingTiles].map((i) => (
            <LandingMarker key={i} i={i} reduce={reduce} />
          ))}
          {reveal.dim === true && (
            <rect
              x={vx}
              y={vy}
              width={vw}
              height={vh}
              fill="#000"
              opacity={0.55}
              className="zk-dim"
            />
          )}
          {reveal.cracks === true &&
            CRACKS.map((d, k) => (
              <path
                key={k}
                d={d}
                pathLength={1}
                fill="none"
                stroke="#0A0A0A"
                strokeWidth={2.4}
                strokeLinejoin="round"
                strokeLinecap="round"
                className="zk-draw"
                style={{ animationDelay: `${k * 80}ms`, animationDuration: "500ms" }}
              />
            ))}
          {hover !== null && !closed && !selected.includes(hover) && (
            <polygon
              points={pts(full(hover))}
              fill="#0A0A0A"
              fillOpacity={0.1}
              stroke="#0A0A0A"
              strokeWidth={2.6}
              strokeDasharray="6 4"
              strokeLinejoin="round"
              transform={reduce ? undefined : "translate(0 -2)"}
            />
          )}
          {selected.map((i) => (
            <g key={i}>
              <polygon
                points={pts(full(i))}
                fill="none"
                stroke="#0A0A0A"
                strokeWidth={5}
                strokeLinejoin="round"
              />
              <polygon
                points={pts(full(i, -0.13))}
                fill="none"
                stroke="#0A0A0A"
                strokeWidth={1.5}
                strokeDasharray="3 3"
              />
            </g>
          ))}
          {hasFocus && (
            <g>
              <polygon
                points={pts(full(cursor, 0.07))}
                fill="none"
                stroke="#FFFFFF"
                strokeWidth={7}
                strokeLinejoin="round"
              />
              <polygon
                points={pts(full(cursor, 0.07))}
                fill="none"
                stroke="#0A0A0A"
                strokeWidth={3}
                strokeLinejoin="round"
              />
            </g>
          )}
          {treasure !== null && burst !== 0 && (
            <Treasure
              i={treasure}
              burst={burst}
              gold={reveal.gold !== false}
              p={p}
              reduce={reduce}
            />
          )}
          {pins.map((pin) => (
            <Pin key={pin.i} pin={pin} reduce={reduce} />
          ))}
          {puffs.map((pf) => (
            <Puff
              key={pf.key}
              i={pf.i}
              onDone={() => {
                setPuffs((list) => list.filter((x) => x.key !== pf.key));
              }}
            />
          ))}
          {reveal.star === true && <Star p={p} reduce={reduce} />}
          {flight !== null && !reduce && <Flight key={flight.n} tiles={flight.tiles} />}
        </g>
        <g data-layer="hit">
          {Array.from({ length: 25 }, (_, i) => (
            <polygon
              key={i}
              points={pts(full(i))}
              fill="transparent"
              pointerEvents="all"
              style={{ cursor: closed ? "default" : "pointer" }}
              data-tile={i}
              data-depth={Math.max(0, Math.min(5, stages[i] ?? 0))}
              data-selected={selected.includes(i)}
              data-landing={landingTiles.has(i) || undefined}
              onClick={() => {
                toggle(i);
              }}
              onPointerEnter={() => {
                setHover(i);
              }}
              onPointerLeave={() => {
                setHover((h) => (h === i ? null : h));
              }}
            />
          ))}
        </g>
      </svg>
      {world ? <WorldFront {...worldProps} active={active} /> : null}
      {world ? (
        <PropButtons
          interactive={interactive}
          cacheLabel={cacheLabel}
          fire={fire}
          active={active}
        />
      ) : null}
      <div id={liveId} className="sr-only" aria-live="polite" data-testid="field-live">
        {hasFocus ? describe(cursor) : ""}
      </div>
      <ul className="sr-only" aria-label="Tiles">
        {stages.map((_, i) => (
          <li key={i}>{describe(i)}</li>
        ))}
      </ul>
    </div>
  );
}

function Pin({ pin, reduce }: { pin: PinPlace; reduce: boolean }) {
  const [hx, hy] = pin.head;
  const [bx, by] = pin.base;
  return (
    <g data-pin={pin.i} className={reduce ? undefined : "zk-pin-drop"}>
      <ellipse cx={bx} cy={by} rx={5} ry={2.5} fill="#0A0A0A" />
      <line x1={bx} y1={by} x2={hx} y2={hy + 10} stroke="#0A0A0A" strokeWidth={3} />
      <circle cx={hx} cy={hy} r={11.5} fill="#0A0A0A" stroke="#FFFFFF" strokeWidth={2.5} />
      <text
        x={hx}
        y={hy + 4.5}
        textAnchor="middle"
        fontFamily="Archivo Variable, Archivo, sans-serif"
        fontWeight={800}
        fontSize={13}
        fill="#FFFFFF"
      >
        {pin.n}
      </text>
    </g>
  );
}

function Treasure({
  i,
  burst,
  gold,
  p,
  reduce,
}: {
  i: number;
  burst: 1 | 2 | 3;
  gold: boolean;
  p: string;
  reduce: boolean;
}) {
  const [cx, cy] = ctr(i);
  const origin = `${f(cx)}px ${f(cy)}px`;
  return (
    <g data-treasure={i}>
      <path
        d={splitPath(i)}
        pathLength={1}
        fill="none"
        stroke="#0A0A0A"
        strokeWidth={2.6}
        strokeLinejoin="round"
        className={reduce ? undefined : "zk-draw"}
        style={{ animationDuration: "200ms" }}
      />
      <g
        className={reduce ? "zk-fade-in" : "zk-burst"}
        style={{ transformOrigin: origin }}
        data-burst={burst}
      >
        <Markup html={burstMarkup(cx, cy, burst, gold, p)} />
      </g>
      {burst >= 3 && (
        <g
          className={reduce ? "zk-fade-in" : "zk-chest-drop"}
          style={{ transformOrigin: origin }}
          data-chest=""
        >
          <Markup
            html={`<g transform="translate(${f(cx - 30)} ${f(cy - 48)}) scale(.5)">${chest(true, gold)}</g>`}
          />
        </g>
      )}
      <polygon
        points={pts(full(i))}
        fill="none"
        stroke="#0A0A0A"
        strokeWidth={4.5}
        strokeLinejoin="round"
      />
    </g>
  );
}

function Puff({ i, onDone }: { i: number; onDone: () => void }) {
  const [cx, cy] = ctr(i);
  const html = useMemo(() => puff(cx, cy), [cx, cy]);
  return (
    <g
      className="zk-puff"
      style={{ transformOrigin: `${f(cx)}px ${f(cy - 12)}px` }}
      onAnimationEnd={onDone}
    >
      <Markup html={html} />
    </g>
  );
}

function Star({ p, reduce }: { p: string; reduce: boolean }) {
  return (
    <g className={reduce ? "zk-fade-in" : "zk-star-fall"} data-star="">
      <circle cx={0} cy={100} r={40} fill={`url(#${p}gw)`} />
      <path
        d={starPath(0, 100, 17, 7)}
        fill="#FFFFFF"
        stroke="#0A0A0A"
        strokeWidth={2.4}
        strokeLinejoin="round"
      />
    </g>
  );
}

/** Dust settling on a tile whose dig is in the mempool, with a marker. */
function LandingMarker({ i, reduce }: { i: number; reduce: boolean }) {
  const [cx, cy] = ctr(i);
  return (
    <g data-landing-marker={i}>
      <polygon
        points={pts(full(i, -0.18))}
        fill="none"
        stroke="#0A0A0A"
        strokeWidth={2}
        strokeDasharray="4 4"
        className={reduce ? undefined : "zk-seek"}
      />
      {!reduce &&
        [-10, 0, 9].map((dx, k) => (
          <circle
            key={k}
            cx={cx + dx}
            cy={cy - 26}
            r={1.8}
            fill="#0A0A0A"
            stroke="#ECEBE6"
            strokeWidth={0.8}
            className="zk-settle"
            style={{ animationDelay: `${k * 260}ms` }}
          />
        ))}
      <g transform={`translate(${f(cx)} ${f(cy + 13)})`}>
        <rect
          x={-22}
          y={-6}
          width={44}
          height={11}
          fill="#0A0A0A"
          stroke="#FFFFFF"
          strokeWidth={1.2}
        />
        <text
          x={0}
          y={2.6}
          textAnchor="middle"
          fontFamily="Archivo Variable, Archivo, sans-serif"
          fontWeight={800}
          fontSize={7}
          letterSpacing={0.8}
          fill="#FFFFFF"
        >
          LANDING
        </text>
      </g>
    </g>
  );
}

/** Ink flies from the seal (top right) to each of your tiles. */
function Flight({ tiles }: { tiles: readonly number[] }) {
  return (
    <g data-flight="">
      {tiles.map((i, k) => {
        const [cx, cy] = ctr(i);
        const sx = 230;
        const sy = -150;
        const d = `M${sx} ${sy} Q${f((sx + cx) / 2 + 40)} ${f(Math.min(sy, cy) - 60)} ${f(cx)} ${f(cy)}`;
        return (
          <path
            key={i}
            d={d}
            pathLength={1}
            fill="none"
            stroke="#FFFFFF"
            strokeWidth={2.4}
            strokeLinecap="round"
            className="zk-flight"
            style={{ animationDelay: `${350 + k * 60}ms` }}
          />
        );
      })}
    </g>
  );
}
