// Serves ../pkg and this directory with COOP/COEP headers and runs the
// benchmark in headless Chromium through Playwright. Usage: node run.mjs
import http from "node:http";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".mjs": "text/javascript" };

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://localhost");
  const file = path.join(root, decodeURIComponent(url.pathname));
  try {
    const info = await stat(file);
    if (!info.isFile()) throw new Error("not a file");
    const body = await readFile(file);
    res.writeHead(200, {
      "Content-Type": types[path.extname(file)] ?? "application/octet-stream",
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp",
      "Cross-Origin-Resource-Policy": "same-origin",
      "Cache-Control": "no-store",
    });
    res.end(body);
  } catch {
    res.writeHead(404).end("not found");
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const port = server.address().port;

const wasmSize = (await stat(path.join(root, "pkg", "zeek_pocket_bg.wasm"))).size;
const browser = await chromium.launch();
const page = await browser.newPage();
page.on("console", (m) => console.error("[page]", m.text()));
await page.goto(`http://127.0.0.1:${port}/bench/index.html`);
const result = await page.waitForFunction(() => window.__benchResult, null, { timeout: 15 * 60 * 1000 });
const data = await result.jsonValue();
data.wasm_size_bytes = wasmSize;
console.log(JSON.stringify(data, null, 2));
await browser.close();
server.close();
if (data.type !== "done") process.exit(1);
