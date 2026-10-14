import { defineConfig } from "vite";

/**
 * The wallet uses WebAssembly threads (SharedArrayBuffer), which browsers
 * only allow on cross-origin isolated pages: these two headers make the page
 * isolated in dev and preview alike.
 */
const isolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  server: { headers: isolation, port: 5173, strictPort: true },
  preview: { headers: isolation, port: 4173, strictPort: true },
  worker: { format: "es" },
  optimizeDeps: { exclude: ["@zeek/pocket"] },
  build: { target: "es2022" },
});
