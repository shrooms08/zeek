/**
 * Live 5: a guaranteed win. The Pocket digs all 25 tiles at 0.001 in one
 * round (5 batched payments of up to 6 tiles; later batches wait for change
 * to confirm), the reveal shows the win, the Pocket proves it, and the share
 * URL, the Verified win page and /share/win/{id} (OG meta + 1200x675 PNG)
 * all check out.
 */
import {
  SHARE,
  engine,
  go,
  expect,
  loadState,
  saveState,
  shot,
  synced,
  test,
  unlock,
  until,
} from "./support";

test("25 tiles, a win, a proof, its share page and the Verified win page", async ({
  page,
  request,
}) => {
  const state = loadState();
  if (state.passphrase === undefined) throw new Error("run step 01 first");
  await unlock(page, state.passphrase);

  let round = state.win?.round;
  if (round === undefined) {
    const partial = state.winPartial;
    const now = await engine<{ round: number; status: string; blocks_remaining: number }>(
      "/v1/board",
    );
    // Only resume while the stopped dig's round has time for the rest.
    const resumeFrom =
      partial !== undefined && now.round === partial.round && now.blocks_remaining >= 8
        ? partial
        : undefined;
    const resume = resumeFrom !== undefined;
    if (!resume) {
      // Start right after a round opens: 5 batches with change waits take minutes.
      await until(
        "a fresh round (21+ blocks left)",
        async () => {
          const b = await engine<{ status: string; blocks_remaining: number }>("/v1/board");
          return b.status === "open" && b.blocks_remaining >= 21 ? true : undefined;
        },
        30 * 60_000,
        5_000,
      );
    }
    await go(page, "/");
    await expect(page.getByTestId("field").or(page.getByTestId("reveal-screen"))).toBeVisible({
      timeout: 60_000,
    });
    // The field opens an unseen reveal of a round this browser dug, once.
    if (page.url().includes("/reveal/")) {
      await expect(page.getByTestId("result-card")).toBeVisible({ timeout: 60_000 });
      await go(page, "/");
    }
    await expect(page.getByTestId("field-screen")).toBeVisible({ timeout: 60_000 });
    // Resuming a dig that stopped part-way digs exactly the tiles not sent.
    const tiles = resumeFrom?.unsent ?? Array.from({ length: 25 }, (_, t) => t);
    // Several tiles: Pro (Lite digs one).
    await page.getByTestId("mode-pro").click();
    for (const t of tiles) await page.locator(`[data-tile="${t}"]`).click();
    await page.getByTestId("amount-input").fill("0.001");
    await page.getByTestId("amount-input").press("Enter");
    await expect(page.getByTestId("tile-count")).toHaveText(String(tiles.length));
    if (!resume) {
      // 25 x 0.001 + fees: four 6-tile payments (0.00035) and one 1-tile (0.0001).
      await expect(page.getByTestId("total-amount")).toHaveText("0.0265");
    }
    await page.getByTestId("dig-button").click();
    await expect(page.getByTestId("dig-confirm")).toBeVisible({ timeout: 60_000 });
    await page.getByTestId("dig-send").click();
    await expect(page.getByTestId("batch-progress")).toBeVisible({ timeout: 120_000 });
    await shot(page, resume ? "dig-pocket-batches-resumed-1440" : "dig-pocket-batches-1440");
    const done = page.getByTestId("dig-done");
    const stopped = page.getByTestId("dig-partial");
    await expect(done.or(stopped).or(page.getByTestId("dig-error"))).toBeVisible({
      timeout: 30 * 60_000,
    });
    if (await stopped.isVisible()) {
      // A real partial dig: record it so a rerun digs only the rest.
      const text = (await stopped.textContent()) ?? "";
      await shot(page, "dig-pocket-partial-1440");
      const notSent = /Not sent: tiles ([0-9,]+)/.exec(text)?.[1]?.split(",").map(Number) ?? [];
      const roundNow = (await engine<{ round: number }>("/v1/board")).round;
      saveState({
        winPartial: {
          round: roundNow,
          txids: [...(resumeFrom?.txids ?? []), ...(text.match(/[0-9a-f]{64}/g) ?? [])],
          unsent: notSent,
        },
      });
      throw new Error(`the dig stopped part-way: ${text}`);
    }
    await expect(done).toBeVisible();
    round = Number(/round (\d+)/.exec((await done.textContent()) ?? "")?.[1]);
    const txids = [
      ...(resumeFrom?.txids ?? []),
      ...(((await page.getByTestId("dig-txids").textContent()) ?? "").match(/[0-9a-f]{64}/g) ?? []),
    ];
    expect(txids).toHaveLength(5);
    saveState({ win: { round, txids } });
  }

  // The reveal: a win (all 25 tiles are ours).
  await go(page, `/reveal/${round}`);
  await until(
    `round ${round} to settle`,
    async () => (await engine<{ settled: unknown }>(`/v1/rounds/${round}`)).settled ?? undefined,
    60 * 60_000,
    20_000,
  );
  const card = page.getByTestId("result-card");
  await expect(card).toBeVisible({ timeout: 5 * 60_000 });
  await expect(card).toHaveAttribute("data-outcome", /^(win|cache-win)$/);
  await shot(page, "reveal-win-1440");

  // The payout must be mined and in the Pocket before it can be proven.
  await until(
    `round ${round} paid`,
    async () => {
      const list = await engine<{ rounds: { round: number; status: string }[] }>(
        `/v1/rounds?limit=50`,
      );
      return list.rounds.find((r) => r.round === round && r.status === "paid");
    },
    60 * 60_000,
    20_000,
  );
  await unlock(page, state.passphrase);
  for (let i = 0; ; i++) {
    await page.getByRole("button", { name: "Sync" }).click();
    await synced(page);
    if (
      (await page.getByTestId("wallet-history").textContent())?.includes(
        `Zeek round ${round}: treasure`,
      )
    )
      break;
    if (i > 30) throw new Error("the treasure payout never reached the Pocket");
    await page.waitForTimeout(30_000);
  }
  await shot(page, "wallet-home-1440");

  let id = state.proofId;
  if (id === undefined) {
    await go(page, `/prove/${round}`);
    await page.getByTestId("prove-create").click();
    await expect(page.getByTestId("prove-done")).toBeVisible({ timeout: 15 * 60_000 });
    const img = page.getByTestId("prove-card");
    await expect
      .poll(() => img.evaluate((e) => (e as HTMLImageElement).naturalWidth), { timeout: 60_000 })
      .toBe(1200);
    await shot(page, "prove-win-1440");
    const link = (await page.getByTestId("prove-link").textContent()) ?? "";
    id = /share\/win\/([0-9a-f]{32})/.exec(link)?.[1] ?? "";
    expect(id).toMatch(/^[0-9a-f]{32}$/);
    saveState({ proofId: id });
  }

  // The share URL: humans are sent on to the Verified win page.
  await page.goto(`${SHARE}/share/win/${id}`);
  await expect(page).toHaveURL(new RegExp(`/win/${id}$`), { timeout: 30_000 });
  await expect(page.getByTestId("win-verdict")).toHaveText("Verified", { timeout: 10 * 60_000 });
  await expect(page.getByTestId("win-amount")).toBeVisible();
  await shot(page, "verified-win-1440");

  // What X sees: OG/Twitter meta pointing at a 1200x675 PNG.
  const html = await (await request.get(`${SHARE}/share/win/${id}`)).text();
  expect(html).toContain('<meta name="twitter:card" content="summary_large_image"');
  expect(html).toContain(`<meta property="og:image" content="${SHARE}/share/win/${id}.png"`);
  expect(html).toContain(`I found the treasure in round ${round}`);
  const png = await request.get(`${SHARE}/share/win/${id}.png`);
  expect(png.headers()["content-type"]).toBe("image/png");
  const bytes = await png.body();
  expect(bytes.readUInt32BE(16)).toBe(1200);
  expect(bytes.readUInt32BE(20)).toBe(675);
});
