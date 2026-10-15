/**
 * The Zeek logo, from the design system (zeek-art.js ZO/ZH): the Z mark
 * alone (favicon, app icon, X avatar) and the lockup with "eek" reversed out
 * of the Z's black lower-right area.
 *
 * The "eek" slant is one config value (EEK_SLANT). Three options:
 *   "20"       about 20°
 *   "30"       about 30°
 *   "diagonal" the angle of the Z's diagonal stroke, measured from the
 *              mark's own geometry (the cut from (488,316) to (222,595)).
 */

/** Outer silhouette of the Z mark (viewBox 128 282 483 358). */
export const Z_OUTER =
  "M220 297L512 297L596 380L530 460L596 527L516 625L220 625L143 527L205 460L143 378Z";
/** The cuts reversed out of it: the diagonal stroke and the small triangle. */
export const Z_CUTS =
  "M176 371L231 316L488 316L222 595L163 537L303 392Q309 371 290 371Z M468 525L517 476L562 525Z";

export const LOGO_VIEWBOX = "128 282 483 358";

/** The diagonal's lean from vertical, in degrees, from the cut's endpoints. */
export const DIAGONAL_DEG = (Math.atan2(488 - 222, 595 - 316) * 180) / Math.PI;

export type EekSlant = "20" | "30" | "diagonal";

/** The active option: the slant that matches the Z's diagonal exactly. */
export const EEK_SLANT: EekSlant = "diagonal";

export function slantDegrees(slant: EekSlant): number {
  return slant === "diagonal" ? DIAGONAL_DEG : Number(slant);
}

/**
 * The lockup's text placement for a slant: anchored at its end so the
 * skewed "k" stays inside the Z's lower-right edge (the edge from
 * (596,527) to (516,625)), with an 8-unit margin.
 */
export function eekPlacement(slant: EekSlant): { x: number; y: number; skew: number } {
  const deg = slantDegrees(slant);
  const y = 606;
  // "k" rises ≈ 43 units above the baseline at font-size 60.
  const rise = 43 * Math.tan((deg * Math.PI) / 180);
  const edgeAtTop = 596 - ((y - 43 - 527) * 80) / 98;
  const x = Math.min(519, edgeAtTop - 8 - rise);
  return { x, y, skew: -deg };
}

/** The mark's markup in `fg` with its cuts in `bg`. */
export function zMarkMarkup(fg: string, bg: string): string {
  return `<path d="${Z_OUTER}" fill="${fg}" stroke="${fg}" stroke-width="14" stroke-linejoin="round"/><path d="${Z_CUTS}" fill="${bg}" stroke="${bg}" stroke-width="3" stroke-linejoin="round"/>`;
}

/** "eek", bold and tight, reversed out (in `bg`) at the given slant. */
export function eekMarkup(bg: string, slant: EekSlant): string {
  const { x, y, skew } = eekPlacement(slant);
  return `<text transform="translate(${x.toFixed(1)} ${y}) skewX(${skew.toFixed(2)})" text-anchor="end" font-family="Archivo Black, Archivo Variable, Archivo, sans-serif" font-weight="900" font-size="60" letter-spacing="-2" fill="${bg}">eek</text>`;
}
