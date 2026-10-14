// Runs the Rust benchmark inside a Web Worker with a rayon thread pool.
import init, { initThreadPool, bench_prove } from "../pkg/zeek_pocket.js";

self.onmessage = async (event) => {
  const { threads, crossOriginIsolated } = event.data;
  try {
    const t0 = performance.now();
    const module = await init();
    const wasmLoadMs = performance.now() - t0;
    const t1 = performance.now();
    await initThreadPool(threads);
    const threadPoolMs = performance.now() - t1;
    const report = bench_prove(1);
    const report25 = bench_prove(25);
    const memoryBytes = module.memory.buffer.byteLength;
    self.postMessage({
      type: "done",
      crossOriginIsolated,
      threads,
      wasm_load_ms: wasmLoadMs,
      thread_pool_ms: threadPoolMs,
      wasm_memory_bytes: memoryBytes,
      report,
      report25,
    });
  } catch (e) {
    self.postMessage({ type: "error", message: String(e), stack: e && e.stack });
  }
};
