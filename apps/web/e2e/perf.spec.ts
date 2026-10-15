/**
 * Performance at 4× CPU throttling (Chromium, CDP): frame timing and long
 * tasks for the idle field, a dig landing and a full win reveal. Results go
 * to docs/examples/phase7b/perf.json.
 *
 * The dig landing needs a depth to grow on demand, so that one scenario uses
 * a clearly labelled test fixture: the board response is replayed with one
 * tile one stage deeper. Nothing else is altered; the reveal is the real
 * settled round 188.
 */
import { expect, test, type Page } from "@playwright/test";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const OUT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../docs/examples/phase7b",
);
/** Merges one scenario's numbers into perf.json (tests may run in fresh workers). */
function save(name: string, value: unknown): void {
  mkdirSync(OUT, { recursive: true });
  const file = path.join(OUT, "perf.json");
  let all: Record<string, unknown> = { cpuThrottle: 4, browser: "chromium" };
  try {
    all = JSON.parse(readFileSync(file, "utf8")) as Record<string, unknown>;
  } catch {
    // first scenario
  }
  all[name] = value;
  writeFileSync(file, JSON.stringify(all, null, 2) + "\n");
}

async function throttle(page: Page): Promise<void> {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
}

/** Records rAF frame gaps and long tasks for `ms`, while `during` runs. */
async function measure(page: Page, ms: number, during?: () => Promise<void>) {
  await page.evaluate((window) => {
    const w = globalThis as unknown as { __perf: { frames: number[]; long: number[] } };
    w.__perf = { frames: [], long: [] };
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) w.__perf.long.push(e.duration);
    }).observe({ type: "longtask" });
    let last = performance.now();
    const end = last + window;
    const tick = (now: number) => {
      w.__perf.frames.push(now - last);
      last = now;
      if (now < end) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, ms);
  if (during !== undefined) await during();
  await page.waitForTimeout(ms + 300);
  const raw = await page.evaluate(
    () => (globalThis as unknown as { __perf: { frames: number[]; long: number[] } }).__perf,
  );
  const frames = raw.frames.slice(1).sort((a, b) => a - b);
  const pct = (p: number) =>
    frames[Math.min(frames.length - 1, Math.floor(frames.length * p))] ?? 0;
  const total = raw.frames.slice(1).reduce((a, b) => a + b, 0);
  return {
    seconds: +(total / 1000).toFixed(2),
    fps: +((frames.length / total) * 1000).toFixed(1),
    frameP50ms: +pct(0.5).toFixed(1),
    frameP95ms: +pct(0.95).toFixed(1),
    frameMaxMs: +(frames[frames.length - 1] ?? 0).toFixed(1),
    framesOver33ms: frames.filter((f) => f > 33.4).length,
    longTasksOver50ms: raw.long.filter((d) => d > 50).length,
    longestTaskMs: +Math.max(0, ...raw.long).toFixed(1),
  };
}

test.use({ viewport: { width: 1440, height: 900 } });

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
});

test("idle field", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.waitForTimeout(1500);
  await throttle(page);
  save("idleField", await measure(page, 10_000));
});

test("a dig landing (fixture: one tile one stage deeper)", async ({ page }) => {
  let deeper = false;
  // Without the stream the app polls the board every 10 s (as when the
  // socket drops); the next poll brings the deeper tile.
  await page.routeWebSocket(/\/v1\/stream/, (ws) => {
    void ws.close();
  });
  await page.route("**/v1/board", async (route) => {
    const response = await route.fetch();
    const board = (await response.json()) as { stages: number[] };
    if (deeper) board.stages = board.stages.map((s, i) => (i === 12 ? Math.min(5, s + 1) : s));
    await route.fulfill({ response, json: board });
  });
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  await page.waitForTimeout(1500);
  const before = await page.locator('[data-tile="12"]').getAttribute("data-depth");
  await throttle(page);
  deeper = true;
  const digLanding = await measure(page, 12_000);
  save("digLanding", digLanding);
  await expect(page.locator('[data-tile="12"]')).not.toHaveAttribute("data-depth", before ?? "0");
});

test("full win reveal (round 188)", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      "zeek.myDigs.v1",
      JSON.stringify([
        {
          round: 188,
          tiles: Array.from({ length: 25 }, (_, i) => i),
          amountEach: "100000",
          via: "pocket",
          txids: [],
          at: Date.now(),
        },
      ]),
    );
  });
  await page.goto("/rounds");
  await expect(page.getByTestId("round-row").first()).toBeVisible({ timeout: 60_000 });
  await throttle(page);
  const reveal = await measure(page, 5_000, async () => {
    await page.evaluate(() => {
      window.history.pushState({}, "", "/reveal/188");
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
  });
  save("reveal", reveal);
  await expect(page.getByTestId("result-card")).toBeVisible({ timeout: 30_000 });
});
