/**
 * Lite / Pro on the dig panel: Lite by default, the choice remembered in
 * this browser, Pro's multi-select and exact totals, the Pro promise, and
 * auto-dig's gate (Pocket only, passphrase once, stops by itself). The
 * auto-dig run uses a fresh, empty Pocket, so it stops before any dig and
 * spends nothing; every stop rule is unit-tested in src/lib/autoDig.test.ts.
 */

import { randomBytes } from "node:crypto";
import { expect, test } from "@playwright/test";

test.use({ viewport: { width: 1440, height: 1000 } });

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
});

test("Lite by default; Pro is remembered in this browser", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  const panel = page.getByTestId("dig-panel");
  await expect(panel).toHaveAttribute("data-mode", "lite");
  await expect(page.getByTestId("pro-tools")).toHaveCount(0);
  await expect(page.getByTestId("mode-lite")).toHaveAttribute("aria-checked", "true");

  await page.getByTestId("mode-pro").click();
  await expect(panel).toHaveAttribute("data-mode", "pro");
  await page.reload();
  await expect(page.getByTestId("dig-panel")).toHaveAttribute("data-mode", "pro");
  await expect(page.getByTestId("pro-tools")).toBeVisible();

  await page.getByTestId("mode-lite").click();
  await page.reload();
  await expect(page.getByTestId("dig-panel")).toHaveAttribute("data-mode", "lite");
});

test("Pro: ALL, quick amounts and the exact total", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.getByTestId("mode-pro").click();
  await page.getByTestId("pro-all").click();
  await expect(page.getByTestId("tile-count")).toHaveText("25");
  // 25 tiles at the default 0.01; five payments (6+6+6+6+1): fees 4 × 0.00035 + 0.0001.
  await expect(page.getByTestId("pro-total")).toHaveText(
    "25 tiles × 0.01 + dig fees 0.0015 = 0.2515 ZEC",
  );
  await page.getByTestId("pro-add-0.001").click();
  await expect(page.getByTestId("pro-total")).toHaveText(
    "25 tiles × 0.011 + dig fees 0.0015 = 0.2765 ZEC",
  );
  await page.getByTestId("pro-add-0.1").click();
  await expect(page.getByTestId("amount-input")).toHaveValue("0.111");
  await expect(page.getByTestId("dig-button")).toContainText("25");
  // Without an unlocked Pocket: no MAX, no auto-dig.
  await expect(page.getByTestId("pro-max")).toBeDisabled();
  await expect(page.getByTestId("pro-rounds")).toHaveCount(0);
  await expect(page.getByText("Auto-dig works with Zeek Pocket only.")).toBeVisible();
  await expect(page.getByTestId("pro-promise")).toHaveText(
    "Pro shows more about you. It never shows more about anyone else.",
  );
  await expect(page.getByTestId("your-digs")).toContainText("No digs from this browser yet.");
  await page.getByTestId("pro-clear").click();
  await expect(page.getByTestId("tile-count")).toHaveText("0");
});

test("auto-dig: passphrase once, then it stops by itself when the Pocket can't cover a round", async ({
  page,
}) => {
  test.setTimeout(300_000);
  const pass = `zeek pro ${randomBytes(9).toString("base64url")}`;
  await page.goto("/wallet");
  await expect(page.getByTestId("wallet-create")).toBeVisible({ timeout: 60_000 });
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

  // In-app navigation: a reload would restart the Pocket worker and lock it.
  await page.evaluate(() => {
    window.history.pushState({}, "", "/");
    window.dispatchEvent(new PopStateEvent("popstate"));
  });
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.getByTestId("mode-pro").click();
  await page.locator('[data-tile="3"]').click();
  await page.getByTestId("pro-rounds").fill("3");
  await page.getByTestId("pro-autodig").click();
  const confirm = page.getByTestId("autodig-confirm");
  await expect(confirm).toBeVisible();
  // 1 tile at 0.01 + 0.0001 fee, three rounds.
  await expect(page.getByTestId("autodig-cap")).toHaveText("0.0303");

  await page.getByTestId("autodig-passphrase").fill("not the passphrase");
  await page.getByTestId("autodig-start").click();
  await expect(confirm.getByRole("alert")).toHaveText("That passphrase didn’t work.");
  await expect(page.getByTestId("autodig-card")).toHaveCount(0);

  await page.getByTestId("autodig-passphrase").fill(pass);
  await page.getByTestId("autodig-start").click();
  await expect(confirm).toHaveCount(0);
  await expect(page.getByTestId("autodig-card")).toBeVisible();
  // An empty Pocket: it stops before the first dig (when a round is open).
  await expect(page.getByTestId("autodig-ended")).toHaveText(
    "Stopped: your Pocket would not cover the next round.",
    { timeout: 240_000 },
  );
  await expect(page.getByTestId("autodig-counter")).toHaveText("0 / 3 rounds");
});
