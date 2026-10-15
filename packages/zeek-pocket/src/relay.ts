/** HTTP client for the engine's chain relay and game API. */

import { PocketError } from "./errors.js";
import type {
  BoardState,
  CampfirePage,
  EngineConfig,
  PendingRound,
  PinnedVault,
  WinVerdict,
} from "./types.js";

/** The engine's stored win proof. */
export interface StoredProof {
  id: string;
  round: number;
  created_at: number;
  proof: string;
  verdict: WinVerdict;
}

export interface ChainTip {
  height: number;
  hash: string;
}

/** Most blocks the relay serves per request. */
export const MAX_BLOCK_RANGE = 500;

type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

/** Deep equality of JSON values, ignoring object key order. */
function sameJson(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a);
  const kb = Object.keys(b);
  return (
    ka.length === kb.length &&
    ka.every((k) => sameJson((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]))
  );
}

/** The fields of `config` that differ from `pinned`. */
export function pinMismatches(config: EngineConfig & { uivk?: string }, pinned: PinnedVault) {
  const differ: string[] = [];
  // Every parameter the build knows must match (review N11); parameters the
  // engine adds with default values are not in the game file.
  const served = config.params as unknown as Record<string, unknown>;
  if (Object.entries(pinned.params).some(([k, v]) => !sameJson(served[k], v))) {
    differ.push("game parameters");
  }
  if (config.params.network !== pinned.network) differ.push("network");
  if (config.vault_address !== pinned.vault_address) differ.push("vault address");
  if (config.sealing_public_key !== pinned.sealing_public_key) differ.push("sealing key");
  if (config.uivk !== pinned.uivk) differ.push("viewing key");
  return differ;
}

export class RelayClient {
  readonly baseUrl: string;
  private readonly fetchImpl: FetchLike;
  private readonly pinned: PinnedVault | undefined;

  /**
   * `pinned`, when given, is checked against every `/v1/config` read; a
   * mismatch is refused with `config_mismatch`.
   */
  constructor(
    baseUrl: string,
    fetchImpl: FetchLike = (input, init) => fetch(input, init),
    pinned?: PinnedVault,
  ) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.fetchImpl = fetchImpl;
    this.pinned = pinned;
  }

  private async request(path: string, init?: RequestInit): Promise<Response> {
    let response: Response;
    try {
      response = await this.fetchImpl(`${this.baseUrl}${path}`, init);
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      throw new PocketError("network", `cannot reach the engine at ${this.baseUrl}: ${detail}`);
    }
    if (!response.ok) {
      let message = `${response.status} ${response.statusText}`;
      try {
        const body = (await response.json()) as { error?: unknown };
        if (typeof body.error === "string") {
          message = body.error;
        }
      } catch {
        // Not JSON: keep the status line.
      }
      const code = response.status === 422 ? "broadcast_rejected" : "relay";
      throw new PocketError(code, message);
    }
    return response;
  }

  private async bytes(path: string): Promise<Uint8Array> {
    const response = await this.request(path);
    return new Uint8Array(await response.arrayBuffer());
  }

  async tip(): Promise<ChainTip> {
    return (await (await this.request("/v1/chain/tip")).json()) as ChainTip;
  }

  /** Compact blocks `start..=end`, length-prefixed protobuf as the relay serves them. */
  async blocks(start: number, end: number): Promise<Uint8Array> {
    if (end < start || end - start >= MAX_BLOCK_RANGE) {
      throw new PocketError("invalid_request", `block range ${start}..=${end} is not allowed`);
    }
    return this.bytes(`/v1/chain/blocks?start=${start}&end=${end}`);
  }

  /** `TreeState` protobuf as of the end of block `height`. */
  async treeState(height: number): Promise<Uint8Array> {
    return this.bytes(`/v1/chain/tree-state?height=${height}`);
  }

  /** `RawTransaction` protobuf. */
  async transaction(txid: string): Promise<Uint8Array> {
    if (!/^[0-9a-f]{64}$/.test(txid)) {
      throw new PocketError("invalid_request", `not a txid: ${txid}`);
    }
    return this.bytes(`/v1/tx/${txid}`);
  }

  async broadcast(raw: Uint8Array<ArrayBuffer>): Promise<void> {
    await this.request("/v1/tx/broadcast", {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: raw,
    });
  }

  /** The engine's `/v1/config`: raw JSON text (handed to the wallet core) and parsed. */
  async config(): Promise<{ json: string; config: EngineConfig }> {
    const json = await (await this.request("/v1/config")).text();
    const config = JSON.parse(json) as EngineConfig & { uivk?: string };
    if (this.pinned !== undefined) {
      const differ = pinMismatches(config, this.pinned);
      if (differ.length > 0) {
        throw new PocketError(
          "config_mismatch",
          `the engine names a different ${differ.join(", ")} than this app was built for; refusing to use it`,
        );
      }
    }
    return { json, config };
  }

  async board(): Promise<BoardState> {
    return (await (await this.request("/v1/board")).json()) as BoardState;
  }

  /** A round's proof bundle as JSON text (handed to the verifier unchanged). */
  async roundBundle(round: number): Promise<string> {
    return (await this.request(`/v1/rounds/${round}/proof`)).text();
  }

  async campfire(
    options: { before?: number; limit?: number; includeHidden?: boolean } = {},
  ): Promise<CampfirePage> {
    const query = new URLSearchParams();
    if (options.before !== undefined) query.set("before", String(options.before));
    if (options.limit !== undefined) query.set("limit", String(options.limit));
    if (options.includeHidden === true) query.set("include_hidden", "1");
    const suffix = query.size > 0 ? `?${query.toString()}` : "";
    return (await (await this.request(`/v1/campfire${suffix}`)).json()) as CampfirePage;
  }

  /** Submits a win proof; the engine verifies it fully before storing. */
  async submitProof(proof: string): Promise<{ stored: boolean; body: StoredProof }> {
    let response: Response;
    try {
      response = await this.fetchImpl(`${this.baseUrl}/v1/proofs`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ proof }),
      });
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      throw new PocketError("network", `cannot reach the engine at ${this.baseUrl}: ${detail}`);
    }
    if (response.status === 201 || response.status === 422) {
      return { stored: response.status === 201, body: (await response.json()) as StoredProof };
    }
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = (await response.json()) as { error?: unknown };
      if (typeof body.error === "string") message = body.error;
    } catch {
      // Not JSON.
    }
    throw new PocketError("relay", message);
  }

  async proof(id: string): Promise<StoredProof> {
    if (!/^[0-9a-f]{32}$/.test(id)) {
      throw new PocketError("invalid_request", `not a proof id: ${id}`);
    }
    return (await (await this.request(`/v1/proofs/${id}`)).json()) as StoredProof;
  }

  async pendingRounds(): Promise<PendingRound[]> {
    const body = (await (await this.request("/v1/rounds/pending")).json()) as {
      rounds: PendingRound[];
    };
    return body.rounds;
  }
}

/** Hex string to bytes. */
export function hexToBytes(hex: string): Uint8Array<ArrayBuffer> {
  if (hex.length % 2 !== 0 || !/^[0-9a-f]*$/i.test(hex)) {
    throw new PocketError("internal", "invalid hex");
  }
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  }
  return out;
}
