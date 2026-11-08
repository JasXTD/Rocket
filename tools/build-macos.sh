#!/bin/bash
# Apple Silicon development build only. Lua remains disabled; no publisher identity,
# credentials, release publication, system Git installation or system settings needed.
set -euo pipefail
umask 077

offline=
case "${1:-}" in
    '') ;;
    --offline) offline=--offline; shift ;;
    *) echo 'usage: bash tools/build-macos.sh [--offline]' >&2; exit 2 ;;
esac
if [ "$#" -ne 0 ]; then
    echo 'usage: bash tools/build-macos.sh [--offline]' >&2
    exit 2
fi
if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    echo 'This build requires native Apple Silicon macOS (not WSL or Rosetta).' >&2
    exit 2
fi
if [ "$(id -u)" = 0 ]; then
    echo 'Run this build as your ordinary user, without sudo.' >&2
    exit 2
fi
for tool in rustup node git xcrun; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Required installed tool is missing: $tool" >&2
        exit 2
    fi
done
if ! xcode-select -p >/dev/null 2>&1; then
    echo 'Xcode command-line tools must already be installed; no installer was started.' >&2
    exit 2
fi

root=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$root"
scope=${ROCKET_MACOS_CHECK_SCOPE:-platform}
case "$scope" in
    platform|followups) ;;
    *) echo 'ROCKET_MACOS_CHECK_SCOPE must be platform or followups.' >&2; exit 2 ;;
esac
toolchain=1.99.1
target=aarch64-apple-darwin
# Consume the full producer before matching. grep -q can close its pipe after
# the host line, causing rustup to receive SIGPIPE and fail under pipefail.
if ! rust_identity=$(rustup run "$toolchain" rustc -vV); then
    echo 'Rust 1.99.1 for aarch64-apple-darwin must already be installed.' >&2
    exit 2
fi
if ! printf '%s\n' "$rust_identity" | grep -Fx "host: $target" >/dev/null; then
    echo 'Rust 1.99.1 must use the native aarch64-apple-darwin host.' >&2
    exit 2
