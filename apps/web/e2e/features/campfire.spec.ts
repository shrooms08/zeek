/**
 * The campfire on the field opens the Campfire chat: by click, and from the
 * keyboard as a button (Enter or Space). Zeb, sitting by it, keeps his own
 * tricks (a random pictogram; five quick taps pop him out of a hole), and
 * the other props keep their reactions.
 */

import { expect, test } from "@playwright/test";

test.use({ viewport: { width: 1440, height: 1000 } });

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
});

test("clicking the campfire opens the Campfire chat", async ({ page }) => {
  const camp = page.getByTestId("campfire-open");
  await expect(camp).toHaveAccessibleName("Open the Campfire chat");
  await camp.click();
  await expect(page).toHaveURL(/\/campfire$/);
  await expect(page.getByTestId("campfire-feed")).toBeVisible({ timeout: 30_000 });
});

for (const key of ["Enter", "Space"] as const) {
  test(`the campfire is a button: ${key} opens the chat`, async ({ page }) => {
    const camp = page.getByTestId("campfire-open");
    await camp.focus();
    // The focus ring is visible on the camp itself.
    expect(await camp.evaluate((el) => getComputedStyle(el).outlineStyle)).not.toBe("none");
    await page.keyboard.press(key);
    await expect(page).toHaveURL(/\/campfire$/);
  });
}

test("the trees still drop their leaves", async ({ page }) => {
  const tree = page.locator('[data-prop][aria-label="Pine tree"]').first();
  await tree.click();
  await expect(tree).toHaveAttribute("data-active", "true");
  await expect(page).toHaveURL(/\/$/);
});

test("Zeb answers with a pictogram and stays on the field", async ({ page }) => {
  const zeb = page.getByTestId("zeb");
  await expect(zeb).toHaveAccessibleName("Zeb the zebra miner");
  await zeb.click();
  // The bubble's anchor is a zero-size point; its picture is what shows.
  await expect(page.getByTestId("world-props").getByRole("status").getByRole("img")).toBeVisible();
  await expect(page).toHaveURL(/\/$/);
});

test("five quick taps on Zeb: he pops out of a hole", async ({ page }) => {
  const zeb = page.getByTestId("zeb");
  for (let k = 0; k < 5; k += 1) await zeb.click();
  await expect(page.getByTestId("world-props").getByRole("status")).toContainText("Found you!");
  await expect(page).toHaveURL(/\/$/);
});
