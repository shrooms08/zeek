/**
 * Shared pieces of the live testnet suite. Every step drives the real app
 * against the real engine; the only shortcuts are the test players
 * (zeek-dig, standing in for "another wallet") and reading the backup
 * words off the screen to answer the quick check.
 *
 * State shared between steps lives in data/web-e2e/state.json (gitignored):
 * the Pocket passphrase (testnet only), round numbers and the proof id.
 */
import { test as base, chromium, expect, type BrowserContext, type Page } from "@playwright/test";
import { execFile } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const run = promisify(execFile);
export const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");
const DIR = path.join(REPO, "data", "web-e2e");
const STATE = path.join(DIR, "state.json");
export const SHOTS = path.join(REPO, "docs", "examples", "phase7a");
export const ENGINE = process.env.VITE_ZEEK_ENGINE_URL ?? "http://127.0.0.1:8787";
export const APP = "http://localhost:4174";

export interface LiveState {
  passphrase?: string;
  funded?: { name: string; txid: string; amount: string }[];
  dig?: { round: number; tiles: number[]; message: string; txids: string[] };
  win?: { round: number; txids: string[] };
  /** A 25-tile dig that stopped part-way: what's left to dig in that round. */
  winPartial?: { round: number; txids: string[]; unsent: number[] };
  proofId?: string;
  other?: { round: number; tile: number };
}

export function loadState(): LiveState {
  try {
    return JSON.parse(readFileSync(STATE, "utf8")) as LiveState;
  } catch {
    return {};
  }
}

export function saveState(update: Partial<LiveState>): LiveState {
  mkdirSync(DIR, { recursive: true });
  const next = { ...loadState(), ...update };
  writeFileSync(STATE, JSON.stringify(next, null, 2), { mode: 0o600 });
  return next;
}

/** Runs the zeek-dig test player CLI (release build). */
export async function zeekDig(...args: string[]): Promise<string> {
  const { stdout } = await run(path.join(REPO, "target/release/zeek-dig"), args, {
    cwd: REPO,
    maxBuffer: 16 * 1024 * 1024,
    timeout: 20 * 60_000,
  });
  return stdout;
}

export async function engine<T>(route: string): Promise<T> {
  const response = await fetch(`${ENGINE}${route}`);
  if (!response.ok) throw new Error(`${route}: ${response.status}`);
  return (await response.json()) as T;
}

/** Polls `check` every `everyMs` until it returns a value. */
export async function until<T>(
  what: string,
  check: () => Promise<T | undefined>,
  timeoutMs: number,
  everyMs = 10_000,
): Promise<T> {
  const start = Date.now();
  for (;;) {
    const value = await check();
    if (value !== undefined) return value;
    if (Date.now() - start > timeoutMs) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, everyMs));
  }
}

export async function shot(page: Page, name: string): Promise<void> {
  mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({ path: path.join(SHOTS, `${name}.png`), fullPage: true });
}

/** One persistent browser profile (IndexedDB holds the Pocket). */
export const test = base.extend<{ context: BrowserContext; page: Page }>({
  // eslint-disable-next-line no-empty-pattern
  context: async ({}, use) => {
    const context = await chromium.launchPersistentContext(path.join(DIR, "profile"), {
      headless: true,
      viewport: { width: 1440, height: 1000 },
      baseURL: APP,
    });
    // Actions fail after a minute instead of waiting out the test timeout.
    context.setDefaultTimeout(60_000);
    await use(context);
    await context.close();
  },
  page: async ({ context }, use) => {
    const page = context.pages()[0] ?? (await context.newPage());
    page.on("pageerror", (e) => {
      console.error("[pageerror]", String(e));
    });
    // Mark the explainer seen so it doesn't cover the flows (smoke tests it).
    await page.addInitScript(() => {
      localStorage.setItem("zeek.seenOnboarding", "1");
    });
    await use(page);
  },
});

export { expect };

/**
 * Navigates inside the app without reloading. A reload restarts the Pocket
 * worker, which locks the wallet (by design: the key lives in its memory).
 */
export async function go(page: Page, route: string): Promise<void> {
  await page.evaluate((to) => {
    window.history.pushState({}, "", to);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, route);
}

/** Unlocks the Pocket if it's locked, then waits for the wallet home. */
export async function unlock(page: Page, passphrase: string): Promise<void> {
  await page.goto("/wallet");
  const home = page.getByTestId("wallet-home");
  const form = page.getByTestId("wallet-unlock");
  await expect(home.or(form)).toBeVisible({ timeout: 60_000 });
  if (await form.isVisible()) {
    await page.getByTestId("unlock-pass").fill(passphrase);
    await page.getByTestId("unlock-submit").click();
    await expect(home).toBeVisible({ timeout: 120_000 });
  }
}

/** Waits for the wallet's sync line to say synced. */
export async function synced(page: Page): Promise<void> {
  await expect(page.getByTestId("wallet-sync")).toContainText("Synced to block", {
    timeout: 30 * 60_000,
  });
}

/** Spendable balance in zatoshis, read off the wallet screen. */
export async function balanceZat(page: Page): Promise<bigint> {
  const text = (await page.getByTestId("wallet-balance").textContent()) ?? "";
  const match = /([0-9]+)(?:\.([0-9]+))?/.exec(text.replace(/,/g, ""));
  if (match === null) throw new Error(`no balance in "${text}"`);
  return (
    BigInt(match[1] ?? "0") * 100_000_000n + BigInt((match[2] ?? "").padEnd(8, "0").slice(0, 8))
  );
}
