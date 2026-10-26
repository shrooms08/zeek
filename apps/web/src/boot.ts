/**
 * The load screen (#zeek-boot, inline in index.html so it paints before
 * this bundle): its gold bar shows how far the app is, and it leaves once
 * the field is ready and it has been up at least MIN_MS, or at CAP_MS
 * regardless, so the app's own loading and reconnecting states take over.
 * It never waits on, or blocks, any loading: it only watches.
 *
 * Bar: 30 % when this bundle starts, 60 % once the current round has
 * loaded, 100 % when the field (or, opened directly, another screen) is
 * rendered.
 */

/** Shortest time on screen, from first paint. */
const MIN_MS = 1200;
/** Longest, from first paint: it leaves even if loading goes on. */
const CAP_MS = 4000;
const OUT_MS = 350;
import { sound } from "./sound/sound";

/** Test-only: end-to-end tests of other screens start without it. */
const SKIP_KEY = "zeek.e2e.skipLoadScreen";

let ready = false;
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

function leave(): void {
  const el = node();
  if (leaving || el === null) return;
  leaving = true;
  // An intro the browser has not started by now never plays.
  sound.cancelAutoplay();
  el.classList.add("zb-out");
  window.setTimeout(
    () => {
      el.remove();
    },
    reducedMotion() ? 0 : OUT_MS,
  );
}

function maybeLeave(): void {
  if (!ready || leaving) return;
  const wait = MIN_MS - sinceFirstPaint();
  if (wait <= 0) leave();
  else window.setTimeout(maybeLeave, wait);
}

/** Moves the bar forward to `percent` (never back). */
export function bootProgress(percent: number): void {
  if (percent <= shown) return;
  shown = percent;
  const bar = node()?.querySelector<HTMLElement>(".zb-bar");
  if (bar) bar.style.width = `${percent}%`;
}

/** The first screen is rendered: the bar fills and the screen may leave. */
export function bootReady(): void {
  bootProgress(100);
  ready = true;
  maybeLeave();
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
    node()?.remove();
    return;
  }
  bootProgress(30);
  // The intro with the logo, if the browser allows sound before a gesture.
  sound.tryAutoplay("enter");
  window.setTimeout(leave, Math.max(0, CAP_MS - sinceFirstPaint()));
}
