/**
 * World props, chest, mine, Zeb and the campfire (ported from zeek-art.js).
 *
 * Changes from the design file: every prop's ground shadow is a 2:1
 * ellipse (a round footprint seen isometrically) instead of a flat
 * 3–6:1 smear; lamp light pools stay round. A cart on rails is new (the
 * design only had a cart beside the stage-4 mine).
 */

import { f, GOLD, GROUND, INK, PAPER, pts, rng, ST, WHITE } from "./ink";

/** Ground shadow of a round footprint of radius r, seen at the 2:1 iso ratio. */
const footShadow = (r: number, cy = 1, opacity = 0.35): string =>
  `<ellipse cx="0" cy="${cy}" rx="${r}" ry="${r / 2}" fill="#000" opacity="${opacity}"/>`;

export type LampPart = "all" | "glow" | "body";

export function lamp(x: number, y: number, p: string, s = 1, part: LampPart = "all"): string {
  const tr = `transform="translate(${f(x)} ${f(y)}) scale(${s})"`;
  const dur = f(3 + (Math.abs(x * 7 + y * 3) % 30) / 10);
  const delay = f((Math.abs(x + y) % 40) / 10);
  const glow = `<g ${tr}><g class="zk-flicker" style="animation-duration:${dur}s;animation-delay:-${delay}s"><ellipse cx="0" cy="2" rx="46" ry="17" fill="url(#${p}gw)" opacity=".55"/><circle cx="0" cy="-58" r="52" fill="url(#${p}gw)" opacity=".55"/></g></g>`;
  const body = `<g ${tr}><line x1="0" y1="2" x2="0" y2="-50" stroke="${INK}" stroke-width="4"/><line x1="0" y1="2" x2="0" y2="-50" stroke="#6B6B6B" stroke-width="1.2"/><rect x="-6" y="-66" width="12" height="15" fill="${WHITE}" stroke="${INK}" stroke-width="2.4"/><path d="M-8 -66 L0 -73 L8 -66 Z" fill="${INK}"/><rect x="-5" y="2" width="10" height="3" fill="${INK}"/></g>`;
  return part === "glow" ? glow : part === "body" ? body : glow + body;
}

export function pine(x: number, y: number, s = 1): string {
  return `<g transform="translate(${f(x)} ${f(y)}) scale(${s})">${footShadow(15)}<rect x="-2.5" y="-9" width="5" height="11" fill="${INK}"/><path d="M0 -64 L12 -40 L6 -40 L17 -21 L9 -21 L21 -5 L-21 -5 L-9 -21 L-17 -21 L-6 -40 L-12 -40 Z" fill="#151515" stroke="${INK}" stroke-width="2" stroke-linejoin="round"/><path d="M-2 -57 L-8 -43 M-5 -36 L-12 -24 M-8 -17 L-15 -8" stroke="#8C8C8C" stroke-width="1.6" stroke-linecap="round"/><path d="M0 -64 L5 -54 L-2 -52 Z" fill="${PAPER}"/></g>`;
}

export function rock(x: number, y: number, s = 1): string {
  return `<g transform="translate(${f(x)} ${f(y)}) scale(${s})">${footShadow(19)}<path d="M-18 0 L-14 -14 L-4 -22 L10 -19 L18 -8 L17 0 Z" fill="#9A9A9A" stroke="${INK}" stroke-width="2.2" stroke-linejoin="round"/><path d="M-4 -22 L0 -10 L18 -8 M0 -10 L-14 -14" fill="none" stroke="${INK}" stroke-width="1.4"/><path d="M3 -9 L15 -7 M2 -5 L16 -3 M5 -13 L13 -11" stroke="${INK}" stroke-width="1"/></g>`;
}

export function tuft(x: number, y: number, c = "#5E5E5E"): string {
  return `<path d="M${f(x - 4)} ${f(y)} l2 -5 l2 5 l2 -6 l2 6" fill="none" stroke="${c}" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>`;
}

