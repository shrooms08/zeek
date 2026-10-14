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
  | "not_isolated"
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
  "not_isolated",
  "internal",
]);

/** Plain data form, as it crosses the worker boundary. */
export interface PocketErrorData {
  code: PocketErrorCode;
  message: string;
  height?: number;
}

export class PocketError extends Error {
  readonly code: PocketErrorCode;
  /** Fork height for `reorg`. */
  readonly height: number | undefined;

  constructor(code: PocketErrorCode, message: string, height?: number) {
    super(message);
    this.name = "PocketError";
    this.code = code;
    this.height = height;
  }

  toData(): PocketErrorData {
    return this.height === undefined
      ? { code: this.code, message: this.message }
      : { code: this.code, message: this.message, height: this.height };
  }

  static fromData(data: PocketErrorData): PocketError {
    return new PocketError(data.code, data.message, data.height);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

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
