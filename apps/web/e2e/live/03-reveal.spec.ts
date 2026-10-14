/**
 * Live 3: wait for the dug round's reveal; the result card matches the
 * engine's settlement (outcome and treasure tile).
 */
import { engine, expect, loadState, shot, test, unlock, until } from "./support";

interface Settled {
  outcome:
    { outcome: "voided" } | { outcome: "drawn"; treasure_tile: number; cache_triggered: boolean };
  treasure_tile: number | null;
  cache_triggered: boolean;
}

test("the reveal's result card matches the settlement", async ({ page }) => {
  const state = loadState();
  if (state.dig === undefined || state.passphrase === undefined)
    throw new Error("run step 02 first");
  const { round, tiles } = state.dig;
  await unlock(page, state.passphrase);
  await page.goto(`/reveal/${round}`);
  await expect(page.getByTestId("reveal-screen")).toBeVisible();
  await shot(page, "reveal-waiting-1440");
  const settled = await until(
    `round ${round} to settle`,
    async () =>
      (await engine<{ settled: Settled | null }>(`/v1/rounds/${round}`)).settled ?? undefined,
    60 * 60_000,
    20_000,
  );
  const card = page.getByTestId("result-card");
  await expect(card).toBeVisible({ timeout: 5 * 60_000 });
  const expected =
    settled.outcome.outcome === "voided"
      ? "voided"
      : tiles.includes(settled.outcome.treasure_tile)
        ? settled.outcome.cache_triggered
          ? "cache-win"
          : "win"
        : "not-this-time";
  await expect(card).toHaveAttribute("data-outcome", expected);
  if (settled.treasure_tile !== null)
    await expect(card).toContainText(`Treasure: tile ${settled.treasure_tile + 1}`);
  await shot(page, "reveal-result-1440");
});
