/**
 * Live 8c: a fresh Pocket on the deployed site, from the faucet to several
 * digs in one round.
 *
 * 1. Create a Pocket (fresh browser profile) and claim the faucet's 1 TAZ
 *    through the card's own "Open the faucet" link.
 * 2. Watch the card: waiting, then "Test coins on the way · ready in about
 *    N min" while they confirm (the balance headline shows the total with
 *    the split), then "You can dig now" only once spendable.
 * 3. The Pocket splits the coins ("Getting your coins ready").
 * 4. In one round: dig with each note in turn (no waiting for change), then
 *    one more dig meets "Your change is on its way" (no Add ZEC).
 *
 * The faucet pays 1 TAZ per address, once every 24 hours. When it already has,
 * ZEEK_FUND=test-wallets keeps everything else the same (the card's own
 * "Open the faucet" link starts the wait) but the coins come from Zeek's
 * own public test Pocket's Withdraw (0.045). Results go to
 * docs/examples/phase8c/live-run-<funding>.json, screenshots to
 * docs/examples/phase8c/live/<funding>/ (funding: faucet or test-wallets).
 *
 *   ZEEK_E2E_APP=https://playzeek.pages.dev pnpm exec playwright test --project=live e2e/live/07
 */

import { randomBytes } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { ENGINE, EXAMPLES, REPO, withdrawFromPublicPocket } from "./support";

const FUND = process.env.ZEEK_FUND ?? "faucet";

const OUT = path.join(REPO, "docs", "examples", EXAMPLES ?? "phase8c");
const SHOTS = path.join(OUT, "live", process.env.ZEEK_FUND ?? "faucet");
const log: Record<string, unknown> = { started: new Date().toISOString() };

async function shot(page: Page, name: string) {
  mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({ path: path.join(SHOTS, `${name}.png`) });
}

