#!/bin/bash
# Native OS-policy and repository-lock regressions; no package, GUI or release.
set -euo pipefail
umask 077
root=$(cd "$(dirname "$0")/../.." && pwd -P)
source "$root/tools/macos/architecture.sh"
rocket_macos_architecture
test "$(id -u)" != 0
cd "$root"
test "${ROCKET_MACOS_CHECK_SCOPE:-}" = security-followups
out=${ROCKET_MACOS_OUTPUT:?Set ROCKET_MACOS_OUTPUT to a new absolute result folder}
case "$out" in /*) ;; *) echo 'Output must be absolute.' >&2; exit 2;; esac
test ! -e "$out" && test ! -L "$out"
mkdir -p "$(dirname "$out")"
mkdir "$out"
exec > >(tee "$out/security-followups.log") 2>&1
stage=preflight
commit=$(git rev-parse HEAD)
finish() {
    result=$?
    trap - EXIT
    node - "$out" "$stage" "$result" "$ROCKET_MAC_ARCH" "$commit" <<'JS'
const fs = require('node:fs'), path = require('node:path');
const [out, stage, code, architecture, commit] = process.argv.slice(2);
fs.writeFileSync(path.join(out, 'result.json'), JSON.stringify({passed:Number(code)===0, stage, exit_code:Number(code), architecture, commit, scope:'security-followups', release_published:false}, null, 2));
JS
    exit "$result"
}
trap finish EXIT
identity=$(rustup run 1.99.1 rustc -vV)
printf '%s\n' "$identity" | grep -Fx "host: $ROCKET_MAC_TARGET" >/dev/null
printf '%s\n' "$identity" > "$out/compiler.log"
sw_vers > "$out/system.log"
# Tests need assertions and the native platform, not a slow optimized app build.
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export MACOSX_DEPLOYMENT_TARGET=15.0 ROCKET_LUA_WORKER_SHA256=
export ROCKET_TEST_REPOSITORIES
ROCKET_TEST_REPOSITORIES=$(mktemp -d /private/tmp/rocket-security.XXXXXXXX)
target="${CARGO_TARGET_DIR:-$root/target/macos-security-followups}"
case "$target" in /*) ;; *) echo 'Cargo target must be absolute.' >&2; exit 2;; esac
cargo_run() { rustup run 1.99.1 cargo "$@"; }
stage=locked-dependencies
cargo_run fetch --locked --target "$ROCKET_MAC_TARGET"
common=(--offline --locked --target-dir "$target")
stage=write-gate
cargo_run test "${common[@]}" -p rocket-git --lib write_gate:: -- --nocapture 2>&1 | tee "$out/write-gate.log"
stage=history-units
cargo_run test "${common[@]}" -p rocket-git --lib history_index::tests:: -- --nocapture 2>&1 | tee "$out/history-units.log"
stage=statistics-units
cargo_run test "${common[@]}" -p rocket-git --lib statistics::tests:: -- --nocapture 2>&1 | tee "$out/statistics-units.log"
stage=history-integration
cargo_run test "${common[@]}" -p rocket-git --test history_index -- --nocapture 2>&1 | tee "$out/history-integration.log"
stage=history-security
cargo_run test "${common[@]}" -p rocket-git --test history_index_security -- --nocapture 2>&1 | tee "$out/history-security.log"
stage=statistics-integration
cargo_run test "${common[@]}" -p rocket-git --test statistics -- --nocapture 2>&1 | tee "$out/statistics-integration.log"
stage=agents-runtime
cargo_run test "${common[@]}" -p rocket-ui --lib agents::runtime:: -- --test-threads=1 --nocapture 2>&1 | tee "$out/agents-runtime.log"
stage=gatekeeper-probe
ROCKET_MACOS_DISPOSABLE_GATEKEEPER_TEST=1 cargo_run test "${common[@]}" -p rocket-ui --lib \
    agents::runtime::program::tests::bundle_quarantine_native_gatekeeper_probe \
    -- --exact --ignored --test-threads=1 --nocapture 2>&1 | tee "$out/gatekeeper-probe.log"
stage=verify-results
node tools/macos/verify-security-followups.cjs "$out"
stage=complete
