#!/usr/bin/env node
// Measures how late the testnet chain ran against the game's schedule, over
// every settled round the engine has seen, and the smallest reveal delay
// that would have voided no round (docs/examples/phase8c/reveal-margin.md).
//
//   node scripts/reveal-margin.mjs [--engine http://127.0.0.1:8787] [--json out.json]
//
// From each round's proof bundle (the engine's own record of the chain):
//   start          nTime of the window's first block
//   close          nTime of block `end` (the first block after the window)
//   mtp            median of the 11 blocks after the window (the timing rule)
// Per round:
//   duration       close − start, against blocks_per_round · target_spacing
//   drift          start − the schedule (round 0's start + r · round length)
//   mtp_lag        mtp − (start + round length): how far after the scheduled
//                  close the chain's median time was
//   needed_delay   mtp + skew − (start + round length) + 1: the smallest
//                  reveal delay with which this round passes the rule
//   ready_after    when the reveal could settle, after the actual close:
//                  max(beacon time, 11th post-close block) − close

import { writeFileSync } from "node:fs";

const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && args[i + 1] !== undefined ? args[i + 1] : fallback;
};
const engine = arg("engine", "http://127.0.0.1:8787").replace(/\/+$/, "");
const out = arg("json", "");

async function get(path) {
  const response = await fetch(`${engine}${path}`, { signal: AbortSignal.timeout(30_000) });
  if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
  return response.json();
}

const board = await get("/v1/board");
const rows = [];
let params;
for (let round = 0; round < board.round; round += 1) {
  let b;
  try {
    b = await get(`/v1/rounds/${round}/proof`);
  } catch {
    continue; // not settled yet
  }
  params = b.params;
  const length = b.params.blocks_per_round * b.params.target_spacing_secs;
  const skew = b.params.honest_clock_skew_secs;
  const post = b.post_close_block_times;
  const mtpWindow = b.params.mtp_window;
  const sorted = [...post.slice(0, mtpWindow)].sort((x, y) => x - y);
  const mtp = sorted[Math.floor(sorted.length / 2)];
  const start = b.start_block_time;
  const close = post[0];
  rows.push({
    round,
    digs: b.notes.length,
    start,
    close,
    mtp,
    duration: close - start,
    mtp_lag: mtp - (start + length),
    needed_delay: mtp + skew - (start + length) + 1,
    ready_after: Math.max(b.beacon_time, post[mtpWindow - 1]) - close,
    verdict: b.timing.verdict,
    margin: b.timing.margin_secs ?? -(b.timing.deficit_secs ?? 0),
    reveal_delay: b.params.reveal_delay_secs,
  });
}
if (params === undefined) throw new Error("no settled round");
const length = params.blocks_per_round * params.target_spacing_secs;
const first = rows[0];
for (const r of rows) r.drift = r.start - (first.start + (r.round - first.round) * length);

function stats(name, values) {
  const v = [...values].sort((a, b) => a - b);
  const q = (p) => v[Math.min(v.length - 1, Math.floor(p * v.length))];
  return { name, n: v.length, min: v[0], p50: q(0.5), p90: q(0.9), p99: q(0.99), max: v.at(-1) };
}
const summary = {
  rounds: rows.length,
  first_round: first.round,
  last_round: rows.at(-1).round,
  round_length_secs: length,
  reveal_delay_secs: params.reveal_delay_secs,
  skew_secs: params.honest_clock_skew_secs,
  voided: rows.filter((r) => r.verdict !== "safe").map((r) => r.round),
  stats: [
    stats(
      "duration (s, scheduled " + length + ")",
      rows.map((r) => r.duration),
    ),
    stats(
      "drift vs schedule (s)",
      rows.map((r) => r.drift),
    ),
    stats(
      "mtp_lag (s)",
      rows.map((r) => r.mtp_lag),
    ),
    stats(
      "needed_delay (s)",
      rows.map((r) => r.needed_delay),
    ),
    stats(
      "ready_after close (s)",
      rows.map((r) => r.ready_after),
    ),
    stats(
      "actual margin (s)",
      rows.map((r) => r.margin),
    ),
  ],
  smallest_delay_zero_voids: Math.max(...rows.map((r) => r.needed_delay)),
  worst_mtp_lag: Math.max(...rows.map((r) => r.mtp_lag)),
};
console.log(JSON.stringify(summary, null, 2));
if (out !== "") writeFileSync(out, JSON.stringify({ summary, rows }, null, 2));
