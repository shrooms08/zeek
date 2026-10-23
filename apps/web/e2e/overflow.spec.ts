/**
 * No text overflows its box, and no page scrolls sideways, on every route
 * and state, at 360, 390, 768, 1024, 1440 and 1920 wide, with long values:
 * prize 12.3456789, Deep Cache 123.4567, every other amount 12.3456789,
 * round 99999, time left 59:59. The engine's real answers are rewritten
 * with those values (and the live stream kept silent, so they stay); the
 * Pocket's own states and the Deep Cache reveal come from fixtures.html,
 * a test-only page of the real components (never deployed).
 *
 * ZEEK_OVERFLOW_SHOTS=<dir> saves a screenshot of every state and width
 * that has a finding (or, with ZEEK_OVERFLOW_PAIRS=<file>, of those pairs).
 */

import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { engineRequests } from "./engine";
import { findOverflows } from "./overflow-check";

const WIDTHS = [360, 390, 768, 1024, 1440, 1920];
const APP = process.env.ZEEK_E2E_APP;
const ENGINE = engineRequests();
const SHOTS = process.env.ZEEK_OVERFLOW_SHOTS;
/** The test-only fixtures build (playwright.config.ts, second web server). */
const FIXTURES = "http://localhost:4175/fixtures.html";
const PAIRS = process.env.ZEEK_OVERFLOW_PAIRS;
const wanted: Set<string> | null =
  PAIRS !== undefined && existsSync(PAIRS)
    ? new Set(JSON.parse(readFileSync(PAIRS, "utf8")) as string[])
    : null;

/** 12.3456789 ZEC, and 123.4567 for the Deep Cache. */
const LONG_ZAT = 1_234_567_890;
const LONG_CACHE = 12_345_670_000;
const AMOUNT_KEY =
  /(_zat$|^pot$|^prize$|^total_paid$|^distributable$|^cache_paid$|^net$|^gross$|^rake$|^cache_in$|^total_fees$|^dig_fees$|^forfeited$|^treasure_tile_total$)/;
const CACHE_KEY = /^(cache|new_cache|previous_cache|cache_zat)$/;

function lengthen(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(lengthen);
  if (value === null || typeof value !== "object") return value;
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(value)) {
    if (typeof v === "number" && CACHE_KEY.test(k)) out[k] = LONG_CACHE;
    else if (typeof v === "number" && AMOUNT_KEY.test(k)) out[k] = LONG_ZAT;
    else out[k] = lengthen(v);
  }
  return out;
}

type RoundState = "open" | "closing" | "closed";

/** The engine's real answers, with long values; a silent live stream. */
async function longEngine(page: Page, round: RoundState = "open") {
  await page.routeWebSocket(/\/v1\/stream/, () => undefined);
  await page.route(ENGINE, async (route) => {
    const url = new URL(route.request().url());
    const response = await route.fetch();
    const type = response.headers()["content-type"] ?? "";
    if (!type.includes("application/json")) {
      await route.fulfill({ response });
      return;
    }
    let body = lengthen(await response.json()) as Record<string, unknown>;
    if (url.pathname.endsWith("/v1/board")) {
      const now = Math.floor(Date.now() / 1000);
      body = {
        ...body,
        round: 99999,
        digs: 9999,
        status: round === "closed" ? "closing" : "open",
        blocks_remaining: round === "open" ? 143 : round === "closing" ? 2 : 0,
        estimated_close_time: round === "open" ? now + 3599 : now,
      };
    }
    if (url.pathname.endsWith("/v1/rounds/pending")) {
      const now = Math.floor(Date.now() / 1000);
      const rounds = (body.rounds as Record<string, unknown>[] | undefined) ?? [];
      body = {
        ...body,
        rounds: [
          {
            round: 99998,
            window: { start: 1, end: 2 },
            status: "closed",
            drand_round: 99_999_999,
            beacon_time: now + 899,
          },
          ...rounds,
        ],
      };
    }
    await route.fulfill({ response, json: body });
  });
}

interface State {
  name: string;
  path: string;
  round?: RoundState;
  /** localStorage before load. */
  storage?: Record<string, string>;
  /** After load: open a menu, pick tiles… */
  act?: (page: Page) => Promise<void>;
  /** What must be visible before checking. */
  ready: string;
  /** Not served by the local preview (engine HTML). */
  publicOnly?: boolean;
  /** The test-only fixtures page (local runs only). */
  localOnly?: boolean;
}

const MY_WIN = JSON.stringify([
  { round: 251, tiles: [8], amountEach: "100000", via: "pocket", txids: [], at: 1 },
  { round: 245, tiles: [0], amountEach: "100000", via: "pocket", txids: [], at: 2 },
]);

