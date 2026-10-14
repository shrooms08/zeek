/** Past rounds: amounts only, never who. */

import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { engine } from "../api/engine";
import type { RoundListStatus } from "../api/types";
import { Empty, Loading, Offline, Screen } from "../components/ui";
import { formatZec, zat } from "../lib/money";

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
    <Screen title="Past rounds" testId="past-rounds">
      {rounds.isPending ? (
        <Loading text="Loading rounds…" />
      ) : rounds.isError ? (
        <Offline onRetry={() => void rounds.refetch()} />
      ) : all.length === 0 ? (
        <Empty title="No rounds yet" text="The first one is being dug now." />
      ) : (
        <ul className="flex flex-col">
          {all.map((r) => (
            <li key={r.round}>
              <Link
                to={`/rounds/${r.round}`}
                className="grid grid-cols-[1fr_auto_auto] sm:grid-cols-[1fr_120px_160px_120px] items-center gap-3 p-3 border-b hair text-white"
                data-testid="round-row"
              >
                <span className="font-extrabold">Round {r.round}</span>
                <span className="text-fog hidden sm:inline">
                  {r.treasure_tile === null ? "—" : `Tile ${r.treasure_tile + 1}`}
                </span>
                <span className="numeral text-right">{formatZec(zat(r.prize))}</span>
                <span className="label text-right">{STATUS_LABEL[r.status]}</span>
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
