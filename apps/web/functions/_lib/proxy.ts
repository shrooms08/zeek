/**
 * The public site's proxy to the services on the operator's Mac.
 *
 * The engine and the second-source relay are reached through Cloudflare
 * quick tunnels, whose hostnames change on every restart. `scripts/zeek-up.sh`
 * stores the current hostnames in Workers KV (`engine`, `relay`), so every
 * public link stays on this site's own origin.
 *
 * The proxy forwards only a fixed list of routes and methods, to the
 * hostname from KV and only if it is a `*.trycloudflare.com` address. It
 * sends the shared secret (`X-Zeek-Proxy`) and the client address Cloudflare
 * reported (`X-Zeek-Client`); the engine's public listener refuses anything
 * without the secret and rate-limits per client. Incoming cookies,
 * authorization and forwarding headers are never passed on.
 *
 * `_headers` does not apply to Function responses, so the isolation headers
 * the app needs (COOP/COEP, CORP) are set here too.
 */

export interface Env {
  /** KV namespace holding the tunnel hostnames. */
  ZEEK: KVNamespace;
  /** Shared secret also stored in `.secrets/proxy-secret` on the Mac. */
  ZEEK_PROXY_SECRET: string;
}

/** A KV entry written by `scripts/zeek-up.sh`. */
interface Upstream {
  url: string;
  updated: number;
}

/** Which service a route belongs to. */
export type Service = "engine" | "relay";

interface Rule {
  method: "GET" | "POST";
  pattern: RegExp;
  /** Largest request body accepted (POST only). */
  maxBody?: number;
  websocket?: boolean;
}

const TXID = "[0-9a-f]{64}";
const ROUND = "[0-9]{1,10}";

/** The engine routes the public site may use. */
const ENGINE_RULES: Rule[] = [
  { method: "GET", pattern: /^\/v1\/(config|board|health|status|campfire|vault\/ledger)$/ },
  { method: "GET", pattern: /^\/v1\/keyholders\/status$/ },
  { method: "GET", pattern: /^\/v1\/rounds(\/pending)?$/ },
  { method: "GET", pattern: new RegExp(`^/v1/rounds/${ROUND}(/proof|/digests)?$`) },
  { method: "GET", pattern: /^\/v1\/proofs\/[0-9a-f]{32}$/ },
  { method: "POST", pattern: /^\/v1\/proofs$/, maxBody: 32 * 1024 },
  { method: "GET", pattern: /^\/v1\/chain\/(tip|blocks|tree-state)$/ },
  { method: "GET", pattern: new RegExp(`^/v1/tx/${TXID}$`) },
  { method: "POST", pattern: /^\/v1\/tx\/broadcast$/, maxBody: 256 * 1024 },
  { method: "GET", pattern: /^\/v1\/relay\/stats$/ },
  { method: "GET", pattern: /^\/v1\/stream$/, websocket: true },
  { method: "GET", pattern: /^\/share\/(round|win)\/[0-9a-z.]{1,48}$/ },
];

/** The second-source relay's routes (`zeek-verify relay`). */
const RELAY_RULES: Rule[] = [
  { method: "GET", pattern: /^\/v1\/chain\/(tip|blocks)$/ },
  { method: "GET", pattern: new RegExp(`^/v1/tx/${TXID}$`) },
];

/** Headers every Function response carries. */
const ISOLATION: Record<string, string> = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
  "Cross-Origin-Resource-Policy": "same-origin",
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "no-referrer",
};

/** Response headers copied from the upstream. */
const PASS_RESPONSE = ["content-type", "cache-control", "location", "last-modified", "etag"];

/** Request headers copied to the upstream. */
const PASS_REQUEST = [
  "accept",
  "content-type",
  "if-none-match",
  "sec-websocket-key",
  "sec-websocket-version",
  "sec-websocket-protocol",
  "sec-websocket-extensions",
];

/** How long one isolate reuses a KV read. */
const KV_REUSE_MS = 10_000;
const remembered = new Map<Service, { at: number; upstream: Upstream | null }>();

