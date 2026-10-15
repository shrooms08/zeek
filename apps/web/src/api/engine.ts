/** REST client for the engine (game API, campfire, proofs). */

import type {
  Board,
  CampfirePage,
  EngineConfig,
  KeyholdersStatus,
  Ledger,
  NoteDigest,
  PendingRound,
  RoundsPage,
  RoundView,
} from "./types";

/**
 * Engine base URL, absolute and without a trailing slash: VITE_ZEEK_ENGINE_URL
 * (an absolute URL, or a path such as `/api` on this site's own origin, which
 * is how the public deployment reaches the engine through its proxy), by
 * default the local engine.
 */
export const ENGINE_URL: string = new URL(
  (import.meta.env.VITE_ZEEK_ENGINE_URL as string | undefined) ?? "http://127.0.0.1:8787",
  window.location.origin,
).href.replace(/\/+$/, "");

/** The vault this build serves; the Pocket refuses any other. */
export const PINNED_VAULT = __ZEEK_PINNED_VAULT__;

/** Why a request failed, in a form screens can show. */
export class EngineError extends Error {
  readonly kind: "offline" | "not-found" | "server";
  readonly status: number;
  constructor(kind: "offline" | "not-found" | "server", message: string, status = 0) {
    super(message);
    this.name = "EngineError";
    this.kind = kind;
    this.status = status;
  }
}

async function get<T>(path: string): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${ENGINE_URL}${path}`);
  } catch {
    throw new EngineError("offline", "Can’t reach the field.");
  }
  if (response.status === 404) throw new EngineError("not-found", "Not found.", 404);
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = (await response.json()) as { error?: unknown };
      if (typeof body.error === "string") message = body.error;
    } catch {
      // keep the status line
    }
    throw new EngineError("server", message, response.status);
  }
  return (await response.json()) as T;
}

export const engine = {
  board: () => get<Board>("/v1/board"),
  config: () => get<EngineConfig>("/v1/config"),
  pending: () => get<{ rounds: PendingRound[] }>("/v1/rounds/pending").then((r) => r.rounds),
  round: (n: number) => get<RoundView>(`/v1/rounds/${n}`),
  rounds: (before?: number) =>
    get<RoundsPage>(`/v1/rounds?limit=20${before === undefined ? "" : `&before=${before}`}`),
  digests: (n: number) =>
    get<{ round: number; notes: NoteDigest[] }>(`/v1/rounds/${n}/digests`).then((r) => r.notes),
  ledger: () => get<Ledger>("/v1/vault/ledger"),
  keyholders: () => get<KeyholdersStatus>("/v1/keyholders/status"),
  campfire: (includeHidden = false) =>
    get<CampfirePage>(`/v1/campfire?limit=50${includeHidden ? "&include_hidden=1" : ""}`),
};

/** Share page and image URLs for a proof (served by the engine). */
export function shareUrls(proofId: string): { page: string; image: string } {
  return {
    page: `${ENGINE_URL}/share/win/${proofId}`,
    image: `${ENGINE_URL}/share/win/${proofId}.png`,
  };
}
