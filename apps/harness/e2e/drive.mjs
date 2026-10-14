// Drives the dev harness in headless Chromium for the live testnet run.
// Usage: node e2e/drive.mjs <command> [args]
//   create <passphrase>               create a Pocket wallet, print its address
//   sync <passphrase>                 unlock, sync, print balance and history
//   dig <passphrase> <tiles> <zec>    unlock, dig from the browser
//   anywallet <payout> <tiles> <zec>  build a ZIP 321 request, print the URI
//   shot <name>                       screenshot of the current page state
// The browser profile (IndexedDB) persists in ../../data/harness-profile.
// Screenshots go to ../../docs/examples/phase5/.
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../..");
const profile = path.join(repo, "data", "harness-profile");
const shots = path.join(repo, "docs", "examples", "phase5");
const url = process.env.HARNESS_URL ?? "http://localhost:5173/";
const [command, ...args] = process.argv.slice(2);

await mkdir(shots, { recursive: true });
const context = await chromium.launchPersistentContext(profile, {
  headless: true,
  viewport: { width: 1500, height: 1500 },
});
const page = context.pages()[0] ?? (await context.newPage());
page.on("console", (m) => {
  if (m.type() === "error") console.error("[page]", m.text());
});
page.on("pageerror", (e) => console.error("[pageerror]", String(e)));
page.on("worker", (w) => {
  w.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warning") console.error("[worker]", m.text());
  });
});

const text = async (id) => (await page.getByTestId(id).textContent()) ?? "";
async function waitFor(id, done, timeoutMs) {
  const start = Date.now();
  for (;;) {
    const value = await text(id);
    if (done(value)) return value;
    if (Date.now() - start > timeoutMs) throw new Error(`timeout waiting for ${id}: ${value}`);
    await page.waitForTimeout(1_000);
  }
}
// A dig or send is finished when its output shows the JSON result or an
// error line (`code: message`); sync progress lines (`blocks:`, `memos:`,
// `done:`) and the "syncing…" placeholder are not results.
const notBusy = (v) =>
  v.startsWith("{") || (/^[a-z_]+: /.test(v) && !/^(blocks|memos|done):/.test(v));
async function shot(name) {
  const file = path.join(shots, `${name}.png`);
  await page.screenshot({ path: file, fullPage: true });
  console.log(`screenshot ${file}`);
}
async function unlock(passphrase) {
  await page.getByTestId("passphrase").fill(passphrase);
  await page.getByTestId("unlock").click();
  await waitFor("pocket-status", (v) => v.startsWith("unlocked") || v.includes(":"), 120_000);
  const status = await text("pocket-status");
  if (!status.startsWith("unlocked")) throw new Error(status);
}
async function sync() {
  await page.getByTestId("sync").click();
  await page.waitForTimeout(500);
  await waitFor(
    "sync-progress",
    (v) =>
      v.startsWith("done") ||
      v.includes("error") ||
      (/^[a-z_]+: /.test(v) && !/^(blocks|memos)/.test(v)),
    30 * 60_000,
  );
  await page.getByTestId("refresh").click();
  await page.waitForTimeout(1_500);
}
async function report() {
  const rows = await page.getByTestId("history").locator("tr").allTextContents();
  console.log(
    JSON.stringify(
      {
        status: await text("pocket-status"),
        address: await text("pocket-address"),
        balance: await text("pocket-balance"),
        progress: await text("sync-progress"),
        history: rows,
      },
      null,
      2,
    ),
  );
}

await page.goto(url);
await page.getByTestId("banner").waitFor();
console.log(await text("banner"));

switch (command) {
  case "create": {
    await page.getByTestId("passphrase").fill(args[0]);
    await page.getByTestId("create").click();
    await waitFor("pocket-status", (v) => v.startsWith("unlocked") || v.includes(":"), 180_000);
    await page.waitForTimeout(1_000);
    await report();
    await shot("01-pocket-created");
    break;
  }
  case "sync": {
    await unlock(args[0]);
    await sync();
    await report();
    await shot(args[1] ?? "pocket-synced");
    break;
  }
  case "dig": {
    await unlock(args[0]);
    await page.getByTestId("dig-tiles").fill(args[1]);
    await page.getByTestId("dig-amount").fill(args[2]);
    const started = Date.now();
    await page.getByTestId("dig").click();
    await page.waitForTimeout(500);
    const result = await waitFor("dig-result", notBusy, 30 * 60_000);
    console.log(`dig finished in ${Math.round((Date.now() - started) / 1000)} s`);
    console.log(result);
    await report();
    await shot(args[3] ?? "pocket-dig");
    break;
  }
  case "anywallet": {
    await page.getByTestId("payout-address").fill(args[0]);
    await page.getByTestId("aw-tiles").fill(args[1]);
    await page.getByTestId("aw-amount").fill(args[2]);
    if (args[3]) await page.getByTestId("aw-round").fill(args[3]);
    await page.getByTestId("aw-build").click();
    await waitFor("aw-result", (v) => v.length > 0, 120_000);
    console.log(await text("aw-result"));
    console.log(`URI ${await page.getByTestId("aw-uri").inputValue()}`);
    await shot(args[4] ?? "anywallet-request");
    break;
  }
  case "shot": {
    await page.waitForTimeout(3_000);
    await shot(args[0]);
    break;
  }
  default:
    console.error("unknown command");
    process.exitCode = 2;
}
await context.close();
