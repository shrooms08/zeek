/** The engine's JSON, as served (amounts are integer zatoshis). */

export type RoundStatus = "open" | "closing" | "revealable" | "settled" | "voided";

export interface Window {
  start: number;
  end: number;
}

export interface Board {
  /** Mean spacing of the last 20 blocks (s); absent from older engines. */
  recent_spacing_secs?: number | null;
  tip_height: number;
  tip_time: number;
  chain_tip: number;
  round: number;
  status: RoundStatus;
  window: Window;
  blocks_remaining: number;
  estimated_close_time: number;
  estimated_start_time: number | null;
  prize_pool_zat: number;
  gross_dug_zat: number;
  rules_version: number;
  cache: number;
  depths: number[];
  stages: number[];
  digs: number;
  /**
   * Digs seen in the mempool, not yet mined: shown as "landing", never
   * counted in the prize or depths. Present on /v1/board; the stream sends
   * them as separate "landing" events.
   */
  landing?: LandingDig[];
}

/** A dig in the mempool (engine mempool watcher). No txid, no address. */
export interface LandingDig {
  id: string;
  round: number;
  tile: number;
  value_zat: number;
  /** SHA-256 of the 512-byte memo field (as /v1/rounds/{n}/digests). */
  memo_sha256: string;
  first_seen: number;
}

export interface PendingRound {
  round: number;
  window: Window;
  status: RoundStatus;
  drand_round: number;
  beacon_time: number;
}

export type PayoutReason =
  { reason: "win"; treasure_share: number; cache_share: number } | { reason: "refund" };

export interface Payout {
  memo_round: number;
  memo_tile: number;
  gross: number;
  fee: number;
  net: number;
  reason: PayoutReason;
}

export type Outcome =
  { outcome: "voided" } | { outcome: "drawn"; treasure_tile: number; cache_triggered: boolean };

export interface PayoutRecord {
  round: number;
  state: "signing" | "broadcast";
  txid: string | null;
  fee: number;
  signers: number[];
}

export interface SettledRound {
  outcome: Outcome;
  treasure_tile: number | null;
  cache_triggered: boolean;
  tile_totals: number[];
  drand_round: number;
  beacon_time: number;
  pot: number;
  rake: number;
  distributable: number;
  winners: number;
  payouts: Payout[];
  previous_cache: number;
  new_cache: number;
  cache_in: number;
  cache_paid: number;
  total_fees: number;
  dig_fees: number;
  total_paid: number;
  forfeited: number;
  shouts?: number;
  shout_fees?: number;
  rules_version: number;
  plan_status: string;
  settled_at_height: number;
  payout: PayoutRecord | null;
}

export interface RoundView {
  round: number;
  status: RoundStatus;
  window: Window;
  digs: number;
  depths: number[];
  settled: SettledRound | null;
}

export type RoundListStatus = "paid" | "paying" | "no_winner" | "voided";

export interface RoundSummary {
  round: number;
  status: RoundListStatus;
  treasure_tile: number | null;
  cache_triggered: boolean;
  pot: number;
  prize: number;
  total_paid: number;
  winners: number;
  refunds: number;
  closed_time: number | null;
  revealed_time: number;
  payout_txid: string | null;
  rules_version: number;
}

export interface RoundsPage {
  rounds: RoundSummary[];
  next_before: number | null;
}

export interface EngineConfig {
  params: {
    network: "test" | "main";
    min_dig_zat: number;
    dig_fee_zat?: number;
    marginal_fee_zat: number;
    blocks_per_round: number;
    target_spacing_secs: number;
    reveal_delay_secs?: number;
    genesis_height: number;
    rake_bps: number;
    cache_odds: number;
    campfire?: { activation_height: number; min_shout_zat: number };
  };
  /** False when digs are closed (mainnet readiness): the vault is withheld. */
  digging_enabled?: boolean;
  /** True while Zeek's own test wallets dig (scripts/demo-digger.sh). */
  demo_digs?: boolean;
  vault_address?: string;
  uivk?: string;
  genesis_height: number;
}

export interface Ledger {
  vault_balance: number;
  cache: number;
  unpaid_obligations: number;
  reserved_fees: number;
  /** Payouts whose sealed receiver does not open; held in the vault. */
  unclaimable: number;
  operator_float: number;
  shortfall: number;
  /** Chain tip of the vault wallet when its balance was read. */
  last_reconciliation_height?: number;
}

export interface KeyholderStatus {
  identifier: number;
  label: string;
  operator: string;
  reachable: boolean | null;
  last_seen: number | null;
  last_check: number | null;
  approved: number;
  refused: number;
}

export interface KeyholdersStatus {
  threshold: number;
  keyholders: KeyholderStatus[];
  refused_attempts: { time: number; rounds: number[]; reason: string; refusals: number }[];
}

export interface CampfireMessage {
  id: string;
  round: number;
  kind: "dig" | "shout";
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

export interface NoteDigest {
  height: number;
  value: number;
  memo_sha256: string;
}

/** WebSocket events (`/v1/stream`). */
export type LiveEvent =
  | ({ event: "board" } & Board)
  | { event: "round_closed"; round: number; drand_round: number; beacon_time: number }
  | { event: "reveal"; round: number; treasure_tile: number; cache_triggered: boolean }
  | { event: "settled"; round: number; new_cache: number }
  | { event: "paid"; round: number; txid: string }
  | ({ event: "campfire" } & CampfireMessage)
  | { event: "landing"; digs: LandingDig[] };

/** One lightwalletd server's health (`GET /v1/status`). */
export interface ChainServerStatus {
  url: string;
  healthy: boolean;
  tip: number | null;
  last_ok: number | null;
  last_error: string | null;
  latency_ms: number | null;
  outvoted: number;
}

/** `GET /v1/status`. */
export interface ServiceStatus {
  now: number;
  network: "test" | "main";
  digging_enabled: boolean;
  engine: {
    uptime_secs: number;
    stored_tip: number | null;
    chain_tip: number | null;
    last_block_time: number | null;
    last_settled_round: number | null;
  };
  threshold: number;
  keyholders: KeyholderStatus[];
  chain_servers: ChainServerStatus[];
}
