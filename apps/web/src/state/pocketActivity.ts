/**
 * What the Pocket is doing besides digging, for the background re-split
 * (src/state/resplit.tsx): a withdrawal being sent, and whether a dig was
 * sent whose change may need splitting again.
 */

let withdrawals = 0;
let armed = false;

/** Runs `send` as a withdrawal: no re-split starts meanwhile. */
export async function asWithdrawal<T>(send: () => Promise<T>): Promise<T> {
  withdrawals += 1;
  try {
    return await send();
  } finally {
    withdrawals -= 1;
  }
}

export function withdrawing(): boolean {
  return withdrawals > 0;
}

/** A dig was sent from the Pocket: look at its change once it confirms. */
export function armResplit(): void {
  armed = true;
}

export function disarmResplit(): void {
  armed = false;
}

export function resplitArmed(): boolean {
  return armed;
}
