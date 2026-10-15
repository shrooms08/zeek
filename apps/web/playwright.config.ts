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
  },
  ...(target === undefined
    ? {
        webServer: {
          command: "pnpm build && pnpm preview",
          url: "http://localhost:4174",
          reuseExistingServer: true,
          timeout: 180_000,
        },
      }
    : {}),
  projects: [
    {
      name: "smoke",
      testMatch: /((smoke|chest)\.spec\.ts|features\/.*\.spec\.ts)$/,
      use: { ...devices["Desktop Chrome"] },
    },
    // Safari's engine: cross-origin isolation and wasm threads must work there too.
    {
      name: "smoke-webkit",
      testMatch: /((smoke|chest)\.spec\.ts|features\/.*\.spec\.ts)$/,
      use: { ...devices["Desktop Safari"] },
    },
    { name: "live", testMatch: /live\/.*\.spec\.ts/, timeout: 3 * 60 * 60_000 },
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
