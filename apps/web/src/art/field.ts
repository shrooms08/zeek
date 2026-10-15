/**
 * The field's ink (ported from zeek-art.js): slab sides, tile grain, holes
 * at depth 0–5, the reveal's cracks, split, light burst, the dust puff and
 * the falling star.
 *
 * Fix from the design file: the island's shadow is a soft isometric diamond
 * under the slab footprint (stacked offset diamonds), not an ellipse.
 */

import { ctr, f, full, GOLD, INK, P, PAPER, pts, rng, T, WHITE, type Point } from "./ink";

/** Soft diamond shadow under the slab: three widening, fading diamonds. */
export function slabShadow(): string {
  const layer = (grow: number, drop: number, opacity: number) => {
    const c: Point = [0, 100 + drop];
    const d: Point[] = [
      [c[0], c[1] - 100 * grow],
      [c[0] + 200 * grow, c[1]],
      [c[0], c[1] + 100 * grow],
      [c[0] - 200 * grow, c[1]],
    ];
    return `<polygon points="${pts(d)}" fill="#000" opacity="${opacity}"/>`;
  };
  return layer(1.16, T + 14, 0.12) + layer(1.09, T + 10, 0.16) + layer(1.03, T + 6, 0.22);
}

export function slabSides(p: string): string {
  const L: Point[] = [
    [-200, 100],
    [0, 200],
    [0, 200 + T],
    [-200, 100 + T],
  ];
  const R: Point[] = [
    [0, 200],
    [200, 100],
    [200, 100 + T],
    [0, 200 + T],
  ];
  let s = `<polygon points="${pts(L)}" fill="${PAPER}" stroke="${INK}" stroke-width="2.4" stroke-linejoin="round"/><polygon points="${pts(R)}" fill="${PAPER}"/><polygon points="${pts(R)}" fill="url(#${p}h3)" stroke="${INK}" stroke-width="2.4" stroke-linejoin="round"/>`;
  for (let k = 1; k < 5; k++) {
    s += `<line x1="${-200 + 40 * k}" y1="${100 + 20 * k}" x2="${-200 + 40 * k}" y2="${100 + 20 * k + T}" stroke="${INK}" stroke-width="1.6"/><line x1="${40 * k}" y1="${200 - 20 * k}" x2="${40 * k}" y2="${200 - 20 * k + T}" stroke="${INK}" stroke-width="1.6"/>`;
  }
  s += `<line x1="-200" y1="${100 + T / 2}" x2="0" y2="${200 + T / 2}" stroke="${INK}" stroke-width="1.1"/>`;
  const rr = rng(3);
  for (let k = 0; k < 16; k++) {
    const x = -196 + rr() * 188;
    const y = 100 + (x + 200) / 2 + 4 + rr() * (T - 9);
    s += `<line x1="${f(x)}" y1="${f(y)}" x2="${f(x + 9)}" y2="${f(y + 4.5)}" stroke="${INK}" stroke-width="1" opacity=".7"/>`;
  }
  return s + `<polygon points="0,0 200,100 0,200 -200,100" fill="${INK}"/>`;
}

/** Hole size, wall depth, rim weight and wall shade per depth stage. */
const HS = [0, 0.15, 0.22, 0.29, 0.35, 0.405] as const;
const HD = [0, 4, 8, 13, 19, 26] as const;
const RIM = [0, 1.6, 2.2, 2.9, 3.6, 4.4] as const;
const UND = [PAPER, PAPER, "#DCDBD5", "#BDBCB6", "#8E8E8A", "#5C5C5A"] as const;
const und = (d: number): string => UND[Math.max(0, Math.min(5, d))] ?? PAPER;

