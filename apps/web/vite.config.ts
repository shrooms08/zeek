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

export default defineConfig({
  plugins: [react(), tailwindcss()],
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
