/**
 * Whether this tab is hidden, and since when (Page Visibility API). Shared
 * by the live stream and polling (paused while hidden) and auto-dig (stops
 * after a whole round hidden).
 */

type Listener = (hidden: boolean) => void;

let since: number | null = null;
const listeners = new Set<Listener>();
let installed = false;

function install(): void {
  if (installed || typeof document === "undefined") return;
  installed = true;
  since = document.visibilityState === "hidden" ? Date.now() : null;
  document.addEventListener("visibilitychange", () => {
    const hidden = document.visibilityState === "hidden";
    since = hidden ? (since ?? Date.now()) : null;
    for (const l of listeners) l(hidden);
  });
}

/** When the tab was hidden (ms since epoch), or null while it is visible. */
export function hiddenSince(): number | null {
  install();
  return since;
}

export function isHidden(): boolean {
  return hiddenSince() !== null;
}

/** Calls `listener` on every change; returns the unsubscribe. */
export function onVisibilityChange(listener: Listener): () => void {
  install();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
