/** The main screen: one loop. */

import { useContext, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { LiveContext } from "../api/LiveProvider";
import { usePocket } from "../api/PocketProvider";
import { usePocketBalance } from "../api/pocketQueries";
import { useBoard, useConfig, usePending } from "../api/queries";
import { DigPanel } from "../components/DigPanel";
import { Field } from "../components/field/Field";
import { hasSeenOnboarding, Onboarding } from "../components/Onboarding";
import { RevealLine } from "../components/RevealLine";
import { BigStat, Empty, Loading, Offline } from "../components/ui";
import { autoPick } from "../lib/digPlan";
import { formatZec, zat } from "../lib/money";
import { formatClock } from "../lib/time";
import { useNow } from "../lib/useNow";
import { useDraft } from "../state/draft";
import { lastSeenReveal, loadMyDigs, markRevealSeen } from "../state/myDigs";

function cryptoRandom(): number {
  const b = new Uint32Array(1);
  crypto.getRandomValues(b);
  return (b[0] ?? 0) / 0x1_0000_0000;
}

export function FieldScreen() {
  const board = useBoard();
  const config = useConfig();
  const pending = usePending();
  const live = useContext(LiveContext);
  const draft = useDraft();
  const navigate = useNavigate();
  const now = useNow();
  const { wallet } = usePocket();
  const balance = usePocketBalance();
  const [onboarding, setOnboarding] = useState(() => !hasSeenOnboarding());

  // A round this browser dug in has closed: the reveal takes over once.
  useEffect(() => {
    if (board.data === undefined) return;
    const mine = loadMyDigs().map((d) => d.round);
    const seen = lastSeenReveal();
    const closed = mine.filter((r) => r < board.data.round && r > seen).sort((a, b) => b - a)[0];
    if (closed !== undefined) {
      markRevealSeen(closed);
      void navigate(`/reveal/${closed}`);
    }
  }, [board.data, navigate]);

  if (board.isPending) return <Loading />;
  if (board.isError) return <Offline onRetry={() => void board.refetch()} />;
  const b = board.data;
  const open = b.status === "open";
  const left = Math.max(0, b.estimated_close_time - now);
  const lastMinute = open && left <= 60;
  const stale = live !== "open";
  const minDig = zat(config.data?.params.min_dig_zat ?? 100_000);
  const campfire =
    config.data?.params.campfire !== undefined &&
    b.tip_height + 1 >= config.data.params.campfire.activation_height;
  const unlocked = wallet.kind === "unlocked" || wallet.kind === "syncing";
  const pocketLabel = unlocked
    ? `Zeek Pocket · ${balance.data === undefined ? "…" : formatZec(zat(balance.data.spendable_zat))}`
    : wallet.kind === "none"
      ? "Zeek Pocket · not set up"
      : "Zeek Pocket · locked";

  const doAutoPick = () => {
    const picked = autoPick(b.stages, Math.max(3, draft.tiles.length), cryptoRandom);
    draft.setTiles(picked);
  };
  const dig = () => {
    if (draft.tiles.length === 0 || !open) return;
    void navigate(
      unlocked
        ? "/dig/pocket"
        : wallet.kind === "none" || wallet.kind === "locked"
          ? "/wallet?then=dig"
          : "/dig/pocket",
    );
  };

  return (
    <main
      className="mx-auto w-full max-w-[1440px] px-4 py-4 grid gap-4 lg:grid-cols-[minmax(260px,1fr)_minmax(0,2.2fr)_minmax(320px,1.1fr)]"
      data-testid="field-screen"
    >
      <div className="flex flex-col gap-4 order-1">
        <BigStat
          icon="chest"
          label="Prize"
          value={formatZec(zat(b.prize_pool_zat))}
          unit="ZEC"
          gold
          stale={stale}
          note={b.prize_pool_zat === 0 ? "Grows with every dig" : undefined}
          testId="prize"
        />
        <BigStat
          icon="hourglass"
          label="Time left"
          value={open ? formatClock(left) : "Closed"}
          progress={lastMinute ? left / 60 : undefined}
          stale={stale}
          note={
            open
              ? `${b.blocks_remaining} blocks left in round ${b.round}`
              : `Round ${b.round} is closed`
          }
          testId="time-left"
        />
        <BigStat
          icon="mine"
          label="Deep Cache"
          value={formatZec(zat(b.cache))}
          unit="ZEC"
          size="m"
          stale={stale}
          testId="deep-cache"
        />
      </div>
      <div className="flex flex-col gap-4 order-3 lg:order-2">
        {b.digs === 0 && draft.tiles.length === 0 && open && (
          <Empty
            title="Tap tiles to dig"
            text="Nothing dug yet this round. Pick tiles, or Auto-pick 3 quiet ones."
          />
        )}
        <Field
          stages={b.stages}
          selected={draft.tiles}
          closed={!open}
          onToggle={draft.toggle}
          onAutoPick={doAutoPick}
          onDig={dig}
          onClear={draft.clear}
          label={`Round ${b.round} field`}
        />
        <RevealLine pending={pending.data ?? []} now={now} />
      </div>
      <div className="order-2 lg:order-3">
        <DigPanel
          tiles={draft.tiles}
          amountEach={draft.amountEach < minDig ? minDig : draft.amountEach}
          minDig={minDig}
          separate={draft.separate}
          message={draft.message}
          campfire={campfire}
          closed={!open}
          pocketLabel={pocketLabel}
          onAmount={draft.setAmountEach}
          onSeparate={draft.setSeparate}
          onMessage={draft.setMessage}
          onAutoPick={doAutoPick}
          onClear={draft.clear}
          onDig={dig}
        />
      </div>
      {onboarding && (
        <Onboarding
          onDone={() => {
            setOnboarding(false);
          }}
        />
      )}
    </main>
  );
}
