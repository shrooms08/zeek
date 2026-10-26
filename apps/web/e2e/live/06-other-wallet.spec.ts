/**
 * Live 6: dig with another wallet. Bob's fresh address is pasted as the
 * payout address, the app's ZIP 321 request is paid with
 * `zeek-dig pay-uri bob` exactly as an external wallet would, and the
 * waiting state resolves to confirmed once the payment is mined.
 */
import { test as base } from "@playwright/test";
import { engine, expect, saveState, shot, until, zeekDig } from "./support";

const test = base.extend({});

test("other wallet: bob pays the QR request and the dig confirms", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
    localStorage.setItem("zeek.e2e.skipLoadScreen", "1");
  });
  // Leave time for the payment to be mined inside the round.
  await until(
    "an open round with at least 8 blocks left",
    async () => {
      const b = await engine<{ status: string; blocks_remaining: number }>("/v1/board");
      return b.status === "open" && b.blocks_remaining >= 8 ? true : undefined;
    },
    30 * 60_000,
  );
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  const tile = 17;
  await page.locator(`[data-tile="${tile}"]`).click();
  await page.getByTestId("amount-input").fill("0.001");
  await page.getByTestId("amount-input").press("Enter");
  await page.getByTestId("other-wallet").click();

  await expect(page.getByTestId("wallet-address")).toBeVisible();
  await shot(page, "dig-other-address-1440");
  // A wrong-network address is refused with the design's message.
  await page.getByTestId("payout-address").fill("zs1notatestnetaddress");
  await page.getByTestId("wallet-next").click();
  await expect(page.getByRole("alert")).toBeVisible();
  const fresh =
    (await zeekDig("address", "bob", "--fresh"))
      .trim()
      .split(/\s+/)
      .find((w) => w.startsWith("utest1")) ?? "";
  expect(fresh).toMatch(/^utest1/);
  await page.getByTestId("payout-address").fill(fresh);
  await page.getByTestId("wallet-next").click();

  const card = page.getByTestId("qr-card");
  await expect(card).toHaveCount(1, { timeout: 120_000 });
  await shot(page, "dig-other-qr-1440");
  const uri = (await card.getAttribute("data-uri")) ?? "";
  expect(uri).toMatch(/^zcash:/);
  const round = Number(
    /round (\d+)/.exec((await page.getByTestId("wallet-scan").textContent()) ?? "")?.[1],
  );
  await zeekDig("pay-uri", "bob", uri);
  const broadcastAt = Date.now();
  await page.getByTestId("wallet-sent").click();
  await expect(page.getByTestId("wallet-waiting")).toBeVisible();
  // The engine's mempool watcher sees the payment before it is mined.
  await expect(page.getByTestId("payment-row").first()).toHaveAttribute(
    "data-status",
    /^(landing|landed)$/,
    {
      timeout: 5 * 60_000,
    },
  );
  const landingSeconds = Math.round((Date.now() - broadcastAt) / 100) / 10;
  const sawLanding =
    (await page.getByTestId("payment-row").first().getAttribute("data-status")) === "landing";
  console.log(
    `[landing] other-wallet payment shown as ${sawLanding ? "landing" : "landed"} ${landingSeconds} s after broadcast`,
  );
  await shot(page, "dig-other-landing-1440");
  await shot(page, "dig-other-waiting-1440");
  await expect(page.getByTestId("wallet-confirmed")).toBeVisible({ timeout: 20 * 60_000 });
  await expect(page.getByTestId("wallet-confirmed")).toContainText("1 of 1 payments landed");
  await shot(page, "dig-other-confirmed-1440");
  saveState({ other: { round, tile: tile - 1, landingSeconds, sawLanding } });
});
