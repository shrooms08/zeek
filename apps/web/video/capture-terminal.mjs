#!/usr/bin/env node
// Runs real commands and records their output with timings, for the
// terminal player (docs/video/terminal/player.html). Nothing is written by
// hand: each step is either a shell comment shown as typed, or a command
// whose real stdout and stderr are captured as they arrive.
//
//   node apps/web/video/capture-terminal.mjs refusal

import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const LOG = "docs/examples/phase4b/rogue-coordinator-refusals.log";

const scripts = {
  refusal: [
    { comment: "# Zcash testnet, round 17, 2026-10-14: the payout coordinator was built with" },
    { comment: "# --features rogue-demo and redirected a payout to its own address." },
    { comment: "# The engine's and keyholders' logs from that live run:" },
    { cmd: `grep "ROGUE DEMO" ${LOG}` },
    { cmd: `grep "REFUSED to sign" ${LOG}` },
    { cmd: `grep -o "not enough keyholders approved the payout: 0 of 2" ${LOG}` },
    { comment: "# The same payout checks, run now against tampered payouts:" },
    {
      cmd: "cargo test --release -p zeek-keyholder --test verify_pczt 2>&1 | grep -E '^test |test result'",
    },
  ],
};

const name = process.argv[2] ?? "refusal";
const steps = scripts[name];
if (steps === undefined) throw new Error(`unknown script ${name}`);
const events = [];
const t0 = Date.now();
for (const step of steps) {
  if ("comment" in step) {
    events.push({ at: Date.now() - t0, type: "comment", text: step.comment });
    continue;
  }
  events.push({ at: Date.now() - t0, type: "cmd", text: step.cmd });
  await new Promise((done, fail) => {
    const child = spawn("sh", ["-c", step.cmd], {
      cwd: root,
      env: { ...process.env, CARGO_TERM_COLOR: "never" },
    });
    const take = (data) =>
      events.push({ at: Date.now() - t0, type: "out", text: data.toString("utf8") });
    child.stdout.on("data", take);
    child.stderr.on("data", take);
    child.on("error", fail);
    child.on("close", (code) => {
      events.push({ at: Date.now() - t0, type: "exit", code });
      done();
    });
  });
}
const out = join(root, "docs/video/terminal", `${name}.json`);
writeFileSync(out, JSON.stringify({ name, recorded: new Date(t0).toISOString(), events }, null, 1));
console.log(`wrote ${out} (${events.length} events)`);
