/**
 * Fetching a beacon from drand's own public HTTP relays (docs.drand.love,
 * HTTP API v1: `GET /{chain-hash}/public/{round}`), for Replay mode's
 * offline check. The relays send `Access-Control-Allow-Origin: *`; they are
 * tried in order until one answers.
 */

/** drand's public HTTP relays. */
export const DRAND_RELAYS = [
  "https://api.drand.sh",
  "https://api2.drand.sh",
  "https://api3.drand.sh",
  "https://drand.cloudflare.com",
] as const;

const TIMEOUT_MS = 8_000;

/** `zeek_verify::offline::PublicBeacon` as JSON. */
export type PublicBeacon =
  | { kind: "fetched"; beacon: { round: number; signature: string }; source: string }
  | { kind: "unreachable"; reason: string };

export async function fetchPublicBeacon(
  chainHash: string,
  round: number,
  fetchImpl: typeof fetch = fetch,
): Promise<PublicBeacon> {
  if (!/^[0-9a-f]{64}$/.test(chainHash) || !Number.isSafeInteger(round) || round < 1) {
    return { kind: "unreachable", reason: "the bundle names no valid drand chain and round" };
  }
  const failures: string[] = [];
  for (const relay of DRAND_RELAYS) {
    const host = new URL(relay).host;
    try {
      const response = await fetchImpl(`${relay}/${chainHash}/public/${round}`, {
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
      if (!response.ok) {
        failures.push(`${host}: HTTP ${response.status}`);
        continue;
      }
      const body = (await response.json()) as { round?: unknown; signature?: unknown };
      if (body.round !== round || typeof body.signature !== "string") {
        failures.push(`${host}: unexpected answer`);
        continue;
      }
      return { kind: "fetched", beacon: { round, signature: body.signature }, source: host };
    } catch (error) {
      failures.push(`${host}: ${error instanceof Error ? error.message : "no answer"}`);
    }
  }
  return { kind: "unreachable", reason: failures.join("; ") };
}
