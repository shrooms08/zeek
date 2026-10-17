/**
 * Never offer a dig into a closing round. The board is the engine's real
 * one with blocks_remaining forced to 2 (the moment a round can no longer
 * safely take a new transaction): Dig is disabled with the reason, and the
 * "Check your dig" step offers the next round instead.
 */

import { randomBytes } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";

test.use({ viewport: { width: 1440, height: 1000 } });

async function closingBoard(page: Page) {
  // A silent stream: no live board events replace the closing board.
  await page.routeWebSocket(/\/v1\/stream/, () => undefined);
  await page.route(/\/v1\/board$/, async (route) => {
    const response = await route.fetch();
    const board = (await response.json()) as Record<string, unknown>;
    await route.fulfill({ response, json: { ...board, status: "open", blocks_remaining: 2 } });
  });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
});

test("the field: Dig is off with fewer than 4 blocks left", async ({ page }) => {
  await closingBoard(page);
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.locator('[data-tile="4"]').click();
  const dig = page.getByTestId("dig-button");
  await expect(dig).toBeDisabled();
  await expect(dig).toContainText("Round closing");
  await expect(page.getByTestId("dig-button-detail")).toHaveText(/^Next round in ~\d+ s$/);
});

test("Check your dig: offers the next round, never the closing one", async ({ page }) => {
  test.setTimeout(240_000);
  // An unlocked Pocket (empty is fine: the round check comes first).
  const pass = `zeek closing ${randomBytes(9).toString("base64url")}`;
  // Before the first load: the app's one live stream must be the silent one.
  await closingBoard(page);
  await page.goto("/wallet");
  await page.getByTestId("create-pass").fill(pass);
  await page.getByTestId("create-again").fill(pass);
  await page.getByTestId("create-submit").click();
  const words = page.getByTestId("backup-words").locator("li span.font-mono");
  await expect(words).toHaveCount(24, { timeout: 120_000 });
  const list = await words.allTextContents();
  await page.getByTestId("backup-written").click();
  for (const input of await page.locator("[data-testid^=check-word-]").all()) {
    const pos = Number((await input.getAttribute("data-testid"))?.replace("check-word-", ""));
    await input.fill(list[pos - 1] ?? "");
  }
  await page.getByTestId("check-words").click();
  await expect(page.getByTestId("wallet-home")).toBeVisible({ timeout: 60_000 });

  await page.evaluate(() => {
    window.history.pushState({}, "", "/dig/pocket");
    window.dispatchEvent(new PopStateEvent("popstate"));
  });
  const closing = page.getByTestId("dig-round-closing");
  await expect(closing).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("dig-confirm")).toHaveCount(0);
  // The closing round and the offer, read together (a real round change
  // may happen during the test).
  const text = (await closing.textContent()) ?? "";
  const round = Number(/Round (\d+) is closing/.exec(text)?.[1]);
  expect(text).toContain(`Dig in round ${round + 1} instead`);
  await page.getByTestId("dig-next-round-button").click();
  await expect(page.getByTestId("dig-next-round")).toContainText(
    /Your dig goes into round \d+ as soon as it opens\./,
  );
});
