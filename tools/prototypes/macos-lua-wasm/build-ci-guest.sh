#!/usr/bin/env bash
# Exact approved Linux toolchain only; no system install or persistent cache.
set -euo pipefail
umask 077
cd "$(dirname "$0")"
test "$(uname -s)" = Linux && test "$(uname -m)" = x86_64
task=${1:?New absolute staging directory required}
out=${2:?New absolute results directory required}
case "$task:$out" in /*:/*) ;; *) exit 2 ;; esac
test ! -e "$task" && test ! -L "$task"
test ! -e "$out" && test ! -L "$out"
mkdir "$out"
exec > >(tee "$out/build.log") 2>&1
node verify-lock.cjs
node --test test-archives.cjs
node prepare-deps.cjs "$task" guest
export CARGO_HOME="$task/cargo-home" CARGO_BUILD_JOBS=3
export WASM_RUST_TOOLCHAIN="$task/nightly" WASM_TARGET_DIR="$task/target"
mkdir "$WASM_RUST_TOOLCHAIN" "$task/tools"
# SHA-256 already matched the scanned official archives before extraction.
for part in rustc cargo rust-std; do
  prefix="$part-nightly-x86_64-unknown-linux-gnu"
  component=$part
  if [ "$part" = rust-std ]; then component=rust-std-x86_64-unknown-linux-gnu; fi
  tar --no-same-owner --no-same-permissions -xf "$task/archives/$prefix.tar.xz" \
    --strip-components=2 -C "$WASM_RUST_TOOLCHAIN" "$prefix/$component"
done
tar --no-same-owner --no-same-permissions -xf "$task/archives/rust-src-nightly.tar.xz" \
  --strip-components=2 -C "$WASM_RUST_TOOLCHAIN" rust-src-nightly/rust-src
tar --no-same-owner --no-same-permissions -xzf "$task/archives/wasi-sdk-34.0-x86_64-linux.tar.gz" -C "$task/tools"
export WASI_SDK_PATH="$task/tools/wasi-sdk-34.0-x86_64-linux"
bash build-guest-unwind.sh
cp "$WASM_TARGET_DIR/wasm32-wasip1/release/rocket_lua_wasm_guest_prototype.wasm" "$out/guest.wasm"
sha256sum "$out/guest.wasm" | cut -d ' ' -f1 > "$out/guest.sha256"
if [ -n "${GITHUB_OUTPUT:-}" ]; then printf 'guest_sha=%s\n' "$(cat "$out/guest.sha256")" >> "$GITHUB_OUTPUT"; fi
