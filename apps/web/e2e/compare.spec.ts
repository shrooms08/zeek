/**
 * Visual check: the design frame (design/mono) beside the live app, at 1440
 * and 390, for each main screen. Saved to docs/examples/phase7b/compare/.
 *
 * The app side is the live engine and the funded Pocket (run after the live
 * steps). One exception, labelled FIXTURE in its image: no Deep Cache round
 * has happened on testnet yet (1 in 625 per round), so that reveal replays
 * round 188 with the Cache flag set.
 */
import { chromium, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { go, loadState, REPO, test, unlock } from "./live/support";

const OUT = path.join(REPO, "docs/examples/phase7b/compare");
const DESIGN = path.join(REPO, "design/mono");
const TMP = path.join(REPO, "data/web-e2e/compare");

interface Pair {
  name: string;
  size: 1440 | 390;
  /** Design page and the frame's caption or data-screen-label. */
  design: { page: string; caption?: string; label?: string };
  /** Drive the app to the screen; returns an optional note for the image. */
  app: (page: Page) => Promise<string | void>;
}

/** Screenshots a design frame by its caption (next sibling) or screen label. */
async function designFrame(page: Page, pageName: string, caption: string | undefined, label: string | undefined, file: string) {
  await page.goto(pathToFileURL(path.join(DESIGN, `${pageName}.html`)).href);
  await page.waitForTimeout(2500);
  const handle = await page.evaluateHandle(
    ([cap, lab]) => {
      if (lab !== undefined) return document.querySelector(`[data-screen-label="${lab}"]`);
      const own = (e: Element) =>
        [...e.childNodes]
          .filter((n) => n.nodeType === 3)
          .map((n) => n.textContent ?? "")
          .join("")
          .trim();
      const hit = [...document.querySelectorAll("*")].find((e) => own(e).startsWith(cap ?? "\u0000"));
      return hit?.nextElementSibling ?? null;
    },
    [caption, label] as const,
  );
  const el = handle.asElement();
  if (el === null) throw new Error(`design frame not found: ${pageName} ${caption ?? label ?? ""}`);
  await el.screenshot({ path: file });
}

async function compose(page: Page, design: string, app: string, out: string, title: string, note: string) {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const img = (f: string) => pathToFileURL(f).href;
  await page.setContent(`<html><body style="margin:0;background:#000;font:600 14px system-ui;color:#D6D6D6">
  <div id="sheet" style="display:inline-flex;gap:24px;padding:20px;background:#000;align-items:flex-start">
    <figure style="margin:0"><figcaption style="margin-bottom:8px;letter-spacing:.1em">DESIGN · ${title}</figcaption><img src="${img(design)}" style="display:block;outline:1px solid #3A3A3A"></figure>
    <figure style="margin:0"><figcaption style="margin-bottom:8px;letter-spacing:.1em">LIVE APP · ${title}${note === "" ? "" : ` · <span style="color:#E8B54A">${note}</span>`}</figcaption><img src="${img(app)}" style="display:block;outline:1px solid #3A3A3A"></figure>
  </div></body></html>`);
  await page.waitForTimeout(300);
  await page.locator("#sheet").screenshot({ path: out });
}

const seedDigs = (page: Page, round: number, tiles: number[]) =>
  page.evaluate(
    ([r, t]) => {
      localStorage.setItem("zeek.myDigs.v1", JSON.stringify([{ round: r, tiles: t, amountEach: "100000", via: "pocket", txids: [], at: Date.now() }]));
      localStorage.setItem("zeek.seenReveal", String(r));
    },
    [round, tiles] as const,
  );

test("design frames beside the live app", async ({ page }) => {
  test.setTimeout(30 * 60_000);
  const state = loadState();
  if (state.passphrase === undefined || state.dig === undefined) throw new Error("run the live steps first");
  const { passphrase, dig } = state;
  const winRound = state.win?.round ?? 188;
  mkdirSync(OUT, { recursive: true });
  mkdirSync(TMP, { recursive: true });

  const pickField = (tiles: number[]) => async (p: Page) => {
    await go(p, "/");
    await expect(p.getByTestId("field")).toBeVisible({ timeout: 60_000 });
    for (const t of tiles) await p.locator(`[data-tile="${t}"]`).click();
  };
  const reveal = (round: number, tiles: number[]) => async (p: Page) => {
    await seedDigs(p, round, tiles);
    await go(p, "/rounds");
    await go(p, `/reveal/${round}`);
    await expect(p.getByTestId("result-card")).toBeVisible({ timeout: 60_000 });
    await p.waitForTimeout(3500);
  };
  const route = (to: string, ready: string) => async (p: Page) => {
    await go(p, to);
    await expect(p.getByTestId(ready).first()).toBeVisible({ timeout: 60_000 });
    await p.waitForTimeout(1200);
  };
  const cacheFixture = async (p: Page) => {
    await p.route(`**/v1/rounds/${winRound}`, async (r) => {
      const res = await r.fetch();
      const body = (await res.json()) as { settled: { outcome: { cache_triggered?: boolean }; cache_triggered: boolean; cache_paid: number } };
      body.settled.outcome.cache_triggered = true;
      body.settled.cache_triggered = true;
      body.settled.cache_paid = 1_849_000_000;
      await r.fulfill({ response: res, json: body });
    });
    await reveal(winRound, Array.from({ length: 25 }, (_, i) => i))(p);
    await p.waitForTimeout(3000);
    return "FIXTURE: no Deep Cache round on testnet yet";
  };

  const pairs: Pair[] = [
    { name: "field", size: 1440, design: { page: "Zeek Field", label: "Field desktop" }, app: pickField([6, 12, 18]) },
    { name: "field", size: 390, design: { page: "Zeek Field", label: "Field mobile selected" }, app: pickField([6, 12]) },
    {
      name: "dig-confirm",
      size: 390,
      design: { page: "Zeek Dig", caption: "3A · CONFIRM" },
      app: async (p) => {
        await pickField([6, 12, 18])(p);
        await p.getByTestId("dig-button").click();
        await expect(p.getByTestId("dig-confirm").or(p.getByTestId("dig-short"))).toBeVisible({ timeout: 60_000 });
      },
    },
    {
      name: "dig-confirm",
      size: 1440,
      design: { page: "Zeek Dig", caption: "4F · DESKTOP · SCAN STEP" },
      app: async (p) => {
        await pickField([6, 12, 18])(p);
        await p.getByTestId("dig-button").click();
        await expect(p.getByTestId("dig-confirm").or(p.getByTestId("dig-short"))).toBeVisible({ timeout: 60_000 });
        return "the design's desktop dig frame is the other-wallet scan step";
      },
    },
    { name: "reveal-win", size: 390, design: { page: "Zeek Reveal", label: "Reveal win" }, app: reveal(winRound, Array.from({ length: 25 }, (_, i) => i)) },
    { name: "reveal-lose", size: 390, design: { page: "Zeek Reveal", label: "Reveal lose" }, app: reveal(dig.round, dig.tiles) },
    { name: "reveal-deep-cache", size: 390, design: { page: "Zeek Reveal", label: "Reveal deep cache" }, app: cacheFixture },
    { name: "reveal-deep-cache", size: 1440, design: { page: "Zeek Reveal", label: "Reveal deep cache desktop" }, app: cacheFixture },
    { name: "past-rounds", size: 1440, design: { page: "Zeek More", caption: "7 · LIST · DESKTOP 1440" }, app: route("/rounds", "round-row") },
    { name: "past-rounds", size: 390, design: { page: "Zeek More", caption: "7 · LIST · MOBILE 390" }, app: route("/rounds", "round-row") },
    {
      name: "check",
      size: 1440,
      design: { page: "Zeek More", caption: "8 · IN PROGRESS · DESKTOP 1440" },
      app: async (p) => {
        await go(p, `/check/${dig.round}`);
        await p.getByTestId("check-run").click();
        await expect(p.getByTestId("check-summary")).toBeVisible({ timeout: 10 * 60_000 });
      },
    },
    {
      name: "check",
      size: 390,
      design: { page: "Zeek More", caption: "8 · IN PROGRESS · MOBILE 390" },
      app: async (p) => {
        await go(p, `/check/${dig.round}`);
        await p.getByTestId("check-run").click();
        await expect(p.getByTestId("check-summary")).toBeVisible({ timeout: 10 * 60_000 });
      },
    },
    { name: "wallet", size: 1440, design: { page: "Zeek More", caption: "9F · DESKTOP · HOME" }, app: route("/wallet", "wallet-home") },
    { name: "wallet", size: 390, design: { page: "Zeek More", caption: "9D · HOME" }, app: route("/wallet", "wallet-home") },
    {
      name: "prove-card",
      size: 1440,
      design: { page: "Zeek More", caption: "10A · CARD" },
      app: async (p) => {
        const id = state.proofId ?? "a9cce3fda3c035c555468fb3ae306ec7";
        await p.goto(`http://127.0.0.1:8787/share/win/${id}.png`);
        return "engine share PNG";
      },
    },
    {
      name: "verified-win",
      size: 1440,
      design: { page: "Zeek More", caption: "10B · VERIFIED WIN PAGE · DESKTOP 1440" },
      app: route(`/win/${state.proofId ?? "a9cce3fda3c035c555468fb3ae306ec7"}`, "win-verdict"),
    },
    {
      name: "verified-win",
      size: 390,
      design: { page: "Zeek More", caption: "10B · VERIFIED WIN PAGE · MOBILE 390" },
      app: route(`/win/${state.proofId ?? "a9cce3fda3c035c555468fb3ae306ec7"}`, "win-verdict"),
    },
    { name: "keys", size: 1440, design: { page: "Zeek More", caption: "11 · DESKTOP 1440" }, app: route("/keys", "keyholder-card") },
    { name: "keys", size: 390, design: { page: "Zeek More", caption: "11 · MOBILE 390" }, app: route("/keys", "keyholder-card") },
    { name: "campfire", size: 1440, design: { page: "Zeek More", caption: "12 · DESKTOP 1440" }, app: route("/campfire", "campfire-message") },
    { name: "campfire", size: 390, design: { page: "Zeek More", caption: "12 · MOBILE 390" }, app: route("/campfire", "campfire-message") },
    {
      name: "first-time",
      size: 390,
      design: { page: "Zeek Reveal", caption: "6A · STEP 1" },
      app: async (p) => {
        await p.evaluate(() => {
          localStorage.removeItem("zeek.seenOnboarding");
        });
        await p.goto("/");
        await expect(p.getByTestId("onboarding")).toBeVisible({ timeout: 60_000 });
        await p.waitForTimeout(800);
      },
    },
    {
      name: "how-to-play",
      size: 1440,
      design: { page: "Zeek Reveal", caption: "6D · DESKTOP" },
      app: route("/how", "how-to-play"),
    },
  ];

  const browser = await chromium.launch();
  const designPage = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  const composer = await browser.newPage();
  for (const pair of pairs) {
    const base = `${pair.name}-${pair.size}`;
    const designFile = path.join(TMP, `${base}-design.png`);
    const appFile = path.join(TMP, `${base}-app.png`);
    await designFrame(designPage, pair.design.page, pair.design.caption, pair.design.label, designFile);
    await page.setViewportSize(pair.size === 1440 ? { width: 1440, height: 900 } : { width: 390, height: 844 });
    await unlock(page, passphrase);
    const note = (await pair.app(page)) ?? "";
    await page.screenshot({ path: appFile });
    await page.unrouteAll({ behavior: "ignoreErrors" });
    await compose(composer, designFile, appFile, path.join(OUT, `${base}.png`), `${pair.name} · ${pair.size}`, note);
    await page.evaluate(() => {
      localStorage.setItem("zeek.seenOnboarding", "1");
    });
  }
  await browser.close();
});
