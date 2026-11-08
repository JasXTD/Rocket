#!/usr/bin/env bash
# Isolated experiment: rebuild std so Lua's C-unwind callbacks can unwind.
set -euo pipefail
cd "$(dirname "$0")"
: "${WASI_SDK_PATH:?Set the verified WASI SDK 34 directory}"
: "${WASM_RUST_TOOLCHAIN:?Set the approved nightly-2026-11-08 directory}"
: "${WASM_TARGET_DIR:?Set a separate guest build directory}"
test -x "$WASI_SDK_PATH/bin/clang"
test -f "$WASM_RUST_TOOLCHAIN/lib/rustlib/src/rust/library/Cargo.lock"
export RUSTC="$WASM_RUST_TOOLCHAIN/bin/rustc"
test "$("$RUSTC" --version)" = 'rustc 1.102.0-nightly (f5cedb83a 2026-11-07)'
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER RUSTC_BOOTSTRAP
export CC_wasm32_wasip1="$WASI_SDK_PATH/bin/clang"
export AR_wasm32_wasip1="$WASI_SDK_PATH/bin/llvm-ar"
export CARGO_TARGET_WASM32_WASIP1_LINKER="$WASI_SDK_PATH/bin/wasm-ld"
export CARGO_TARGET_DIR="$WASM_TARGET_DIR"
test -f "$WASI_SDK_PATH/share/wasi-sysroot/lib/wasm32-wasip1/eh/libunwind.a"
export CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="-Cpanic=unwind -Ctarget-feature=+exception-handling -Cllvm-args=-wasm-use-legacy-eh=false -Clink-arg=--max-memory=134217728 -Clink-arg=-z -Clink-arg=stack-size=1048576 -Lnative=$WASI_SDK_PATH/share/wasi-sysroot/lib/wasm32-wasip1/eh -Lnative=$WASI_SDK_PATH/share/wasi-sysroot/lib/wasm32-wasip1"
"$WASM_RUST_TOOLCHAIN/bin/cargo" build --offline --locked --release \
  -Zbuild-std=std,panic_unwind -Zbuild-std-features=panic-unwind \
  --target wasm32-wasip1 -p rocket-lua-wasm-guest-prototype --lib
