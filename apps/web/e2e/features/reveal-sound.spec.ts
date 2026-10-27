/**
 * The reveal's result sound when the result is reached before sound is
 * allowed: a reload on the field opens a finished round this browser dug in
 * (and lost) behind the load screen; after the tap, miss plays once, when
 * the load screen has gone. Uses the engine's latest drawn round. Every
 * audio play() is recorded.
 */

import { expect, test } from "@playwright/test";

const ENGINE =
  process.env.ZEEK_E2E_APP === undefined
    ? (process.env.VITE_ZEEK_ENGINE_URL ?? "http://127.0.0.1:8787")
    : `${process.env.ZEEK_E2E_APP}/api`;

test.use({ storageState: { cookies: [], origins: [] }, viewport: { width: 1440, height: 1000 } });
test("reload on the field lands on a lost reveal: miss after the tap", async ({ page }) => {
  const list = (await (await fetch(`${ENGINE}/v1/rounds`)).json()) as {
    rounds: { round: number; status: string; treasure_tile: number | null }[];
  };
  const r = list.rounds.find((x) => x.treasure_tile !== null);
  if (r?.treasure_tile == null) throw new Error("no drawn round");
  // A tile next to the treasure: this browser dug and lost.
  const tile = (r.treasure_tile + 1) % 25;
  await page.addInitScript(
    ([round, t]) => {
      localStorage.setItem("zeek.seenOnboarding", "1");
      localStorage.setItem("zeek.seenReveal", String(round - 1));
      localStorage.setItem(
        "zeek.myDigs.v1",
        JSON.stringify([{ round, at: 1, tiles: [t], amountEach: "100000", via: "pocket" }]),
      );
      const w = window as unknown as { __plays: [string, number][] };
      w.__plays = [];
      const proto = HTMLMediaElement.prototype;
      const original = Object.getOwnPropertyDescriptor(proto, "play")?.value as (
        this: HTMLMediaElement,
      ) => Promise<void>;
      proto.play = function (this: HTMLMediaElement) {
        w.__plays.push([this.src.split("/").pop() ?? "", Math.round(performance.now())]);
        return original.call(this).catch(() => undefined);
      };
    },
    [r.round, tile] as const,
  );
  await page.goto("/");
  const s = page.getByTestId("load-screen");
  await expect(s).toHaveAttribute("role", "button", { timeout: 30_000 });
  await expect(page).toHaveURL(new RegExp(`/reveal/${r.round}`), { timeout: 30_000 });
  await page.waitForTimeout(4_000); // the reveal sequence finishes behind the screen
  const tapAt = await page.evaluate(() => Math.round(performance.now()));
  // Nothing played before the tap: sound was not allowed yet.
  expect(await page.evaluate(() => (window as unknown as { __plays: unknown[] }).__plays)).toEqual(
    [],
  );
  await page.mouse.click(700, 500);
  await expect(s).toHaveCount(0, { timeout: 4_000 });
  await page.waitForTimeout(1_000);
  const plays = await page.evaluate(
    () => (window as unknown as { __plays: [string, number][] }).__plays,
  );
  await expect(page.getByTestId("result-card")).toHaveAttribute("data-outcome", "not-this-time");
  const misses = plays.filter(([name]) => name === "miss.mp3");
  expect(misses).toHaveLength(1);
  // Once the load screen has gone (3.2 s after the tap), not under it.
  expect((misses[0]?.[1] ?? 0) - tapAt).toBeGreaterThanOrEqual(3_000);
});
