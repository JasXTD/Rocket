#!/usr/bin/env bash
# Affected tests only. The fixed guest was built from the same checked-out SHA.
set -euo pipefail
umask 077
cd "$(dirname "$0")"
guest=${1:?Verified fixed guest path required}
out=${2:?New absolute results directory required}
: "${CARGO_TARGET_DIR:?Isolated target directory required}"
case "$guest:$out" in /*:/*) ;; *) exit 2 ;; esac
test ! -e "$out" && test ! -L "$out"
mkdir "$out"
exec > >(tee "$out/build.log") 2>&1
rustc --version | grep -Eq '^rustc 1\.99\.1 \('
unset RUSTFLAGS RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER CARGO_ENCODED_RUSTFLAGS ISLE_SOURCE_DIR
node verify-lock.cjs
node --test test-archives.cjs test-controller.cjs
cargo test --offline --locked -p rocket-lua-wasm-guest-prototype --lib
cargo build --offline --locked -p rocket-lua-wasm-guest-prototype --bin reference
node test-worker.cjs "$CARGO_TARGET_DIR/debug/reference" "$out/native-reference.json" native-reference
cargo test --offline --locked -p rocket-lua-wasm-probe --lib
cargo build --offline --locked -p rocket-lua-wasm-probe --bin rocket-lua-wasm-probe
"$CARGO_TARGET_DIR/debug/rocket-lua-wasm-probe" "$guest" "$out/compiled"
export ROCKET_WASM_GUEST="$out/compiled/guest.cwasm"
cargo build --offline --locked -p rocket-lua-wasm-probe --features embedded --bin rocket-lua-wasm-worker-prototype
node test-worker.cjs "$CARGO_TARGET_DIR/debug/rocket-lua-wasm-worker-prototype" "$out/pulley.json" pulley "$out/native-reference.json"
node - "$guest" "$ROCKET_WASM_GUEST" "$CARGO_TARGET_DIR/debug/rocket-lua-wasm-worker-prototype" "$out/artifacts.json" <<'JS'
const fs = require('node:fs'), crypto = require('node:crypto');
const inputs = process.argv.slice(2), output = inputs.pop(), labels = ['wasm', 'pulley', 'worker'];
const artifacts = inputs.map((p, i) => ({ name: labels[i], bytes: fs.statSync(p).size, sha256: crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex') }));
fs.writeFileSync(output, JSON.stringify({ production_ready: false, platform: process.platform, arch: process.arch, artifacts }, null, 2) + '\n', { flag: 'wx' });
JS
