#!/bin/sh
# Builds crates/zeek-pocket for wasm32-unknown-unknown with threads (atomics,
# shared memory) and writes the wasm-bindgen output to packages/zeek-pocket/wasm.
#
# Needs: rustup toolchain `nightly` with `rust-src` and the wasm32 target
# (build-std recompiles std with atomics), `llvm-tools` for llvm-ar (Apple's ar
# cannot index wasm objects, which secp256k1-sys builds), clang with the
# WebAssembly backend (Apple clang has it), and wasm-bindgen-cli 0.2.128.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
host="$(rustc +nightly -vV | sed -n 's/^host: //p')"
sysroot="$(rustc +nightly --print sysroot)"
export AR_wasm32_unknown_unknown="$sysroot/lib/rustlib/$host/bin/llvm-ar"
export CC_wasm32_unknown_unknown="${CC_wasm32_unknown_unknown:-clang}"
export RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--max-memory=4294967296 -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base --cfg zcash_unstable="nu7"'
cargo +nightly build -p zeek-pocket --lib --release --target wasm32-unknown-unknown -Z build-std=panic_abort,std "$@"
out="$root/packages/zeek-pocket/wasm"
rm -rf "$out"
wasm-bindgen target/wasm32-unknown-unknown/release/zeek_pocket.wasm --target web --out-dir "$out"
# The rayon worker helper imports the package by directory (bundler mode).
cat > "$out/package.json" <<'JSON'
{
  "name": "zeek-pocket-wasm",
  "private": true,
  "type": "module",
  "main": "zeek_pocket.js",
  "module": "zeek_pocket.js",
  "types": "zeek_pocket.d.ts"
}
JSON
ls -l "$out/zeek_pocket_bg.wasm"
