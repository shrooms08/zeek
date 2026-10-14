/**
 * Cross-checking the relay's blocks with an independent second source.
 *
 * Sources that work from a browser (CORS) as of October 2026:
 * - mainnet: Blockchair (`api.blockchair.com/zcash/dashboards/block/{h}`,
 *   `Access-Control-Allow-Origin: *`);
 * - testnet: no public source was found (Blockchair has no Zcash testnet,
 *   the CipherScan testnet API returned HTTP 500 on every route, and public
 *   testnet lightwalletd servers do not offer gRPC-web with CORS). Run
 *   `zeek-verify relay --lightwalletd <any lightwalletd>` and configure it
 *   as a `relay` source; without one the verifier reports the cross-check
 *   as unavailable instead of passing.
 */

import type { SecondSource } from "./protocol.js";
import type { CrossCheck } from "./types.js";

export interface BlockHash {
  height: number;
  hash: string;
}

/** Compares the relay's hashes with the second source's. */
export function compareHashes(
  primary: readonly BlockHash[],
  secondary: readonly BlockHash[],
  source: string,
): CrossCheck {
  const theirs = new Map(secondary.map((b) => [b.height, b.hash.toLowerCase()]));
  const missing: number[] = [];
  const differ: number[] = [];
  for (const block of primary) {
    const other = theirs.get(block.height);
    if (other === undefined) {
      missing.push(block.height);
    } else if (other !== block.hash.toLowerCase()) {
      differ.push(block.height);
    }
  }
  if (differ.length > 0) {
    return {
      status: "fail",
      source,
      detail: `block hashes differ at heights ${differ.join(", ")}: the two sources disagree about the chain`,
    };
  }
  if (missing.length > 0 || primary.length === 0) {
    return {
      status: "unavailable",
      source,
      detail:
        primary.length === 0
          ? "no blocks to compare"
          : `the second source did not return heights ${missing.join(", ")}`,
    };
  }
  return {
    status: "pass",
    source,
    detail: `all ${primary.length} block hashes (heights ${primary[0]?.height ?? 0}..${primary.at(-1)?.height ?? 0}) match`,
  };
}

/** Describes a second source for reports. */
export function sourceName(source: SecondSource): string {
  return source.kind === "relay" ? `relay ${source.url}` : "Blockchair (mainnet)";
}

type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

/** Fetches one block hash from Blockchair (mainnet). */
export async function blockchairHash(
  height: number,
  fetchImpl: FetchLike = (i, init) => fetch(i, init),
): Promise<BlockHash | undefined> {
  const response = await fetchImpl(`https://api.blockchair.com/zcash/dashboards/block/${height}`);
  if (!response.ok) {
    return undefined;
  }
  const body = (await response.json()) as {
    data?: Record<string, { block?: { hash?: string } } | undefined>;
  };
  const hash = body.data?.[String(height)]?.block?.hash;
  return hash === undefined ? undefined : { height, hash };
}
