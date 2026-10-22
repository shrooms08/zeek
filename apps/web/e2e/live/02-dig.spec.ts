/**
 * Live 1b + 2: dig 2 tiles with a message from the Pocket, then see the
 * depth change on the field and the message at the Campfire.
 */
import { go, engine, expect, loadState, saveState, shot, test, unlock, until } from "./support";

const TILES = [6, 18]; // engine indices (UI tiles 7 and 19)

test("dig 2 tiles with a message; depth changes; message reaches the Campfire", async ({
  page,
}) => {
  const state = loadState();
  if (state.passphrase === undefined) throw new Error("run step 01 first");
  await unlock(page, state.passphrase);
  const message = `phase 7a: two tiles from the web app ${new Date().toISOString().slice(0, 16)}`;

  await until(
    "an open round with at least 8 blocks left",
    async () => {
      const b = await engine<{ status: string; blocks_remaining: number }>("/v1/board");
      return b.status === "open" && b.blocks_remaining >= 8 ? true : undefined;
    },
    30 * 60_000,
  );
  await go(page, "/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  const before = await Promise.all(
    TILES.map(async (t) =>
      Number(await page.locator(`[data-tile="${t}"]`).getAttribute("data-depth")),
    ),
  );
  // Several tiles: Pro (Lite digs one).
  await page.getByTestId("mode-pro").click();
  for (const t of TILES) await page.locator(`[data-tile="${t}"]`).click();
  await page.getByTestId("amount-input").fill("0.001");
  await page.getByTestId("amount-input").press("Enter");
  await page.getByTestId("dig-message").fill(message);
  await expect(page.getByTestId("tile-count")).toHaveText("2");
  await expect(page.getByTestId("total-amount")).toHaveText("0.00215");
  await shot(page, "field-selected-1440");
  await page.getByRole("button", { name: /dig 2 tiles/i }).click();

  await expect(page.getByTestId("dig-confirm")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("confirm-total")).toContainText("0.00215");
  await shot(page, "dig-pocket-confirm-1440");
  await page.getByTestId("dig-send").click();
  await expect(page.getByTestId("dig-sealing")).toBeVisible();
  await shot(page, "dig-pocket-sealing-1440");
  const done = page.getByTestId("dig-done");
  await expect(done).toBeVisible({ timeout: 20 * 60_000 });
  await shot(page, "dig-pocket-done-1440");
  const round = Number(/round (\d+)/.exec((await done.textContent()) ?? "")?.[1]);
  const txids =
    ((await page.getByTestId("dig-txids").textContent()) ?? "").match(/[0-9a-f]{64}/g) ?? [];
  expect(txids).toHaveLength(1);
  saveState({ dig: { round, tiles: TILES, message, txids } });
  await done.getByRole("button").first().click();

  // Within seconds the mempool watcher shows the dig landing on its tiles,
  // then the field shows the new depth once it is mined and ingested.
  await go(page, "/");
  const landed = await page
    .locator(`[data-tile="${TILES[0] ?? 0}"][data-landing="true"]`)
    .waitFor({ timeout: 90_000 })
    .then(() => true)
    .catch(() => false);
  console.log(
    `[landing] pocket dig ${landed ? "shown landing on the field" : "mined before a landing was seen"}`,
  );
  for (const [i, t] of TILES.entries()) {
    await expect
      .poll(
        async () => Number(await page.locator(`[data-tile="${t}"]`).getAttribute("data-depth")),
        { timeout: 20 * 60_000, intervals: [10_000] },
      )
      .toBeGreaterThan(before[i] ?? 0);
  }
  await shot(page, "field-dug-1440");

  await go(page, "/campfire");
  await expect(page.getByTestId("campfire-feed")).toContainText(message, { timeout: 10 * 60_000 });
  const row = page.getByTestId("campfire-message").filter({ hasText: message });
  await expect(row).toContainText(`R${round}`);
  await expect(row).toContainText("TILE 7 19");
  await shot(page, "campfire-1440");
});
