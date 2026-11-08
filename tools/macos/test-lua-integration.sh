#!/usr/bin/env bash
# Focused native acceptance: real launcher, consumers, binding and app packaging.
set -euo pipefail
umask 077
root=$(cd "$(dirname "$0")/../.." && pwd)
guest=${1:?Same-run fixed guest required}
out=${2:?New absolute results directory required}
root_cargo=${3:?Separate root Cargo home required}
test "$(uname -s)" = Darwin && test "$(uname -m)" = arm64 && test "$(id -u)" != 0
test "${ROCKET_MACOS_SANDBOX_DISPOSABLE_USER:-}" = 1
case "$guest:$out:$root_cargo" in /*:/*:/*) ;; *) exit 2 ;; esac
test ! -e "$out" && test ! -L "$out"
mkdir "$out"
exec > >(tee "$out/build.log") 2>&1
export MACOSX_DEPLOYMENT_TARGET=15.0 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
bash "$root/tools/macos/build-lua-helper.sh" "$guest" "$out/helper"
export ROCKET_MACOS_LUA_APP="$out/helper/RocketLua.app"
export ROCKET_TEST_MAC_LUA_APP="$ROCKET_MACOS_LUA_APP"
export ROCKET_LUA_WORKER_SHA256=$(cat "$out/helper/worker.sha256")
export CARGO_HOME="$root_cargo" CARGO_TARGET_DIR="$out/root-target"
cd "$root"
cargo test --locked -p rocket-git --lib integration_worker:: -- --test-threads=1
cargo test --locked -p rocket-integrations --features runtime --test unsupported_worker
# Build the actual app and helper consumers with the final signed digest.
cargo build --locked -p rocket-ui -p rocket-cli -p rocket-mcp -p rocket-git --bins
cargo build --locked -p rocket-ui --example macos_icon
export ROCKET_MACOS_ICON_TOOL="$CARGO_TARGET_DIR/debug/examples/macos_icon"
node --test tools/tests/test-macos-bundle.cjs
"$ROCKET_MACOS_ICON_TOOL" "$out/Rocket.iconset"
/usr/bin/iconutil --convert icns --output "$out/Rocket.icns" "$out/Rocket.iconset"
printf 'Mac Lua integration development package. Source: %s\nHelper SHA-256: %s\n' "$(git rev-parse HEAD)" "$ROCKET_LUA_WORKER_SHA256" > "$out/BUILD.md"
node tools/macos/bundle.cjs stage "$CARGO_TARGET_DIR/debug" "$out/Rocket.icns" "$out/package" "$out/BUILD.md"
node tools/macos/bundle.cjs finalize "$out/package"
export ROCKET_TEST_MAC_HOST="$out/package/Rocket.app/Contents/MacOS/rocket"
cargo test --locked -p rocket-integrations --test macos_worker -- --test-threads=1
node tools/macos/bundle.cjs verify "$out/package"
/usr/bin/ditto -c -k --keepParent "$out/package" "$out/Rocket-macos-lua-development.zip"
mkdir "$out/roundtrip"
/usr/bin/ditto -x -k "$out/Rocket-macos-lua-development.zip" "$out/roundtrip"
node tools/macos/bundle.cjs verify "$out/roundtrip/package"
cp "$out/package/bundle-manifest.json" "$out/bundle-manifest.json"
cp "$out/helper/compiled/module.json" "$out/module.json"
node - "$out" <<'JS'
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const dir=process.argv[2],file=path.join(dir,'Rocket-macos-lua-development.zip');
const record={platform:process.platform,arch:process.arch,launcher:true,consumer:true,bundle:true,roundtrip:true,worker_sha256:process.env.ROCKET_LUA_WORKER_SHA256,bytes:fs.statSync(file).size,sha256:crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')};
fs.writeFileSync(path.join(dir,'integration.json'),JSON.stringify(record,null,2)+'\n',{flag:'wx'});
JS
