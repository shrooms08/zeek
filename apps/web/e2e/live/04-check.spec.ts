/**
 * Live 4: Past rounds -> round detail -> Check a round. Every applicable
 * check passes; the second-source line is honest (unavailable without a
 * second source, then compared against the second relay).
 */
import { engine, expect, loadState, shot, test, until } from "./support";

const SECOND_RELAY = "http://127.0.0.1:8790";

test("past rounds, round detail, and every applicable check passes", async ({ page }) => {
  const state = loadState();
  if (state.dig === undefined) throw new Error("run step 02 first");
  const { round } = state.dig;
  // The bundle is complete once the payout (if any) is mined and opened.
  await until(
    `round ${round} payout recorded`,
    async () => {
      const page = await engine<{ rounds: { round: number; status: string }[] }>(
        `/v1/rounds?limit=50`,
      );
      const r = page.rounds.find((x) => x.round === round);
      return r !== undefined && r.status !== "paying" ? r : undefined;
    },
    60 * 60_000,
    20_000,
  );
  await page.goto("/rounds");
  const row = page.getByTestId("round-row").filter({ hasText: `Round ${round}` });
  await expect(row).toBeVisible({ timeout: 60_000 });
  await shot(page, "past-rounds-1440");
  await row.click();
  await expect(page.getByTestId("round-detail")).toBeVisible();
  await expect(page.getByTestId("tile-amounts").locator("li")).toHaveCount(25);
  await shot(page, "round-detail-1440");
  await page.getByTestId("check-this-round").click();

  for (const second of ["", SECOND_RELAY]) {
    await page.getByTestId("check-second").fill(second);
    await page.getByTestId("check-run").click();
    await expect(page.getByTestId("check-summary")).toBeVisible({ timeout: 15 * 60_000 });
    await expect(page.getByTestId("check-summary")).toHaveText("Every check that applies passed.");
    for (const id of ["beacon", "digs", "timing", "settlement", "payout"]) {
      await expect(page.getByTestId(`check-${id}`)).toHaveAttribute("data-state", /^(pass|na)$/);
    }
    await expect(page.getByTestId("cross-check")).toHaveAttribute(
      "data-status",
      second === "" ? "unavailable" : "pass",
    );
    await shot(page, second === "" ? "check-no-second-source-1440" : "check-1440");
  }
});
