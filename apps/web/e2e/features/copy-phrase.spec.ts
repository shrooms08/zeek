/**
 * The recovery phrase's copy button: copies all 24 words, says "Copied",
 * shows the warning, keeps the write-it-down check, and 60 s later clears
 * the clipboard if it still holds the phrase. Creates a real Pocket in a
 * fresh browser profile (needs the engine, like the rest of smoke).
 */

import { randomBytes } from "node:crypto";
import { expect, test } from "@playwright/test";

test("copy the 24 words, then the clipboard is cleared after 60 s", async ({
  page,
  context,
  browserName,
}) => {
  test.setTimeout(240_000);
  // Only Chromium lets a test grant clipboard access; WebKit checks the
  // button and the warning.
  const canRead = browserName === "chromium";
  if (canRead) await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.clock.install();
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
  const pass = `zeek copy ${randomBytes(9).toString("base64url")}`;
  await page.goto("/wallet");
  await expect(page.getByTestId("wallet-create")).toBeVisible({ timeout: 60_000 });
  await page.getByTestId("create-pass").fill(pass);
  await page.getByTestId("create-again").fill(pass);
  await page.getByTestId("create-submit").click();
  const words = page.getByTestId("backup-words").locator("li span.font-mono");
  await expect(words).toHaveCount(24, { timeout: 120_000 });
  const phrase = (await words.allTextContents()).join(" ");

  await expect(page.getByTestId("copy-phrase-warning")).toHaveText(
    "Anyone who gets these words gets your coins. Clipboards can be read by other apps.",
  );
  await page.getByTestId("copy-phrase").click();
  if (!canRead) {
    await expect(page.getByTestId("copy-phrase")).toContainText(/Copied|Copy words/);
    await expect(page.getByTestId("backup-written")).toBeVisible();
    return;
  }
  await expect(page.getByTestId("copy-phrase")).toHaveText("Copied");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(phrase);
  // The write-it-down check is still the way on.
  await expect(page.getByTestId("backup-written")).toBeVisible();

  await page.clock.fastForward(61_000);
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()), { timeout: 10_000 })
    .toBe("");
});

test("the clear leaves something else that was copied since", async ({
  page,
  context,
  browserName,
}) => {
  test.skip(browserName !== "chromium", "needs a granted clipboard");
  test.setTimeout(240_000);
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.clock.install();
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
  const pass = `zeek copy ${randomBytes(9).toString("base64url")}`;
  await page.goto("/wallet");
  await expect(page.getByTestId("wallet-create")).toBeVisible({ timeout: 60_000 });
  await page.getByTestId("create-pass").fill(pass);
  await page.getByTestId("create-again").fill(pass);
  await page.getByTestId("create-submit").click();
  await expect(page.getByTestId("backup-words").locator("li")).toHaveCount(24, {
    timeout: 120_000,
  });
  await page.getByTestId("copy-phrase").click();
  await expect(page.getByTestId("copy-phrase")).toHaveText("Copied");
  await page.evaluate(() => navigator.clipboard.writeText("something else"));
  await page.clock.fastForward(61_000);
  await page.waitForTimeout(500);
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("something else");
});
