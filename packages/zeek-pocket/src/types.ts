/** Shapes exchanged with the wallet worker and the engine. */

/** Balance in zatoshis. */
export interface Balance {
  spendable_zat: number;
  pending_zat: number;
  /** Notes in the closed legacy Orchard pool, not spendable. */
  orchard_pool_zat: number;
  total_zat: number;
  scanned_height: number;
}

export interface SyncProgress {
  scannedHeight: number;
  tipHeight: number;
  birthdayHeight: number;
  /** 0 to 1 over the range this sync covers. */
  fraction: number;
  phase: "blocks" | "memos" | "done";
}

export interface ScanReport {
  from_height: number;
  scanned_height: number;
  blocks: number;
  new_notes: number;
  spent_notes: number;
}

export type OutgoingStatus = "built" | "broadcast" | "mined" | "expired";

export type OutgoingKind =
  | {
      dig: {
        round: number;
        tiles: number[];
        amount_each_zat: number;
        receiver_indices: number[];
        memo_encoding: string;
      };
    }
  | { send: { to: string; amount_zat: number; memo: string | null } };

export interface Outgoing {
  txid: string;
  kind: OutgoingKind;
  fee_zat: number;
  spends: [string, number][];
  change_zat: number;
  target_height: number;
  expiry_height: number;
  status: OutgoingStatus;
  mined_height: number | null;
}

export type HistoryEntry =
  | {
      type: "received";
      txid: string;
      height: number;
      value_zat: number;
      /** Memo text (or a description of a non-text memo); null until fetched. */
      memo: string | null;
      pool: "Ironwood" | "Orchard";
      spent: boolean;
    }
  | ({ type: "sent" } & Outgoing);

export interface BuiltTransaction {
  txid: string;
  rawHex: string;
  feeZat: number;
  targetHeight: number;
  expiryHeight: number;
  spends: number;
  outputs: number;
  provingMs: number;
}

export interface DigResult {
  txid: string;
  round: number;
  tiles: number[];
  amountEachZat: number;
  feeZat: number;
  provingMs: number;
}

export interface SendResult {
  txid: string;
  amountZat: number;
  feeZat: number;
  provingMs: number;
}

/** The any-wallet request: a ZIP 321 URI and a QR code of it. */
export interface AnyWalletDig {
  uri: string;
  round: number;
  tiles: number[];
  amountEachZat: number;
  totalZat: number;
  /** The `zeek1:` text memo of each payment, in tile order. */
  memos: string[];
  /** What to tell the player (paste a fresh receive address, and why). */
  notice: string;
  /** The URI as an SVG QR code. */
  qrSvg: string;
}

/** The engine's `/v1/board`. */
export interface BoardState {
  tip_height: number;
  tip_time: number;
  chain_tip: number;
  round: number;
  status: string;
  blocks_remaining: number;
  estimated_close_time: number;
  estimated_start_time: number | null;
  prize_pool_zat: number;
  gross_dug_zat: number;
  rules_version: number;
  cache: number;
  depths: number[];
  digs: number;
}

/** A closed round waiting for its reveal (`/v1/rounds/pending`). */
export interface PendingRound {
  round: number;
  status: string;
  drand_round: number;
  beacon_time: number;
}

/** The parts of the engine's `/v1/config` the wallet reads. */
export interface EngineConfig {
  params: {
    network: string;
    min_dig_zat: number;
    dig_fee_zat?: number;
    text_memo_activation_height?: number;
    blocks_per_round: number;
    genesis_height: number;
  };
  vault_address: string;
  sealing_public_key: string;
  genesis_height: number;
}
