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
  | { send: { to: string; amount_zat: number; memo: string | null } }
  | { shout: { amount_zat: number; message: string } };

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
  round: number;
  tiles: number[];
  amountEachZat: number;
  /** One per transaction, in the order they were sent. */
  batches: DigBatch[];
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
/**
 * The vault this app was built for (from the public game config, compiled
 * into the build). The wallet refuses an engine whose `/v1/config` names a
 * different vault, sealing key, viewing key or network, so a compromised
 * engine, proxy or tunnel cannot redirect digs or unseal receivers.
 */
export interface PinnedVault {
  network: string;
  vault_address: string;
  sealing_public_key: string;
  uivk: string;
  /** The game parameters (memo format, activation heights, minimums). */
  params: Record<string, unknown>;
}

export interface EngineConfig {
  /** False when digs are closed (mainnet readiness): the vault is withheld. */
  digging_enabled?: boolean;
  params: {
    network: string;
    min_dig_zat: number;
    dig_fee_zat?: number;
    text_memo_activation_height?: number;
    campfire?: { activation_height: number; min_shout_zat: number };
    blocks_per_round: number;
    genesis_height: number;
  };
  vault_address: string;
  sealing_public_key: string;
  genesis_height: number;
}

/** One transaction of a (possibly batched) dig. */
export interface DigBatch {
  txid: string;
  tiles: number[];
  feeZat: number;
  provingMs: number;
  /** Wasm linear memory after this batch (bytes; it only grows, so this is the peak so far). */
  wasmMemoryBytes: number;
}

/** Progress of a batched dig. */
export interface DigProgress {
  phase: "sync" | "waiting" | "proving" | "broadcast" | "sent";
  batch: number;
  batches: number;
  tiles: number[];
  txid?: string;
  /** Why a batch is waiting: a random privacy gap between separate
   * payments, or the previous batch's change confirming. */
  waitingFor?: "privacy-gap" | "change";
  /** Seconds of the current wait (privacy gap) or waited so far (change). */
  seconds?: number;
}

/** A Campfire message as the engine serves it. */
export interface CampfireMessage {
  id: string;
  round: number;
  kind: "dig" | "shout";
  /** Tiles dug by the transaction with this message (absent for shouts). */
  tiles?: number[];
  height: number;
  time: number;
  value_zat: number;
  nickname?: string;
  text?: string;
  hidden: boolean;
  reason?: string;
}

export interface CampfirePage {
  messages: CampfireMessage[];
  next_before: number | null;
}

/** The any-wallet shout request. */
export interface AnyWalletShout {
  uri: string;
  amountZat: number;
  memo: string;
  notice: string;
  qrSvg: string;
}

/** A round check, as `zeek-verify` reports it. */
export interface CheckResult {
  id: "beacon" | "digs" | "timing" | "settlement" | "payout";
  title: string;
  status: "pass" | "warn" | "fail" | "not_available";
  summary: string;
  detail: string[];
}

export interface RoundReport {
  round: number;
  bundle_version: number;
  rules_version: number | null;
  checks: CheckResult[];
}

/** Result of comparing block hashes with an independent second source. */
export interface CrossCheck {
  status: "pass" | "fail" | "unavailable";
  source: string;
  detail: string;
}

export interface VerifyProgress {
  step: "bundle" | "blocks" | "scan" | "transactions" | "checks" | "cross-check" | "done";
  detail: string;
}

export interface RoundVerification {
  report: RoundReport;
  crossCheck: CrossCheck;
  /** Where blocks and transactions came from. */
  source: string;
}

export interface WinStep {
  name: string;
  passed: boolean;
  detail: string;
}

export interface WinVerdict {
  valid: boolean;
  claim: string;
  reason: string | null;
  steps: WinStep[];
}

export interface WinProofResult {
  id: string;
  round: number;
  payoutIndex: number;
  /** The proof, base64url. */
  proof: string;
  verdict: WinVerdict;
  /** Engine's response status: 201 stored, 422 rejected. */
  stored: boolean;
}
