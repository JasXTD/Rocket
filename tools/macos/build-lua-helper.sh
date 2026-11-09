#!/usr/bin/env bash
# Uses the approved, isolated fixed-guest toolchain. No system installs/downloads.
set -euo pipefail
umask 077
root=$(cd "$(dirname "$0")/../.." && pwd)
guest=${1:?Same-source fixed guest.wasm required}
out=${2:?New absolute helper output required}
profile=${3:-debug}
case "$profile" in debug|release) ;; *) echo 'Expected debug or release helper profile' >&2; exit 2 ;; esac
cargo_profile=()
if [ "$profile" = release ]; then cargo_profile=(--release); fi
: "${CARGO_TARGET_DIR:?Separate helper target directory required}"
test "$(uname -s)" = Darwin && test "$(uname -m)" = arm64
case "$guest:$out" in /*:/*) ;; *) exit 2 ;; esac
test ! -e "$out" && test ! -L "$out"
mkdir "$out"
cd "$root/tools/prototypes/macos-lua-wasm"
node verify-lock.cjs
export MACOSX_DEPLOYMENT_TARGET=15.0
cargo build --offline --locked "${cargo_profile[@]}" -p rocket-lua-wasm-probe --bin rocket-lua-wasm-probe
"$CARGO_TARGET_DIR/$profile/rocket-lua-wasm-probe" "$guest" "$out/compiled"
export ROCKET_WASM_GUEST="$out/compiled/guest.cwasm"
cargo build --offline --locked "${cargo_profile[@]}" -p rocket-lua-wasm-probe --features embedded --bin rocket-lua-wasm-worker-prototype
node "$root/tools/macos/lua-helper.cjs" prepare "$CARGO_TARGET_DIR/$profile/rocket-lua-wasm-worker-prototype" "$out/RocketLua.app" > "$out/worker.sha256"
# These sources were already audited/scanned. Preserve upstream license texts
# for every dependency in this isolated helper graph, not just the main VM.
node "$root/tools/macos/lua-notices.cjs" "${CARGO_HOME:?}/../vendor" "$out/LUA-NOTICES"