/** The hole of tile i at depth d (1–5): opening, walls, floor, rim, spoil. */
export function hole(i: number, d: number, p: string): string {
  const r = Math.floor(i / 5);
  const c = i % 5;
  const q = (s: number, t: number) => P(r, c, s, t);
  const h = HS[d] ?? 0;
  const D = HD[d] ?? 0;
  const o: Point[] = [
    q(0.5 - h, 0.5 - h),
    q(0.5 + h, 0.5 - h),
    q(0.5 + h, 0.5 + h),
    q(0.5 - h, 0.5 + h),
  ];
  const [tp, rt, , lf] = o as [Point, Point, Point, Point];
  const dn = (v: Point): Point => [v[0], v[1] + D];
  const id = `${p}c${i}`;
  const e = 0.035;
  const lip: Point[] = [
    q(0.5 - h - e, 0.5 - h - e),
    q(0.5 + h + e, 0.5 - h - e),
    q(0.5 + h + e, 0.5 + h + e),
    q(0.5 - h - e, 0.5 + h + e),
  ];
  let g =
    d >= 2
      ? `<polygon points="${pts(lip)}" fill="none" stroke="${INK}" stroke-width="1.1" stroke-dasharray="2 3" opacity=".8"/>`
      : "";
  g += `<clipPath id="${id}"><polygon points="${pts(o)}"/></clipPath><polygon points="${pts(o)}" fill="${d >= 5 ? INK : und(d)}"/><g clip-path="url(#${id})">`;
  const fl = o.map(dn);
  g +=
    d >= 4
      ? `<polygon points="${pts(fl)}" fill="${d >= 5 ? INK : "#2A2A2A"}"/>`
      : `<polygon points="${pts(fl)}" fill="${und(d + 1)}"/><polygon points="${pts(fl)}" fill="url(#${p}x${d})"/>`;
  const w1 = [lf, tp, dn(tp), dn(lf)];
  const w2 = [tp, rt, dn(rt), dn(tp)];
  g += `<g data-hatch><polygon points="${pts(w1)}" fill="${und(d)}"/><polygon points="${pts(w1)}" fill="url(#${p}h${d})" stroke="${INK}" stroke-width="1.2"/><polygon points="${pts(w2)}" fill="${und(d + 1)}"/><polygon points="${pts(w2)}" fill="url(#${p}x${d})" stroke="${INK}" stroke-width="1.2"/></g></g>`;
  g += `<polygon data-rim pathLength="1" points="${pts(o)}" fill="none" stroke="${INK}" stroke-width="${RIM[d] ?? 1}" stroke-linejoin="round"/>`;
  const rc = rng(i * 13 + d);
  let cd = "";
  for (let k = 0; k < 2 + d * 3; k++) {
    const ee = 0.5 + h + 0.05 + rc() * 0.1;
    const t = 0.15 + rc() * 0.7;
    const [x, y] = k % 2 ? q(ee, t) : q(t, ee);
    const rad = 0.9 + rc() * (1 + d * 0.12);
    cd += `M${f(x - rad)} ${f(y)}a${f(rad)} ${f(rad)} 0 1 0 ${f(2 * rad)} 0a${f(rad)} ${f(rad)} 0 1 0 ${f(-2 * rad)} 0`;
  }
  return g + `<path d="${cd}" fill="${INK}"/>`;
}

/**
 * Tile i's top face and wood grain (no hole). The grain is seeded per tile,
 * so it never changes between renders. `lit` is the revealed treasure.
 */
export function tileFace(i: number, lit: "gold" | "white" | null): string {
  const r = Math.floor(i / 5);
  const c = i % 5;
  const q = (s: number, t: number) => P(r, c, s, t);
  const rr = rng(1000 + i);
  const e = 0.035;
  let g = `<polygon points="${pts([q(e, e), q(1 - e, e), q(1 - e, 1 - e), q(e, 1 - e)])}" fill="${lit === "gold" ? GOLD : lit === "white" ? WHITE : PAPER}" stroke="${INK}" stroke-width="1.8" stroke-linejoin="round"/>`;
  const along = (r * 3 + c) % 2 === 0;
  let gd = "";
  for (let k = 1; k <= 4; k++) {
    const t = k / 5 + (rr() - 0.5) * 0.05;
    const a = 0.08 + rr() * 0.18;
    const b = 0.92 - rr() * 0.18;
    const m = (a + b) / 2;
    const w = t + (rr() - 0.5) * 0.09;
    const [x1, y1] = along ? q(a, t) : q(t, a);
    const [x2, y2] = along ? q(b, t) : q(t, b);
    const [mx, my] = along ? q(m, w) : q(w, m);
    gd += `M${f(x1)} ${f(y1)}Q${f(mx)} ${f(my)} ${f(x2)} ${f(y2)}`;
  }
  g += `<path d="${gd}" fill="none" stroke="${INK}" stroke-width="1" stroke-linecap="round" opacity="${lit !== null ? 0.35 : 0.75}"/>`;
  return g;
}

/** A knot in the wood, on some untouched tiles. */
export function tileKnot(i: number): string {
  const r = Math.floor(i / 5);
  const c = i % 5;
  const rr = rng(2000 + i);
  if (rr() >= 0.35) return "";
  const along = (r * 3 + c) % 2 === 0;
  const [kx, ky] = P(r, c, 0.25 + rr() * 0.5, 0.25 + rr() * 0.5);
  return `<ellipse cx="${f(kx)}" cy="${f(ky)}" rx="5" ry="2.2" transform="rotate(${along ? 26.6 : -26.6} ${f(kx)} ${f(ky)})" fill="none" stroke="${INK}" stroke-width="1"/>`;
}

export const CRACKS = [
  "M0 100 L-22 92 L-40 104 L-70 96 L-96 110 L-130 104 L-160 116",
  "M0 100 L18 110 L34 102 L60 118 L88 112 L120 128 L150 122",
  "M0 100 L6 78 L-6 60 L4 40 L-4 20",
  "M0 100 L-4 124 L10 146 L0 170 L6 188",
] as const;

