/**
 * Checking a payout address before it is sealed into a dig. The wasm core
 * does the real validation (an Orchard-protocol receiver on the right
 * network); these messages explain common mistakes in plain words.
 */

export type AddressVerdict =
  | { ok: true }
  | {
      ok: false;
      reason: "empty" | "sapling" | "transparent" | "wrong-network" | "unknown";
      message: string;
    };

export function checkPayoutAddress(text: string, network: "test" | "main"): AddressVerdict {
  const a = text.trim();
  const prefix = network === "test" ? "utest1" : "u1";
  if (a === "") return { ok: false, reason: "empty", message: "Paste an address." };
  if (a.startsWith(prefix) && a.length > 40) return { ok: true };
  if (/^(zs|ztestsapling)1/.test(a)) {
    return {
      ok: false,
      reason: "sapling",
      message: `That’s an old private address. Use your wallet’s newer one that starts with “${prefix}”.`,
    };
  }
  if (/^(t1|t3|tm|t2|tex1|textest1)/.test(a)) {
    return {
      ok: false,
      reason: "transparent",
      message: `That address is public. Anyone could see your winnings. Paste one that starts with “${prefix}”.`,
    };
  }
  if (
    (network === "test" && a.startsWith("u1")) ||
    (network === "main" && a.startsWith("utest1"))
  ) {
    return {
      ok: false,
      reason: "wrong-network",
      message: `Wrong network. This is ${network === "test" ? "Testnet" : "Mainnet"}, so it must start with “${prefix}”.`,
    };
  }
  return {
    ok: false,
    reason: "unknown",
    message: `That doesn’t look like a private address starting with “${prefix}”.`,
  };
}

/** `utest1qz7m…v9hn3d` for showing the user their own address compactly. */
export function shortAddress(address: string): string {
  return address.length <= 20 ? address : `${address.slice(0, 10)}…${address.slice(-8)}`;
}
