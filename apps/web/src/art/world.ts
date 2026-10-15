/**
 * The island around the field (ported from zeek-art.js worldBack /
 * worldFront) as a table of props, so each prop can be drawn, boiled and
 * made interactive on its own.
 */

import { f, GROUND, rng } from "./ink";
import { cart, lamp, mine, pine, rails, rock } from "./props";

export type PropKind = "tree" | "rock" | "lamp" | "mine" | "cart";

export interface PropDef {
  id: string;
  kind: PropKind;
  x: number;
  y: number;
  scale: number;
  /** Behind the field (drawn before the tiles) or in front of it. */
  layer: "back" | "front";
  /** Screen-reader label of the prop's button. */
  label: string;
  /** Hit circle (world units) around the prop's visual centre. */
  hit: { cx: number; cy: number; r: number };
}

const tree = (
  id: string,
  x: number,
  y: number,
  scale: number,
  layer: "back" | "front",
): PropDef => ({
  id,
  kind: "tree",
  x,
  y,
  scale,
  layer,
  label: "Pine tree",
  hit: { cx: x, cy: y - 34 * scale, r: 26 * scale },
});

/** Rails run from here along the iso x-axis; the cart rolls on them. */
export const RAILS = { x: 214, y: 40, len: 92 } as const;

export const PROPS: readonly PropDef[] = [
  tree("tree-1", -60, -46, 0.85, "back"),
  tree("tree-2", -100, -30, 1, "back"),
  tree("tree-3", -140, -6, 0.95, "back"),
  tree("tree-4", -178, 18, 1.05, "back"),
  tree("tree-5", -218, 42, 0.9, "back"),
  tree("tree-6", -252, 64, 0.8, "back"),
  tree("tree-7", -205, 70, 0.7, "back"),
  tree("tree-8", 60, -46, 0.8, "back"),
  tree("tree-9", 98, -26, 0.9, "back"),
  tree("tree-10", 248, 68, 0.9, "back"),
  {
    id: "mine",
    kind: "mine",
    x: 165,
    y: 22,
    scale: 1,
    layer: "back",
    label: "Mine entrance: press and hold to hear the Deep Cache",
    hit: { cx: 165, cy: -8, r: 46 },
  },
  {
    id: "cart",
    kind: "cart",
    x: RAILS.x,
    y: RAILS.y,
    scale: 1,
    layer: "back",
    label: "Mine cart",
    hit: { cx: RAILS.x + 20, cy: RAILS.y + 2, r: 22 },
  },
  {
    id: "rock-1",
    kind: "rock",
    x: -120,
    y: 40,
    scale: 0.7,
    layer: "back",
    label: "Rock",
    hit: { cx: -120, cy: 32, r: 16 },
  },
  tree("tree-11", -300, 108, 0.75, "front"),
  {
    id: "rock-2",
    kind: "rock",
    x: 300,
    y: 96,
    scale: 0.9,
    layer: "front",
    label: "Rock",
    hit: { cx: 300, cy: 86, r: 20 },
  },
  {
    id: "lamp-1",
    kind: "lamp",
    x: -252,
    y: 140,
    scale: 1,
    layer: "front",
    label: "Lamp",
    hit: { cx: -252, cy: 110, r: 22 },
  },
  {
    id: "lamp-2",
    kind: "lamp",
    x: 252,
    y: 140,
    scale: 1,
    layer: "front",
    label: "Lamp",
    hit: { cx: 252, cy: 110, r: 22 },
  },
  {
    id: "rock-3",
    kind: "rock",
    x: -150,
    y: 192,
    scale: 0.7,
    layer: "front",
    label: "Rock",
    hit: { cx: -150, cy: 184, r: 16 },
  },
  {
    id: "rock-4",
    kind: "rock",
    x: 132,
    y: 204,
    scale: 0.6,
    layer: "front",
    label: "Rock",
    hit: { cx: 132, cy: 197, r: 14 },
  },
];

/** A prop's ink, drawn around its own origin (so it can sway or wobble in place). */
export function propMarkup(
  prop: PropDef,
  p: string,
  mineStage: number,
  erupt: boolean,
  gold: boolean,
): string {
  switch (prop.kind) {
    case "tree":
      return pine(0, 0, prop.scale);
    case "rock":
      return rock(0, 0, prop.scale);
    case "lamp":
      return lamp(0, 0, p, prop.scale, "body");
    case "mine":
      return mine(mineStage, erupt, gold, p);
    case "cart":
      return cart();
  }
}

/** Ground of the island: two side faces, the top, tufts and flowers. */
export function worldGround(): string {
  let s = `<polygon points="-360,90 0,280 0,300 -360,110" fill="#242424"/><polygon points="0,280 360,90 360,110 0,300" fill="#1A1A1A"/><polygon points="0,-100 360,90 0,280 -360,90" fill="${GROUND}" stroke="#4A4A4A" stroke-width="1.5"/>`;
  const rr = rng(21);
  let td = "";
  let fd = "";
  for (let k = 0; k < 40; k++) {
    const x = (rr() - 0.5) * 620;
    const y = -60 + rr() * 300;
    const top = x < 0 ? 90 - ((x + 360) * 190) / 360 : -100 + (x * 190) / 360;
    const bot = x < 0 ? 90 + ((x + 360) * 190) / 360 : 280 - (x * 190) / 360;
    if (y < top + 10 || y > bot - 8) continue;
    if (Math.abs(x) < 215 && y > Math.abs(x) / 2 - 10 && y < 240 - Math.abs(x) / 2) continue;
    if (k % 3) td += `M${f(x - 4)} ${f(y)}l2 -5l2 5l2 -6l2 6`;
    else fd += `M${f(x - 2.5)} ${f(y - 2.5)}l5 5M${f(x + 2.5)} ${f(y - 2.5)}l-5 5`;
  }
  s += `<path d="${td}" fill="none" stroke="#5E5E5E" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/><path d="${fd}" stroke="#D6D6D6" stroke-width="1.6" stroke-linecap="round"/>`;
  s += `<g transform="translate(${RAILS.x} ${RAILS.y})">${rails(RAILS.len)}</g>`;
  return s;
}

/** Lamp light pools (round, unboiled, flickering). */
export function lampGlows(p: string): string {
  return PROPS.filter((x) => x.kind === "lamp")
    .map((l) => lamp(l.x, l.y, p, l.scale, "glow"))
    .join("");
}
