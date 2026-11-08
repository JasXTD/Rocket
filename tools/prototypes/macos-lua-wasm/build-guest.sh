#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
: "${WASI_SDK_PATH:?Set the verified WASI SDK 34 directory}"
: "${WASM_SYSROOT:?Set the isolated Rust 1.99.1 WASI sysroot}"
: "${CARGO_TARGET_DIR:?Set an isolated target directory}"
test -x "$WASI_SDK_PATH/bin/clang"
test -d "$WASM_SYSROOT/lib/rustlib/wasm32-wasip1/lib"
export CC_wasm32_wasip1="$WASI_SDK_PATH/bin/clang"
export AR_wasm32_wasip1="$WASI_SDK_PATH/bin/llvm-ar"
export CARGO_TARGET_WASM32_WASIP1_LINKER="$WASI_SDK_PATH/bin/wasm-ld"
# Characterization baseline only. The approved prebuilt Rust std cannot support
# panic=unwind; tests must expose the callback-error trap, not call it a pass.
export CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="--sysroot=$WASM_SYSROOT -Cpanic=abort -Ctarget-feature=+exception-handling -Clink-arg=--max-memory=134217728 -Clink-arg=-z -Clink-arg=stack-size=1048576 -Lnative=$WASI_SDK_PATH/share/wasi-sysroot/lib/wasm32-wasip1"
cargo build --manifest-path Cargo.toml --offline --locked --release --target wasm32-wasip1 -p rocket-lua-wasm-guest-prototype --lib
