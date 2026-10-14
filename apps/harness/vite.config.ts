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
  // @zeek/pocket is served as source (its worker and wasm must keep their
  // URLs), so its dependencies are pre-bundled up front; otherwise Vite finds
  // them on first use and reloads the page, killing the wallet worker
  // mid-call.
  optimizeDeps: {
    exclude: ["@zeek/pocket"],
    include: ["@zeek/pocket > idb", "@zeek/pocket > qrcode"],
  },
  build: { target: "es2022" },
});
