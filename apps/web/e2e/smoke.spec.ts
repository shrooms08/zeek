import { expect, test, type Page } from "@playwright/test";

const SCREENS = [
  "/",
  "/rounds",
  "/check",
  "/keys",
  "/campfire",
  "/status",
  "/how",
  "/wallet",
  "/nope",
];

function watch(page: Page): string[] {
  const problems: string[] = [];
  page.on("requestfailed", (r) => {
    // A closed WebSocket on navigation is not a failed asset.
    if (!r.url().startsWith("ws"))
      problems.push(`failed ${r.url()} ${r.failure()?.errorText ?? ""}`);
  });
  page.on("response", (r) => {
    if (r.status() >= 400) problems.push(`${r.status()} ${r.url()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror ${String(e)}`));
  page.on("console", (m) => {
    if (m.type() === "error") problems.push(`console ${m.text()}`);
  });
  return problems;
}

test("the app is cross-origin isolated and every asset loads under COEP", async ({ page }) => {
  const problems = watch(page);
  const response = await page.goto("/");
  expect(response?.headers()["cross-origin-opener-policy"]).toBe("same-origin");
  expect(response?.headers()["cross-origin-embedder-policy"]).toBe("require-corp");
  expect(await page.evaluate(() => crossOriginIsolated)).toBe(true);
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 30_000 });
  // Fonts: load every face the app uses (a blocked font request would fail
  // under COEP), then check each one is available.
  const fonts = await page.evaluate(async () => {
    const faces = ["900 20px 'Archivo Variable'", "20px 'Archivo Black'", "14px 'JetBrains Mono'"];
    await Promise.all(faces.map((f) => document.fonts.load(f)));
    return faces.map((f) => document.fonts.check(f));
  });
  expect(fonts).toEqual([true, true, true]);
  expect(problems).toEqual([]);
});

for (const path of SCREENS) {
  test(`${path} renders without errors`, async ({ page }) => {
    const problems = watch(page);
    await page.goto(path);
    await page.waitForLoadState("networkidle");
    await expect(page.locator("main, [role=dialog]").first()).toBeVisible();
    // 404 for the unknown route is the app's own page, not a server 404.
    expect(problems).toEqual([]);
  });
}

test("first-time explainer, then the field plays by keyboard", async ({ page }) => {
  await page.goto("/");
  const onboarding = page.getByTestId("onboarding");
  await expect(onboarding).toBeVisible();
  for (let i = 0; i < 3; i++) await page.getByTestId("onboarding-next").click();
  await expect(onboarding).toBeHidden();
  const field = page.getByTestId("field");
  await expect(field).toBeVisible({ timeout: 30_000 });
  await field.focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Space");
  await expect(page.getByTestId("field-live")).toContainText("selected");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("field-live")).not.toContainText("selected");
});

test("the Pocket starts its wasm worker with threads", async ({ page }) => {
  const problems = watch(page);
  await page.goto("/wallet");
  // Pocket.start initialises the wasm and its thread pool before the wallet
  // screen can say whether a Pocket exists.
  await expect(page.getByTestId("wallet-create")).toBeVisible({ timeout: 90_000 });
  expect(await page.evaluate(() => typeof SharedArrayBuffer)).toBe("function");
  expect(problems).toEqual([]);
});

test.describe("at 390 wide", () => {
  test.use({ viewport: { width: 390, height: 844 } });
  for (const path of [
    "/",
    "/rounds",
    "/check",
    "/keys",
    "/campfire",
    "/status",
    "/wallet",
    "/how",
    "/nope",
  ]) {
    test(`${path} never scrolls sideways`, async ({ page }) => {
      await page.addInitScript(() => {
        localStorage.setItem("zeek.seenOnboarding", "1");
      });
      await page.goto(path);
      await page.waitForLoadState("networkidle");
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(overflow).toBeLessThanOrEqual(0);
    });
  }

  test("with the live stream down the menu stays on screen", async ({ page }) => {
    await page.routeWebSocket(/\/v1\/stream/, (ws) => {
      void ws.close();
    });
    await page.addInitScript(() => {
      localStorage.setItem("zeek.seenOnboarding", "1");
    });
    await page.goto("/nope");
    await expect(page.getByTestId("reconnecting")).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
    const menu = await page.getByTestId("menu-button").boundingBox();
    expect((menu?.x ?? 0) + (menu?.width ?? 0)).toBeLessThanOrEqual(390);
  });
});
