/** Typed, human-readable errors of Zeek Pocket. */

/** Codes the wallet core reports, plus the ones the TypeScript layer adds. */
export type PocketErrorCode =
  | "bad_passphrase"
  | "weak_passphrase"
  | "invalid_mnemonic"
  | "insufficient_funds"
  | "closing_too_soon"
  | "not_synced"
  | "invalid_address"
  | "invalid_amount"
  | "invalid_request"
  | "reorg"
  | "bad_chain_data"
  | "corrupt_state"
  | "build_failed"
  | "overflow"
  | "locked"
  | "no_wallet"
  | "wallet_exists"
  | "network"
  | "relay"
  | "broadcast_rejected"
  | "broadcast_unknown"
  | "not_isolated"
  | "partial_dig"
  | "config_mismatch"
  | "internal";

const CODES: ReadonlySet<string> = new Set<PocketErrorCode>([
  "bad_passphrase",
  "weak_passphrase",
  "invalid_mnemonic",
  "insufficient_funds",
  "closing_too_soon",
  "not_synced",
  "invalid_address",
  "invalid_amount",
  "invalid_request",
  "reorg",
  "bad_chain_data",
  "corrupt_state",
  "build_failed",
  "overflow",
  "locked",
  "no_wallet",
  "wallet_exists",
  "network",
  "relay",
  "broadcast_rejected",
  "broadcast_unknown",
  "not_isolated",
  "partial_dig",
  "config_mismatch",
  "internal",
]);

/** What a batched dig sent before a later batch failed. */
export interface PartialDig {
  /** Batches broadcast (never resent). */
  sent: { txid: string; tiles: number[] }[];
  /** Tiles not dug. */
  unsent: number[];
}

/** Plain data form, as it crosses the worker boundary. */
export interface PocketErrorData {
  code: PocketErrorCode;
  message: string;
  height?: number;
  partial?: PartialDig;
}

export class PocketError extends Error {
  readonly code: PocketErrorCode;
  /** Fork height for `reorg`. */
  readonly height: number | undefined;
  /** For `partial_dig`: what was sent and what was not. */
  readonly partial: PartialDig | undefined;

  constructor(code: PocketErrorCode, message: string, height?: number, partial?: PartialDig) {
    super(message);
    this.name = "PocketError";
    this.code = code;
    this.height = height;
    this.partial = partial;
  }

  toData(): PocketErrorData {
    const data: PocketErrorData = { code: this.code, message: this.message };
    if (this.height !== undefined) {
      data.height = this.height;
    }
    if (this.partial !== undefined) {
      data.partial = this.partial;
    }
    return data;
  }

  static fromData(data: PocketErrorData): PocketError {
    return new PocketError(data.code, data.message, data.height, data.partial);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/**
 * Whether a failed broadcast may still have reached the chain: the
 * connection dropped, the relay gave up (it may have handed the transaction
 * on before timing out), or a node answered that it already has it (or a
 * transaction spending the same notes). Only a plain refusal is proof that
 * nothing was sent.
 */
export function isUnclearBroadcast(error: PocketError): boolean {
  if (error.code === "network" || error.code === "relay") return true;
  return (
    error.code === "broadcast_rejected" && /already|duplicate|known|mempool/i.test(error.message)
  );
}

/** What the player is told when a broadcast may or may not have reached the chain. */
export const BROADCAST_UNKNOWN_TEXT =
  "We couldn’t confirm it reached the chain. Check History in a minute before trying again.";

/** Normalises anything thrown (wasm `{ code, message }` objects included). */
export function toPocketError(error: unknown): PocketError {
  if (error instanceof PocketError) {
    return error;
  }
  if (isRecord(error) && typeof error.code === "string" && typeof error.message === "string") {
    const code = CODES.has(error.code) ? (error.code as PocketErrorCode) : "internal";
    const height = typeof error.height === "number" ? error.height : undefined;
    return new PocketError(code, error.message, height);
  }
  if (error instanceof TypeError) {
    // fetch() rejects with a TypeError when the network or CORS fails.
    return new PocketError("network", `network error: ${error.message}`);
  }
  if (error instanceof Error) {
    return new PocketError("internal", error.message);
  }
  return new PocketError("internal", String(error));
}
