/**
 * Test-only page (e2e/overflow.spec.ts): the real screens, inside the real
 * providers, with long values no testnet round has produced yet — a Pocket
 * balance of 9999.99999999, a Deep Cache that broke open, digs at the
 * closing moment. ?state= picks one. The query cache is filled in advance
 * and the Pocket is a stand-in that only answers the reads these screens
 * make. Built only with ZEEK_FIXTURES=1 (dist-fixtures/); never deployed.
 */

import "@fontsource-variable/archivo";
import "@fontsource/archivo-black";
import "@fontsource/jetbrains-mono";
import "../styles/index.css";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Balance, HistoryEntry, Pocket } from "@zeek/pocket";
import { StrictMode, useEffect, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { LiveContext } from "../api/LiveProvider";
import { PocketContext, type PocketApi } from "../api/PocketProvider";
import type { Board, EngineConfig, PendingRound, RoundView } from "../api/types";
import { FaucetCard } from "../components/FaucetCard";
import { Shell } from "../components/Shell";
import DigPocketScreen from "../screens/DigPocketScreen";
import RevealScreen from "../screens/RevealScreen";
import WalletScreen from "../screens/WalletScreen";
import { AutoDigProvider } from "../state/autoDig";
import { DraftProvider, useDraft } from "../state/draft";
import { FieldFxProvider } from "../state/fieldFx";

const LONG = 1_234_567_890; // 12.3456789 ZEC
const CACHE = 12_345_670_000; // 123.4567 ZEC
const BIG = 999_999_999_999; // 9999.99999999 ZEC
const now = Math.floor(Date.now() / 1000);
const state = new URLSearchParams(window.location.search).get("state") ?? "check-your-dig";

const board = (blocksRemaining: number): Board => ({
  recent_spacing_secs: 25,
  tip_height: 4_600_000,
  tip_time: now,
  chain_tip: 4_600_000,
  round: 99999,
  status: "open",
  window: { start: 4_599_990, end: 4_600_014 },
  blocks_remaining: blocksRemaining,
  estimated_close_time: now + 3599,
  estimated_start_time: null,
  prize_pool_zat: LONG,
  gross_dug_zat: LONG,
  rules_version: 4,
  cache: CACHE,
  depths: Array.from({ length: 25 }, () => 60),
  stages: Array.from({ length: 25 }, () => 3),
  digs: 9999,
  landing: [],
});

const config: EngineConfig = {
  params: {
    network: "test",
    min_dig_zat: 100_000,
    dig_fee_zat: 5_000,
    marginal_fee_zat: 5_000,
    blocks_per_round: 24,
    target_spacing_secs: 25,
    reveal_delay_secs: 900,
    genesis_height: 4_499_347,
    rake_bps: 500,
    cache_odds: 625,
    campfire: { activation_height: 0, min_shout_zat: 50_000 },
  },
  digging_enabled: true,
  genesis_height: 4_499_347,
};

const balance = (b: Partial<Balance>): Balance => ({
  spendable_zat: BIG,
  pending_zat: LONG,
  orchard_pool_zat: 0,
  total_zat: BIG + LONG,
  scanned_height: 4_600_000,
  ready_in_blocks: 3,
  all_ready_in_blocks: 9,
  spendable_notes: [BIG],
  ...b,
});

const LONG_ADDRESS = `utest1${"q7x9k2m4p8w3r6t5".repeat(13)}`;
const history: HistoryEntry[] = [
  {
    type: "received",
    txid: "f".repeat(64),
    height: 4_599_999,
    value_zat: BIG,
    memo: `Zeek round 99999: treasure — ${"averyveryverylongwordwithoutanyspaces".repeat(3)}`,
    pool: "Ironwood",
    spent: false,
  },
];

const cacheWin: RoundView = {
  round: 99990,
  status: "settled",
  window: { start: 4_599_000, end: 4_599_024 },
  digs: 9999,
  depths: Array.from({ length: 25 }, () => 60),
  settled: {
    outcome: { outcome: "drawn", treasure_tile: 7, cache_triggered: true },
    treasure_tile: 7,
    cache_triggered: true,
    tile_totals: Array.from({ length: 25 }, () => LONG),
    drand_round: 99_999_999,
    beacon_time: now - 60,
    pot: LONG,
    rake: LONG,
    distributable: LONG,
    winners: 9999,
    payouts: [],
    previous_cache: CACHE,
    new_cache: CACHE,
    cache_in: LONG,
    cache_paid: CACHE,
    total_fees: LONG,
    dig_fees: LONG,
    total_paid: CACHE,
    forfeited: 0,
    rules_version: 4,
    plan_status: "paid",
    settled_at_height: 4_599_040,
    payout: null,
  },
};
const pending: PendingRound[] = [
  {
    round: 99998,
    window: { start: 1, end: 2 },
    status: "closing",
    drand_round: 99_999_999,
    beacon_time: now + 899,
  },
];

/** The stand-in Pocket: only the reads these screens make. */
function standIn(b: Balance): Pocket {
  const pocket = {
    getBalance: () => Promise.resolve(b),
    getAddress: () => Promise.resolve(LONG_ADDRESS),
    history: () => Promise.resolve(history),
    sync: () => Promise.resolve(b),
    prepareCoins: () => Promise.resolve(null),
    splitPlan: () => Promise.resolve(null),
    status: () =>
      Promise.resolve({
        exists: true,
        unlocked: true,
        address: LONG_ADDRESS,
        scannedHeight: 4_600_000,
        birthdayHeight: 4_500_000,
      }),
  };
  return pocket as unknown as Pocket;
}

function Draft({ tiles, separate }: { tiles: number[]; separate: boolean }) {
  const draft = useDraft();
  const { setTiles, setSeparate, setAmountEach } = draft;
  useEffect(() => {
    setTiles(tiles);
    setSeparate(separate);
    setAmountEach(11_100_000n);
  }, [tiles, separate, setTiles, setSeparate, setAmountEach]);
  return null;
}

function App({
  children,
  b,
  blocks,
  route,
}: {
  children: ReactNode;
  b: Balance;
  blocks: number;
  route: string;
}) {
  const client = new QueryClient({
    defaultOptions: { queries: { staleTime: Infinity, retry: false, refetchOnWindowFocus: false } },
  });
  client.setQueryData(["board"], board(blocks));
  client.setQueryData(["config"], config);
  client.setQueryData(["pending"], pending);
  client.setQueryData(["pocket", "balance"], b);
  client.setQueryData(["pocket", "history"], history);
  client.setQueryData(["round", 99990], cacheWin);
  const pocket: PocketApi = {
    pocket: () => Promise.resolve(standIn(b)),
    wallet: { kind: "unlocked" },
    dispatch: () => undefined,
    refresh: () => Promise.resolve(),
  };
  return (
    <QueryClientProvider client={client}>
      <LiveContext.Provider value="open">
        <PocketContext.Provider value={pocket}>
          <AutoDigProvider>
            <DraftProvider>
              <FieldFxProvider>
                <MemoryRouter initialEntries={[route]}>
                  <Shell>{children}</Shell>
                </MemoryRouter>
              </FieldFxProvider>
            </DraftProvider>
          </AutoDigProvider>
        </PocketContext.Provider>
      </LiveContext.Provider>
    </QueryClientProvider>
  );
}

const ALL = Array.from({ length: 25 }, (_, i) => i);
const screens: Record<string, () => ReactNode> = {
  "check-your-dig": () => (
    <App b={balance({})} blocks={143} route="/dig/pocket">
      <Draft tiles={ALL} separate={false} />
      <DigPocketScreen />
    </App>
  ),
  "check-your-dig-closing": () => (
    <App b={balance({})} blocks={2} route="/dig/pocket">
      <Draft tiles={ALL} separate={false} />
      <DigPocketScreen />
    </App>
  ),
  "notes-short": () => (
    <App b={balance({ spendable_notes: [BIG] })} blocks={143} route="/dig/pocket">
      <Draft tiles={[3, 9]} separate />
      <DigPocketScreen />
    </App>
  ),
  "change-on-its-way": () => (
    <App
      b={balance({
        spendable_zat: 0,
        pending_zat: BIG,
        spendable_notes: [],
        all_ready_in_blocks: 3,
      })}
      blocks={143}
      route="/dig/pocket"
    >
      <Draft tiles={ALL} separate={false} />
      <DigPocketScreen />
    </App>
  ),
  wallet: () => (
    <App b={balance({})} blocks={143} route="/wallet">
      <WalletScreen />
    </App>
  ),
  "faucet-coming": () => (
    <App b={balance({ spendable_zat: 0 })} blocks={143} route="/wallet">
      <div className="p-4 max-w-[560px]">
        <FaucetCard
          address={LONG_ADDRESS}
          spendable={0n}
          pending={BigInt(BIG)}
          readyInBlocks={99}
          spacing={25}
          sync={() => Promise.resolve()}
        />
      </div>
    </App>
  ),
  "deep-cache-reveal": () => (
    <App b={balance({})} blocks={143} route="/reveal/99990">
      <Routes>
        <Route path="/reveal/:round" element={<RevealScreen />} />
      </Routes>
    </App>
  ),
};

localStorage.setItem("zeek.seenOnboarding", "1");
localStorage.setItem("zeek-awaiting-coins", JSON.stringify({ from: "0", since: Date.now() }));
localStorage.setItem(
  "zeek.myDigs.v1",
  JSON.stringify([
    { round: 99990, tiles: [7], amountEach: "100000", via: "pocket", txids: [], at: 1 },
  ]),
);
const render = screens[state];
const root = document.getElementById("root");
if (root === null || render === undefined) throw new Error(`no fixture ${state}`);
createRoot(root).render(<StrictMode>{render()}</StrictMode>);
