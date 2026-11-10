#!/bin/bash
# One native batch for Mac updates, spelling and menus. Synthetic data only.
set -euo pipefail
umask 077
test "$(id -u)" != 0
root=$(cd "$(dirname "$0")/../.." && pwd -P)
source "$root/tools/macos/architecture.sh"
rocket_macos_architecture
cd "$root"
out=${ROCKET_MACOS_OUTPUT:?ROCKET_MACOS_OUTPUT is required}
mkdir -p "$out"
exec > >(tee "$out/desktop-integration.log") 2>&1
export MACOSX_DEPLOYMENT_TARGET=15.0 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export ROCKET_LUA_WORKER_SHA256=
target="$root/target/macos-desktop-cargo"
args=(--locked --target-dir "$target")
cargo_run() { rustup run 1.99.1 cargo "$@"; }
node --test tools/tests/test-macos-update-package.cjs
xcrun clang -arch "$ROCKET_MAC_CLANG_ARCH" -mmacosx-version-min=15.0 -fobjc-arc -fblocks -Wall -Wextra -Werror -framework AppKit tools/tests/macos-menu-probe.m -o "$out/menu-probe"
"$out/menu-probe"
cargo_run test "${args[@]}" -p rocket-git --lib app_update:: -- --nocapture
cargo_run build "${args[@]}" -p rocket-git --example update_transport
node tools/tests/test-macos-update-transport.cjs "$target/debug/examples/update_transport" "$out"
cargo_run test "${args[@]}" -p rocket-spell --lib -- --nocapture
cargo_run run "${args[@]}" -p rocket-spell --example macos_probe
cargo_run test "${args[@]}" -p rocket-ui --lib app_updates:: -- --nocapture
cargo_run test "${args[@]}" -p rocket-ui --lib shortcuts::tests -- --test-threads=1
cargo_run test "${args[@]}" -p rocket-ui --lib --no-run --message-format=json > "$out/test-artifacts.json"
test_exe=$(node - "$out/test-artifacts.json" <<'JS'
const fs=require('fs'),events=fs.readFileSync(process.argv[2],'utf8').trim().split('\n').map(s=>JSON.parse(s));
const tests=events.filter(e=>e.reason==='compiler-artifact'&&e.target.name==='rocket_ui'&&e.profile.test&&e.executable);if(tests.length!==1)throw Error('Expected one native UI test executable');process.stdout.write(tests[0].executable);
JS
)
node tools/tests/test-macos-update-transaction.cjs "$test_exe" "$out"
printf '%s\n' '{"passed":true,"scope":"macos-updates-spelling-menus","distribution_signed":false}' > "$out/desktop-result.json"
