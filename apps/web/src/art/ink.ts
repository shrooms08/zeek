/**
 * The ink kit's core, ported from design/mono's zeek-art.js: palette, the
 * isometric projection every piece of art shares, number formatting, a
 * seeded RNG (so art is identical on every render) and the defs (hatch
 * patterns, glow gradients).
 *
 * The generators return SVG markup strings built only from numbers and
 * constants (never user text), which the React components mount. That keeps
 * the port line-for-line with the design's procedural art.
 */

export const INK = "#0A0A0A";
export const PAPER = "#ECEBE6";
export const GOLD = "#E8B54A";
export const WHITE = "#FFFFFF";
export const GROUND = "#3A3A3A";

/** Half a tile's width in field units (a tile is 80 × 40). */
export const A = 40;
/** Slab thickness under the field. */
export const T = 26;

export type Point = readonly [number, number];

/**
 * Formats a coordinate. The design's kit silently drew NaN at 0, which hid
 * broken geometry; here a non-finite number is a bug and throws.
 */
export function f(n: number): string {
  if (!Number.isFinite(n)) throw new Error(`ink: non-finite coordinate ${String(n)}`);
  return n.toFixed(1);
}

export const pts = (a: readonly Point[]): string => a.map((p) => `${f(p[0])},${f(p[1])}`).join(" ");

/** xorshift32, seeded: the same seed always draws the same art. */
export function rng(seed: number): () => number {
  let s = ((seed + 1) * 2654435761) >>> 0 || 1;
  return () => {
    s ^= s << 13;
    s >>>= 0;
    s ^= s >>> 17;
    s ^= s << 5;
    s >>>= 0;
    return s / 4294967296;
  };
}

/** Field point: row r, column c, and (s, t) ∈ [0, 1]² inside that tile. */
export const P = (r: number, c: number, s: number, t: number): Point => [
  (c - r) * A + (s - t) * A,
  ((c + r) * A) / 2 + ((s + t) * A) / 2,
];

export const rowOf = (i: number): number => Math.floor(i / 5);
export const colOf = (i: number): number => i % 5;

/** Centre of tile i's top face. */
export const ctr = (i: number): Point => P(rowOf(i), colOf(i), 0.5, 0.5);

/** Tile i's diamond, grown by k (negative shrinks). */
export const full = (i: number, k = 0): Point[] => {
  const r = rowOf(i);
  const c = colOf(i);
  return [P(r, c, -k, -k), P(r, c, 1 + k, -k), P(r, c, 1 + k, 1 + k), P(r, c, -k, 1 + k)];
};

/** The field's outer diamond (the slab's top face). */
export const FIELD_DIAMOND: Point[] = [
  [0, 0],
  [200, 100],
  [0, 200],
  [-200, 100],
];

/** viewBoxes of the field alone and of the field in its world. */
export const VIEWBOX_FIELD = "-215 -75 430 320";
export const VIEWBOX_WORLD = "-370 -175 740 490";

export const ST = `stroke="${INK}" stroke-width="3" stroke-linejoin="round" stroke-linecap="round"`;

/** Hatch spacing and stroke per depth stage (wider = lighter). */
const HATCH_GAP = [0, 7, 5.2, 4, 3, 2.2] as const;
const HATCH_W = [0, 1, 1.1, 1.3, 1.5, 1.7] as const;

/**
 * Patterns and gradients, namespaced by `p` so several SVGs can coexist:
 * `${p}h1…5` hatch, `${p}x1…5` cross-hatch, `${p}gw` white glow, `${p}gg`
 * gold glow (treasure only).
 */
export function defs(p: string): string {
  let s = "";
  for (let d = 1; d <= 5; d++) {
    const w = HATCH_GAP[d] ?? 4;
    const sw = HATCH_W[d] ?? 1;
    s += `<pattern id="${p}h${d}" patternUnits="userSpaceOnUse" width="${w}" height="${w}"><path d="M-1 1L1 -1M0 ${w}L${w} 0M${w - 1} ${w + 1}L${w + 1} ${w - 1}" stroke="${INK}" stroke-width="${sw}"/></pattern>`;
    s += `<pattern id="${p}x${d}" patternUnits="userSpaceOnUse" width="${w}" height="${w}"><path d="M-1 1L1 -1M0 ${w}L${w} 0M${w - 1} ${w + 1}L${w + 1} ${w - 1}M-1 ${w - 1}L1 ${w + 1}M0 0L${w} ${w}M${w - 1} -1L${w + 1} 1" stroke="${INK}" stroke-width="${sw}"/></pattern>`;
  }
  s += `<radialGradient id="${p}gw"><stop offset="0" stop-color="#fff" stop-opacity=".95"/><stop offset=".35" stop-color="#fff" stop-opacity=".35"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient>`;
  s += `<radialGradient id="${p}gg"><stop offset="0" stop-color="${GOLD}" stop-opacity="1"/><stop offset=".4" stop-color="${GOLD}" stop-opacity=".5"/><stop offset="1" stop-color="${GOLD}" stop-opacity="0"/></radialGradient>`;
  return s;
}

/** Geometry attributes whose numbers a boil variant may nudge. */
const GEOMETRY = /\b(d|points|x1|y1|x2|y2|cx|cy|rx|ry)="([^"]*)"/g;

/**
 * Boiling lines without filters: variant v (1, 2) of `markup` nudges every
 * geometry number by up to ±amp, deterministically. Variant 0 is the
 * original. Paths with arcs keep their flags intact (they are skipped).
 * Swapping three pre-built variants is a compositor-only opacity change;
 * the design's feTurbulence filters would re-filter the art every frame.
 */
export function boilVariant(markup: string, v: number, amp = 0.7): string {
  if (v === 0) return markup;
  const r = rng(v * 7919);
  return markup.replace(GEOMETRY, (whole, name: string, value: string) => {
    if (name === "d" && /[aA]/.test(value)) return whole;
    if (name === "rx" || name === "ry") {
      const n = Number(value);
      return Number.isFinite(n)
        ? `${name}="${f(Math.max(0.1, n + (r() - 0.5) * amp * 0.6))}"`
        : whole;
    }
    const nudged = value.replace(/-?\d+(?:\.\d+)?/g, (num) =>
      f(Number(num) + (r() - 0.5) * 2 * amp),
    );
    return `${name}="${nudged}"`;
  });
}

/** An id prefix safe inside url(#…) (React's useId has colons). */
export function safeId(id: string): string {
  return `zk${id.replace(/[^a-zA-Z0-9]/g, "")}_`;
}
