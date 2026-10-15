/**
 * Screen captures for the demo (WebM, 1440 × 900) into docs/examples/phase7b/:
 *   field-interactions.webm  the idle island, then every prop played with
 *   dig-landing.webm         an anonymous dig landing (puff, shake, hatch)
 *   reveal-win.webm          a full win reveal of a real round
 * The dig landing needs a depth to grow on demand, so it uses the same
 * clearly labelled fixture as the perf spec (the board replayed with one
 * tile one stage deeper); everything else is the live engine.
 */
import { chromium, expect, type BrowserContext, type Page } from "@playwright/test";
import { mkdirSync, renameSync } from "node:fs";
import path from "node:path";
import { loadState, REPO, test } from "./live/support";

const OUT = path.join(REPO, "docs/examples/phase7b");
const SIZE = { width: 1440, height: 900 };

async function recording(
  name: string,
  body: (page: Page, context: BrowserContext) => Promise<void>,
) {
  mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  const context = await browser.newContext({
    baseURL: "http://localhost:4174",
    viewport: SIZE,
    recordVideo: { dir: path.join(OUT, ".video"), size: SIZE },
  });
  await context.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
  const page = await context.newPage();
  await body(page, context);
  const video = page.video();
  await context.close();
  await browser.close();
  if (video !== null) renameSync(await video.path(), path.join(OUT, `${name}.webm`));
}

test.setTimeout(10 * 60_000);

test("field interactions", async () => {
  await recording("field-interactions", async (page) => {
    await page.goto("/");
    await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
    await page.waitForTimeout(3000);
    for (const id of ["tree-3", "lamp-1", "cart", "rock-2", "tree-9", "fire"]) {
      await page.locator(`[data-prop="${id}"]`).click();
      await page.waitForTimeout(1400);
    }
    const mine = page.locator('[data-prop="mine"]');
    await mine.hover();
    await page.mouse.down();
    await page.waitForTimeout(900);
    await page.mouse.up();
    await page.waitForTimeout(2500);
    const zeb = page.locator('[data-prop="zeb"]');
    for (let k = 0; k < 5; k++) await zeb.click();
    await page.waitForTimeout(3000);
    for (const t of [6, 12, 18]) {
      await page.locator(`[data-tile="${t}"]`).click();
      await page.waitForTimeout(500);
    }
    await page.waitForTimeout(1500);
  });
});

test("an anonymous dig landing (fixture: one tile one stage deeper)", async () => {
  await recording("dig-landing", async (page) => {
    let deeper = false;
    await page.routeWebSocket(/\/v1\/stream/, (ws) => {
      void ws.close();
    });
    await page.route("**/v1/board", async (route) => {
      const response = await route.fetch();
      const board = (await response.json()) as { stages: number[] };
      if (deeper)
        board.stages = board.stages.map((s, i) => (i === 12 || i === 7 ? Math.min(5, s + 1) : s));
      await route.fulfill({ response, json: board });
    });
    await page.goto("/");
    await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
    await page.waitForTimeout(1500);
    deeper = true;
    await expect(page.locator('[data-tile="12"]')).not.toHaveAttribute("data-depth", "0", {
      timeout: 20_000,
    });
    await page.waitForTimeout(2500);
  });
});

test("a full win reveal", async () => {
  const round = loadState().win?.round ?? 188;
  await recording("reveal-win", async (page) => {
    await page.addInitScript((r) => {
      localStorage.setItem(
        "zeek.myDigs.v1",
        JSON.stringify([
          {
            round: r,
            tiles: Array.from({ length: 25 }, (_, i) => i),
            amountEach: "100000",
            via: "pocket",
            txids: [],
            at: Date.now(),
          },
        ]),
      );
    }, round);
    await page.goto(`/reveal/${round}`);
    await expect(page.getByTestId("result-card")).toBeVisible({ timeout: 60_000 });
    await page.waitForTimeout(3500);
  });
});
