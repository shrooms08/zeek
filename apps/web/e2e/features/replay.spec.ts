/**
 * Replay mode with the engine stopped (as the browser sees it: every engine
 * request answered "offline", the live stream refused): the field plays
 * back real saved rounds, labelled as a replay; the offline checks run in
 * the browser and never tick what needs the chain; when the engine answers
 * again the field goes live by itself. Also the "very busy" variant (the
 * site's free requests used up: the page comes back instead of JSON).
 *
 * The build must hold the snapshot (scripts/snapshot-rounds.mjs; the smoke
 * web server writes it after building).
 */

import { expect, test, type Page } from "@playwright/test";

/** The engine as the app reaches it: the local engine, or the site's /api. */
const APP = process.env.ZEEK_E2E_APP;
const ENGINE =
  APP === undefined
    ? /^http:\/\/127\.0\.0\.1:8787\//
    : new RegExp(`^${APP.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&")}/api/`);

test.use({ viewport: { width: 1440, height: 1000 } });

async function engineDown(page: Page, as: "offline" | "busy") {
  await page.route(ENGINE, (route) =>
    as === "offline"
      ? route.fulfill({
          status: 503,
          contentType: "application/json",
          body: JSON.stringify({ error: "The Zeek engine is offline right now.", offline: true }),
        })
      : route.fulfill({ status: 200, contentType: "text/html", body: "<!doctype html><p>app</p>" }),
  );
  await page.routeWebSocket(/\/v1\/stream/, (ws) => {
    void ws.close();
  });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
});

test("engine off: a labelled replay of real rounds, then live again", async ({ page }) => {
  test.setTimeout(240_000);
  await engineDown(page, "offline");
  await page.goto("/");
  await expect(page.getByTestId("replay")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("replay-badge")).toHaveText("Replay");
  await expect(page.getByTestId("replay-status")).toHaveText(
    "Live play resumes when the Zeek engine is back.",
  );
  await expect(page.getByTestId("replay-panel")).toContainText("real testnet round");
  // It plays: the caption moves from the opening into the blocks.
  await expect(page.getByTestId("replay-caption")).toContainText(/Block \d+ of \d+/, {
    timeout: 30_000,
  });

  // Offline checks: performed parts pass; chain parts are not ticked.
  await page.getByTestId("replay-verify").click();
  await expect(page.getByTestId("offline-verify")).toBeVisible({ timeout: 120_000 });
  const status = (part: string) =>
    page.locator(`[data-testid=offline-part][data-part="${part}"]`).getAttribute("data-status");
  expect(await status("beacon.signature")).toBe("pass");
  expect(await status("beacon.draw")).toBe("pass");
  expect(await status("settlement.recompute")).toBe("pass");
  for (const p of ["beacon.binding", "digs.chain", "timing.chain", "payout.chain"]) {
    expect(await status(p)).toBe("needs_chain");
    await expect(page.locator(`[data-part="${p}"]`)).toContainText("Needs the live engine");
    await expect(page.locator(`[data-part="${p}"] svg`)).toHaveCount(0);
  }

  // The engine is back: the field goes live without a reload.
  await page.unroute(ENGINE);
  await page.getByTestId("replay-check-now").click();
  await expect(page.getByTestId("field-screen")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("replay")).toHaveCount(0);
});

test("site out of free requests: 'very busy', and the replay", async ({ page }) => {
  await engineDown(page, "busy");
  await page.goto("/");
  await expect(page.getByTestId("replay")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("replay-status")).toHaveText(
    "Zeek is very busy, try again shortly. Live play resumes when the Zeek engine is back.",
  );
});
