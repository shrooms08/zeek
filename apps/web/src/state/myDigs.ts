/**
 * This browser's record of its own digs: round, tiles and amounts only (no
 * addresses, no memos), so the reveal can say "You dug tiles 8 and 17" and
 * estimate a share. Kept in localStorage; never sent anywhere.
 */

export interface MyDig {
  round: number;
  /** Tile indices 0–24. */
  tiles: number[];
  /** Zatoshis per tile, as a decimal string (bigint-safe). */
  amountEach: string;
  via: "pocket" | "wallet";
  /** Transaction ids (Pocket) when known. */
  txids: string[];
  at: number;
}

const KEY = "zeek.myDigs.v1";

export function loadMyDigs(): MyDig[] {
  try {
    const raw = localStorage.getItem(KEY);
    return raw === null ? [] : (JSON.parse(raw) as MyDig[]);
  } catch {
    return [];
  }
}

export function recordMyDig(dig: MyDig): void {
  try {
    const all = loadMyDigs().filter((d) => !(d.round === dig.round && d.at === dig.at));
    all.push(dig);
    localStorage.setItem(KEY, JSON.stringify(all.slice(-200)));
  } catch {
    // Storage unavailable (private window): the reveal falls back to the Pocket's history.
  }
}

/** Tiles and total per tile this browser dug in `round`. */
export function digsInRound(
  round: number,
  digs: readonly MyDig[] = loadMyDigs(),
): Map<number, bigint> {
  const out = new Map<number, bigint>();
  for (const d of digs.filter((x) => x.round === round)) {
    for (const t of d.tiles) out.set(t, (out.get(t) ?? 0n) + BigInt(d.amountEach));
  }
  return out;
}

const SEEN_REVEAL = "zeek.seenReveal";

/** The newest round whose reveal this browser has shown (-1 if none). */
export function lastSeenReveal(): number {
  try {
    return Number(localStorage.getItem(SEEN_REVEAL) ?? "-1");
  } catch {
    return Number.MAX_SAFE_INTEGER;
  }
}

/** Records that the reveal of `round` was shown, so the field doesn't open it again. */
export function markRevealSeen(round: number): void {
  try {
    if (round > lastSeenReveal()) localStorage.setItem(SEEN_REVEAL, String(round));
  } catch {
    // private window: the reveal may open once more
  }
}
