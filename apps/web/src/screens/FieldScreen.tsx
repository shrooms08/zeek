/**
 * The main screen: one loop. Desktop (design 1A/1B): the island fills the
 * screen, Prize and Time left float top-left, the dig panel on the right,
 * the reveal line bottom-left, the Deep Cache by the mine. Mobile (1C–1G):
 * Prize and Time left side by side, the field as the largest element, a
 * compact dig sheet fixed at the bottom. Dig modals and How to play open
 * over it (child routes), so the field keeps living underneath.
 */

import { motion } from "framer-motion";
import { Suspense, useCallback, useContext, useEffect, useState } from "react";
import { Outlet, useLocation, useNavigate } from "react-router-dom";
import { Mine, Zeb } from "../art/Characters";
import { EngineError } from "../api/engine";
import { LiveContext } from "../api/LiveProvider";
import { usePocket } from "../api/PocketProvider";
import { usePocketBalance } from "../api/pocketQueries";
import { useBlockSpacing, useBoard, useConfig, usePending } from "../api/queries";
import { DigPanel } from "../components/DigPanel";
import { Field } from "../components/field/Field";
import { FitAmount } from "../components/FitAmount";
import { Icon } from "../components/Icon";
import { hasSeenOnboarding, Onboarding } from "../components/Onboarding";
import { PictoBubble } from "../components/PictoBubble";
import { ReplayMode } from "../components/ReplayMode";
import { RevealLine } from "../components/RevealLine";
import { TimeLeft } from "../components/TimeLeft";
import { Loading, Offline } from "../components/ui";
import { autoPick, cacheStage } from "../lib/digPlan";
import { livePrize } from "../lib/prize";
import { liteTiles, loadDigMode, saveDigMode, tapTile, type DigMode } from "../lib/digMode";
import { formatZec, zat } from "../lib/money";
import { roundAcceptsDigs } from "../lib/readiness";
import { useMedia } from "../lib/useMedia";
import { prefersReducedMotion, useNow } from "../lib/useNow";
import { CountUp, PULSE } from "../motion/CountUp";
import { bootProgress, bootReady } from "../boot";
import { useLoop } from "../sound/cues";
import { sound } from "../sound/sound";
import { useDraft } from "../state/draft";
import { useFieldFx } from "../state/fieldFx";
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
  const fx = useFieldFx();
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const now = useNow();
  const { wallet } = usePocket();
  const balance = usePocketBalance();
  const desktop = useMedia("(min-width: 1024px)");
  const wide = useMedia("(min-width: 640px)");
  const [onboarding, setOnboarding] = useState(() => !hasSeenOnboarding());
  const [prizePulse, setPrizePulse] = useState(0);
  const [mode, setMode] = useState<DigMode>(loadDigMode);
  // Switching to Lite dropped tiles: say so until the pick changes.
  const [liteTrimmed, setLiteTrimmed] = useState(false);
  const spacing = useBlockSpacing();

  // A round this browser dug in has closed: the reveal takes over once.
  useEffect(() => {
    if (board.data === undefined || pathname !== "/") return;
    const mine = loadMyDigs().map((d) => d.round);
    const seen = lastSeenReveal();
    const closed = mine.filter((r) => r < board.data.round && r > seen).sort((a, b) => b - a)[0];
    if (closed !== undefined) {
      markRevealSeen(closed);
      void navigate(`/reveal/${closed}`);
    }
  }, [board.data, navigate, pathname]);

  // The load screen: the round has loaded, and with it the field renders
  // (its prompt shows), or the engine does not answer and Replay or the
  // offline message renders (it leaves by itself, without a tap).
  const roundLoaded = board.data !== undefined;
  useEffect(() => {
    if (roundLoaded) bootProgress(60);
    if (roundLoaded) bootReady();
    else if (board.isError) bootReady({ silent: true });
  }, [roundLoaded, board.isError]);

  // Waiting for a reveal: a round this browser dug in has closed and its
  // result is not in yet ("Round N reveals in m:ss").
  const myRounds = new Set(loadMyDigs().map((d) => d.round));
  useLoop(
    "waiting-loop",
    (pending.data ?? []).some((p) => myRounds.has(p.round)),
  );

  const onGrow = useCallback(() => {
    setPrizePulse((n) => n + 1);
  }, []);

  if (board.isPending) return <Loading />;
  if (board.isError) {
    const e = board.error;
    // The engine is off or the site is out of free requests: play back
    // saved rounds until it answers again.
    if (e instanceof EngineError && (e.kind === "offline" || e.kind === "busy")) {
      return (
        <main className="zk-field-replay" data-testid="field-screen-replay">
          <ReplayMode reason={e.kind} />
        </main>
      );
    }
    return <Offline onRetry={() => void board.refetch()} />;
  }
  const b = board.data;
  const open = b.status === "open";
  const left = Math.max(0, b.estimated_close_time - now);
  const stale = live !== "open";
  const minDig = zat(config.data?.params.min_dig_zat ?? 100_000);
  const campfire =
    config.data?.params.campfire !== undefined &&
    b.tip_height + 1 >= config.data.params.campfire.activation_height;
  const unlocked = wallet.kind === "unlocked" || wallet.kind === "syncing";
  const pocketLabel = unlocked
    ? `Zeek Pocket · ${balance.data === undefined ? "…" : formatZec(zat(balance.data.spendable_zat))}`
    : wallet.kind === "none"
      ? "Watching · no wallet needed to watch"
      : "Zeek Pocket · locked";
  // Mainnet readiness: the engine withholds the vault and accepts no digs.
  const diggingOff = config.data?.digging_enabled === false;
  const cache = zat(b.cache);
  const stage = cacheStage(cache);
  const landing = (b.landing ?? []).filter((d) => d.round === b.round);
  // Mined prize plus this round's landing digs (src/lib/prize.ts).
  const prizeNow = livePrize(b, config.data?.params);
  const empty = b.digs === 0 && landing.length === 0 && draft.tiles.length === 0 && open;
  const modal = pathname !== "/";

  // "While you wait: dig round N": straight to the tiles.
  const focusField = () => {
    const el = document.querySelector<SVGElement>('[data-testid="field"]');
    el?.scrollIntoView({ block: "center", behavior: prefersReducedMotion() ? "auto" : "smooth" });
    el?.focus({ preventScroll: true });
  };
  // Lite picks one quiet tile; Pro at least 3 (or as many as are picked).
  const doAutoPick = () => {
    const count = mode === "lite" ? 1 : Math.max(3, draft.tiles.length);
    draft.setTiles(autoPick(b.stages, count, cryptoRandom));
    setLiteTrimmed(false);
  };
  const toggle = (i: number) => {
    sound.play("tile-pick");
    draft.setTiles(tapTile(mode, draft.tiles, i));
    setLiteTrimmed(false);
  };
  // Digs stop 4 blocks before the round ends: a payment sent later might
  // not be mined inside the window (the Pocket refuses it too).
  const accepting = roundAcceptsDigs(b);
  const dig = () => {
    if (draft.tiles.length === 0 || !accepting || diggingOff) return;
    void navigate(
      unlocked
        ? "/dig/pocket"
        : wallet.kind === "none" || wallet.kind === "locked"
          ? "/wallet?then=dig"
          : "/dig/pocket",
    );
  };

  const prize = (
    <motion.section
      key={prizePulse}
      className={`zk-stat ${desktop ? "px-6 py-5" : "p-3"}`}
      aria-label="Prize"
      data-testid="prize"
      animate={prizePulse > 0 && !prefersReducedMotion() ? PULSE.animate : { scale: 1 }}
      transition={PULSE.transition}
    >
      <div className="label flex items-center gap-2">
        <Icon name="chest" size={desktop ? 22 : 16} />
        Prize
        {stale && (
          <span className="text-fog normal-case tracking-normal font-normal">· reconnecting</span>
        )}
      </div>
      <FitAmount
        size={desktop ? 68 : 40}
        min={14}
        unit="ZEC"
        unitScale={desktop ? 0.3 : 0.4}
        color="var(--color-gold)"
        testId="prize-amount"
      >
        <CountUp value={prizeNow.total} onGrow={onGrow} />
      </FitAmount>
      {prizeNow.landing > 0n ? (
        <p className="text-fog text-sm mt-1 min-w-0" data-testid="prize-landing">
          +{formatZec(prizeNow.landing)} landing
        </p>
      ) : (
        prizeNow.total === 0n &&
        desktop && <p className="text-fog text-sm mt-1">Grows with every dig</p>
      )}
    </motion.section>
  );

  const time = (
    <TimeLeft
      secondsLeft={left}
      open={open}
      stale={stale}
      compact={!desktop}
      note={
        open
          ? `${b.blocks_remaining} blocks left in round ${b.round}`
          : `Round ${b.round} is closed`
      }
    />
  );

  const cacheBadge = (
    <div
      className="zk-badge"
      data-testid="deep-cache"
      aria-label={`Deep Cache ${formatZec(cache)} ZEC`}
    >
      <div className="label text-fog flex items-center gap-1.5" style={{ fontSize: 11 }}>
        <Icon name="mine" size={14} />
        Deep Cache
      </div>
      <FitAmount size={desktop ? 26 : 18} min={11}>
        <CountUp value={cache} />
      </FitAmount>
    </div>
  );

  const field = (
    <Field
      stages={b.stages}
      selected={draft.tiles}
      closed={!open}
      onToggle={toggle}
      onAutoPick={doAutoPick}
      onDig={dig}
      onClear={draft.clear}
      label={`Round ${b.round} field`}
      world={wide}
      mineStage={stage}
      cacheLabel={formatZec(cache)}
      interactive={!fx.sealing && !modal}
      landing={landing}
      flight={fx.flight}
      onCampfire={() => void navigate("/campfire")}
    />
  );

  const panel = (
    <DigPanel
      tiles={draft.tiles}
      amountEach={draft.amountEach < minDig ? minDig : draft.amountEach}
      minDig={minDig}
      separate={draft.separate}
      message={draft.message}
      campfire={campfire}
      closed={!accepting || diggingOff}
      closedReason={
        diggingOff
          ? { label: "Digs not open yet" }
          : open && !accepting
            ? {
                label: "Round closing",
                detail:
                  b.blocks_remaining === 0
                    ? "Next round any moment"
                    : `Next round in ~${Math.max(1, Math.round(b.blocks_remaining * spacing))} s`,
              }
            : undefined
      }
      pocketLabel={pocketLabel}
      compact={!desktop}
      onAmount={draft.setAmountEach}
      onSeparate={draft.setSeparate}
      onMessage={draft.setMessage}
      onAutoPick={doAutoPick}
      onClear={draft.clear}
      onDig={dig}
      mode={mode}
      onMode={(m) => {
        setMode(m);
        saveDigMode(m);
        if (m === "lite") {
          // One tile is one payment: nothing to dig separately.
          const kept = liteTiles(draft.tiles);
          draft.setTiles(kept.tiles);
          draft.setSeparate(false);
          setLiteTrimmed(kept.trimmed);
        } else {
          setLiteTrimmed(false);
        }
      }}
      liteTrimmed={liteTrimmed && mode === "lite"}
      onTiles={draft.setTiles}
      spendable={
        unlocked && balance.data !== undefined ? zat(balance.data.spendable_zat) : undefined
      }
      roundMs={
        (config.data?.params.blocks_per_round ?? 24) *
        (config.data?.params.target_spacing_secs ?? 25) *
        1000
      }
    />
  );

  return (
    <main className={desktop ? "zk-field-desktop" : "zk-field-mobile"} data-testid="field-screen">
      {desktop ? (
        <div className="relative mx-auto h-full w-full max-w-[1440px]">
          <div className="zk-field-stage">
            {field}
            <div className="absolute" style={{ left: "69%", top: "17%" }}>
              {cacheBadge}
            </div>
            {empty && <EmptyGuide />}
          </div>
          <div className="absolute left-10 top-8 z-10 flex w-[340px] flex-col gap-5">
            {prize}
            {time}
          </div>
          <div className="absolute right-10 top-8 z-10 w-[360px]">{panel}</div>
          <div className="absolute bottom-9 left-10 z-10 w-[360px]">
            <RevealLine
              pending={pending.data ?? []}
              now={now}
              openRound={open && !diggingOff ? b.round : undefined}
              onDigNow={focusField}
            />
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-3 px-4 pt-4">
          <div className="grid grid-cols-2 gap-3">
            {prize}
            {time}
          </div>
          <div className="flex items-center justify-end gap-2">
            <Mine stage={stage} width={44} live={false} />
            {cacheBadge}
          </div>
          <div className="relative">
            {field}
            {empty && !wide && (
              <p className="text-center text-fog" aria-hidden="true">
                Nothing dug yet. Tap tiles, or Auto-pick.
              </p>
            )}
          </div>
          <RevealLine
            pending={pending.data ?? []}
            now={now}
            openRound={open && !diggingOff ? b.round : undefined}
            onDigNow={focusField}
          />
          <div className="zk-sheet">{panel}</div>
        </div>
      )}
      {onboarding && (
        <Onboarding
          onDone={() => {
            setOnboarding(false);
          }}
        />
      )}
      <Suspense fallback={null}>
        <Outlet />
      </Suspense>
    </main>
  );
}

/** Empty round: Zeb on the island explains the game in pictures. */
function EmptyGuide() {
  return (
    <div
      className="pointer-events-none absolute"
      style={{ left: "31%", top: "28%" }}
      data-testid="empty"
    >
      <div className="flex items-start gap-3">
        <Zeb pose="idle" width={84} label={null} />
        <div>
          <PictoBubble
            icons={["shovel", "tile", "nobody"]}
            joiners={["+", "="]}
            tail="left"
            label="Dig a tile. Nobody sees who."
            reduce={prefersReducedMotion()}
          />
        </div>
      </div>
    </div>
  );
}