export function flower(x: number, y: number): string {
  return `<path d="M${f(x - 2.5)} ${f(y - 2.5)} l5 5 M${f(x + 2.5)} ${f(y - 2.5)} l-5 5" stroke="#D6D6D6" stroke-width="1.6" stroke-linecap="round"/>`;
}

/** Rails along the iso x-axis (slope 1/2), `len` long, from the origin. */
export function rails(len: number): string {
  const dx = len;
  const dy = len / 2;
  let s = `<path d="M0 -5 L${f(dx)} ${f(dy - 5)} M0 5 L${f(dx)} ${f(dy + 5)}" stroke="${INK}" stroke-width="2.4" stroke-linecap="round"/>`;
  for (let k = 0; k <= 6; k++) {
    const x = (dx * k) / 6;
    const y = (dy * k) / 6;
    s += `<path d="M${f(x - 3)} ${f(y - 7)} L${f(x + 3)} ${f(y + 7)}" stroke="#6B6B6B" stroke-width="2.6" stroke-linecap="round"/>`;
  }
  return s;
}

/** A mine cart, drawn at its own origin (the ground point between its wheels). */
export function cart(): string {
  return `${footShadow(16, 2, 0.4)}<path d="M-15 -17 L15 -17 L11 -3 L-11 -3 Z" fill="${PAPER}" stroke="${INK}" stroke-width="2.2" stroke-linejoin="round"/><path d="M-12 -12 H12 M-11 -8 H11" stroke="${INK}" stroke-width="1"/><path d="M-11 -17 Q0 -26 11 -17" fill="#6B6B6B" stroke="${INK}" stroke-width="1.6"/><circle cx="-7" cy="0" r="3.6" fill="${INK}"/><circle cx="7" cy="0" r="3.6" fill="${INK}"/>`;
}

export function chest(open: boolean, gold: boolean): string {
  const g = gold ? GOLD : WHITE;
  let s = "";
  if (open) {
    s += `<g data-glint="" style="transform-origin:60px 40px"><path d="M30 30 L22 -6 M60 26 V-12 M90 30 L98 -6" stroke="${INK}" stroke-width="7" stroke-linecap="round"/><path d="M30 30 L22 -6 M60 26 V-12 M90 30 L98 -6" stroke="${g}" stroke-width="3.5" stroke-linecap="round"/></g>`;
    s += `<path d="M16 46 L24 12 Q60 2 96 12 L104 46 Z" fill="${PAPER}" ${ST}/><path d="M30 16 L26 40 M60 9 V40 M90 16 L94 40" stroke="${INK}" stroke-width="1.5"/>`;
    s += `<path d="M18 46 Q60 32 102 46 Z" fill="${INK}"/>`;
    s += '<g data-glint="" style="transform-origin:60px 40px">';
    for (const [x, y] of [
      [44, 40],
      [60, 36],
      [76, 40],
      [52, 30],
      [68, 31],
    ] as const) {
      s += `<circle cx="${x}" cy="${y}" r="7" fill="${g}" stroke="${INK}" stroke-width="2.4"/>`;
    }
    s += "</g>";
  } else {
    s += `<path d="M14 48 Q14 20 60 20 Q106 20 106 48 Z" fill="${PAPER}" ${ST}/><path d="M30 46 Q30 26 34 23 M86 46 Q90 26 86 23" stroke="${INK}" stroke-width="7" fill="none"/>`;
  }
  s += `<rect x="14" y="46" width="92" height="46" rx="3" fill="${PAPER}" ${ST}/><path d="M16 62 H104 M16 77 H104" stroke="${INK}" stroke-width="1.4"/><rect x="26" y="46" width="8" height="46" fill="${INK}"/><rect x="86" y="46" width="8" height="46" fill="${INK}"/><rect x="51" y="50" width="18" height="20" rx="2" fill="${gold ? GOLD : INK}" stroke="${INK}" stroke-width="2.5"/><circle cx="60" cy="57" r="2.6" fill="${gold ? INK : WHITE}"/><rect x="58.8" y="58" width="2.4" height="7" fill="${gold ? INK : WHITE}"/>`;
  return s;
}

