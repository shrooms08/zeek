/**
 * Live check on the deployed site: one dig from the suite's Pocket (Lite,
 * one tile, the smallest amount). The prize grows as soon as the dig lands
 * in the mempool, with "+X landing" under it; once it is mined the line
 * goes and the total stays (unless someone else digs meanwhile).
 *
 *   ZEEK_E2E_APP=https://playzeek.pages.dev pnpm exec playwright test --project=live e2e/live/09
 */

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { REPO, engine, expect, go, loadState, test, unlock } from "./support";

const OUT = path.join(REPO, "docs", "examples", "prize-landing", "live");
const log: Record<string, unknown> = { started: new Date().toISOString() };

async function picture(page: Page, name: string) {
  mkdirSync(OUT, { recursive: true });
  await page.screenshot({ path: path.join(OUT, `${name}.png`) });
}

async function prizeText(page: Page): Promise<string> {
  return ((await page.getByTestId("prize-amount").textContent()) ?? "").replace("ZEC", "").trim();
}

test("one dig: the prize grows when it lands, and stays when it is mined", async ({ page }) => {
  test.setTimeout(30 * 60_000);
  const { passphrase } = loadState();
  if (passphrase === undefined) throw new Error("run 01-create-fund first");
  // Dig only with time left in the round (the app refuses the last 4 blocks).
  for (;;) {
    const b = await engine<{ status: string; blocks_remaining: number }>("/v1/board");
    if (b.status === "open" && b.blocks_remaining >= 10) break;
    await page.waitForTimeout(20_000);
  }
  await unlock(page, passphrase);
  await go(page, "/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.getByTestId("mode-lite").click();
  await expect(page.getByTestId("prize-landing")).toHaveCount(0, { timeout: 120_000 });
  log.before = await prizeText(page);
  await picture(page, "1-before");

  const tile = page.locator('[data-selected="false"]').first();
  await tile.click();
  await page.getByTestId("amount-input").fill("0.001");
  await page.getByTestId("amount-input").press("Enter");
  await page.getByTestId("dig-button").click();
  await page.getByTestId("dig-send").click();
  await expect(page.getByTestId("dig-done")).toBeVisible({ timeout: 10 * 60_000 });
  log.sent = new Date().toISOString();
  await page.keyboard.press("Escape");
  await go(page, "/");

  // Landing: the prize already counts it.
  const line = page.getByTestId("prize-landing");
  await expect(line).toBeVisible({ timeout: 120_000 });
  log.landingSeen = new Date().toISOString();
  log.landingLine = await line.textContent();
  log.withLanding = await prizeText(page);
  expect(log.withLanding).not.toBe(log.before);
  await picture(page, "2-landing");

  // Mined: the line goes; the total stays.
  await expect(line).toHaveCount(0, { timeout: 15 * 60_000 });
  log.minedSeen = new Date().toISOString();
  log.afterMined = await prizeText(page);
  await picture(page, "3-mined");
  mkdirSync(OUT, { recursive: true });
  writeFileSync(path.join(OUT, "run.json"), JSON.stringify(log, null, 2) + "\n");
  expect(log.afterMined).toBe(log.withLanding);
});
