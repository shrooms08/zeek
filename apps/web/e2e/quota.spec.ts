/**
 * Cloudflare quota use: how many requests reach Pages Functions (/api,
 * /share, /relay, and the live WebSocket) from one open tab, visible and
 * hidden, and from one share page view. Static files are listed apart: they
 * do not invoke Functions (see public/_routes.json).
 *
 *   ZEEK_E2E_APP=https://playzeek.pages.dev ZEEK_QUOTA_MINUTES=20 \
 *     pnpm exec playwright test --project=quota
 *
 * Hidden is emulated (`document.visibilityState` and `visibilitychange`),
 * because a headless page is always visible. Results go to
 * test-results/quota-<label>.json and the console.
 */

import { writeFileSync, mkdirSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";

const MINUTES = Number(process.env.ZEEK_QUOTA_MINUTES ?? "20");
const LABEL = process.env.ZEEK_QUOTA_LABEL ?? "run";
const FUNCTION_PATH = /^\/(api|share|relay)(\/|$)/;

interface Tally {
  functions: Record<string, number>;
  sockets: number;
  statics: number;
}

function tally(page: Page, origin: string): Tally {
  const t: Tally = { functions: {}, sockets: 0, statics: 0 };
  page.on("request", (r) => {
    const url = new URL(r.url());
    if (url.origin !== origin) return;
    if (FUNCTION_PATH.test(url.pathname)) {
      // Group by route shape, not by round number.
      const key = `${r.method()} ${url.pathname.replace(/\/[0-9]+(?=\/|$|\.)/g, "/N")}`;
      t.functions[key] = (t.functions[key] ?? 0) + 1;
    } else {
      t.statics += 1;
    }
  });
  page.on("websocket", (ws) => {
    if (new URL(ws.url()).pathname.startsWith("/api/")) t.sockets += 1;
  });
  return t;
}

const total = (t: Tally) => Object.values(t.functions).reduce((s, n) => s + n, 0) + t.sockets;

async function hide(page: Page, hidden: boolean) {
  await page.evaluate((h) => {
    (window as unknown as { __zkHidden: boolean }).__zkHidden = h;
    document.dispatchEvent(new Event("visibilitychange"));
  }, hidden);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
    const w = window as unknown as { __zkHidden: boolean };
    w.__zkHidden = false;
    Object.defineProperty(document, "visibilityState", {
      get: () => (w.__zkHidden ? "hidden" : "visible"),
    });
    Object.defineProperty(document, "hidden", { get: () => w.__zkHidden });
  });
});

function report(name: string, minutes: number, t: Tally) {
  const perHour = (total(t) * 60) / minutes;
  const out = { name, minutes, functionRequests: total(t), perHour, ...t };
  mkdirSync("test-results", { recursive: true });
  writeFileSync(`test-results/quota-${LABEL}-${name}.json`, JSON.stringify(out, null, 2));
  console.log(JSON.stringify(out));
}

test("one tab: requests per hour, visible then hidden", async ({ page, baseURL }) => {
  test.setTimeout((2 * MINUTES + 5) * 60_000);
  const origin = new URL(baseURL ?? "").origin;
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  const visible = tally(page, origin);
  await page.waitForTimeout(MINUTES * 60_000);
  page.removeAllListeners("request");
  page.removeAllListeners("websocket");
  report("visible", MINUTES, visible);

  const hidden = tally(page, origin);
  await hide(page, true);
  await page.waitForTimeout(MINUTES * 60_000);
  report("hidden", MINUTES, hidden);
});

test("one share page view (a person following a link)", async ({ page, baseURL }) => {
  test.setTimeout(180_000);
  const origin = new URL(baseURL ?? "").origin;
  const proof = process.env.ZEEK_QUOTA_PROOF;
  test.skip(proof === undefined, "set ZEEK_QUOTA_PROOF to a proof id");
  const t = tally(page, origin);
  await page.goto(`/share/win/${proof ?? ""}`);
  await expect(page.getByTestId("win-verdict")).toBeVisible({ timeout: 60_000 });
  await page.waitForTimeout(30_000);
  report("share-view", 0.5, t);
});
