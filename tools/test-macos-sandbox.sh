#!/bin/bash
# Test-only OS characterization. This does not build or enable a release worker.
set -euo pipefail
umask 077
if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ] || [ "$(id -u)" = 0 ]; then
    echo 'Use native Apple Silicon macOS with an ordinary disposable OS user.' >&2
    exit 2
fi
if [ "${ROCKET_MACOS_SANDBOX_DISPOSABLE_USER:-}" != 1 ]; then
    echo 'This probe creates a fresh app container. A disposable OS user must opt in.' >&2
    exit 2
fi
root=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$root"
out=${ROCKET_MACOS_OUTPUT:?Choose a new absolute output directory}
case "$out" in /*) ;; *) echo 'Output must be absolute.' >&2; exit 2 ;; esac
if [ -e "$out" ] || [ -L "$out" ]; then
    echo 'Output exists; refusing to replace earlier results.' >&2
    exit 2
fi
mkdir "$out"
out=$(cd "$out" && pwd -P)
exec > >(tee "$out/build.log") 2>&1
trap 'status=$?; if [ "$status" -ne 0 ]; then echo "Sandbox characterization failed (exit $status). Lua remains disabled."; fi' EXIT
{
    echo '# macOS Lua sandbox characterization'
    echo 'Test-only App Sandbox probe; no supported or production Lua sandbox.'
    printf 'Commit: '; git rev-parse HEAD
    sw_vers
    xcrun --sdk macosx --show-sdk-version
    rustup run 1.99.1 rustc -vV
} > "$out/build-info.md"
node --test tools/tests/test-macos-sandbox-harness.cjs
node tools/tests/test-macos-sandbox.cjs "$out/sandbox"
# These are the actual host and worker refusal paths, not the research probe.
# Keep the normal packages and Windows/Linux profiles untouched.
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0
export MACOSX_DEPLOYMENT_TARGET=15.0 ROCKET_LUA_WORKER_SHA256=
common=(--locked --target aarch64-apple-darwin --target-dir "$root/target/macos-stage1-cargo")
rustup run 1.99.1 cargo test "${common[@]}" -p rocket-git --lib integration_worker::preflight_tests
rustup run 1.99.1 cargo test "${common[@]}" -p rocket-integrations --features runtime --test unsupported_worker
echo 'Characterization and production-refusal checks finished. This is NOT sandbox qualification.'
