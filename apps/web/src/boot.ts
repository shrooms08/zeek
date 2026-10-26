/**
 * The load screen (#zeek-boot, inline in index.html so it paints before
 * this bundle). Its gold bar shows how far the app is. It never waits on, or
 * blocks, any loading: it only watches.
 *
 * Opened on the field: once the field is ready and the screen has been up
 * at least MIN_MS (or at PROMPT_MS regardless, when the app's own loading
 * states take over behind it), the bar gives way to "Tap to dig in" and the
 * whole screen becomes one button. The tap is the gesture that allows
 * sound: it unlocks it, plays the enter sound, flashes the logo gold and
 * fades the screen out; the music follows the enter sound.
 *
 * Opened anywhere else (a deep link), or when the field falls back to
 * Replay or the offline message: it leaves by itself, silently, and the
 * player's first gesture later starts the music (src/sound/cues.ts).
 *
 * Bar: 30 % when this bundle starts, 60 % once the current round has
 * loaded, 100 % when the field (or another screen) is rendered.
 */

import { sound } from "./sound/sound";

/** Shortest time on screen, from first paint. */
const MIN_MS = 1200;
/** The prompt shows by then, from first paint, even if loading goes on. */
const PROMPT_MS = 4000;
/** Leaving by itself. */
const OUT_MS = 350;
/** Leaving after the tap. */
const TAP_OUT_MS = 400;
/** The gold flash: 120 ms up, 300 ms back (index.html, zb-flash). */
const FLASH_MS = 420;

/** Test-only: end-to-end tests of other screens start without it. */
const SKIP_KEY = "zeek.e2e.skipLoadScreen";

let ready = false;
let silent = false;
let prompting = false;
let leaving = false;
let shown = 0;

function node(): HTMLElement | null {
  return document.getElementById("zeek-boot");
}

/** Milliseconds since the first paint (or since navigation, if unknown). */
function sinceFirstPaint(): number {
  const paint = performance.getEntriesByName("first-paint")[0];
  return performance.now() - (paint?.startTime ?? 0);
}

function reducedMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** Fades the screen out over `ms` (at once under reduced motion), then removes it. */
function leave(ms: number): void {
  const el = node();
  if (leaving || el === null) return;
  leaving = true;
  el.removeEventListener("pointerdown", onTap);
  el.removeEventListener("keydown", onTap);
  const now = reducedMotion();
  el.style.transitionDuration = `${now ? 0 : ms}ms`;
  el.classList.add("zb-out");
  window.setTimeout(
    () => {
      el.remove();
    },
    now ? 0 : ms,
  );
}

/** The bar gives way to "Tap to dig in"; the screen becomes a button. */
function prompt(): void {
  const el = node();
  if (prompting || leaving || el === null) return;
  prompting = true;
  el.setAttribute("role", "button");
  el.setAttribute("tabindex", "0");
  el.setAttribute("aria-label", "Tap to start Zeek");
  el.classList.add("zb-tap");
  el.addEventListener("pointerdown", onTap);
  el.addEventListener("keydown", onTap);
  el.focus({ preventScroll: true });
}

function onTap(e: Event): void {
  if (e instanceof KeyboardEvent && e.repeat) return;
  e.preventDefault();
  const el = node();
  if (el === null || leaving) return;
  sound.unlock();
  sound.playIntro("enter");
  if (reducedMotion()) {
    leave(0);
    return;
  }
  el.classList.add("zb-flash");
  // Nothing else may tap it while it flashes.
  el.removeEventListener("pointerdown", onTap);
  el.removeEventListener("keydown", onTap);
  window.setTimeout(() => {
    leave(TAP_OUT_MS);
  }, FLASH_MS);
}

/** Once ready and up long enough: the prompt, or (silent) leaving. */
function settle(): void {
  if (!ready || leaving) return;
  const wait = MIN_MS - sinceFirstPaint();
  if (wait > 0) {
    window.setTimeout(settle, wait);
    return;
  }
  if (silent) leave(OUT_MS);
  else prompt();
}

/**
 * True while the load screen holds the first gesture: it is up and not
 * leaving by itself, so its tap (not just any gesture) starts the sound.
 */
export function bootHoldsSound(): boolean {
  return node() !== null && !silent;
}

/** Moves the bar forward to `percent` (never back). */
export function bootProgress(percent: number): void {
  if (percent <= shown) return;
  shown = percent;
  const bar = node()?.querySelector<HTMLElement>(".zb-bar");
  if (bar) bar.style.width = `${percent}%`;
}

/**
 * The first screen is rendered: the bar fills, then the prompt shows. With
 * `silent` (a deep link, Replay, the offline message) it leaves instead,
 * without a tap or a sound, even if the prompt is already showing.
 */
export function bootReady(options: { silent?: boolean } = {}): void {
  bootProgress(100);
  ready = true;
  if (options.silent === true) silent = true;
  settle();
}

/** Called once as the bundle starts. */
export function startBoot(): void {
  let skip = false;
  try {
    skip = localStorage.getItem(SKIP_KEY) === "1";
  } catch {
    // no storage: show it
  }
  if (skip) {
    silent = true;
    node()?.remove();
    return;
  }
  bootProgress(30);
  window.setTimeout(
    () => {
      if (!silent) prompt();
    },
    Math.max(0, PROMPT_MS - sinceFirstPaint()),
  );
}
