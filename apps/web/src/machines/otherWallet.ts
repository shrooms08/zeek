/**
 * Digging from another wallet: where should winnings go → scan the
 * payments → waiting (each payment "looking…" until it is mined; the
 * engine cannot see the mempool) → confirmed (or late: refunded).
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

export type PaymentStatus = "not-sent" | "landed" | "late";

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
      const all = event.status.every((s) => s !== "not-sent");
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
