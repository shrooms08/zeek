#!/usr/bin/env node
// Records a terminal replay (docs/video/terminal/player.html) at 1920×1080
// and writes docs/video/clips/<name>.mp4 and <name>.gif.
//
//   node apps/web/video/record-terminal.mjs refusal

import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const name = process.argv[2] ?? "refusal";
const port = 8098;
const server = spawn(
  "python3",
  [
    "-m",
    "http.server",
    "--bind",
    "127.0.0.1",
    "--directory",
    join(root, "docs/video/terminal"),
    String(port),
  ],
  { stdio: "ignore" },
);
await new Promise((r) => setTimeout(r, 800));
const dir = mkdtempSync(join(tmpdir(), "zeek-term-"));
const clips = join(root, "docs/video/clips");
try {
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1080 },
    recordVideo: { dir, size: { width: 1920, height: 1080 } },
  });
  const page = await context.newPage();
  await page.goto(`http://127.0.0.1:${port}/player.html?data=${name}.json`);
  await page.waitForSelector("body[data-done='1']", { timeout: 300_000 });
  await page.waitForTimeout(2_500);
  await context.close();
  await browser.close();
  const webm = join(
    dir,
    readdirSync(dir).find((f) => f.endsWith(".webm")),
  );
  const mp4 = join(clips, `${name}.mp4`);
  execFileSync("ffmpeg", [
    "-y",
    "-loglevel",
    "error",
    "-ss",
    "0.4",
    "-i",
    webm,
    "-c:v",
    "libx264",
    "-pix_fmt",
    "yuv420p",
    "-crf",
    "18",
    "-movflags",
    "+faststart",
    mp4,
  ]);
  const palette = join(dir, "palette.png");
  execFileSync("ffmpeg", [
    "-y",
    "-loglevel",
    "error",
    "-i",
    mp4,
    "-vf",
    "fps=10,scale=1280:-1:flags=lanczos,palettegen",
    palette,
  ]);
  execFileSync("ffmpeg", [
    "-y",
    "-loglevel",
    "error",
    "-i",
    mp4,
    "-i",
    palette,
    "-lavfi",
    "fps=10,scale=1280:-1:flags=lanczos[x];[x][1:v]paletteuse",
    join(clips, `${name}.gif`),
  ]);
} finally {
  server.kill();
  rmSync(dir, { recursive: true, force: true });
}
console.log(`wrote docs/video/clips/${name}.mp4 and .gif`);
