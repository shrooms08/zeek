/**
 * Live 1a: create a Pocket in the app (passphrase, 24-word backup, quick
 * check), fund it from alice with `zeek-dig send`, and sync until it shows.
 * Also moves carol's and the old harness Pocket's TAZ in, so the 25-tile
 * round (step 05) can be paid for.
 */
import { execFile } from "node:child_process";
import { randomBytes } from "node:crypto";
import path from "node:path";
import { promisify } from "node:util";
import {
  balanceZat,
  expect,
  loadState,
  REPO,
  saveState,
  shot,
  synced,
  test,
  unlock,
  zeekDig,
} from "./support";

const run = promisify(execFile);

test("create a Pocket and fund it from alice", async ({ page }) => {
  let state = loadState();
  if (state.passphrase === undefined) {
    state = saveState({ passphrase: `zeek web ${randomBytes(9).toString("base64url")}` });
    const pass = state.passphrase ?? "";
    await page.goto("/wallet");
    await expect(page.getByTestId("wallet-create")).toBeVisible({ timeout: 60_000 });
    await shot(page, "wallet-create-1440");
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
    await expect(page.getByTestId("wallet-home")).toBeVisible({ timeout: 60_000 });
  }
  await unlock(page, state.passphrase ?? "");
  await page.getByTestId("wallet-add").click();
  const address = (await page.getByTestId("deposit-address").textContent())?.trim() ?? "";
  expect(address).toMatch(/^utest1/);

  // Each payment is recorded as soon as it's sent, so a rerun never resends.
  const txid = (out: string) => /[0-9a-f]{64}/.exec(out)?.[0] ?? out.trim();
  const sources: { name: string; amount: string; send: () => Promise<string> }[] = [
    {
      name: "alice",
      amount: "0.0033",
      send: () =>
        zeekDig(
          "send",
          "alice",
          "--to",
          address,
          "--amount",
          "0.0033",
          "--memo",
          "Zeek Pocket funding from alice",
        ),
    },
    {
      name: "carol",
      amount: "0.0071",
      send: () => zeekDig("send", "carol", "--to", address, "--amount", "0.0071"),
    },
    {
      // The old harness Pocket (Phase 5), driven through its own page.
      name: "harness",
      amount: "0.0210",
      send: async () =>
        (
          await run("node", ["e2e/drive.mjs", "send", "zeek-pocket-live-test", address, "0.0210"], {
            cwd: path.join(REPO, "apps/harness"),
            timeout: 25 * 60_000,
          })
        ).stdout,
    },
  ];
  // Faucet TAZ (Valar) reached alice after the first 25-tile dig stopped
  // part-way; this top-up pays for a fresh 25-tile round.
  sources.push({
    name: "alice-faucet-topup",
    amount: "0.04",
    send: () =>
      zeekDig(
        "send",
        "alice",
        "--to",
        address,
        "--amount",
        "0.04",
        "--memo",
        "Zeek Pocket top-up from alice",
      ),
  });
  for (const source of sources) {
    const funded = loadState().funded ?? [];
    if (funded.some((f) => f.name === source.name)) continue;
    const out = await source.send();
    saveState({
      funded: [...funded, { name: source.name, amount: source.amount, txid: txid(out) }],
    });
  }

  // Sync until all three payments are spendable (10 confirmations).
  const want = 5_000_000n;
  const start = Date.now();
  for (;;) {
    await page.getByRole("button", { name: "Sync" }).click();
    await synced(page);
    if ((await balanceZat(page)) >= want) break;
    if (Date.now() - start > 60 * 60_000) throw new Error("funding never became spendable");
    await page.waitForTimeout(60_000);
  }
  await expect(page.getByTestId("history-row").first()).toBeVisible();
  await expect(page.getByTestId("wallet-history")).toContainText("Zeek Pocket funding from alice");
});
