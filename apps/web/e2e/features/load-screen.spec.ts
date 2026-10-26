/**
 * The load screen (index.html + src/boot.ts): on screen at once; once the
 * field is ready, "Tap to dig in", and the tap plays the enter sound once
 * and, in time with it, fades to black and then to the field; a deep link skips the tap; never wider than the
 * viewport with the prompt showing. Also the sound and music toggles
 * (header, Menu, the M key), which persist. These tests start without the
 * global skip (playwright.config.ts). Every audio play() is recorded.
 */

import { expect, test, type Page } from "@playwright/test";

test.use({ storageState: { cookies: [], origins: [] }, viewport: { width: 1440, height: 1000 } });

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
      return original.call(this).catch(() => undefined);
    };
  });
});

const plays = (page: Page) =>
  page.evaluate(() => (window as unknown as { __plays: string[] }).__plays);

/** The prompt shows (the field is ready, or 4 s have passed). */
async function prompted(page: Page) {
  const screen = page.getByTestId("load-screen");
  await expect(screen).toHaveAttribute("role", "button", { timeout: 10_000 });
  await expect(screen.locator(".zb-text")).toBeVisible();
  return screen;
}

test("the load screen waits for a tap, which plays enter once and shows the field", async ({
  page,
}) => {
  await page.goto("/");
  const screen = page.getByTestId("load-screen");
  await expect(screen).toBeVisible();
  await expect(screen).toHaveAttribute("role", "status");
  await prompted(page);
  await expect(screen).toHaveAttribute("aria-label", "Tap to start Zeek");
  await expect(screen).toHaveAttribute("tabindex", "0");
  await expect(screen).toBeFocused();
  await expect(screen.locator(".zb-track")).toHaveCSS("opacity", "0");
  expect((await screen.innerText()).replace(/\s+/g, " ").trim()).toBe("eek TAP TO DIG IN");
  // It does not leave by itself.
  await page.waitForTimeout(1_500);
  await expect(screen).toBeVisible();
  expect(await plays(page)).toEqual([]);

  await page.mouse.click(720, 500);
  // The exit runs in time with the enter sound: black first, then the field.
  await expect(screen).toHaveClass(/zb-exit/);
  await page.waitForTimeout(1_000);
  await expect(screen).toHaveCSS("pointer-events", "auto");
  await expect(screen.locator(".zb-content")).toHaveCSS("opacity", "0");
  await expect(screen).toHaveCount(0, { timeout: 4_000 });
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.waitForTimeout(300);
  expect((await plays(page)).filter((p) => p === "enter.mp3")).toHaveLength(1);
});

test("a key works as the tap; with sound effects muted it is silent", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.sfxMuted", "1");
  });
  await page.goto("/");
  const screen = await prompted(page);
  await page.keyboard.press("Enter");
  await expect(screen).toHaveCount(0, { timeout: 4_000 });
  expect(await plays(page)).not.toContain("enter.mp3");
});

test("under reduced motion the tap fades it out at once, without the sequence", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  const screen = await prompted(page);
  await page.keyboard.press("Space");
  await expect(screen).not.toHaveClass(/zb-exit/);
  await expect(screen).toHaveCount(0, { timeout: 1_000 });
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
});

test("a deep link skips the tap and plays nothing", async ({ page }) => {
  await page.goto("/status");
  await expect(page.getByTestId("load-screen")).toHaveCount(0, { timeout: 6_000 });
  expect(await plays(page)).toEqual([]);
});

for (const width of [360, 390, 768, 1024, 1440, 1920]) {
  test(`at ${width} wide the load screen never scrolls sideways`, async ({ page }) => {
    await page.setViewportSize({ width, height: width < 768 ? 844 : 900 });
    await page.goto("/");
    await prompted(page);
    const fit = await page.evaluate(() => {
      const doc = document.documentElement;
      const parts = [...document.querySelectorAll("#zeek-boot svg, #zeek-boot .zb-text")];
      return {
        scroll: doc.scrollWidth - doc.clientWidth,
        inside: parts.every((el) => {
          const r = el.getBoundingClientRect();
          return r.left >= 0 && r.right <= doc.clientWidth && r.bottom <= doc.clientHeight;
        }),
      };
    });
    expect(fit).toEqual({ scroll: 0, inside: true });
  });
}

test("both toggles flip and persist after a reload; M toggles sound effects", async ({ page }) => {
  const enter = async () => {
    const screen = await prompted(page);
    await page.mouse.click(10, 10);
    await expect(screen).toHaveCount(0, { timeout: 4_000 });
  };
  await page.goto("/");
  await enter();
  const sfx = page.getByTestId("sfx-toggle");
  const music = page.getByTestId("music-toggle");
  await expect(sfx).toHaveAttribute("aria-pressed", "true");
  await expect(music).toHaveAttribute("aria-pressed", "true");
  await sfx.click();
  await music.click();
  await expect(sfx).toHaveAttribute("aria-pressed", "false");
  await expect(music).toHaveAttribute("aria-pressed", "false");

  await page.reload();
  await enter();
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
