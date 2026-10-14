//! Zeek Pocket: the browser wallet core.
//!
//! Everything here is plain Rust that also compiles natively (for tests);
//! the `wasm32` build adds the `wasm-bindgen` surface and the rayon thread
//! pool initialiser that the Web Worker calls once.

#![forbid(unsafe_code)]

pub mod bench;

#[cfg(target_arch = "wasm32")]
pub use wasm_bindgen_rayon::init_thread_pool;

use wasm_bindgen::prelude::*;

/// Installs the panic hook so Rust panics surface in the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Builds, proves, signs and extracts one Ironwood transaction (one spend,
/// `payments` payments plus change) and returns the timings as JSON (see [`bench::BenchReport`]).
#[wasm_bindgen]
pub fn bench_prove(payments: u32) -> Result<JsValue, JsValue> {
    let report =
        bench::run(js_sys::Date::now, payments).map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_wasm_bindgen::to_value(&report).map_err(|e| JsValue::from_str(&e.to_string()))
}
