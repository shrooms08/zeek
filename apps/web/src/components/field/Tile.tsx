/**
 * One tile (an SVG group). Depth is drawn with shape, never colour alone:
 * the hole grows, its wall deepens and the floor darkens with the stage.
 * Phase 7b replaces the line art inside these groups.
 */

import { memo } from "react";
import { center, diamond, HOLE_SCALE, WALL_DEPTH } from "./iso";

export type TileState = "idle" | "hover" | "selected" | "focus" | "disabled" | "treasure";

export interface TileProps {
  index: number;
  depth: number;
  hovered: boolean;
  focused: boolean;
  /** 1-based pick order, if selected. */
  pick: number | null;
  treasure: boolean;
  disabled: boolean;
  onToggle: (index: number) => void;
  onHover: (index: number | null) => void;
}

const FLOOR = ["#ECEBE6", "#D6D6D6", "#A6A6A6", "#6B6B6B", "#3A3A3A", "#0A0A0A"];

export const Tile = memo(function Tile(props: TileProps) {
  const { index, depth, hovered, focused, pick, treasure, disabled } = props;
  const { x, y } = center(index);
  const stage = Math.max(0, Math.min(5, depth));
  const hole = HOLE_SCALE[stage] ?? 0;
  const wall = WALL_DEPTH[stage] ?? 0;
  return (
    <g
      data-tile={index}
      data-depth={stage}
      data-selected={pick !== null}
      onClick={() => {
        if (!disabled) props.onToggle(index);
      }}
      onMouseEnter={() => {
        props.onHover(index);
      }}
      onMouseLeave={() => {
        props.onHover(null);
      }}
      style={{ cursor: disabled ? "default" : "pointer" }}
    >
      <polygon
        points={diamond(x, y)}
        fill={treasure ? "var(--color-gold)" : disabled ? "#1C1C1C" : "#ECEBE6"}
        stroke="#0A0A0A"
        strokeWidth={2}
      />
      {hole > 0 && (
        <g aria-hidden="true">
          {/* The wall: the hole's back edge seen through its opening. */}
          <polygon
            points={diamond(x, y - wall / 3, hole)}
            fill="#3A3A3A"
            stroke="#0A0A0A"
            strokeWidth={1.5}
          />
          <polygon
            points={diamond(x, y + wall / 3, hole * 0.82)}
            fill={FLOOR[stage]}
            stroke="#0A0A0A"
            strokeWidth={1.5}
          />
        </g>
      )}
      {(hovered || focused || pick !== null) && !disabled && (
        <polygon
          points={diamond(x, y, 0.92)}
          fill="none"
          stroke={focused ? "#0A0A0A" : "#0A0A0A"}
          strokeWidth={focused ? 5 : 3}
          strokeDasharray={hovered && pick === null && !focused ? "8 6" : undefined}
        />
      )}
      {focused && (
        <polygon points={diamond(x, y, 1.04)} fill="none" stroke="#FFFFFF" strokeWidth={4} />
      )}
      {pick !== null && (
        <g aria-hidden="true">
          <line x1={x} y1={y} x2={x} y2={y - 34} stroke="#0A0A0A" strokeWidth={3} />
          <circle cx={x} cy={y - 46} r={14} fill="#0A0A0A" stroke="#FFFFFF" strokeWidth={3} />
          <text x={x} y={y - 41} textAnchor="middle" fontSize={15} fontWeight={900} fill="#FFFFFF">
            {pick}
          </text>
        </g>
      )}
    </g>
  );
});
