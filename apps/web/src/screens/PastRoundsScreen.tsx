/** Past rounds: amounts only, never who. */

import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { engine } from "../api/engine";
import type { RoundListStatus } from "../api/types";
import { Icon } from "../components/Icon";
import { Empty, Loading, Offline, Screen } from "../components/ui";
import { formatZec, zat } from "../lib/money";

const STATUS_CHIP: Record<RoundListStatus, string> = {
  paid: "zk-chip-fill",
  paying: "",
  no_winner: "",
  voided: "zk-chip-dashed",
};

export const STATUS_LABEL: Record<RoundListStatus, string> = {
  paid: "Paid",
  paying: "Paying",
  no_winner: "No winner",
  voided: "Voided",
};

export default function PastRoundsScreen() {
  const rounds = useInfiniteQuery({
    queryKey: ["rounds", "pages"],
    queryFn: ({ pageParam }) => engine.rounds(pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => last.next_before ?? undefined,
  });
  const all = rounds.data?.pages.flatMap((p) => p.rounds) ?? [];
  return (
    <Screen title="Past rounds" kicker="Amounts only. Never who." testId="past-rounds" wide>
      {rounds.isPending ? (
        <Loading text="Loading rounds…" />
      ) : rounds.isError ? (
        <Offline onRetry={() => void rounds.refetch()} />
      ) : all.length === 0 ? (
        <Empty title="No rounds yet" text="The first one is being dug now." />
      ) : (
        <ul className="zk-rows">
          {all.map((r) => (
            <li key={r.round}>
              <Link
                to={`/rounds/${r.round}`}
                className="zk-roundrow grid grid-cols-[minmax(0,1fr)_auto_auto] sm:grid-cols-[minmax(170px,1fr)_120px_minmax(0,1fr)_auto_auto] items-center gap-4 px-5 py-4 text-white"
                data-testid="round-row"
              >
                <span className="min-w-0">
                  <span
                    className="block font-black text-2xl [overflow-wrap:anywhere]"
                    style={{ fontStretch: "112%" }}
                  >
                    Round {r.round}
                  </span>
                  {/* On phones the prize sits under the round. */}
                  <span className="sm:hidden text-mist flex items-center gap-2 font-mono text-sm">
                    <Icon name="chest" size={14} />
                    {formatZec(zat(r.prize))}
                  </span>
                </span>
                <span className="text-mist hidden sm:flex items-center gap-2">
                  <Icon name="tile" size={16} />
                  {r.treasure_tile === null ? "—" : `Tile ${r.treasure_tile + 1}`}
                </span>
                <span className="text-mist hidden sm:flex items-center gap-2 font-mono min-w-0">
                  <Icon name="chest" size={16} />
                  {formatZec(zat(r.prize))}
                </span>
                <span className={`zk-chip ${STATUS_CHIP[r.status]}`}>{STATUS_LABEL[r.status]}</span>
                <Icon name="arrow" size={20} />
              </Link>
            </li>
          ))}
        </ul>
      )}
      {rounds.hasNextPage && (
        <button
          type="button"
          className="btn"
          onClick={() => void rounds.fetchNextPage()}
          disabled={rounds.isFetchingNextPage}
        >
          More rounds
        </button>
      )}
      <p className="text-fog text-sm">
        Paid: winners were paid. No winner: nobody dug the treasure tile, so the prize went to the
        Deep Cache. Voided: the round closed too late to be safe from the draw, so every dig was
        refunded minus fees. Rounds nobody dug are not listed.
      </p>
    </Screen>
  );
}
