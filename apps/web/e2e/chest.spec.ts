/**
 * Visual regression for the treasure chest: the chest must sit on its tile,
 * in front of the tile's outline and glow, with the light around it
 * (src/components/field/Field.test.tsx checks the draw order and geometry).
 *
 * Uses rounds that are settled for good on the testnet engine, so the
 * pictures never change: round 251 (treasure tile 9) and the verified win
 * of round 247 (proof 78ea3288…). The Deep Cache has not broken open on
 * testnet yet, so there is no real Deep Cache reveal to capture; it draws
 * the same treasure (covered by the unit test).
 */

import { expect, test } from "@playwright/test";

const ROUND = process.env.ZEEK_CHEST_ROUND ?? "251";
const PROOF = process.env.ZEEK_CHEST_PROOF ?? "78ea3288c9ef4fea6c07ec9eeb26c9e0";
const ENGINE = process.env.VITE_ZEEK_ENGINE_URL ?? "http://127.0.0.1:8787";

test.use({ reducedMotion: "reduce", viewport: { width: 1440, height: 1000 } });

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
});

test("verified win: the chest sits on the winning tile", async ({ page }) => {
  await page.goto(`/win/${PROOF}`);
  const treasure = page.locator("[data-treasure]").first();
  await expect(treasure).toBeVisible({ timeout: 60_000 });
  await expect(page.locator("[data-chest]")).toBeVisible();
  await expect(treasure).toHaveScreenshot("chest-verified-win.png", {
    animations: "disabled",
    maxDiffPixelRatio: 0.01,
  });
});

test("reveal: the chest sits on the treasure tile", async ({ page }) => {
  await page.goto(`/reveal/${ROUND}`);
  await expect(page.getByTestId("result-card")).toBeVisible({ timeout: 60_000 });
  const treasure = page.locator("[data-treasure]").first();
  await expect(page.locator("[data-chest]")).toBeVisible();
  await expect(treasure).toHaveScreenshot("chest-reveal.png", {
    animations: "disabled",
    maxDiffPixelRatio: 0.01,
  });
});

test("share cards (resvg) keep the chest art", async ({ request }) => {
  for (const [path, name] of [
    [`/share/round/${ROUND}.png`, "share-round.png"],
    [`/share/win/${PROOF}.png`, "share-win.png"],
  ] as const) {
    const response = await request.get(`${ENGINE}${path}`);
    expect(response.status()).toBe(200);
    expect(await response.body()).toMatchSnapshot(name, { maxDiffPixelRatio: 0.002 });
  }
});
