/**
 * The field: an isometric 5×5 SVG in named layers so Phase 7b can add the
 * full art, motion and interactive props without touching the logic:
 *
 *   ground   – the slab and island
 *   shadow   – hard shadows under the slab
 *   tiles    – the 25 tiles (depth, hover, selection)
 *   props    – trees, lamps, the mine (empty in 7a)
 *   effects  – dust, light burst, treasure glint
 *   overlay  – pins and focus (drawn by tiles in 7a)
 *
 * Keyboard (one tab stop): arrows move, Space/Enter selects, A auto-picks,
 * D opens the dig, Esc clears. A live region reads the focused tile.
 */

import { useCallback, useId, useState, type KeyboardEvent } from "react";
import { DEPTH_WORDS } from "../../lib/digPlan";
import { center, diamond, moveCursor, SLAB, TILE_H, TILE_W, VIEWBOX } from "./iso";
import { Tile } from "./Tile";

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
}

export function Field(props: FieldProps) {
  const { stages, selected, treasure = null, burst = 0, closed = false } = props;
  const [hover, setHover] = useState<number | null>(null);
  const [cursor, setCursor] = useState(12);
  const [hasFocus, setHasFocus] = useState(false);
  const liveId = useId();
  const toggle = useCallback(
    (i: number) => {
      if (!closed) props.onToggle?.(i);
    },
    [closed, props],
  );

  const describe = (i: number): string => {
    const stage = stages[i] ?? 0;
    const pick = selected.indexOf(i);
    const parts = [`Tile ${i + 1}`, DEPTH_WORDS[stage] ?? "untouched"];
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

  // Slab outline: the field's outer diamond, extruded by SLAB.
  const north = center(0);
  const east = center(4);
  const south = center(24);
  const west = center(20);
  const outline = {
    n: { x: north.x, y: north.y - TILE_H / 2 },
    e: { x: east.x + TILE_W / 2, y: east.y },
    s: { x: south.x, y: south.y + TILE_H / 2 },
    w: { x: west.x - TILE_W / 2, y: west.y },
  };

  return (
    <div className="relative w-full">
      <svg
        viewBox={VIEWBOX}
        className="block w-full h-auto select-none"
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
      >
        <g data-layer="ground">
          <polygon
            points={`${outline.w.x},${outline.w.y} ${outline.s.x},${outline.s.y} ${outline.s.x},${outline.s.y + SLAB} ${outline.w.x},${outline.w.y + SLAB}`}
            fill="#ECEBE6"
            stroke="#0A0A0A"
            strokeWidth={2}
          />
          <polygon
            points={`${outline.s.x},${outline.s.y} ${outline.e.x},${outline.e.y} ${outline.e.x},${outline.e.y + SLAB} ${outline.s.x},${outline.s.y + SLAB}`}
            fill="#A6A6A6"
            stroke="#0A0A0A"
            strokeWidth={2}
          />
        </g>
        <g data-layer="shadow" />
        <g data-layer="tiles" opacity={closed && treasure === null ? 0.7 : 1}>
          {stages.map((depth, i) => (
            <Tile
              key={i}
              index={i}
              depth={depth}
              hovered={hover === i}
              focused={hasFocus && cursor === i}
              pick={selected.includes(i) ? selected.indexOf(i) + 1 : null}
              treasure={treasure === i}
              disabled={closed}
              onToggle={toggle}
              onHover={setHover}
            />
          ))}
        </g>
        <g data-layer="props" />
        <g data-layer="effects">
          {treasure !== null && burst > 0 && (
            <polygon
              points={diamond(center(treasure).x, center(treasure).y, 0.4 + burst * 0.25)}
              fill="var(--color-gold)"
              opacity={0.25 * burst}
              data-burst={burst}
            />
          )}
        </g>
        <g data-layer="overlay" />
      </svg>
      <div id={liveId} className="sr-only" aria-live="polite">
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
