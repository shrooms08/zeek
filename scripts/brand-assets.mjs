/**
 * Brand assets from the Z mark (apps/web/src/art/logoGeometry.ts):
 *   docs/examples/phase7b/logo-options.png  the three "eek" slants, large and at 32 px, dark and light
 *   apps/web/public/favicon.svg, favicon-32.png, apple-touch-icon.png,
 *   icon-192.png, icon-512.png, icon-maskable-512.png   (Z mark alone)
 *   docs/examples/phase7b/x-avatar.png                   (400 × 400, Z mark alone)
 * Run: node scripts/brand-assets.mjs   (Node ≥ 23 runs the .ts module directly)
 */
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "../node_modules/.pnpm/node_modules/playwright/index.mjs";
import {
  DIAGONAL_DEG,
  eekMarkup,
  LOGO_VIEWBOX,
  zMarkMarkup,
} from "../apps/web/src/art/logoGeometry.ts";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const font = path.join(
  root,
  "apps/web/node_modules/@fontsource/archivo-black/files/archivo-black-latin-400-normal.woff2",
);
const fontCss = `@font-face{font-family:"Archivo Black";src:url("data:font/woff2;base64,${readFileSync(font).toString("base64")}") format("woff2");}`;
const NIGHT = "#0D0D0D";
const WHITE = "#FFFFFF";
const lockup = (slant, fg, bg, h) =>
  `<svg viewBox="${LOGO_VIEWBOX}" height="${h}" width="${(h * 483) / 358}">${zMarkMarkup(fg, bg)}${eekMarkup(bg, slant)}</svg>`;
const OPTIONS = [
  ["20", "≈ 20°"],
  ["30", "≈ 30°"],
  ["diagonal", `${DIAGONAL_DEG.toFixed(1)}°, the Z's diagonal (default)`],
];
const cell = (slant, label, dark) => {
  const fg = dark ? WHITE : "#0A0A0A";
  const bg = dark ? NIGHT : "#ECEBE6";
  return `<div style="background:${bg};padding:28px;display:flex;flex-direction:column;gap:18px;align-items:flex-start">
    <div style="font:700 13px/1 system-ui;letter-spacing:.12em;color:${dark ? "#A6A6A6" : "#3A3A3A"};text-transform:uppercase">${label} · ${dark ? "dark" : "light"}</div>
    ${lockup(slant, fg, bg, 220)}
    <div style="display:flex;gap:18px;align-items:center">${lockup(slant, fg, bg, 32)}<span style="font:12px system-ui;color:${dark ? "#A6A6A6" : "#3A3A3A"}">32 px</span></div>
  </div>`;
};
const page = `<!doctype html><html><head><meta charset="utf-8"><style>${fontCss}body{margin:0;background:#000}</style></head><body>
<div id="sheet" style="display:grid;grid-template-columns:repeat(3,1fr);gap:4px;background:#3A3A3A;width:1560px">
${OPTIONS.map(([s, l]) => cell(s, l, true)).join("")}${OPTIONS.map(([s, l]) => cell(s, l, false)).join("")}
</div></body></html>`;

const markSvg = (fg, bg, size, pad, round) => {
  const [x, y, w, h] = LOGO_VIEWBOX.split(" ").map(Number);
  const scale = (size - 2 * pad) / Math.max(w, h);
  const ox = (size - w * scale) / 2 - x * scale;
  const oy = (size - h * scale) / 2 - y * scale;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}" width="${size}" height="${size}">${
    round === "circle"
      ? `<circle cx="${size / 2}" cy="${size / 2}" r="${size / 2}" fill="${bg}"/>`
      : `<rect width="${size}" height="${size}" rx="${round}" fill="${bg}"/>`
  }<g transform="translate(${ox.toFixed(2)} ${oy.toFixed(2)}) scale(${scale.toFixed(5)})">${zMarkMarkup(fg, bg)}</g></svg>`;
};

const b = await chromium.launch();
const p = await b.newPage({ deviceScaleFactor: 1 });
await p.setContent(page);
await p.evaluate(() => document.fonts.ready);
await p
  .locator("#sheet")
  .screenshot({ path: path.join(root, "docs/examples/phase7b/logo-options.png") });

const pub = path.join(root, "apps/web/public");
writeFileSync(path.join(pub, "favicon.svg"), markSvg(WHITE, NIGHT, 64, 4, 12));
const shots = [
  ["favicon-32.png", markSvg(WHITE, NIGHT, 32, 2, 6)],
  ["apple-touch-icon.png", markSvg(WHITE, NIGHT, 180, 26, 0)],
  ["icon-192.png", markSvg(WHITE, NIGHT, 192, 22, 40)],
  ["icon-512.png", markSvg(WHITE, NIGHT, 512, 60, 108)],
  ["icon-maskable-512.png", markSvg(WHITE, NIGHT, 512, 112, 0)],
];
for (const [file, svg] of shots) {
  const size = Number(/width="(\d+)"/.exec(svg)?.[1] ?? 32);
  await p.setViewportSize({ width: size, height: size });
  await p.setContent(`<html><body style="margin:0">${svg}</body></html>`);
  await p.screenshot({
    path: path.join(pub, file),
    omitBackground: true,
    clip: { x: 0, y: 0, width: size, height: size },
  });
}
await p.setViewportSize({ width: 400, height: 400 });
await p.setContent(
  `<html><body style="margin:0">${markSvg(NIGHT, WHITE, 400, 70, "circle")}</body></html>`,
);
await p.screenshot({
  path: path.join(root, "docs/examples/phase7b/x-avatar.png"),
  clip: { x: 0, y: 0, width: 400, height: 400 },
});
await b.close();
console.log("brand assets written");