export function json(status: number, body: unknown, extra: Record<string, string> = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      ...ISOLATION,
      "Content-Type": "application/json",
      "Cache-Control": "no-store",
      ...extra,
    },
  });
}

/** Only https quick-tunnel hosts are ever contacted. */
export function validUpstream(value: unknown): Upstream | null {
  if (typeof value !== "object" || value === null) return null;
  const { url, updated } = value as { url?: unknown; updated?: unknown };
  if (typeof url !== "string" || typeof updated !== "number") return null;
  if (!/^https:\/\/[a-z0-9-]+\.trycloudflare\.com$/.test(url)) return null;
  return { url, updated };
}

async function upstreamOf(env: Env, service: Service): Promise<Upstream | null> {
  const now = Date.now();
  const hit = remembered.get(service);
  if (hit !== undefined && now - hit.at < KV_REUSE_MS) return hit.upstream;
  const upstream = validUpstream(await env.ZEEK.get(service, "json"));
  remembered.set(service, { at: now, upstream });
  return upstream;
}

/** The rule allowing `method path` for `service`, if any. */
export function allowed(service: Service, method: string, path: string): Rule | undefined {
  const rules = service === "engine" ? ENGINE_RULES : RELAY_RULES;
  return rules.find((r) => r.method === method && r.pattern.test(path));
}

/**
 * Forwards `request` (whose path, after the site prefix, is `path`) to
 * `service`.
 */
export async function forward(
  request: Request,
  env: Env,
  service: Service,
  path: string,
): Promise<Response> {
  const url = new URL(request.url);
  const rule = allowed(service, request.method, path);
  if (rule === undefined) {
    return json(404, { error: "not found" });
  }
  const upstream = await upstreamOf(env, service);
  if (upstream === null) {
    return json(503, {
      error: `The Zeek ${service} is offline right now (no tunnel address is published).`,
      offline: true,
    });
  }

  const headers = new Headers();
  for (const name of PASS_REQUEST) {
    const value = request.headers.get(name);
    if (value !== null) headers.set(name, value);
  }
  const client = request.headers.get("CF-Connecting-IP");
  if (client === null) return json(400, { error: "no client address" });
  headers.set("X-Zeek-Client", client);
  headers.set("X-Zeek-Proxy", env.ZEEK_PROXY_SECRET);

  const target = `${upstream.url}${path}${url.search}`;
  if (rule.websocket === true) {
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return json(426, { error: "this route is a WebSocket" });
    }
    headers.set("Upgrade", "websocket");
    let response: Response;
    try {
      response = await fetch(target, { headers });
    } catch {
      return json(502, { error: `The Zeek ${service} did not answer.`, offline: true });
    }
    if (response.status !== 101 || response.webSocket === null) {
      return json(502, { error: `The Zeek ${service} refused the live connection.` });
    }
    return new Response(null, { status: 101, webSocket: response.webSocket });
  }

  let body: ArrayBuffer | undefined;
  if (rule.method === "POST") {
    const declared = Number(request.headers.get("Content-Length") ?? "0");
    const limit = rule.maxBody ?? 0;
    if (declared > limit) return json(413, { error: "request too large" });
    body = await request.arrayBuffer();
    if (body.byteLength > limit) return json(413, { error: "request too large" });
  }

  let response: Response;
  try {
    response = await fetch(target, {
      method: rule.method,
      headers,
      ...(body === undefined ? {} : { body }),
      redirect: "manual",
    });
  } catch {
    return json(502, { error: `The Zeek ${service} did not answer.`, offline: true });
  }
  const out = new Headers(ISOLATION);
  for (const name of PASS_RESPONSE) {
    const value = response.headers.get(name);
    if (value !== null) out.set(name, value);
  }
  if (!out.has("cache-control")) out.set("Cache-Control", "no-store");
  return new Response(response.body, { status: response.status, headers: out });
}

/** The path below the site prefix, from a `[[path]]` route parameter. */
export function subpath(prefix: string, param: string | string[] | undefined): string {
  const parts = param === undefined ? [] : Array.isArray(param) ? param : [param];
  return `${prefix}/${parts.join("/")}`.replace(/\/+$/, "");
}