/** Mine entrance, stage 1–5 (Deep Cache size), optionally erupting. */
export function mine(stage: number, erupt: boolean, gold: boolean, p: string): string {
  let s = `<ellipse cx="0" cy="-2" rx="62" ry="24" fill="#000" opacity=".4"/>`;
  const lc = gold ? GOLD : WHITE;
  if (erupt) {
    s += `<circle data-mglow="" class="zk-mglow" cx="0" cy="-30" r="150" fill="url(#${p}${gold ? "gg" : "gw"})"/><g data-mrays="" class="zk-mrays" style="transform-origin:0px -20px">`;
    for (let k = 0; k < 13; k++) {
      const a = ((-172 + k * 13) * Math.PI) / 180;
      const L = k % 2 ? 120 : 165;
      const w = k % 2 ? 0.05 : 0.075;
      s += `<polygon points="${pts([
        [0, -20],
        [Math.cos(a - w) * L, -20 + Math.sin(a - w) * L],
        [Math.cos(a + w) * L, -20 + Math.sin(a + w) * L],
      ])}" fill="${lc}" stroke="${INK}" stroke-width="1.6" stroke-linejoin="round"/>`;
    }
    s += "</g>";
  }
  s += erupt ? '<g data-mound="" class="zk-mound">' : '<g data-mound="">';
  s += `<path d="M-58 2 Q-62 -36 -36 -58 Q-8 -80 22 -68 Q54 -54 60 2 Z" fill="#8E8E8E" stroke="${INK}" stroke-width="2.6" stroke-linejoin="round"/>`;
  s += `<path d="M24 -64 Q52 -50 57 0 L36 0 Q36 -36 20 -62Z" fill="url(#${p}h3)"/><path d="M-42 -30 l8 4 l-2 8 M-22 -62 l6 8 l-4 6 M42 -40 l-6 6" fill="none" stroke="${INK}" stroke-width="1.6"/>`;
  const k = stage <= 1 ? 0.72 : 1;
  s += `<path d="M${f(-18 * k)} 2 L${f(-18 * k)} ${f(-26 * k)} Q0 ${f(-44 * k)} ${f(18 * k)} ${f(-26 * k)} L${f(18 * k)} 2Z" fill="${erupt ? lc : INK}" stroke="${INK}" stroke-width="2.6"/>`;
  if (stage >= 2 && stage < 5) {
    s += `<rect x="-25" y="-35" width="7" height="37" fill="${PAPER}" stroke="${INK}" stroke-width="2.2"/><rect x="18" y="-35" width="7" height="37" fill="${PAPER}" stroke="${INK}" stroke-width="2.2"/><rect x="-30" y="-42" width="60" height="8" fill="${PAPER}" stroke="${INK}" stroke-width="2.2"/><path d="M-26 -38 H-6 M4 -38 H26 M-21.5 -28 V-12 M21.5 -26 V-6" stroke="${INK}" stroke-width="1"/>`;
  }
  if (stage >= 5) {
    s += `<path d="M-31 2 L-31 -30 Q0 -62 31 -30 L31 2 L22 2 L22 -27 Q0 -50 -22 -27 L-22 2Z" fill="#CFCFCA" stroke="${INK}" stroke-width="2.4" stroke-linejoin="round"/><path d="M-31 -14 H-22 M22 -14 H31 M-30 -32 L-21 -28 M30 -32 L21 -28 M-16 -48 L-12 -40 M16 -48 L12 -40" stroke="${INK}" stroke-width="1.6"/><path d="M-5 -55 L5 -55 L4 -45 L-4 -45 Z" fill="${PAPER}" stroke="${INK}" stroke-width="2"/>`;
    s += `<rect x="-24" y="-82" width="48" height="13" fill="${PAPER}" stroke="${INK}" stroke-width="2.2"/><line x1="-14" y1="-69" x2="-14" y2="-62" stroke="${INK}" stroke-width="2"/><line x1="14" y1="-69" x2="14" y2="-62" stroke="${INK}" stroke-width="2"/><text x="0" y="-72" text-anchor="middle" font-family="Archivo Variable,Archivo,sans-serif" font-weight="900" font-size="9" letter-spacing="1.5" fill="${INK}">DEEP</text>`;
  }
  if (stage >= 3 && !erupt) {
    s += `<circle cx="0" cy="-22" r="22" fill="url(#${p}gw)" opacity=".8"/><line x1="0" y1="${stage >= 5 ? -45 : -34}" x2="0" y2="-28" stroke="${INK}" stroke-width="1.6"/><rect x="-4" y="-28" width="8" height="10" fill="${WHITE}" stroke="${INK}" stroke-width="2"/>`;
  }
  if (stage >= 4) {
    s += `<path d="M-9 2 L-22 30 M9 2 L22 30" stroke="${INK}" stroke-width="2.4"/><path d="M-12 9 H12 M-15 16 H15 M-18 23 H18" stroke="${INK}" stroke-width="2"/>`;
  }
  if (stage >= 5 && !erupt) s += lamp(-48, 2, p, 0.6) + lamp(48, 2, p, 0.6);
  s += "</g>";
  if (erupt) {
    const rr = rng(9);
    for (let i = 0; i < 10; i++) {
      const x = (rr() - 0.5) * 170;
      const y = -40 - rr() * 90;
      const z = 4 + rr() * 4;
      s += `<g data-gem="${f(x)},${f(y)}" class="zk-gem" style="--gx:${f(-x)}px;--gy:${f(-20 - y)}px;animation-delay:${700 + i * 60}ms"><path d="M${f(x)} ${f(y - z * 1.3)} L${f(x + z)} ${f(y)} L${f(x)} ${f(y + z * 1.3)} L${f(x - z)} ${f(y)}Z" fill="${lc}" stroke="${INK}" stroke-width="1.8" stroke-linejoin="round"/><path d="M${f(x - z)} ${f(y)} H${f(x + z)}" stroke="${INK}" stroke-width="1"/></g>`;
    }
    for (let i = 0; i < 8; i++) {
      const x = (rr() - 0.5) * 200;
      const y = -20 - rr() * 110;
      s += `<path d="M${f(x)} ${f(y - 5)} V${f(y + 5)} M${f(x - 5)} ${f(y)} H${f(x + 5)}" stroke="${WHITE}" stroke-width="2"/>`;
    }
  }
  return s;
}

