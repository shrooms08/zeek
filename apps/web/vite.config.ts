import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

/**
 * Zeek Pocket proves transactions with WebAssembly threads
 * (SharedArrayBuffer), which browsers allow only on cross-origin isolated
 * pages. These headers make the app isolated in `vite dev` and `vite
 * preview`; production hosting must send the same two headers (see
 * public/_headers). Every asset is same-origin (fonts are self-hosted), so
 * `require-corp` blocks nothing.
 */
export const isolationHeaders = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

/**
 * The vault this build serves, read from the public game config
 * (ZEEK_GAME_CONFIG, default the testnet game). The Pocket refuses an
 * engine whose /v1/config names anything else (security review H3).
 */
function pinnedVault() {
  const path =
    process.env.ZEEK_GAME_CONFIG ??
    fileURLToPath(new URL("../../config/game.testnet.json", import.meta.url));
  const game = JSON.parse(readFileSync(path, "utf8")) as {
    params: { network: string };
    vault_address: string;
    sealing_public_key: string;
    uivk: string;
  };
  return {
    network: game.params.network,
    vault_address: game.vault_address,
    sealing_public_key: game.sealing_public_key,
    uivk: game.uivk,
  };
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: { __ZEEK_PINNED_VAULT__: JSON.stringify(pinnedVault()) },
  server: { headers: isolationHeaders, port: 5174, strictPort: true },
  preview: { headers: isolationHeaders, port: 4174, strictPort: true },
  worker: { format: "es" },
  optimizeDeps: {
    exclude: ["@zeek/pocket"],
    include: ["@zeek/pocket > idb", "@zeek/pocket > qrcode"],
  },
  build: { target: "es2022" },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
  },
});
