/**
 * The camp on the field (the fire and Zeb sitting by it) opens the
 * Campfire chat: by click, and from the keyboard as a button (Enter or
 * Space). The other props keep their own reactions.
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