const STATES: State[] = [
  { name: "field-lite", path: "/", ready: "field" },
  {
    name: "field-lite-picked",
    path: "/",
    ready: "field",
    act: async (p) => {
      await p.locator('[data-tile="3"]').click();
      await p.locator('[data-tile="9"]').click();
    },
  },
  {
    name: "field-pro",
    path: "/",
    ready: "field",
    // Pro's "Your digs" lists these; the reveals count as seen, so the field
    // does not open them.
    storage: { "zeek.digMode": "pro", "zeek.myDigs.v1": MY_WIN, "zeek.seenReveal": "999999" },
    act: async (p) => {
      await p.getByTestId("pro-all").click();
      await p.getByTestId("pro-add-0.1").click();
    },
  },
  {
    name: "field-closing",
    path: "/",
    round: "closing",
    ready: "field",
    act: async (p) => {
      await p.locator('[data-tile="3"]').click();
    },
  },
  {
    name: "field-closed",
    path: "/",
    round: "closed",
    ready: "field",
    act: async (p) => {
      await p.locator('[data-tile="3"]').click();
    },
  },
  {
    name: "menu",
    path: "/",
    ready: "field",
    act: async (p) => {
      await p.getByTestId("menu-button").click();
    },
  },
  { name: "how-to-play", path: "/how", ready: "how-to-play" },
  { name: "dig-other-wallet", path: "/dig/wallet", ready: "field" },
  { name: "rounds", path: "/rounds", ready: "round-row" },
  { name: "round-detail", path: "/rounds/251", ready: "round-detail" },
  {
    name: "reveal-win",
    path: "/reveal/251",
    storage: { "zeek.myDigs.v1": MY_WIN, "zeek.seenReveal": "999999" },
    ready: "result-card",
  },
  {
    name: "reveal-lose",
    path: "/reveal/245",
    storage: { "zeek.myDigs.v1": MY_WIN, "zeek.seenReveal": "999999" },
    ready: "result-card",
  },
  { name: "reveal-waiting", path: "/reveal/99998", ready: "reveal-screen" },
  { name: "verified-win", path: "/win/78ea3288c9ef4fea6c07ec9eeb26c9e0", ready: "win-screen" },
  {
    name: "share-page",
    path: "/share/win/78ea3288c9ef4fea6c07ec9eeb26c9e0",
    ready: "win-screen",
    publicOnly: true,
  },
  { name: "check", path: "/check/251", ready: "check-screen" },
  { name: "campfire", path: "/campfire", ready: "campfire-feed" },
  { name: "status", path: "/status", ready: "status-screen" },
  { name: "keys", path: "/keys", ready: "keys-screen" },
  { name: "wallet-create", path: "/wallet", ready: "wallet-create" },
  { name: "not-found", path: "/no-such-page", ready: "not-found" },
  // The Pocket's own states and the Deep Cache reveal (fixtures.html).
  ...(
    [
      ["check-your-dig", "dig-confirm"],
      ["check-your-dig-closing", "dig-round-closing"],
      ["notes-short", "dig-notes-short"],
      ["change-on-its-way", "dig-change-coming"],
      ["wallet", "wallet-home"],
      ["faucet-coming", "faucet-coming"],
      ["deep-cache-reveal", "result-card"],
    ] as const
  ).map(([name, ready]): State => ({
    name: `fixture-${name}`,
    path: `${FIXTURES}?state=${name}`,
    ready,
    localOnly: true,
  })),
];

async function check(page: Page, name: string, width: number, browserName: string) {
  await page.waitForTimeout(400);
  const found = await findOverflows(page);
  const key = `${name}-${width}`;
  const report = process.env.ZEEK_OVERFLOW_REPORT;
  if (report !== undefined && found.length > 0) {
    // Written at once: a failed test restarts the worker.
    mkdirSync(path.dirname(report), { recursive: true });
    appendFileSync(report, JSON.stringify({ browser: browserName, key, found }) + "\n");
  }
  const shoot = SHOTS !== undefined && (wanted === null ? found.length > 0 : wanted.has(key));
  if (shoot && browserName === "chromium") {
    mkdirSync(SHOTS, { recursive: true });
    await page.screenshot({ path: path.join(SHOTS, `${key}.png`) });
  }
  return found;
}

test.describe("no text overflows its box", () => {
  test.use({ reducedMotion: "reduce" });

  for (const state of STATES) {
    test(state.name, async ({ page, browserName }) => {
      test.skip(
        state.publicOnly === true && APP === undefined,
        "served by the engine, not the preview",
      );
      test.setTimeout(240_000);
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.addInitScript((storage) => {
        localStorage.setItem("zeek.seenOnboarding", "1");
        for (const [k, v] of Object.entries(storage)) localStorage.setItem(k, v);
      }, state.storage ?? {});
      await longEngine(page, state.round ?? "open");
      const problems: string[] = [];
      for (const width of WIDTHS) {
        await page.setViewportSize({ width, height: width < 768 ? 844 : 900 });
        await page.goto(state.path);
        await expect(page.getByTestId(state.ready).first()).toBeVisible({ timeout: 60_000 });
        if (state.act !== undefined) await state.act(page);
        const found = await check(page, state.name, width, browserName);
        for (const f of found)
          problems.push(`${width}px ${f.kind} ${f.what} "${f.text}" (${f.detail})`);
      }
      expect(problems, problems.join("\n")).toEqual([]);
    });
  }

  // Fitting must not mean collapsing: with room to spare, the long prize
  // keeps a big size (a stale measurement once shrank it to the minimum).
  test("the long prize stays big where there is room", async ({ page }) => {
    test.setTimeout(120_000);
    await page.addInitScript(() => {
      localStorage.setItem("zeek.seenOnboarding", "1");
    });
    await longEngine(page);
    for (const width of [1024, 1440, 1920]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto("/");
      const prize = page.getByTestId("prize-amount");
      await expect(prize).toContainText("12.3456");
      await page.waitForTimeout(1500);
      const px = await prize
        .locator(".zk-fit-inner")
        .evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
      expect(px, `${width}px`).toBeGreaterThanOrEqual(40);
    }
  });
});