async function go(page: Page, route: string) {
  await page.evaluate((to) => {
    window.history.pushState({}, "", to);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, route);
}

async function board(): Promise<{ round: number; status: string; blocks_remaining: number }> {
  return (await (await fetch(`${ENGINE}/v1/board`)).json()) as {
    round: number;
    status: string;
    blocks_remaining: number;
  };
}

async function digOne(page: Page, tile: number): Promise<"done" | "change"> {
  await go(page, "/");
  await expect(page.getByTestId("field")).toBeVisible();
  await page.locator(`[data-tile="${tile}"]`).click();
  await page.getByTestId("amount-input").fill("0.001");
  await page.getByTestId("amount-input").press("Enter");
  await page.getByTestId("dig-button").click();
  const confirm = page.getByTestId("dig-confirm");
  const change = page.getByTestId("dig-change-coming");
  await expect(confirm.or(change)).toBeVisible({ timeout: 60_000 });
  if (await change.isVisible()) return "change";
  await page.getByTestId("dig-send").click();
  const done = page.getByTestId("dig-done");
  await expect(done.or(page.getByTestId("dig-error"))).toBeVisible({ timeout: 300_000 });
  await expect(done).toBeVisible();
  const txids = (await page.getByTestId("dig-txids").textContent()) ?? "";
  (log.digs as unknown[]).push({ tile, txids: txids.match(/[0-9a-f]{64}/g) ?? [] });
  await page.getByRole("button", { name: "Back to the field" }).click();
  return "done";
}

test("a fresh Pocket: faucet, confirming, split, several digs in one round", async ({
  page,
  context,
}) => {
  test.setTimeout(90 * 60_000);
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
    localStorage.setItem("zeek.e2e.skipLoadScreen", "1");
  });
  page.setDefaultTimeout(60_000);

  // 1. A fresh Pocket.
  const pass = `zeek 8c ${randomBytes(9).toString("base64url")}`;
  await page.goto("/wallet");
  await page.getByTestId("create-pass").fill(pass);
  await page.getByTestId("create-again").fill(pass);
  await page.getByTestId("create-submit").click();
  const words = page.getByTestId("backup-words").locator("li span.font-mono");
  await expect(words).toHaveCount(24, { timeout: 120_000 });
  const list = await words.allTextContents();
  await page.getByTestId("backup-written").click();
  for (const input of await page.locator("[data-testid^=check-word-]").all()) {
    const pos = Number((await input.getAttribute("data-testid"))?.replace("check-word-", ""));
    await input.fill(list[pos - 1] ?? "");
  }
  await page.getByTestId("check-words").click();
  await expect(page.getByTestId("wallet-home")).toBeVisible();
  await expect(page.getByTestId("faucet")).toBeVisible();

  // The faucet, through the card's own link (it starts the wait).
  await page.getByTestId("wallet-add").click();
  const address = ((await page.getByTestId("deposit-address").textContent()) ?? "").trim();
  expect(address).toMatch(/^utest1/);
  const [faucet] = await Promise.all([
    context.waitForEvent("page"),
    page.getByTestId("faucet-link").click(),
  ]);
  await faucet.waitForLoadState();
  log.funding = FUND;
  if (FUND === "faucet") {
    await faucet.locator("#address").fill(address);
    await expect(faucet.locator("#submit")).toBeEnabled({ timeout: 60_000 });
    await faucet.locator("#submit").click();
    const txid = faucet.locator("#tx-id");
    await expect(txid).toHaveText(/[0-9a-f]{64}/, { timeout: 300_000 });
    log.faucetTxid = await txid.textContent();
    await faucet.close();
  } else {
    await faucet.close();
    log.pocketTxid = await withdrawFromPublicPocket(address, "0.045");
  }

  // 2. Waiting, then on the way (confirming), then ready.
  await expect(page.getByTestId("faucet-waiting")).toBeVisible();
  await shot(page, "1-faucet-waiting");
  await expect(page.getByTestId("faucet-coming")).toBeVisible({ timeout: 20 * 60_000 });
  log.comingText = await page.getByTestId("faucet-ready-in").textContent();
  await expect(page.getByTestId("wallet-confirming")).toBeVisible();
  log.balanceWhileConfirming = await page.getByTestId("wallet-balance").textContent();
  log.spendableWhileConfirming = await page.getByTestId("wallet-spendable").textContent();
  await shot(page, "2-coins-on-the-way");
  // Never "dig now" while nothing is spendable.
  await expect(page.getByTestId("faucet-arrived")).toHaveCount(0);
  await expect(page.getByTestId("faucet-arrived")).toBeVisible({ timeout: 20 * 60_000 });
  log.arrivedSpendable = await page.getByTestId("wallet-spendable").textContent();
  await shot(page, "3-you-can-dig-now");

  // 3. The split: the history shows it; then wait until it is spendable.
  const history = page.getByTestId("wallet-history");
  await expect(history).toContainText("Getting your coins ready", { timeout: 5 * 60_000 });
  log.split = (await history.textContent())?.match(/Getting your coins ready[^·]*· \d+ parts/)?.[0];
  await shot(page, "4-getting-your-coins-ready");
  for (let i = 0; ; i += 1) {
    await page.getByRole("button", { name: "Sync" }).click();
    await page.waitForTimeout(20_000);
    const spendable = (await page.getByTestId("wallet-spendable").textContent()) ?? "0";
    const confirming = await page.getByTestId("wallet-confirming").count();
    if (Number(spendable) > 0 && confirming === 0) break;
    if (i > 40) throw new Error("the split never confirmed");
  }
  log.afterSplitSpendable = await page.getByTestId("wallet-spendable").textContent();

  // 4. Several digs in one round, then the change-on-its-way screen.
  await expect
    .poll(
      async () => {
        const b = await board();
        return b.status === "open" && b.blocks_remaining >= 18;
      },
      { timeout: 15 * 60_000, intervals: [5_000] },
    )
    .toBe(true);
  const round = (await board()).round;
  log.round = round;
  log.digs = [];
  let changeSeen = false;
  for (const tile of [2, 7, 12, 17, 22]) {
    const r = await digOne(page, tile);
    if (r === "change") {
      changeSeen = true;
      log.changeCountdown = await page.getByTestId("change-countdown").textContent();
      log.changeChoice =
        (await page.getByTestId("change-queue").count()) > 0 ? "queue offered" : "too late";
      await expect(page.getByText("Add ZEC")).toHaveCount(0);
      await shot(page, "6-change-on-its-way");
      break;
    }
    if ((log.digs as unknown[]).length === 2) await shot(page, "5-second-dig-same-round");
  }
  log.roundAfter = (await board()).round;
  log.changeSeen = changeSeen;
  log.finished = new Date().toISOString();
  writeFileSync(path.join(OUT, `live-run-${FUND}.json`), JSON.stringify(log, null, 2));
  expect((log.digs as unknown[]).length).toBeGreaterThanOrEqual(2);
  expect(log.roundAfter).toBe(round);
});
