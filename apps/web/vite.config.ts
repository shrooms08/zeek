import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { pinnedVault } from "../../scripts/pinned-vault.mjs";

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
 * Everything public/_headers sends in production, including the content
 * security policy (security review M4: no inline or third-party script; the
 * wallet's WebAssembly needs 'wasm-unsafe-eval'; the art's SVG markup uses
 * inline style attributes). `vite dev` keeps only isolation, because its
 * hot-reload preamble is an inline script.
 */
export const productionHeaders = {
  ...isolationHeaders,
  "Cross-Origin-Resource-Policy": "same-origin",
  "Content-Security-Policy": [
    "default-src 'self'",
    "script-src 'self' 'wasm-unsafe-eval'",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self' data:",
    // The engine in local runs; the mainnet second source in the verifier.
    "connect-src 'self' http://127.0.0.1:8787 ws://127.0.0.1:8787 http://127.0.0.1:8790 https://api.blockchair.com",
    "worker-src 'self' blob:",
    "manifest-src 'self'",
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'self'",
    "frame-ancestors 'none'",
  ].join("; "),
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "no-referrer",
};

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // The vault the Pocket accepts (security review H3).
  define: { __ZEEK_PINNED_VAULT__: JSON.stringify(pinnedVault()) },
  server: { headers: isolationHeaders, port: 5174, strictPort: true },
  preview: { headers: productionHeaders, port: 4174, strictPort: true },
  worker: { format: "es" },
  optimizeDeps: {
    exclude: ["@zeek/pocket"],
    include: ["@zeek/pocket > idb", "@zeek/pocket > qrcode"],
  },
  build: { target: "es2022" },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx", "functions/**/*.test.ts"],
  },
});