/* ---------- Zeb ---------- */

export type ZebPose = "idle" | "cheer" | "shrug" | "peek" | "chest" | "campfire";

function lantern(x: number, y: number, p: string, glow = true): string {
  return `<g class="zk-lantern" style="transform-origin:${x}px ${y}px">${glow ? `<circle cx="${x}" cy="${y + 15}" r="28" fill="url(#${p}gw)" opacity=".75"/>` : ""}<path d="M${x - 6} ${y + 5} Q${x} ${y - 4} ${x + 6} ${y + 5}" fill="none" stroke="${PAPER}" stroke-width="4.5"/><path d="M${x - 6} ${y + 5} Q${x} ${y - 4} ${x + 6} ${y + 5}" fill="none" stroke="${INK}" stroke-width="2.2"/><rect x="${x - 7}" y="${y + 4}" width="14" height="4" fill="${INK}" stroke="${PAPER}" stroke-width="1.4"/><rect x="${x - 6}" y="${y + 8}" width="12" height="13" fill="${WHITE}" stroke="${INK}" stroke-width="2.4"/><path d="M${x} ${y + 19} Q${x - 3} ${y + 15} ${x} ${y + 11} Q${x + 3} ${y + 15} ${x} ${y + 19}Z" fill="${INK}"/><rect x="${x - 7}" y="${y + 21}" width="14" height="4" fill="${INK}" stroke="${PAPER}" stroke-width="1.4"/></g>`;
}

