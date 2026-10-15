/**
 * Digging from another wallet: where should winnings go → scan the
 * payments → waiting (each payment "looking…", then "seen, landing" as soon
 * as the engine's mempool watcher sees it, then mined) → confirmed (or late:
 * refunded).
 */

export interface PaymentModel {
  /** The ZIP 321 URI (one or more tiles). */
  uri: string;
  tiles: number[];
  amount: bigint;
  /** SHA-256 (hex) of each tile's 512-byte memo field, in tile order. */
  memoDigests: string[];
  qrSvg: string;
}

/**
 * not-sent: nothing seen yet; landing: seen in the mempool (the engine's
 * mempool watcher), not mined yet; landed: mined inside the round; late:
 * mined after it (refunded).
 */
export type PaymentStatus = "not-sent" | "landing" | "landed" | "late";

export type OtherWalletState =
  | { kind: "address"; error: string | null }
  | { kind: "scan"; round: number; payments: PaymentModel[] }
  | { kind: "waiting"; round: number; payments: PaymentModel[]; status: PaymentStatus[] }
  | { kind: "confirmed"; round: number; payments: PaymentModel[]; status: PaymentStatus[] };

export type OtherWalletEvent =
  | { type: "address-error"; message: string }
  | { type: "built"; round: number; payments: PaymentModel[] }
  | { type: "sent" }
  | { type: "seen"; status: PaymentStatus[] }
  | { type: "back" };

export const initialOtherWallet: OtherWalletState = { kind: "address", error: null };

export function otherWalletReducer(
  state: OtherWalletState,
  event: OtherWalletEvent,
): OtherWalletState {
  switch (event.type) {
    case "address-error":
      return { kind: "address", error: event.message };
    case "built":
      return { kind: "scan", round: event.round, payments: event.payments };
    case "sent":
      return state.kind === "scan"
        ? {
            kind: "waiting",
            round: state.round,
            payments: state.payments,
            status: state.payments.map(() => "not-sent"),
          }
        : state;
    case "seen": {
      if (state.kind !== "waiting") return state;
      // Confirmed once every payment is mined (landing is not yet final).
      const all = event.status.every((s) => s === "landed" || s === "late");
      return all
        ? { kind: "confirmed", round: state.round, payments: state.payments, status: event.status }
        : { ...state, status: event.status };
    }
    case "back":
      return initialOtherWallet;
  }
}

/**
 * Status of each payment from the round's note digests: landed when every
 * tile memo is seen inside the round's window, late when seen after it.
 */
export function paymentStatus(
  payments: readonly PaymentModel[],
  seen: readonly { height: number; memo_sha256: string }[],
  windowEnd: number,
): PaymentStatus[] {
  return payments.map((p) => {
    const hits = p.memoDigests.map((d) => seen.find((s) => s.memo_sha256 === d));
    if (hits.some((h) => h === undefined)) return "not-sent";
    return hits.some((h) => h !== undefined && h.height >= windowEnd) ? "late" : "landed";
  });
}

/** SHA-256 of a memo's text as the wallet stores it: UTF-8, zero-padded to 512 bytes. */
export async function memoDigest(memoText: string): Promise<string> {
  const field = new Uint8Array(512);
  field.set(new TextEncoder().encode(memoText).slice(0, 512));
  const hash = await crypto.subtle.digest("SHA-256", field);
  return Array.from(new Uint8Array(hash), (b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Marks payments that are not mined yet but whose every tile memo is among
 * the engine's landing digs (seen in the mempool).
 */
export function withLanding(
  payments: readonly PaymentModel[],
  status: readonly PaymentStatus[],
  landingDigests: ReadonlySet<string>,
): PaymentStatus[] {
  return payments.map((p, i) => {
    const s = status[i] ?? "not-sent";
    if (s !== "not-sent") return s;
    return p.memoDigests.length > 0 && p.memoDigests.every((d) => landingDigests.has(d))
      ? "landing"
      : s;
  });
}
