// Runs the Rust benchmark inside a Web Worker with a rayon thread pool.
import init, { initThreadPool, bench_prove } from "../pkg/zeek_pocket.js";

self.onmessage = async (event) => {
  const { threads, crossOriginIsolated, plan } = event.data;
  try {
    const t0 = performance.now();
    const module = await init();
    const wasmLoadMs = performance.now() - t0;
    const t1 = performance.now();
    await initThreadPool(threads);
    const threadPoolMs = performance.now() - t1;
    // One transaction per entry of `plan` (payments = tiles), in sequence,
    // in this one worker: wasm memory only grows, so the reading after each
    // batch is the peak so far.
    const batches = [];
    for (const payments of plan) {
      const report = bench_prove(payments);
      batches.push({ payments, ...report, wasm_memory_bytes: module.memory.buffer.byteLength });
    }
    const memoryBytes = module.memory.buffer.byteLength;
    self.postMessage({
      type: "done",
      crossOriginIsolated,
      threads,
      wasm_load_ms: wasmLoadMs,
      thread_pool_ms: threadPoolMs,
      wasm_memory_bytes: memoryBytes,
      plan,
      batches,
    });
  } catch (e) {
    self.postMessage({ type: "error", message: String(e), stack: e && e.stack });
  }
};
