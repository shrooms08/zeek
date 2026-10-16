/** <RevealLine>: "Round N reveals in m:ss", expandable to the pending list. */

import { useState } from "react";
import { Link } from "react-router-dom";
import type { PendingRound } from "../api/types";
import { formatClock } from "../lib/time";
import { Icon } from "./Icon";

function when(p: PendingRound, now: number): string {
  const left = p.beacon_time - now;
  return left > 0 ? `reveals in ${formatClock(left)}` : "is being revealed…";
}

export function RevealLine({
  pending,
  now,
  openRound,
  onDigNow,
}: {
  pending: PendingRound[];
  now: number;
  /** The round open for digs now, if any. */
  openRound?: number | undefined;
  /** Takes the player to the field's tiles. */
  onDigNow?: (() => void) | undefined;
}) {
  const [open, setOpen] = useState(false);
  const first = pending[0];
  if (first === undefined) return null;
  return (
    <section className="panel" aria-label="Reveals" data-testid="reveal-line">
      <button
        type="button"
        className="w-full flex items-center gap-3 p-3 text-left text-white"
        aria-expanded={open}
        onClick={() => {
          setOpen((o) => !o);
        }}
      >
        <Icon name="sparkle" size={18} />
        <span className="flex-1">
          Round {first.round} {when(first, now)}
        </span>
        <Icon name="back" size={16} />
      </button>
      {openRound !== undefined && onDigNow !== undefined && openRound > first.round && (
        <button
          type="button"
          className="w-full flex items-center gap-3 border-t hair px-3 py-2 text-left text-white underline"
          onClick={onDigNow}
          data-testid="reveal-line-dig"
        >
          <Icon name="shovel" size={16} /> While you wait: dig round {openRound}
        </button>
      )}
      {open && (
        <ul className="border-t hair">
          {pending.map((p) => (
            <li key={p.round} className="flex justify-between p-3 border-b hair">
              <Link className="underline text-white" to={`/reveal/${p.round}`}>
                Round {p.round}
              </Link>
              <span className="text-fog">{when(p, now)}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
