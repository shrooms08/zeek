/**
 * The wait for a reveal: on a closed round's screen, a progress bar to the
 * reveal, why the wait exists, "While you wait: dig round N" to the open
 * round, and an opt-in notification. The same call to action sits in the
 * field's "Round X reveals in" pill. Uses a real closed round (one is
 * almost always waiting: rounds close every ~10 minutes and reveal ~15
 * minutes later).
 */

import { expect, test } from "@playwright/test";

const ENGINE =
  process.env.ZEEK_E2E_APP === undefined
    ? (process.env.VITE_ZEEK_ENGINE_URL ?? "http://127.0.0.1:8787")
    : `${process.env.ZEEK_E2E_APP}/api`;

test.use({ viewport: { width: 1440, height: 1000 } });

async function pendingRound(): Promise<number> {
  for (let tries = 0; tries < 60; tries += 1) {
    const r = await fetch(`${ENGINE}/v1/rounds/pending`);
    const body = (await r.json()) as { rounds: { round: number; beacon_time: number }[] };
    const now = Date.now() / 1000;
    const waiting = body.rounds.find((p) => p.beacon_time - now > 60);
    if (waiting !== undefined) return waiting.round;
    await new Promise((resolve) => setTimeout(resolve, 10_000));
  }
  throw new Error("no closed round waiting for its reveal");
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
});

test("a closed round: progress, why, and 'While you wait: dig round N'", async ({ page }) => {
  test.setTimeout(720_000);
  const n = await pendingRound();
  await page.goto(`/reveal/${n}`);
  await expect(page.getByTestId("reveal-waiting")).toBeVisible({ timeout: 60_000 });
  const bar = page.getByTestId("reveal-progress");
  await expect(bar).toBeVisible();
  const value = Number(await bar.getAttribute("aria-valuenow"));
  expect(value).toBeGreaterThanOrEqual(0);
  expect(value).toBeLessThanOrEqual(100);
  await expect(page.getByTestId("reveal-why")).toContainText("Why the wait");
  // The offer depends on what this browser allows.
  const state = await page.evaluate(() =>
    "Notification" in window ? Notification.permission : "unsupported",
  );
  if (state === "default") {
    await expect(page.getByTestId("reveal-notify")).toHaveText("Tell me when it lands");
  } else if (state === "denied") {
    await expect(page.getByTestId("reveal-notify-blocked")).toBeVisible();
  } else {
    await expect(page.getByTestId("reveal-notify")).toHaveCount(0);
  }
  const dig = page.getByTestId("reveal-dig-next");
  await expect(dig).toContainText(/While you wait: dig round \d+/);
  await dig.click();
  await expect(page.getByTestId("field")).toBeVisible();
});

test("the field's reveal pill: 'While you wait: dig round N' goes to the tiles", async ({
  page,
}) => {
  test.setTimeout(720_000);
  await pendingRound();
  await page.goto("/");
  const cta = page.getByTestId("reveal-line-dig");
  await expect(cta).toBeVisible({ timeout: 60_000 });
  await cta.click();
  await expect(page.getByTestId("field")).toBeFocused();
});

test("with notifications allowed, it says it will tell you", async ({
  page,
  context,
  browserName,
}) => {
  test.skip(browserName !== "chromium", "a test can grant notifications in Chromium only");
  test.setTimeout(720_000);
  await context.grantPermissions(["notifications"]);
  const n = await pendingRound();
  await page.goto(`/reveal/${n}`);
  await expect(page.getByTestId("reveal-waiting")).toBeVisible({ timeout: 60_000 });
  const state = await page.evaluate(() =>
    "Notification" in window ? Notification.permission : "unsupported",
  );
  test.skip(state !== "granted", `this browser build reports notifications as ${state}`);
  await expect(page.getByTestId("reveal-notify-on")).toContainText(`round ${n} is revealed`);
});
