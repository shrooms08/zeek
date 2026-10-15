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
  ServiceStatus,
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

/**
 * The second-source relay from the build (VITE_ZEEK_SECOND_SOURCE: an
 * absolute URL or a path on this site, such as `/relay`), or "".
 */
const SECOND_SETTING = (import.meta.env.VITE_ZEEK_SECOND_SOURCE as string | undefined) ?? "";
export const SECOND_SOURCE: string =
  SECOND_SETTING === ""
    ? ""
    : new URL(SECOND_SETTING, window.location.origin).href.replace(/\/+$/, "");

/** The vault this build serves; the Pocket refuses any other. */
export const PINNED_VAULT = __ZEEK_PINNED_VAULT__;

/**
 * Why a request failed, in a form screens can show:
 * - offline: the engine (or its tunnel) does not answer;
 * - busy: the public site's free daily allowance of proxied requests is
 *   used up (Cloudflare then serves the app's own page, or its error page,
 *   instead of the engine's JSON);
 * - not-found, server: the engine answered with an error.
 */
export type EngineErrorKind = "offline" | "busy" | "not-found" | "server";

export class EngineError extends Error {
  readonly kind: EngineErrorKind;
  readonly status: number;
  constructor(kind: EngineErrorKind, message: string, status = 0) {
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
  const json = (response.headers.get("content-type") ?? "").includes("application/json");
  if (!json) {
    throw new EngineError("busy", "Zeek is very busy, try again shortly.", response.status);
  }
  if (response.status === 404) throw new EngineError("not-found", "Not found.", 404);
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    let offline = false;
    try {
      const body = (await response.json()) as { error?: unknown; offline?: unknown };
      if (typeof body.error === "string") message = body.error;
      offline = body.offline === true;
    } catch {
      // keep the status line
    }
    throw new EngineError(
      offline || response.status === 502 || response.status === 503 ? "offline" : "server",
      message,
      response.status,
    );
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
  status: () => get<ServiceStatus>("/v1/status"),
  health: () => get<{ ok: boolean }>("/v1/health"),
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
