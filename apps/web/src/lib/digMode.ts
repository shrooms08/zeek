/** The dig panel's mode, remembered in this browser (Lite by default). */

export type DigMode = "lite" | "pro";

const KEY = "zeek.digMode";

export function loadDigMode(): DigMode {
  try {
    return localStorage.getItem(KEY) === "pro" ? "pro" : "lite";
  } catch {
    return "lite";
  }
}

export function saveDigMode(mode: DigMode): void {
  try {
    localStorage.setItem(KEY, mode);
  } catch {
    // Storage unavailable (private window): the mode lasts for this visit.
  }
}