function arm(d: string, hx: number, hy: number): string {
  return `<path d="${d}" fill="none" stroke="${INK}" stroke-width="10" stroke-linecap="round"/><path d="${d}" fill="none" stroke="${PAPER}" stroke-width="4.5" stroke-linecap="round"/><circle cx="${hx}" cy="${hy}" r="4.6" fill="${INK}" stroke="${PAPER}" stroke-width="1.5"/>`;
}

function zebHead(pose: ZebPose, p: string): string {
  let s = "";
  s += `<path d="M84 30 L98 24 L93 32 L102 33 L93 39 L99 44 L86 44 Z" fill="${INK}" stroke="${PAPER}" stroke-width="2" stroke-linejoin="round"/>`;
  s += `<path d="M38 43 L31 23 L49 36 Z" fill="${PAPER}" ${ST}/><path d="M82 43 L89 23 L71 36 Z" fill="${PAPER}" ${ST}/><path d="M32.5 27 L31 23 L36 26Z M87.5 27 L89 23 L84 26Z" fill="${INK}"/>`;
  s += `<ellipse cx="60" cy="57" rx="26" ry="22" fill="${PAPER}" ${ST}/>`;
  s += `<path d="M34.5 51 L45 50.5 L36 56.5 Z M85.5 51 L75 50.5 L84 56.5Z M36.5 63 L45 63 L39 67.5Z M83.5 63 L75 63 L81 67.5Z" fill="${INK}"/><path d="M53 41 L60 46.5 L67 41" fill="none" stroke="${INK}" stroke-width="3"/>`;
  const ex = pose === "peek" ? 3 : 0;
  s += '<g class="zk-eyes" style="transform-origin:60px 55px">';
  if (pose === "cheer") {
    s += `<path d="M43 57 Q48 50 53 57 M67 57 Q72 50 77 57" fill="none" stroke="${INK}" stroke-width="3.2" stroke-linecap="round"/>`;
  } else {
    s += `<circle cx="${48 + ex}" cy="55" r="4.6" fill="${INK}"/><circle cx="${72 + ex}" cy="55" r="4.6" fill="${INK}"/><circle cx="${49.6 + ex}" cy="53.4" r="1.6" fill="${WHITE}"/><circle cx="${73.6 + ex}" cy="53.4" r="1.6" fill="${WHITE}"/>`;
    if (pose === "shrug")
      s += `<path d="M42 50.5 H54 M66 50.5 H78" stroke="${INK}" stroke-width="3" stroke-linecap="round"/><path d="M41 46 Q47 43 53 46 M67 46 Q73 43 79 46" fill="none" stroke="${INK}" stroke-width="2" stroke-linecap="round"/>`;
  }
  s += "</g>";
  s += `<ellipse cx="60" cy="71" rx="15" ry="10" fill="${INK}"/><ellipse cx="54" cy="68.5" rx="2.1" ry="1.4" fill="${WHITE}"/><ellipse cx="66" cy="68.5" rx="2.1" ry="1.4" fill="${WHITE}"/>`;
  if (pose === "cheer") s += `<path d="M53 74 Q60 83 67 74 Z" fill="${WHITE}"/>`;
  else if (pose === "shrug")
    s += `<path d="M55 76.5 H65" stroke="${WHITE}" stroke-width="1.8" stroke-linecap="round"/>`;
  else
    s += `<path d="M54 75 Q60 78.5 66 75" fill="none" stroke="${WHITE}" stroke-width="1.8" stroke-linecap="round"/>`;
  s += `<circle cx="60" cy="29" r="20" fill="url(#${p}gw)" opacity=".7"/>`;
  s += `<path d="M32 42 Q32 14 60 14 Q88 14 88 42 Z" fill="${INK}" stroke="${PAPER}" stroke-width="2.5" stroke-linejoin="round"/><path d="M60 16 V36" stroke="#3A3A3A" stroke-width="3"/>`;
  s += `<path d="M25 43 H95" stroke="${PAPER}" stroke-width="10" stroke-linecap="round"/><path d="M25 43 H95" stroke="${INK}" stroke-width="5.5" stroke-linecap="round"/><circle cx="60" cy="29" r="7" fill="${WHITE}" stroke="${INK}" stroke-width="2.6"/>`;
  return s;
}

