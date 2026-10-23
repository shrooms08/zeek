/**
 * The prize counts digs as soon as they land in the mempool: the big number
 * is the mined prize plus this round's landing digs, with "+X landing"
 * under it for the part not mined yet (src/lib/prize.ts). The engine's real
 * board is changed in three steps (a dig landing, the same dig mined, a
 * landing dig dropped from the mempool); the live stream is kept silent so
 * the steps stay as set.
 */

import { expect, test, type Page } from "@playwright/test";
import type { Board, LandingDig } from "../../src/api/types";
import { formatZec } from "../../src/lib/money";
import { prizeOfPot } from "../../src/lib/prize";
import { engineRequests } from "../engine";

const ENGINE = engineRequests("/v1/board");
const FEE = 5_000;
const VALUE = 1_000_000;

test.use({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce" });

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("zeek.seenOnboarding", "1");
  });
  await page.routeWebSocket(/\/v1\/stream/, () => undefined);
});

/** Serves the real board changed by `change`. */
async function board(page: Page, change: (b: Board) => Board) {
  await page.unroute(ENGINE);
  await page.route(ENGINE, async (route) => {
    const response = await route.fetch();
    const real = (await response.json()) as Board;
    await route.fulfill({ response, json: change(real) });
  });
}

function landing(round: number): LandingDig {
  return { id: "test-landing", round, tile: 3, value_zat: VALUE, memo_sha256: "00", first_seen: 1 };
}

const amount = (page: Page) => page.getByTestId("prize-amount");
const line = (page: Page) => page.getByTestId("prize-landing");

test("a landing dig counts at once, once, and only until it is mined or dropped", async ({
  page,
}) => {
  let mined: Board | undefined;
  // 1. A dig lands: the prize grows now, "+X landing" under it.
  await board(page, (b) => {
    mined = b;
    return { ...b, landing: [landing(b.round)] };
  });
  await page.goto("/");
  await expect(page.getByTestId("field")).toBeVisible({ timeout: 60_000 });
  const b = mined as Board;
  const pot = BigInt(b.gross_dug_zat - b.digs * FEE);
  const total = prizeOfPot(pot + BigInt(VALUE - FEE), 500n);
  const plus = total - BigInt(b.prize_pool_zat);
  await expect(amount(page)).toContainText(formatZec(total));
  await expect(line(page)).toHaveText(`+${formatZec(plus)} landing`);

  // 2. Mined: on the board, no longer landing. Same total, no line.
  await board(page, (real) => ({
    ...real,
    digs: real.digs + 1,
    gross_dug_zat: real.gross_dug_zat + VALUE,
    prize_pool_zat: Number(total),
    landing: [],
  }));
  await page.reload();
  await expect(amount(page)).toContainText(formatZec(total));
  await expect(line(page)).toHaveCount(0);

  // 3. A landing dig that left the mempool unmined: back to the mined prize.
  await board(page, (real) => ({ ...real, landing: [] }));
  await page.reload();
  await expect(amount(page)).toContainText(formatZec(BigInt(b.prize_pool_zat)));
  await expect(line(page)).toHaveCount(0);

  // A landing dig of another round never counts.
  await board(page, (real) => ({ ...real, landing: [landing(real.round + 1)] }));
  await page.reload();
  await expect(amount(page)).toContainText(formatZec(BigInt(b.prize_pool_zat)));
  await expect(line(page)).toHaveCount(0);
});