/** The jagged line that splits the treasure tile. */
export function splitPath(i: number): string {
  const [cx, cy] = ctr(i);
  return `M${f(cx - 30)} ${f(cy - 6)} L${f(cx - 14)} ${f(cy + 2)} L${f(cx - 4)} ${f(cy - 6)} L${f(cx + 8)} ${f(cy + 4)} L${f(cx + 30)} ${f(cy - 4)}`;
}

/** Light burst over (cx, cy): frame 1 glint, 2 full burst, 3 settled. */
export function burst(cx: number, cy: number, b: 1 | 2 | 3, gold: boolean, p: string): string {
  let s = `<circle cx="${f(cx)}" cy="${f(cy)}" r="${b === 1 ? 70 : b === 2 ? 220 : 150}" fill="url(#${p}${gold ? "gg" : "gw"})"/>`;
  if (b >= 2) {
    const n = b === 2 ? 15 : 11;
    for (let k = 0; k < n; k++) {
      const a = ((-168 + k * (156 / (n - 1))) * Math.PI) / 180;
      const L = (k % 2 ? 200 : 300) * (b === 2 ? 1 : 0.65);
      const w = k % 2 ? 0.035 : 0.055;
      s += `<polygon points="${pts([
        [cx, cy],
        [cx + Math.cos(a - w) * L, cy + Math.sin(a - w) * L],
        [cx + Math.cos(a + w) * L, cy + Math.sin(a + w) * L],
      ])}" fill="${gold ? GOLD : WHITE}" stroke="${INK}" stroke-width="1.4" stroke-linejoin="round"/>`;
    }
  }
  const rr = rng(4);
  for (let k = 0; k < (b === 1 ? 5 : 9); k++) {
    const x = cx + (rr() - 0.5) * (b === 1 ? 90 : 240);
    const y = cy - 20 - rr() * (b === 1 ? 50 : 150);
    const z = 3 + rr() * 4;
    s += `<path d="M${f(x)} ${f(y - z)} L${f(x + z * 0.3)} ${f(y - z * 0.3)} L${f(x + z)} ${f(y)} L${f(x + z * 0.3)} ${f(y + z * 0.3)} L${f(x)} ${f(y + z)} L${f(x - z * 0.3)} ${f(y + z * 0.3)} L${f(x - z)} ${f(y)} L${f(x - z * 0.3)} ${f(y - z * 0.3)}Z" fill="${gold && b > 1 ? GOLD : WHITE}" stroke="${INK}" stroke-width="1" class="zk-twinkle" style="animation-delay:-${f(k * 0.37)}s"/>`;
  }
  return s;
}

/** Ink dust puff at (cx, cy) (one frame; animated with steps(6)). */
export function puff(cx: number, cy: number): string {
  const c = [
    [0, -10, 9],
    [-11, -6, 7],
    [11, -7, 7.5],
    [-6, -18, 6.5],
    [7, -19, 7],
    [0, -27, 5.5],
  ] as const;
  let s = "";
  for (const [x, y, r] of c)
    s += `<circle cx="${f(cx + x)}" cy="${f(cy + y)}" r="${r}" fill="${INK}" stroke="${INK}" stroke-width="4.5"/>`;
  for (const [x, y, r] of c)
    s += `<circle cx="${f(cx + x)}" cy="${f(cy + y)}" r="${r}" fill="${PAPER}"/>`;
  s += `<path d="M${f(cx - 4)} ${f(cy - 14)} q4 -3 8 0 M${f(cx + 4)} ${f(cy - 22)} q3 -2 5 0" fill="none" stroke="${INK}" stroke-width="1.2"/>`;
  for (const [x, y] of [
    [-21, -12],
    [22, -18],
    [-17, -30],
    [17, -33],
    [-25, -3],
    [26, -6],
  ] as const)
    s += `<circle cx="${f(cx + x)}" cy="${f(cy + y)}" r="1.7" fill="${INK}" stroke="${PAPER}" stroke-width=".8"/>`;
  s += `<path d="M${f(cx - 28)} ${f(cy - 22)} l-6 -3 M${f(cx + 30)} ${f(cy - 26)} l6 -3 M${f(cx)} ${f(cy - 38)} v-6" stroke="${PAPER}" stroke-width="1.8" stroke-linecap="round"/>`;
  return s;
}

export function starPath(cx: number, cy: number, R: number, r: number): string {
  let d = "";
  for (let k = 0; k < 10; k++) {
    const a = -Math.PI / 2 + (k * Math.PI) / 5;
    const q = k % 2 ? r : R;
    d += `${k ? "L" : "M"}${f(cx + Math.cos(a) * q)} ${f(cy + Math.sin(a) * q)}`;
  }
  return d + "Z";
}

/** The pin over tile i showing pick number n, lifted by `lift` (reveal). */
export function pinGeometry(i: number, lift = 0): { cx: number; cy: number; top: number } {
  const [cx, cy] = ctr(i);
  return { cx, cy, top: cy - 44 - lift };
}

export { full };
