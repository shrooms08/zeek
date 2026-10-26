/**
 * The dig sound plays the instant DIG is pressed (pointer, or Enter on the
 * button), once per press. Every audio play() in the page is recorded.
 */

import { expect, test, type Page } from "@playwright/test";

// A response still being rewritten when a test ends is not a failure.
test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: "ignoreErrors" });
});

test.use({ viewport: { width: 1440, height: 1000 } });

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
    const w = window as unknown as { __plays: string[] };
    w.__plays = [];
    const proto = HTMLMediaElement.prototype;
    const original = Object.getOwnPropertyDescriptor(proto, "play")?.value as (
      this: HTMLMediaElement,
    ) => Promise<void>;
    proto.play = function (this: HTMLMediaElement) {
      w.__plays.push(this.src.split("/").pop() ?? "");
      if (this.src.endsWith("dig.mp3")) console.log("DIGPLAY", new Error().stack);
      return original.call(this).catch(() => undefined);
    };
  });
  // An open round whatever the real clock says; the stream stays silent.
  await page.routeWebSocket(/\/v1\/stream/, () => undefined);
  await page.route(/\/v1\/board$/, async (route) => {
    const response = await route.fetch();
    const real = (await response.json()) as { blocks_remaining: number };
    await route.fulfill({
      response,
      json: { ...real, status: "open", blocks_remaining: Math.max(real.blocks_remaining, 20) },
    });
  });
});

const plays = (page: Page) =>
  page.evaluate(() => (window as unknown as { __plays: string[] }).__plays);

test("pressing DIG plays the dig sound at once, once", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.locator('[data-tile="7"]').click();
  expect(await plays(page)).toContain("tile-pick.mp3");
  const dig = page.getByTestId("dig-button");
  await dig.dispatchEvent("pointerdown");
  expect((await plays(page)).filter((p) => p === "dig.mp3")).toHaveLength(1);
});

test("Enter on DIG plays it too", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.locator('[data-tile="7"]').click();
  await page.getByTestId("dig-button").focus();
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await plays(page)).filter((p) => p === "dig.mp3").length).toBe(1);
  // Still once a moment later (Enter also arrives as framer-motion's pointerdown).
  await page.waitForTimeout(500);
  expect((await plays(page)).filter((p) => p === "dig.mp3")).toHaveLength(1);
});
