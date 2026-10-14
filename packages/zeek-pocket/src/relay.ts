/** HTTP client for the engine's chain relay and game API. */

import { PocketError } from "./errors.js";
import type { BoardState, EngineConfig, PendingRound } from "./types.js";

export interface ChainTip {
  height: number;
  hash: string;
}

/** Most blocks the relay serves per request. */
export const MAX_BLOCK_RANGE = 500;

type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

export class RelayClient {
  readonly baseUrl: string;
  private readonly fetchImpl: FetchLike;

  constructor(baseUrl: string, fetchImpl: FetchLike = (input, init) => fetch(input, init)) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.fetchImpl = fetchImpl;
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
    return { json, config: JSON.parse(json) as EngineConfig };
  }

  async board(): Promise<BoardState> {
    return (await (await this.request("/v1/board")).json()) as BoardState;
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
