#!/usr/bin/env node
// Records the public-board illustration (docs/video/public-board) for 15 s
// at 1920×1080 and writes docs/video/clips/public-board.mp4.
//
//   node apps/web/video/record-public-board.mjs

import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const port = 8099;
const server = spawn(
  "python3",
  [
    "-m",
    "http.server",
    "--bind",
    "127.0.0.1",
    "--directory",
    join(root, "docs/video/public-board"),
    String(port),
  ],
  { stdio: "ignore" },
);
await new Promise((r) => setTimeout(r, 800));
const dir = mkdtempSync(join(tmpdir(), "zeek-board-"));
try {
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1080 },
    recordVideo: { dir, size: { width: 1920, height: 1080 } },
  });
  const page = await context.newPage();
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForTimeout(15_600);
  await context.close();
  await browser.close();
  const webm = join(
    dir,
    readdirSync(dir).find((f) => f.endsWith(".webm")),
  );
  execFileSync("ffmpeg", [
    "-y",
    "-loglevel",
    "error",
    "-ss",
    "0.5",
    "-t",
    "15",
    "-i",
    webm,
    "-c:v",
    "libx264",
    "-pix_fmt",
    "yuv420p",
    "-crf",
    "20",
    "-movflags",
    "+faststart",
    join(root, "docs/video/clips/public-board.mp4"),
  ]);
} finally {
  server.kill();
  rmSync(dir, { recursive: true, force: true });
}
console.log("wrote docs/video/clips/public-board.mp4");
