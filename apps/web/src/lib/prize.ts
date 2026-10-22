/**
 * The prize as it will be once the digs now in the mempool are mined: the
 * board's prize counts mined digs only, a block (~25 s) behind a dig.
 *
 * The prize is `pot − floor(pot · rake_bps / 10000)` with pot = Σ(value −
 * dig_fee) over the round's valid digs (zeek-core `settle::prize_pool`).
 * The board gives the mined part as `gross_dug_zat` and `digs`; the
 * landing digs of this round add their own `value − dig_fee`. The rake is
 * taken once over the whole pot, exactly as settlement will.
 *
 * Never counted twice: the engine leaves a landing dig out of the board
 * once its transaction is stored (it de-duplicates by txid, which the app
 * never sees), landing events replace the whole list (a dig that left the
 * mempool unmined drops out with the next one), and a repeated id counts
 * once. Only this round's landing digs count.
 */

import type { Board, LandingDig } from "../api/types";

export interface LivePrize {
  /** Mined prize plus what this round's landing digs add. */
  total: bigint;
  /** The not-yet-mined part (0 when nothing is landing). */
  landing: bigint;
}

export interface PrizeRules {
  rake_bps: number;
  /** Absent from older engines: then landing digs are not added. */
  dig_fee_zat?: number | undefined;
}

/** The prize of a pot (after dig fees): minus the rake, rounded down. */
export function prizeOfPot(pot: bigint, rakeBps: bigint): bigint {
  return pot - (pot * rakeBps) / 10_000n;
}

/** This round's landing digs, each id once. */
export function roundLanding(round: number, digs: readonly LandingDig[]): LandingDig[] {
  const byId = new Map<string, LandingDig>();
  for (const d of digs) if (d.round === round && !byId.has(d.id)) byId.set(d.id, d);
  return [...byId.values()];
}

export function livePrize(
  board: Pick<Board, "round" | "prize_pool_zat" | "gross_dug_zat" | "digs" | "landing">,
  rules: PrizeRules | undefined,
): LivePrize {
  const mined = BigInt(board.prize_pool_zat);
  const plain = { total: mined, landing: 0n };
  if (rules?.dig_fee_zat === undefined) return plain;
  const fee = BigInt(rules.dig_fee_zat);
  const rake = BigInt(rules.rake_bps);
  const pot = BigInt(board.gross_dug_zat) - BigInt(board.digs) * fee;
  // The board's own prize must follow from this pot; otherwise the rule
  // differs from what this app knows, and only the mined prize is shown.
  if (pot < 0n || prizeOfPot(pot, rake) !== mined) return plain;
  let added = 0n;
  for (const d of roundLanding(board.round, board.landing ?? [])) {
    const value = BigInt(d.value_zat);
    if (value > fee) added += value - fee;
  }
  if (added === 0n) return plain;
  const total = prizeOfPot(pot + added, rake);
  return { total, landing: total - mined };
}
