import { readFileSync } from "node:fs";
import { defineConfig, devices } from "@playwright/test";

/**
 * End-to-end tests run against the production build (`vite preview`, which
 * sends the same COOP/COEP headers as production) and the live testnet
 * engine at VITE_ZEEK_ENGINE_URL. Nothing is mocked.
 *
 * - smoke: every screen loads cross-origin isolated, every asset (fonts
 *   included) loads under COEP, and no request fails.
 * - live: the Phase 7a testnet flows, one numbered step per file, sharing
 *   one browser profile and a state file in data/web-e2e/ so each step can
 *   be rerun on its own (rounds take ~25 minutes).
 */
/** A deployed site to test instead of the local preview (live suite). */
const target = process.env.ZEEK_E2E_APP;

// The engine the local preview and the tests use: VITE_ZEEK_ENGINE_URL, else
// the hosted engine named in .zeek-deploy (ENGINE_ORIGIN, on Railway), else
// an engine on this Mac. Set before the web servers start, so the build, its
// CSP and the Replay snapshot all use the same one (e2e/engine.ts).
if (process.env.VITE_ZEEK_ENGINE_URL === undefined) {
  try {
    const hosted = /^ENGINE_ORIGIN=(.+)$/m.exec(readFileSync(".zeek-deploy", "utf8"))?.[1];
    if (hosted !== undefined) process.env.VITE_ZEEK_ENGINE_URL = hosted.trim();
  } catch {
    // No deployment file: the local engine.
  }
}

export default defineConfig({
  testDir: "e2e",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: target ?? "http://localhost:4174",
    trace: "retain-on-failure",
    actionTimeout: 60_000,
    // Every test starts without the load screen (src/boot.ts), except the
    // load screen's own tests, which clear this.
    storageState: {
      cookies: [],
      origins: [target ?? "http://localhost:4174", "http://localhost:4175"].map((origin) => ({
        origin,
        localStorage: [{ name: "zeek.e2e.skipLoadScreen", value: "1" }],
      })),
    },
  },
  ...(target === undefined
    ? {
        webServer: [
          {
            // The snapshot of saved rounds for Replay mode (needs the engine).
            command:
              "pnpm build && node ../../scripts/snapshot-rounds.mjs --engine ${VITE_ZEEK_ENGINE_URL:-http://127.0.0.1:8787} --out dist/replay && pnpm preview",
            url: "http://localhost:4174",
            reuseExistingServer: true,
            timeout: 180_000,
          },
          {
            // Test-only fixtures (e2e/overflow.spec.ts), never deployed.
            command:
              "ZEEK_FIXTURES=1 pnpm exec vite build && pnpm exec vite preview --outDir dist-fixtures --port 4175 --strictPort",
            url: "http://localhost:4175/fixtures.html",
            reuseExistingServer: true,
            timeout: 180_000,
          },
        ],
      }
    : {}),
  projects: [
    // These tests wait up to 60-120 s at a time on the live engine (a round,
    // a sync): the default 30 s test timeout would cut those waits short.
    {
      name: "smoke",
      testMatch: /((smoke|chest)\.spec\.ts|features\/.*\.spec\.ts)$/,
      use: { ...devices["Desktop Chrome"] },
      timeout: 180_000,
    },
    // Safari's engine: cross-origin isolation and wasm threads must work there too.
    {
      name: "smoke-webkit",
      testMatch: /((smoke|chest)\.spec\.ts|features\/.*\.spec\.ts)$/,
      use: { ...devices["Desktop Safari"] },
      timeout: 180_000,
    },
    { name: "live", testMatch: /live\/.*\.spec\.ts/, timeout: 3 * 60 * 60_000 },
    // No text overflows its box (both engines), at six widths.
    { name: "overflow", testMatch: "overflow.spec.ts", use: { ...devices["Desktop Chrome"] } },
    {
      name: "overflow-webkit",
      testMatch: "overflow.spec.ts",
      use: { ...devices["Desktop Safari"] },
    },
    { name: "quota", testMatch: "quota.spec.ts", use: { ...devices["Desktop Chrome"] } },
    { name: "record", testMatch: "record.spec.ts", timeout: 10 * 60_000 },
    { name: "compare", testMatch: "compare.spec.ts", timeout: 30 * 60_000 },
    {
      name: "perf",
      testMatch: "perf.spec.ts",
      use: { ...devices["Desktop Chrome"] },
      timeout: 120_000,
    },
    {
      name: "shots",
      testMatch: "screenshots.spec.ts",
      timeout: 30 * 60_000,
      expect: { timeout: 30_000 },
    },
  ],
});
