#!/usr/bin/env node
// Saves the last settled rounds into the web build for Replay mode (shown
// while the engine is offline): real data only, copied from the engine.
//
//   node scripts/snapshot-rounds.mjs --out apps/web/dist/replay \
//     [--engine http://127.0.0.1:8787] [--count 20]
//
// Writes:
//   config.json           the public config (params, vault address, UIVK)
//   index.json            the saved rounds (number, status, treasure tile)
//   rounds/N.proof.json   each round's proof bundle, as the engine serves it
//   rounds/N.replay.json  its replay (zeek-verify replay: depth stages per
//                         block, the draw, the payout; never amounts per tile)
//
// It refuses a mainnet engine whose digging is off: that config withholds
// the vault, which must not be published (docs/mainnet-runbook.md).

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, rmSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && args[i + 1] !== undefined ? args[i + 1] : fallback;
};
const engine = arg("engine", "http://127.0.0.1:8787").replace(/\/+$/, "");
const out = resolve(arg("out", ""));
const count = Number(arg("count", "20"));
const verify = join(root, "target/release/zeek-verify");

function fail(message) {
  console.error(`snapshot-rounds: ${message}`);
  process.exit(1);
}

if (arg("out", "") === "") fail("--out DIR is required");
if (!Number.isInteger(count) || count < 1 || count > 100) fail("--count must be 1..100");

async function get(path) {
  const response = await fetch(`${engine}${path}`, { signal: AbortSignal.timeout(30_000) });
  if (!response.ok) fail(`${path}: HTTP ${response.status}`);
  return response.json();
}

const config = await get("/v1/config");
if (config.digging_enabled === false || config.vault_address === undefined) {
  fail("the engine withholds its vault (digging off): nothing is saved");
}
const publicConfig = {
  params: config.params,
  vault_address: config.vault_address,
  uivk: config.uivk,
};

// The newest settled rounds (a round still being paid is not final yet).
const settled = [];
let before;
while (settled.length < count) {
  const page = await get(`/v1/rounds?limit=50${before === undefined ? "" : `&before=${before}`}`);
  for (const r of page.rounds) {
    if (r.status !== "paying" && settled.length < count) settled.push(r);
  }
  if (page.next_before === null || page.next_before === undefined) break;
  before = page.next_before;
}
if (settled.length === 0) fail("the engine has no settled round yet");

rmSync(out, { recursive: true, force: true });
mkdirSync(join(out, "rounds"), { recursive: true });
const scratch = mkdtempSync(join(tmpdir(), "zeek-snapshot-"));
const index = [];
for (const summary of settled) {
  const bundle = await get(`/v1/rounds/${summary.round}/proof`);
  const text = JSON.stringify(bundle);
  writeFileSync(join(out, "rounds", `${summary.round}.proof.json`), text);
  const file = join(scratch, `${summary.round}.json`);
  writeFileSync(file, text);
  const replay = execFileSync(verify, ["replay", "--bundle", file], { encoding: "utf8" });
  writeFileSync(join(out, "rounds", `${summary.round}.replay.json`), replay.trim());
  index.push({
    round: summary.round,
    status: summary.status,
    treasure_tile: summary.treasure_tile,
    closed_time: summary.closed_time,
  });
}
rmSync(scratch, { recursive: true, force: true });
writeFileSync(join(out, "config.json"), JSON.stringify(publicConfig));
writeFileSync(
  join(out, "index.json"),
  JSON.stringify({
    saved_at: Math.floor(Date.now() / 1000),
    network: config.params.network,
    rounds: index,
  }),
);
console.log(
  `snapshot-rounds: saved rounds ${index.at(-1).round}..${index[0].round} (${index.length}) to ${out}`,
);