fi
out=${ROCKET_MACOS_OUTPUT:-"$root/target/macos-stage1"}
case "$out" in
    /*) ;;
    *) echo 'ROCKET_MACOS_OUTPUT must be an absolute, new directory.' >&2; exit 2 ;;
esac
if [ -e "$out" ] || [ -L "$out" ]; then
    echo 'The output directory already exists; choose a new directory to preserve it.' >&2
    exit 2
fi
mkdir -p "$(dirname "$out")"
mkdir "$out"
exec > >(tee "$out/build.log") 2>&1
trap 'result=$?; if [ "$result" -ne 0 ]; then printf "FAILED: macOS preparation stopped (exit %s). No release was produced.\n" "$result"; fi' EXIT

# Keep these resource settings local to this build. They do not change the normal
# Windows/Linux profiles. No cross-run Actions cache is uploaded in this first pass.
export CARGO_BUILD_JOBS=2
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export MACOSX_DEPLOYMENT_TARGET=15.0
export ROCKET_LUA_WORKER_SHA256=
build_target="$root/target/macos-stage1-cargo"
common=(--locked --target "$target" --target-dir "$build_target")
# macOS ships Bash 3.2: expanding an empty array with nounset fails there.
if [ -n "$offline" ]; then common+=("$offline"); fi
cargo_run() { rustup run "$toolchain" cargo "$@"; }

{
    echo '# macOS development app build identity'
    echo
    echo 'Development app with ad-hoc integrity signatures; not Developer-ID signed or notarized.'
    echo 'Lua sandbox: unavailable (execution refused). Deployment target: macOS 15.'
    printf 'Affected check scope: %s\n' "$scope"
    printf 'Commit: '; git rev-parse HEAD
    printf 'Local changes: '; git status --porcelain | wc -l
    sw_vers
    printf '%s\n' "$rust_identity"
    printf 'macOS SDK: '; xcrun --sdk macosx --show-sdk-version
    git --version
} > "$out/build-info.md"

# New-platform compilation is broad; execution is limited to the affected boundary.
cargo_run check "${common[@]}" --workspace --all-targets
if [ "$scope" = platform ]; then
# Darwin ABI must agree with the installed SDK, not just Rust's cross compiler.
xcrun --sdk macosx clang -std=c11 -Wall -Wextra -Werror tools/tests/macos-terminal-abi.c -o "$out/macos-terminal-abi"
"$out/macos-terminal-abi"
xcrun --sdk macosx clang -std=c11 -Wall -Wextra -Werror -Wno-deprecated-declarations \
    tools/tests/macos-auth-abi.c -o "$out/macos-auth-abi"
"$out/macos-auth-abi"
xcrun --sdk macosx clang -std=c11 -Wall -Wextra -Werror tools/tests/macos-desktop-abi.c -o "$out/macos-desktop-abi"
"$out/macos-desktop-abi"
cargo_run test "${common[@]}" -p rocket-git --lib system_languages::
for filter in shortcuts:: desktop:: folder_browser::; do
    cargo_run test "${common[@]}" -p rocket-ui --lib "$filter" -- --test-threads=1
done
for filter in forge::protection::macos_policy ssh_keys:: \
    forge::protection::macos::tests; do
    cargo_run test "${common[@]}" -p rocket-git --lib "$filter"
done
# Keychain creation can register a search-list entry. Only the ephemeral CI/OS
# user may opt into this fixture; no default Keychain item is read or written.
if [ "${ROCKET_MACOS_DISPOSABLE_KEYCHAIN_TEST:-}" = 1 ]; then
    cargo_run test "${common[@]}" -p rocket-git --lib \
        forge::protection::macos::tests::disposable_keychain -- --ignored --test-threads=1
fi
cargo_run test "${common[@]}" -p rocket-terminal --lib --test unix_session --test unix_signals
cargo_run test "${common[@]}" -p rocket-mcp-control
cargo_run test "${common[@]}" -p rocket-mcp --lib terminal_probes
cargo_run test "${common[@]}" -p rocket-mcp --lib http::tests
cargo_run test "${common[@]}" -p rocket-ui --lib terminal_settings::tests
cargo_run test "${common[@]}" -p rocket-git --lib integration_worker::preflight_tests
# Core platform services only. These calls are real Darwin runtime tests, unlike
# cross-compilation. Fixtures use the runner's native filesystem and synthetic data.
for filter in advisory_tool::tests::sha256 local_files::unix_tests local_storage:: \
    runner::macos_git::tests process_tree:: runner::tests write_gate::tests; do
    cargo_run test "${common[@]}" -p rocket-git --lib "$filter"
done
export ROCKET_TEST_REPOSITORIES
# Darwin sockaddr_un has only 104 bytes; its default per-user TMPDIR plus the
# nested repository name leaves no room for the disposable agent's socket.
# Use a new private native directory, never a normal user's SSH files or agent.
ROCKET_TEST_REPOSITORIES=$(mktemp -d /private/tmp/rocket-core.XXXXXXXX)
echo "Isolated repository fixtures: $ROCKET_TEST_REPOSITORIES"
for suite in preview_lock repository_setup clone file_actions conflict_editor ssh_keys ssh_discovery ssh_direct custom_commands; do
    cargo_run test "${common[@]}" -p rocket-git --test "$suite"
done
# Never recursively clean a failed fixture; rmdir requires successful cleanup.
rmdir "$ROCKET_TEST_REPOSITORIES"
unset ROCKET_TEST_REPOSITORIES
# Builds the actual worker only for negative tests: it must exit before reading
# input. It is deliberately absent from the downloadable development artifact.
cargo_run test "${common[@]}" -p rocket-integrations --features runtime --test unsupported_worker
fi
# Native notifications, fallback glyphs and their license/package consumers.
xcrun --sdk macosx clang -std=c11 -Wall -Wextra -Werror tools/tests/macos-watch-abi.c -o "$out/macos-watch-abi"
"$out/macos-watch-abi"
cargo_run test "${common[@]}" -p rocket-git --lib watch::platform::tests
for filter in fonts:: legal::; do
    cargo_run test "${common[@]}" -p rocket-ui --lib "$filter" -- --test-threads=1
done
export ROCKET_TEST_REPOSITORIES
ROCKET_TEST_REPOSITORIES=$(mktemp -d /private/tmp/rocket-watch.XXXXXXXX)
cargo_run test "${common[@]}" -p rocket-git --test watch -- --test-threads=1
for filter in native_watch_ native_refresh_preserves_selection_drafts_and_detects_repeated_edits \
    worktree_refresh_reuses_history_but_reads_changes_and_selected_diff_fresh \
    linked_worktree_observes_shared_refs_and_writes_supersede_auto_refresh \
    metadata_and_included_configuration_force_complete_refreshes; do
    cargo_run test "${common[@]}" -p rocket-ui --test refresh_state "$filter" -- --test-threads=1
done
rmdir "$ROCKET_TEST_REPOSITORIES"
unset ROCKET_TEST_REPOSITORIES
cargo_run build "${common[@]}" --workspace --bins

cargo_run build "${common[@]}" -p rocket-ui --example macos_icon
icon_tool="$build_target/$target/debug/examples/macos_icon"
ROCKET_MACOS_ICON_TOOL="$icon_tool" node --test tools/tests/test-macos-bundle.cjs
"$icon_tool" "$out/Rocket.iconset"
node tools/macos/bundle.cjs iconset "$out/Rocket.iconset"
/usr/bin/iconutil -c icns "$out/Rocket.iconset" -o "$out/Rocket.icns"
/usr/bin/iconutil -c iconset "$out/Rocket.icns" -o "$out/roundtrip.iconset"
node tools/macos/bundle.cjs iconset "$out/roundtrip.iconset"
stage="$out/Rocket-macos-arm64-development"
node tools/macos/bundle.cjs stage "$build_target/$target/debug" "$out/Rocket.icns" "$stage" "$out/build-info.md"
node tools/macos/bundle.cjs finalize "$stage"
app_bin="$stage/Rocket.app/Contents/MacOS"
for executable in rocket rocket-mcp rocket-cli rocket-rebase-editor rocket-ssh; do
    /usr/bin/otool -L "$app_bin/$executable" >> "$out/native-links.log"
done
node tools/tests/test-macos-smoke.cjs "$app_bin" "$out"
if [ "$scope" = platform ]; then
node --test tools/tests/test-platform-process.cjs
ROCKET_MCP_EXE="$app_bin/rocket-mcp" ROCKET_MCP_RESULTS="$out/mcp" node tools/tests/test-mcp-terminal-interrupt.cjs
ROCKET_MCP_EXE="$app_bin/rocket-mcp" ROCKET_MCP_RESULTS="$out/mcp" node tools/tests/test-mcp-ssh-discovery.cjs
fi
ROCKET_MCP_EXE="$app_bin/rocket-mcp" ROCKET_MCP_RESULTS="$out/mcp" node tools/tests/test-mcp-watch.cjs
ROCKET_MCP_EXE="$app_bin/rocket-mcp" ROCKET_MCP_RESULTS="$out/mcp" node tools/tests/test-mcp-terminal-unicode.cjs
ROCKET_MCP_EXE="$app_bin/rocket-mcp" ROCKET_MCP_RESULTS="$out/mcp" node tools/tests/test-mcp-desktop.cjs
node tools/macos/bundle.cjs verify "$stage"
(
    cd "$stage"
    /usr/bin/shasum -a 256 -c SHA256SUMS
)
archive=Rocket-macos-arm64-development.zip
/usr/bin/ditto -c -k --keepParent "$stage" "$out/$archive"
mkdir "$out/archive-check"
/usr/bin/ditto -x -k "$out/$archive" "$out/archive-check"
node tools/macos/bundle.cjs verify "$out/archive-check/Rocket-macos-arm64-development"
(cd "$out"; /usr/bin/shasum -a 256 "$archive" > "$archive.sha256")
echo 'PASS macOS compilation, affected native checks, packaged helpers, ad-hoc signatures and archive roundtrip.'
echo 'Finder/Gatekeeper/EC2 Mac acceptance, Lua sandbox and distribution signing remain pending.'
