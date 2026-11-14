#!/usr/bin/env bash
# Focused native acceptance: real launcher, consumers, binding and app packaging.
set -euo pipefail
umask 077
root=$(cd "$(dirname "$0")/../.." && pwd)
source "$root/tools/macos/architecture.sh"
rocket_macos_architecture
guest=${1:?Same-run fixed guest required}
out=${2:?New absolute results directory required}
root_cargo=${3:?Separate root Cargo home required}
mode=${4:-integration}
case "$mode" in integration|complete-package|release-package) ;; *) exit 2 ;; esac
profile=debug
cargo_profile=()
if [ "$mode" = release-package ]; then profile=release; cargo_profile=(--release); fi
test "$(id -u)" != 0
test "${ROCKET_MACOS_SANDBOX_DISPOSABLE_USER:-}" = 1
case "$guest:$out:$root_cargo" in /*:/*:/*) ;; *) exit 2 ;; esac
test ! -e "$out" && test ! -L "$out"
mkdir "$out"
exec > >(tee "$out/build.log") 2>&1
export MACOSX_DEPLOYMENT_TARGET=15.0 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
bash "$root/tools/macos/build-lua-helper.sh" "$guest" "$out/helper" "$profile"
export ROCKET_MACOS_LUA_APP="$out/helper/RocketLua.app"
export ROCKET_TEST_MAC_LUA_APP="$ROCKET_MACOS_LUA_APP"
export ROCKET_LUA_WORKER_SHA256=$(cat "$out/helper/worker.sha256")
export CARGO_HOME="$root_cargo" CARGO_TARGET_DIR="$out/root-target"
cd "$root"
if [ "$mode" = integration ]; then
  cargo test --locked -p rocket-git --lib integration_worker:: -- --test-threads=1
  cargo test --locked -p rocket-integrations --features runtime --test unsupported_worker
fi
# Build the actual app and helper consumers with the final signed digest.
cargo build --locked "${cargo_profile[@]}" -p rocket-ui -p rocket-cli -p rocket-mcp -p rocket-git --bins
cargo build --locked "${cargo_profile[@]}" -p rocket-ui --example macos_icon
export ROCKET_MACOS_ICON_TOOL="$CARGO_TARGET_DIR/$profile/examples/macos_icon"
node --test tools/tests/test-macos-bundle.cjs
"$ROCKET_MACOS_ICON_TOOL" "$out/Rocket.iconset"
/usr/bin/iconutil --convert icns --output "$out/Rocket.icns" "$out/Rocket.iconset"
printf 'macOS %s preview package. Source: %s\nBuild profile: %s\nHelper SHA-256: %s\nIncludes native menus, updates, spelling and bound Lua. Ad-hoc signatures are not Developer ID signatures. Notarization and physical Mac acceptance are pending. Install manually; this archive is not eligible for in-app installation.\n' "$ROCKET_MAC_ARCH" "$(git rev-parse HEAD)" "$profile" "$ROCKET_LUA_WORKER_SHA256" > "$out/BUILD.md"
node tools/macos/bundle.cjs stage "$CARGO_TARGET_DIR/$profile" "$out/Rocket.icns" "$out/package" "$out/BUILD.md"
node tools/macos/bundle.cjs finalize "$out/package"
export ROCKET_TEST_MAC_HOST="$out/package/Rocket.app/Contents/MacOS/rocket"
if [ "$mode" = integration ] || [ "$mode" = release-package ]; then
  cargo test --locked "${cargo_profile[@]}" -p rocket-integrations --test macos_worker -- --test-threads=1
  if [ "$mode" = release-package ]; then
    cargo test --locked --release -p rocket-git --lib runner::scope_tests::system_curl_ignores_environment_and_git_adjacent_helpers -- --exact --test-threads=1
  fi
else
  # Security regressions must run without an environment workaround for roots.
  cargo test --locked -p rocket-mcp --lib policy::tests -- --test-threads=1
  cargo test --locked -p rocket-git --test fixture_storage -- --test-threads=1
  cargo test --locked -p rocket-git --lib the_time_limit_ends_a_search_without_an_error -- --test-threads=1
  cargo test --locked -p rocket-git --lib tiny_command_limits_preserve_readable_blob_pairs -- --test-threads=1
  cargo test --locked -p rocket-integrations --lib protected_save_keeps_actionable_mac_guidance -- --test-threads=1
  cargo test --locked -p rocket-ui --lib menus::native::tests -- --test-threads=1
  cargo test --locked -p rocket-mcp --lib native_menu -- --test-threads=1
  cargo test --locked -p rocket-mcp --lib terminal_probes -- --test-threads=1
  cargo test --locked -p rocket-terminal --lib isolated_zsh -- --test-threads=1
  cargo test --locked -p rocket-terminal --test unix_session owned_history_keeps_the_selected_program -- --test-threads=1
  xcrun clang -arch "$ROCKET_MAC_CLANG_ARCH" -mmacosx-version-min=15.0 -fobjc-arc -fblocks -Wall -Wextra -Werror -framework AppKit tools/tests/macos-menu-probe.m -o "$out/menu-probe"
  "$out/menu-probe"
fi
node tools/macos/bundle.cjs verify "$out/package"
archive=Rocket-macos-lua-development.zip
if [ "$mode" = release-package ]; then
  version=$(node -p "require('./tools/macos/bundle.cjs').version()")
  archive="Rocket-$version-macos-$ROCKET_MAC_ARCH-preview.zip"
fi
export ROCKET_MAC_PACKAGE_ARCHIVE="$archive" ROCKET_MAC_PACKAGE_PROFILE="$profile"
/usr/bin/ditto -c -k --keepParent "$out/package" "$out/$archive"
mkdir "$out/roundtrip"
/usr/bin/ditto -x -k "$out/$archive" "$out/roundtrip"
node tools/macos/bundle.cjs verify "$out/roundtrip/package"
"$out/roundtrip/package/Rocket.app/Contents/MacOS/rocket-cli" --json --version > "$out/cli-version.json"
node - "$out" <<'JS'
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const dir=process.argv[2],result=JSON.parse(fs.readFileSync(path.join(dir,'cli-version.json'))),manifest=JSON.parse(fs.readFileSync(path.join(dir,'package/bundle-manifest.json')));
assert(result.ok&&result.version===manifest.version&&result.data.version===manifest.version,'Packaged CLI version mismatch');
assert(manifest.lua.runtime==='pulley','Release requires the bound Lua helper');
JS
if [ "$mode" = complete-package ] || [ "$mode" = release-package ]; then
  # Execute the archive's extracted copies, with the unchanged bound helper.
  export ROCKET_MCP_EXE="$out/roundtrip/package/Rocket.app/Contents/MacOS/rocket-mcp"
  export ROCKET_MCP_RESULTS="$out/mcp-fixtures"
  unset ROCKET_MACOS_LUA_APP ROCKET_TEST_MAC_LUA_APP ROCKET_TEST_MAC_HOST
  mkdir "$out/evidence"
  collect_evidence() {
    node - "$out" <<'JS'
const fs=require('node:fs'),path=require('node:path');const out=process.argv[2],root=path.join(out,'mcp-fixtures');
if(fs.existsSync(root))for(const fixture of fs.readdirSync(root)){const dir=path.join(root,fixture);if(!fs.statSync(dir).isDirectory())continue;
 for(const name of fs.readdirSync(dir))if(name==='result.json'||name==='failure-state.json'||name==='failure.json'||name.endsWith('.png'))fs.copyFileSync(path.join(dir,name),path.join(out,'evidence',fixture+'-'+name));}
JS
  }
  trap collect_evidence EXIT
  node tools/tests/test-mcp-desktop.cjs
  # Full acceptance must use the ordinary process environment. Native spelling
  # instrumentation belongs in the separate focused diagnostic runner.
  unset ROCKET_SPELL_OBSERVER
  node tools/tests/test-mcp-macos-package.cjs
  if [ "$mode" = release-package ]; then
    # Qualify the changed Agents lifecycle against the actual extracted release.
    export ROCKET_AGENTS_EXE="$ROCKET_MCP_EXE"
    export ROCKET_AGENTS_REPORT_ROOT="$out/mcp-fixtures"
    node tools/tests/test-mcp-agents.cjs
    # Exercise changed hosting adapters through the actual extracted CLI. All
    # requests use loopback fixtures and synthetic credentials, never accounts.
    export ROCKET_CLI_EXE="$out/roundtrip/package/Rocket.app/Contents/MacOS/rocket-cli"
    export ROCKET_CLI_RESULTS="$out/mcp-fixtures"
    node tools/tests/test-cli-hosting-other-inline.cjs
  fi
  node tools/macos/bundle.cjs verify "$out/roundtrip/package"
fi
cp "$out/package/bundle-manifest.json" "$out/bundle-manifest.json"
cp "$out/helper/compiled/module.json" "$out/module.json"
node - "$out" <<'JS'
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const dir=process.argv[2],file=path.join(dir,process.env.ROCKET_MAC_PACKAGE_ARCHIVE);
const record={platform:process.platform,arch:process.arch,profile:process.env.ROCKET_MAC_PACKAGE_PROFILE,launcher:true,consumer:true,bundle:true,roundtrip:true,worker_sha256:process.env.ROCKET_LUA_WORKER_SHA256,bytes:fs.statSync(file).size,sha256:crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')};
fs.writeFileSync(path.join(dir,'integration.json'),JSON.stringify(record,null,2)+'\n',{flag:'wx'});
fs.writeFileSync(file+'.sha256',record.sha256+'  '+path.basename(file)+'\n',{flag:'wx'});
JS
