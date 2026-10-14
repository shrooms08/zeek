/**
 * Wallet lifecycle: unknown → none | locked | unlocked; creating → backup →
 * unlocked; unlocked ⇄ syncing. Three wrong passphrases in a row lock the
 * form for 30 seconds (the seed's Argon2id already makes each guess slow;
 * this keeps the UI honest about it).
 */

export type WalletState =
  | { kind: "unknown" }
  | { kind: "none" }
  | { kind: "creating" }
  | { kind: "backup" }
  | { kind: "locked"; failures: number; lockedUntil: number; error: string | null }
  | { kind: "unlocking"; failures: number }
  | { kind: "unlocked" }
  | { kind: "syncing"; fraction: number; scanned: number; tip: number };

export type WalletEvent =
  | { type: "status"; exists: boolean; unlocked: boolean }
  | { type: "create" }
  | { type: "created" }
  | { type: "backed-up" }
  | { type: "unlock" }
  | { type: "unlock-ok" }
  | { type: "unlock-failed"; message: string; now: number }
  | { type: "lock" }
  | { type: "forgotten" }
  | { type: "sync"; fraction: number; scanned: number; tip: number }
  | { type: "synced" }
  | { type: "failed"; message: string };

export const initialWallet: WalletState = { kind: "unknown" };
export const MAX_TRIES = 3;
export const LOCKOUT_SECONDS = 30;

function locked(failures = 0, lockedUntil = 0, error: string | null = null): WalletState {
  return { kind: "locked", failures, lockedUntil, error };
}

export function walletReducer(state: WalletState, event: WalletEvent): WalletState {
  switch (event.type) {
    case "status":
      if (state.kind === "backup" || state.kind === "creating") return state;
      if (!event.exists) return { kind: "none" };
      if (event.unlocked) return state.kind === "syncing" ? state : { kind: "unlocked" };
      return state.kind === "locked" ? state : locked();
    case "create":
      return state.kind === "none" ? { kind: "creating" } : state;
    case "created":
      return state.kind === "creating" ? { kind: "backup" } : state;
    case "backed-up":
      return state.kind === "backup" ? { kind: "unlocked" } : state;
    case "unlock":
      return state.kind === "locked" ? { kind: "unlocking", failures: state.failures } : state;
    case "unlock-ok":
      return { kind: "unlocked" };
    case "unlock-failed": {
      if (state.kind !== "unlocking") return state;
      const failures = state.failures + 1;
      if (failures >= MAX_TRIES) {
        return locked(
          0,
          event.now + LOCKOUT_SECONDS,
          `${event.message} Wait ${LOCKOUT_SECONDS} s before trying again.`,
        );
      }
      const left = MAX_TRIES - failures;
      return locked(
        failures,
        0,
        `${event.message} ${left} ${left === 1 ? "try" : "tries"} before a ${LOCKOUT_SECONDS} s wait.`,
      );
    }
    case "lock":
      return state.kind === "unlocked" || state.kind === "syncing" ? locked() : state;
    case "forgotten":
      return { kind: "none" };
    case "sync":
      return state.kind === "unlocked" || state.kind === "syncing"
        ? { kind: "syncing", fraction: event.fraction, scanned: event.scanned, tip: event.tip }
        : state;
    case "synced":
      return state.kind === "syncing" ? { kind: "unlocked" } : state;
    case "failed":
      return state.kind === "creating"
        ? { kind: "none" }
        : state.kind === "syncing"
          ? { kind: "unlocked" }
          : state;
  }
}

/** Whether the unlock form is usable at `now`. */
export function canTryUnlock(state: WalletState, now: number): boolean {
  return state.kind === "locked" && now >= state.lockedUntil;
}
