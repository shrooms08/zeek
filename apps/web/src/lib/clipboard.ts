/**
 * Copying the recovery phrase, and clearing it from the clipboard again
 * after a while (best effort).
 *
 * Browsers differ (MDN, Clipboard API, security considerations):
 * - Chromium reads the clipboard only with the `clipboard-read` permission
 *   and would ask for it with a prompt, so the clear reads only when that
 *   permission is already granted and never asks.
 * - Firefox and Safari have no such permission (querying it rejects); a
 *   read of text this page copied itself needs no prompt there, and any
 *   other read is refused, which is fine.
 * The phrase is only ever held in memory here: never logged, stored or sent.
 */

/** The parts of `navigator.clipboard` used here. */
export interface ClipboardLike {
  writeText(text: string): Promise<void>;
  readText(): Promise<string>;
}

/** What the clear may do in this browser. */
export type ReadAccess = "granted" | "would-prompt" | "no-permission-api";

export interface ClearEnv {
  clipboard: ClipboardLike;
  hasFocus: () => boolean;
  readAccess: () => Promise<ReadAccess>;
}

/** How long a copied phrase may stay on the clipboard. */
export const CLEAR_AFTER_MS = 60_000;

/**
 * Overwrites the clipboard with an empty string if it still holds `phrase`
 * and the page still has focus. Never throws and never prompts.
 * Returns what it did (for tests).
 */
export async function clearIfUnchanged(
  env: ClearEnv,
  phrase: string,
): Promise<"cleared" | "changed" | "skipped"> {
  try {
    if (!env.hasFocus()) return "skipped";
    const access = await env.readAccess();
    if (access === "would-prompt") return "skipped";
    const now = await env.clipboard.readText();
    if (now !== phrase) return "changed";
    await env.clipboard.writeText("");
    return "cleared";
  } catch {
    return "skipped";
  }
}

/** The browser's own environment for `clearIfUnchanged`. */
export function browserClearEnv(): ClearEnv {
  return {
    clipboard: navigator.clipboard,
    hasFocus: () => document.hasFocus(),
    readAccess: async () => {
      try {
        const status = await navigator.permissions.query({
          name: "clipboard-read" as PermissionName,
        });
        return status.state === "granted" ? "granted" : "would-prompt";
      } catch {
        return "no-permission-api";
      }
    },
  };
}
