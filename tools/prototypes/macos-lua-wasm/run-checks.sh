#!/usr/bin/env bash
# Characterization only: nonzero exit means compatibility is NOT established.
set -euo pipefail
umask 077
cd "$(dirname "$0")"
: "${WASI_SDK_PATH:?Verified SDK directory required}"
: "${WASM_SYSROOT:?Isolated WASI std directory required}"
: "${CARGO_TARGET_DIR:?Isolated target directory required}"
out=${1:?Choose a new absolute results directory}
case "$out" in /*) ;; *) echo 'Results path must be absolute' >&2; exit 2 ;; esac
test ! -e "$out" && test ! -L "$out"
mkdir "$out"
exec > >(tee "$out/build.log") 2>&1
node verify-lock.cjs
node --test test-controller.cjs
cargo test --offline --locked -p rocket-lua-wasm-guest-prototype --lib
cargo build --offline --locked -p rocket-lua-wasm-guest-prototype --bin reference
node test-worker.cjs "$CARGO_TARGET_DIR/debug/reference" "$out/native-reference.json" native-reference
cargo test --offline --locked -p rocket-lua-wasm-probe --lib
bash build-guest.sh
cargo build --offline --locked -p rocket-lua-wasm-probe --bin rocket-lua-wasm-probe
guest="$CARGO_TARGET_DIR/wasm32-wasip1/release/rocket_lua_wasm_guest_prototype.wasm"
"$CARGO_TARGET_DIR/debug/rocket-lua-wasm-probe" "$guest" "$out/compiled"
export ROCKET_WASM_GUEST="$out/compiled/guest.cwasm"
cargo build --offline --locked -p rocket-lua-wasm-probe --features embedded --bin rocket-lua-wasm-worker-prototype
node test-worker.cjs "$CARGO_TARGET_DIR/debug/rocket-lua-wasm-worker-prototype" "$out/pulley.json"
