/**
 * The load screen (index.html + src/boot.ts): on screen at once, gone within
 * 4.5 s with the field behind it, never wider than the viewport; and the
 * sound and music toggles (header, Menu, the M key), which persist. These
 * tests start without the global skip (playwright.config.ts).
 */

import { expect, test } from "@playwright/test";

test.use({ storageState: { cookies: [], origins: [] }, viewport: { width: 1440, height: 1000 } });

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
});

test("the load screen shows on load, then leaves for the field within 4.5 s", async ({ page }) => {
  await page.goto("/");
  const screen = page.getByTestId("load-screen");
  await expect(screen).toBeVisible();
  await expect(screen).toHaveAttribute("role", "status");
  await expect(screen).toHaveAttribute("aria-label", "Loading Zeek");
  // No text but the logo's own "eek".
  expect((await screen.innerText()).trim()).toBe("eek");
  await expect(screen).toHaveCount(0, { timeout: 4_500 });
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
});

for (const width of [360, 390, 768, 1024, 1440, 1920]) {
  test(`at ${width} wide the load screen never scrolls sideways`, async ({ page }) => {
    await page.setViewportSize({ width, height: width < 768 ? 844 : 900 });
    await page.goto("/");
    const screen = page.getByTestId("load-screen");
    await expect(screen).toBeVisible();
    const fit = await page.evaluate(() => {
      const doc = document.documentElement;
      const parts = [...document.querySelectorAll("#zeek-boot svg, #zeek-boot .zb-track")];
      return {
        scroll: doc.scrollWidth - doc.clientWidth,
        inside: parts.every((el) => {
          const r = el.getBoundingClientRect();
          return r.left >= 0 && r.right <= doc.clientWidth;
        }),
      };
    });
    await expect(screen).toBeVisible();
    expect(fit).toEqual({ scroll: 0, inside: true });
  });
}

test("both toggles flip and persist after a reload; M toggles sound effects", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("load-screen")).toHaveCount(0, { timeout: 4_500 });
  const sfx = page.getByTestId("sfx-toggle");
  const music = page.getByTestId("music-toggle");
  await expect(sfx).toHaveAttribute("aria-pressed", "true");
  await expect(music).toHaveAttribute("aria-pressed", "true");
  await sfx.click();
  await music.click();
  await expect(sfx).toHaveAttribute("aria-pressed", "false");
  await expect(music).toHaveAttribute("aria-pressed", "false");

  await page.reload();
  await expect(sfx).toHaveAttribute("aria-pressed", "false");
  await expect(music).toHaveAttribute("aria-pressed", "false");
  const stored = await page.evaluate(() => [
    localStorage.getItem("zeek.sfxMuted"),
    localStorage.getItem("zeek.musicMuted"),
  ]);
  expect(stored).toEqual(["1", "1"]);

  // The Menu has the same two.
  await page.getByTestId("menu-button").click();
  await expect(page.getByTestId("menu-sfx-toggle")).toHaveText("Sounds: off");
  await expect(page.getByTestId("menu-music-toggle")).toHaveText("Music: off");
  await page.getByTestId("menu-music-toggle").click();
  await expect(page.getByTestId("menu-music-toggle")).toHaveText("Music: on");
  await expect(page.getByTestId("menu-music-toggle")).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("Escape");

  // M toggles sound effects.
  await page.keyboard.press("m");
  await expect(sfx).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("M");
  await expect(sfx).toHaveAttribute("aria-pressed", "false");
});