/** Zeb the zebra miner (the guide, never a player), in a 120 × 140 box. */
export function zeb(pose: ZebPose, p: string, gold: boolean): string {
  let s = "";
  if (pose === "peek") {
    s += `<g transform="translate(0 30)"><g class="zk-head" style="transform-origin:60px 80px">${zebHead("peek", p)}</g></g>`;
    s += `<path d="M0 104 Q30 96 60 100 Q90 96 120 104 V140 H0Z" fill="${GROUND}" stroke="${PAPER}" stroke-width="2.5"/>${tuft(20, 112, "#6B6B6B")}${tuft(96, 116, "#6B6B6B")}`;
    s += `<circle cx="40" cy="100" r="5" fill="${INK}" stroke="${PAPER}" stroke-width="1.6"/><circle cx="80" cy="100" r="5" fill="${INK}" stroke="${PAPER}" stroke-width="1.6"/>`;
    return `<g class="zk-pose-peek">${s}</g>`;
  }
  const sit = pose === "campfire";
  if (sit) {
    s += `<rect x="40" y="114" width="15" height="10" rx="3" fill="${PAPER}" ${ST}/><rect x="65" y="114" width="15" height="10" rx="3" fill="${PAPER}" ${ST}/><rect x="40" y="118" width="6" height="6" fill="${INK}"/><rect x="74" y="118" width="6" height="6" fill="${INK}"/>`;
  } else {
    s += `<rect x="46" y="108" width="9" height="22" rx="2" fill="${PAPER}" ${ST}/><rect x="65" y="108" width="9" height="22" rx="2" fill="${PAPER}" ${ST}/><rect x="46" y="124" width="9" height="6" fill="${INK}"/><rect x="65" y="124" width="9" height="6" fill="${INK}"/>`;
  }
  const by = sit ? 6 : 0;
  s += `<g transform="translate(0 ${by})"><path d="M41 80 Q38 112 47 116 L73 116 Q82 112 79 80 Z" fill="${PAPER}" ${ST}/><path d="M40 92 L52 90 L41 97Z M80 92 L68 90 L79 97Z M42 104 L54 103 L44 109Z M78 104 L66 103 L76 109Z" fill="${INK}"/></g>`;
  if (pose === "idle") {
    s +=
      arm("M44 86 Q34 96 37 107", 37, 107) +
      lantern(90, 99, p) +
      arm("M76 86 Q88 93 90 101", 90, 101);
  }
  if (pose === "cheer") {
    s +=
      lantern(94, 56, p) + arm("M44 86 Q30 76 27 60", 27, 60) + arm("M76 86 Q90 76 94 58", 94, 58);
    s += `<path d="M14 40 L6 34 M18 28 L14 18 M106 30 L114 22 M110 42 L118 40" stroke="${PAPER}" stroke-width="3" stroke-linecap="round"/>`;
  }
  if (pose === "shrug") {
    s +=
      lantern(104, 110, p, false) +
      '<g class="zk-arms">' +
      arm("M44 88 Q28 92 25 79", 25, 79) +
      arm("M76 88 Q92 92 95 79", 95, 79) +
      "</g>";
  }
  if (pose === "chest") {
    s +=
      `<circle cx="60" cy="112" r="44" fill="url(#${p}${gold ? "gg" : "gw"})" opacity=".8"/><g transform="translate(28 92) scale(.53)">${chest(true, gold)}</g>` +
      arm("M44 88 Q38 100 33 112", 33, 112) +
      arm("M76 88 Q82 100 87 112", 87, 112);
  }
  if (sit) s += arm("M44 92 Q38 104 46 112", 46, 112) + arm("M76 92 Q82 104 74 112", 74, 112);
  s += `<g transform="translate(0 ${by})"><g class="zk-head" style="transform-origin:60px 80px">${zebHead(pose, p)}</g></g>`;
  return `<g class="zk-pose-${pose}"><g class="zk-body">${s}</g></g>`;
}

