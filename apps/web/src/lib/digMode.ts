/**
 * The dig panel's mode, remembered in this browser (Lite by default).
 *
 * Lite digs one tile: tapping another tile moves the pick, Auto-pick picks
 * one, and "Dig separately" is hidden (one tile is one payment). Pro picks
 * many (ALL, Clear, quick amounts, MAX, the exact total, Dig separately,
 * auto-dig, Your digs).
 */

export type DigMode = "lite" | "pro";

/** One line under the mode switch. */
export const MODE_LINE: Record<DigMode, string> = {
  lite: "One tile, quick dig.",
  pro: "Many tiles, auto-dig, your history.",
};

/** Shown after switching to Lite dropped extra tiles. */
export const LITE_TRIMMED = "Lite digs one tile. Switch to Pro for more.";

/** The picked tiles after tapping `tile`. */
export function tapTile(mode: DigMode, tiles: readonly number[], tile: number): number[] {
  if (tiles.includes(tile)) return tiles.filter((t) => t !== tile);
  return mode === "lite" ? [tile] : [...tiles, tile];
}

/** The picked tiles Lite keeps (the first one), and whether any were dropped. */
export function liteTiles(tiles: readonly number[]): { tiles: number[]; trimmed: boolean } {
  return { tiles: tiles.slice(0, 1), trimmed: tiles.length > 1 };
}

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
