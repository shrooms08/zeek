/**
 * Live 7: every screen at desktop 1440 and mobile 390, from the live engine
 * and the funded Pocket (run after the live steps). Backup words are
 * blurred before capture: a seed never goes into a screenshot. Offline and
 * loading states are real app states under a dropped or slow engine
 * connection, not fake data.
 */
import { chromium, type Page } from "@playwright/test";
import { expect, loadState, shot, test, unlock } from "./live/support";

const SIZES = [
  { name: "1440", width: 1440, height: 1000 },
  { name: "390", width: 390, height: 844 },
];

async function settle(page: Page): Promise<void> {
  await page.waitForLoadState("networkidle").catch(() => undefined);
  await page.waitForTimeout(800);
}

test("screens at 1440 and 390", async ({ page }) => {
  const state = loadState();
  if (
    state.passphrase === undefined ||
    state.win === undefined ||
    state.dig === undefined ||
    state.proofId === undefined
  ) {
    throw new Error("run the live steps first");
  }
  const { passphrase, win, dig, proofId } = state;
  await unlock(page, passphrase);
  const routes: [string, string, (p: Page) => Promise<void>][] = [
    ["field", "/", (p) => expect(p.getByTestId("field")).toBeVisible()],
    ["menu", "/", (p) => p.getByTestId("menu-button").click()],
    ["how-to-play", "/how", (p) => expect(p.getByTestId("how-to-play")).toBeVisible()],
    ["dig-other-address", "/dig/wallet", () => Promise.resolve()],
    [
      "reveal",
      `/reveal/${win.round}`,
      (p) => expect(p.getByTestId("result-card")).toBeVisible({ timeout: 60_000 }),
    ],
    [
      "reveal-not-this-time",
      `/reveal/${dig.round}`,
      (p) => expect(p.getByTestId("result-card")).toBeVisible({ timeout: 60_000 }),
    ],
    ["past-rounds", "/rounds", (p) => expect(p.getByTestId("round-row").first()).toBeVisible()],
    [
      "round-detail",
      `/rounds/${win.round}`,
      (p) => expect(p.getByTestId("round-detail")).toBeVisible(),
    ],
    ["check-idle", `/check/${win.round}`, () => Promise.resolve()],
    [
      "wallet-home",
      "/wallet",
      (p) => expect(p.getByTestId("wallet-home")).toBeVisible({ timeout: 60_000 }),
    ],
    ["wallet-deposit", "/wallet", (p) => p.getByTestId("wallet-add").click()],
    [
      "wallet-withdraw",
      "/wallet",
      (p) => p.getByRole("button", { name: "Withdraw", exact: true }).click(),
    ],
    ["prove-win", `/prove/${win.round}`, () => Promise.resolve()],
    [
      "verified-win",
      `/win/${proofId}`,
      (p) => expect(p.getByTestId("win-verdict")).toHaveText("Verified", { timeout: 5 * 60_000 }),
    ],
    ["keys", "/keys", (p) => expect(p.getByTestId("keyholder-card")).toHaveCount(3)],
    ["campfire", "/campfire", (p) => expect(p.getByTestId("campfire-feed")).toBeVisible()],
    ["not-found", "/nope", () => Promise.resolve()],
  ];
  for (const size of SIZES) {
    await page.setViewportSize({ width: size.width, height: size.height });
    for (const [name, route, ready] of routes) {
      await page.goto(route);
      await ready(page);
      await settle(page);
      await shot(page, `${name}-${size.name}`);
    }
    // Check a round, run to the end.
    await page.goto(`/check/${dig.round}`);
    await page.getByTestId("check-run").click();
    await expect(page.getByTestId("check-summary")).toBeVisible({ timeout: 15 * 60_000 });
    await shot(page, `check-done-${size.name}`);
    // Unlock screen.
    await page.goto("/wallet");
    await page.getByTestId("wallet-lock").click();
    await expect(page.getByTestId("wallet-unlock")).toBeVisible();
    await shot(page, `wallet-unlock-${size.name}`);
    await unlock(page, passphrase);
    // Offline and loading: the engine unreachable, then slow.
    await page.route("http://127.0.0.1:8787/**", (r) => r.abort());
    await page.goto("/rounds");
    await expect(page.getByTestId("offline")).toBeVisible({ timeout: 60_000 });
    await shot(page, `offline-${size.name}`);
    await page.unroute("http://127.0.0.1:8787/**");
    await page.route("http://127.0.0.1:8787/v1/rounds**", async (r) => {
      await new Promise((resolve) => setTimeout(resolve, 5_000));
      await r.continue().catch(() => undefined);
    });
    await page.goto("/rounds");
    await expect(page.getByTestId("loading")).toBeVisible();
    await shot(page, `loading-${size.name}`);
    await page.unrouteAll({ behavior: "ignoreErrors" });
  }

  // First-time screens in a fresh browser: explainer, create, backup.
  // The throwaway wallet is never funded; its words are blurred anyway.
  const browser = await chromium.launch();
  for (const size of SIZES) {
    const fresh = await browser.newContext({ baseURL: "http://localhost:4174", viewport: size });
    const p = await fresh.newPage();
    await p.goto("/");
    await expect(p.getByTestId("onboarding")).toBeVisible({ timeout: 60_000 });
    await settle(p);
    await shot(p, `onboarding-${size.name}`);
    await p.getByRole("button", { name: "Skip" }).click();
    await p.goto("/wallet");
    await expect(p.getByTestId("wallet-create")).toBeVisible({ timeout: 60_000 });
    await shot(p, `wallet-create-${size.name}`);
    await p.getByTestId("create-pass").fill("screenshot only, never funded");
    await p.getByTestId("create-again").fill("screenshot only, never funded");
    await p.getByTestId("create-submit").click();
    await expect(p.getByTestId("backup-words")).toBeVisible({ timeout: 120_000 });
    await p.addStyleTag({
      content: "[data-testid=backup-words] .font-mono { filter: blur(7px); }",
    });
    await shot(p, `wallet-backup-${size.name}`);
    await fresh.close();
  }
  await browser.close();
});
