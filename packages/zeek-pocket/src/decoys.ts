/**
 * Hiding which transactions the wallet cares about.
 *
 * Reading a memo needs the full transaction, and asking the relay for one
 * reveals interest in it. Each wanted txid is therefore fetched together with
 * several decoy txids from the same block range (transactions with shielded
 * actions the wallet saw while scanning), and the whole list is shuffled, so
 * the relay sees a batch of plausible requests instead of exactly the
 * wallet's.
 */

export interface TxRef {
  txid: string;
  height: number;
}

export interface PlannedFetch extends TxRef {
  /** True for the wallet's own transactions; decoys are fetched and dropped. */
  wanted: boolean;
}

/** A uniform float in [0, 1). */
export type RandomSource = () => number;

/** Cryptographically random source for production use. */
export const cryptoRandom: RandomSource = () => {
  const buffer = new Uint32Array(1);
  crypto.getRandomValues(buffer);
  return (buffer[0] ?? 0) / 0x1_0000_0000;
};

function shuffle<T>(items: T[], random: RandomSource): T[] {
  const out = [...items];
  for (let i = out.length - 1; i > 0; i -= 1) {
    const j = Math.floor(random() * (i + 1));
    const a = out[i];
    const b = out[j];
    if (a !== undefined && b !== undefined) {
      out[i] = b;
      out[j] = a;
    }
  }
  return out;
}

/**
 * Plans the fetches: every wanted txid plus up to `decoysPerWanted` decoys
 * each, preferring decoys whose height is close to a wanted one, without
 * repeats, in random order.
 */
export function planTransactionFetches(
  wanted: readonly TxRef[],
  decoyPool: readonly TxRef[],
  decoysPerWanted: number,
  random: RandomSource = cryptoRandom,
): PlannedFetch[] {
  const wantedIds = new Set(wanted.map((w) => w.txid));
  const available = shuffle(
    decoyPool.filter((d) => !wantedIds.has(d.txid)),
    random,
  );
  const used = new Set<string>();
  const plan: PlannedFetch[] = [];
  for (const target of wanted) {
    plan.push({ ...target, wanted: true });
    const nearest = [...available]
      .filter((d) => !used.has(d.txid))
      .sort((a, b) => Math.abs(a.height - target.height) - Math.abs(b.height - target.height))
      .slice(0, decoysPerWanted);
    for (const decoy of nearest) {
      used.add(decoy.txid);
      plan.push({ ...decoy, wanted: false });
    }
  }
  return shuffle(plan, random);
}
