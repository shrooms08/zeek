/**
 * Restoring from 24 words without losing the Pocket already in this browser:
 * the request is checked and the restored Pocket built first (a wrong word,
 * a bad birthday or a network error stops here), and only then is anything
 * stored, in one write that replaces the old Pocket.
 */

import { PocketError } from "./errors.js";

export async function restoreOver<Built, Result>(request: {
  exists: () => Promise<boolean>;
  /** The height the wallet was made at (or earlier). */
  birthdayHeight: number;
  /** Replace a Pocket already in this browser. */
  replace: boolean;
  /** Builds the restored Pocket, scanning from `fromHeight`; stores nothing. */
  build: (fromHeight: number) => Promise<Built>;
  /** Stores it, replacing any old one. */
  commit: (built: Built) => Promise<Result>;
}): Promise<Result> {
  const { birthdayHeight } = request;
  if (!Number.isInteger(birthdayHeight) || birthdayHeight < 1) {
    throw new PocketError(
      "invalid_request",
      "The birthday must be a block height, a whole number such as 4500000.",
    );
  }
  if (!request.replace && (await request.exists())) {
    throw new PocketError("wallet_exists", "a wallet already exists in this browser");
  }
  const built = await request.build(Math.max(1, birthdayHeight - 1));
  return request.commit(built);
}