/* ---------- campfire ---------- */

function flame(cx: number, cy: number, v: 0 | 1 | 2): string {
  const tx = [-8, -13, -3][v] ?? 0;
  const ty = [-72, -80, -66][v] ?? 0;
  const sx = [6, 10, 2][v] ?? 0;
  const sy = [-62, -58, -66][v] ?? 0;
  const ix = [-2, -6, 2][v] ?? 0;
  const iy = [-42, -46, -38][v] ?? 0;
  return `<g><path d="M${cx} ${cy - 6} C${cx - 30} ${cy - 14} ${cx - 24} ${cy - 50} ${cx + tx} ${cy + ty} C${cx - 6} ${cy - 56} ${cx + 4} ${cy - 52} ${cx + sx} ${cy + sy} C${cx + 22} ${cy - 44} ${cx + 30} ${cy - 16} ${cx} ${cy - 6}Z" fill="${WHITE}" stroke="${INK}" stroke-width="3" stroke-linejoin="round"/><path d="M${cx} ${cy - 10} C${cx - 12} ${cy - 14} ${cx - 10} ${cy - 30} ${cx + ix} ${cy + iy} C${cx + 2} ${cy - 32} ${cx + 12} ${cy - 26} ${cx} ${cy - 10}Z" fill="${INK}"/></g>`;
}

/** Stones and crossed logs (boiled separately). */
export function fireBase(cx: number, cy: number): string {
  let b = "";
  for (const [x, y] of [
    [-34, 0],
    [-22, 8],
    [0, 11],
    [22, 8],
    [34, 0],
    [26, -7],
    [-26, -7],
  ] as const) {
    b += `<ellipse cx="${cx + x}" cy="${cy + y}" rx="9" ry="6" fill="#8E8E8E" stroke="${INK}" stroke-width="2"/>`;
  }
  b += `<g transform="rotate(-16 ${cx} ${cy})"><rect x="${cx - 32}" y="${cy - 8}" width="64" height="12" rx="6" fill="${PAPER}" stroke="${INK}" stroke-width="2.4"/><path d="M${cx - 24} ${cy - 3} H${cx + 10} M${cx - 14} ${cy + 1} H${cx + 20}" stroke="${INK}" stroke-width="1.2"/></g><g transform="rotate(16 ${cx} ${cy})"><rect x="${cx - 32}" y="${cy - 8}" width="64" height="12" rx="6" fill="${PAPER}" stroke="${INK}" stroke-width="2.4"/><circle cx="${cx + 26}" cy="${cy - 2}" r="3" fill="none" stroke="${INK}" stroke-width="1.4"/></g>`;
  return b;
}

export function fireGlow(cx: number, cy: number, p: string): string {
  return `<g class="zk-flicker" style="animation-duration:1.7s"><ellipse cx="${cx}" cy="${cy}" rx="120" ry="34" fill="url(#${p}gw)" opacity=".55"/><circle cx="${cx}" cy="${cy - 40}" r="90" fill="url(#${p}gw)" opacity=".55"/></g>`;
}

/** Three flame frames (cycled at 120 ms) and rising sparks. */
export function fireFlames(cx: number, cy: number): string {
  let s = `<g class="zk-frames">${([0, 1, 2] as const).map((v) => flame(cx, cy, v)).join("")}</g>`;
  for (const [x, y, dl] of [
    [20, -84, 0],
    [-26, -96, 0.8],
    [4, -108, 1.6],
  ] as const) {
    s += `<g class="zk-spark" style="animation-delay:-${dl}s"><path d="M${cx + x} ${cy + y - 3} v6 M${cx + x - 3} ${cy + y} h6" stroke="${WHITE}" stroke-width="2"/></g>`;
  }
  return s;
}
