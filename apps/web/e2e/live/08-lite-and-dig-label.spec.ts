/**
 * Live check on the deployed site: Lite digs one tile, and "Check your dig"
 * offers a plain "Dig" button. Nothing is sent: the modal is closed without
 * pressing it. Uses the suite's own Pocket (01-create-fund).
 *
 *   ZEEK_E2E_APP=https://playzeek.pages.dev pnpm exec playwright test --project=live e2e/live/08
 */

import { expect, go, loadState, shot, test, unlock } from "./support";

test("Lite: one tile; Check your dig: Dig", async ({ page }) => {
  test.setTimeout(10 * 60_000);
  const { passphrase } = loadState();
  if (passphrase === undefined) throw new Error("run 01-create-fund first");
  await unlock(page, passphrase);
  await go(page, "/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.getByTestId("mode-lite").click();
  await expect(page.getByTestId("mode-line")).toHaveText("One tile, quick dig.");

  const tile = (n: number) => page.locator(`[data-tile="${n}"]`);
  if ((await tile(9).getAttribute("data-selected")) === "true") await tile(9).click();
  await tile(3).click();
  await tile(9).click();
  await expect(page.getByTestId("tile-count")).toHaveText("1");
  await expect(tile(9)).toHaveAttribute("data-selected", "true");
  await expect(tile(3)).toHaveAttribute("data-selected", "false");
  await expect(page.getByTestId("separate")).toHaveCount(0);
  await shot(page, "live-lite-one-tile-1440");

  // The smallest dig, so the suite's Pocket covers it.
  await page.getByTestId("amount-input").fill("0.001");
  await page.getByTestId("amount-input").press("Enter");
  await page.getByTestId("dig-button").click();
  const send = page.getByTestId("dig-send");
  await expect(send).toBeVisible({ timeout: 120_000 });
  await expect(send).toHaveText("Dig");
  // Let the modal finish fading in before the picture.
  await page.waitForTimeout(800);
  await shot(page, "live-check-your-dig-1440");
  // Close without digging.
  await page.keyboard.press("Escape");
  await expect(send).toHaveCount(0);
});
