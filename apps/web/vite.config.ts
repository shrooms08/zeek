import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";
import { pinnedVault } from "../../scripts/pinned-vault.mjs";

/**
 * Zeek Pocket proves transactions with WebAssembly threads
 * (SharedArrayBuffer), which browsers allow only on cross-origin isolated
 * pages. These headers make the app isolated in `vite dev` and `vite
 * preview`; production hosting must send the same two headers (the build
 * writes them to dist/_headers). Every asset is same-origin (fonts are
 * self-hosted), so `require-corp` blocks nothing; the hosted engine and
 * relay are reached with CORS fetches and a WebSocket, which it does not
 * block.
 */
export const isolationHeaders = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

/**
 * The hosted engine's and relay's origins when the build names absolute
 * https URLs for them (VITE_ZEEK_ENGINE_URL, VITE_ZEEK_SECOND_SOURCE), plus
 * the engine's WebSocket origin: the only additions to `connect-src`.
 */
function hostedOrigins(): string[] {
  const out: string[] = [];
  const services = [
    ["VITE_ZEEK_ENGINE_URL", true],
    ["VITE_ZEEK_SECOND_SOURCE", false],
  ] as const;
  for (const [key, stream] of services) {
    const value = process.env[key];
    if (value === undefined || !value.startsWith("https://")) continue;
    const url = new URL(value);
    out.push(url.origin);
    if (stream) out.push(`wss://${url.host}`);
  }
  return out;
}

/** `connect-src`: local runs also reach the engine and relay on this Mac. */
function connectSrc(local: boolean): string {
  return [
    "connect-src 'self'",
    ...(local ? ["http://127.0.0.1:8787 ws://127.0.0.1:8787 http://127.0.0.1:8790"] : []),
    ...hostedOrigins(),
    // The mainnet second source in the verifier; drand's public relays
    // (Replay mode's beacon check).
    "https://api.blockchair.com https://api.drand.sh https://api2.drand.sh https://api3.drand.sh https://drand.cloudflare.com",
  ].join(" ");
}

function policy(local: boolean): string {
  return [
    "default-src 'self'",
    "script-src 'self' 'wasm-unsafe-eval'",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self' data:",
    connectSrc(local),
    "worker-src 'self' blob:",
    "manifest-src 'self'",
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'self'",
    "frame-ancestors 'none'",
  ].join("; ");
}

/**
 * Everything the site sends in production (dist/_headers), including the content
 * security policy (security review M4: no inline or third-party script; the
 * wallet's WebAssembly needs 'wasm-unsafe-eval'; the art's SVG markup uses
 * inline style attributes). `vite dev` keeps only isolation, because its
 * hot-reload preamble is an inline script.
 */
export const productionHeaders = {
  ...isolationHeaders,
  "Cross-Origin-Resource-Policy": "same-origin",
  // `vite preview` (the smoke tests) also allows the local engine.
  "Content-Security-Policy": policy(true),
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "no-referrer",
  "Permissions-Policy": "camera=(), microphone=(), geolocation=(), payment=()",
};

/**
 * Writes dist/_headers (Cloudflare Pages) from `productionHeaders`, so the
 * deployed site and `vite preview` send one policy. It does not apply to
 * Pages Function responses (/api, /share, /relay); functions/_lib/proxy.ts
 * sets their own.
 */
function pagesHeaders(): Plugin {
  return {
    name: "zeek-pages-headers",
    apply: "build",
    generateBundle() {
      const deployed = { ...productionHeaders, "Content-Security-Policy": policy(false) };
      const lines = Object.entries(deployed).map(([k, v]) => `  ${k}: ${v}`);
      this.emitFile({
        type: "asset",
        fileName: "_headers",
        source: ["/*", ...lines, ""].join("\n"),
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), tailwindcss(), pagesHeaders()],
  // The vault the Pocket accepts (security review H3).
  define: { __ZEEK_PINNED_VAULT__: JSON.stringify(pinnedVault()) },
  server: { headers: isolationHeaders, port: 5174, strictPort: true },
  preview: { headers: productionHeaders, port: 4174, strictPort: true },
  worker: { format: "es" },
  optimizeDeps: {
    exclude: ["@zeek/pocket"],
    include: ["@zeek/pocket > idb", "@zeek/pocket > qrcode"],
  },
  // ZEEK_FIXTURES=1 also builds fixtures.html (test only) into dist-fixtures/;
  // the deployed build never contains it.
  build:
    process.env.ZEEK_FIXTURES === "1"
      ? {
          target: "es2022",
          outDir: "dist-fixtures",
          rollupOptions: { input: { main: "index.html", fixtures: "fixtures.html" } },
        }
      : { target: "es2022" },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx", "functions/**/*.test.ts"],
  },
});
