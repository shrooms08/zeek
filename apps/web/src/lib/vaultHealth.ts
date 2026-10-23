/**
 * Whether the vault covers what it holds for others: the Deep Cache, unpaid
 * payouts and their fees, and unclaimable payouts (`GET /v1/vault/ledger`,
 * crates/zeek-engine/src/ledger.rs). A shortfall is a warning on the status
 * page; so is a ledger whose vault balance was read long before the chain
 * tip, since its numbers may then be out of date.
 */

import type { Ledger } from "../api/types";
import { formatZec, zat } from "./money";

/** A ledger read more than this many blocks before the tip is reported stale. */
export const STALE_LEDGER_BLOCKS = 30;

export interface VaultHealth {
  health: "ok" | "warn";
  detail: string;
}

export function vaultHealth(ledger: Ledger, tip: number | null): VaultHealth {
  const owed = ledger.cache + ledger.unpaid_obligations + ledger.reserved_fees + ledger.unclaimable;
  const at = ledger.last_reconciliation_height;
  const behind = tip === null || at === undefined ? 0 : tip - at;
  const when =
    at === undefined
      ? ""
      : ` Checked at block ${at.toLocaleString("en-US")}${behind > 0 ? ` (${behind} blocks ago)` : ""}.`;
  if (ledger.shortfall > 0) {
    return {
      health: "warn",
      detail: `Short by ${formatZec(zat(ledger.shortfall))} ZEC: holds ${formatZec(zat(ledger.vault_balance))} ZEC for ${formatZec(zat(owed))} ZEC owed (the Deep Cache and unpaid payouts).${when}`,
    };
  }
  if (behind > STALE_LEDGER_BLOCKS) {
    return {
      health: "warn",
      detail: `Holds ${formatZec(zat(ledger.vault_balance))} ZEC, but that was read ${behind} blocks ago; the numbers may be out of date.${when}`,
    };
  }
  return {
    health: "ok",
    detail: `Holds ${formatZec(zat(ledger.vault_balance))} ZEC: covers the Deep Cache (${formatZec(zat(ledger.cache))} ZEC) and every unpaid payout, ${formatZec(zat(ledger.operator_float))} ZEC to spare.${when}`,
  };
}
