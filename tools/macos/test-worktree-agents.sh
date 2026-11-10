#!/bin/bash
# Focused native Mac acceptance. Uses installed tools and locked dependencies only.
set -euo pipefail
umask 077
test "$(id -u)" != 0
root=$(cd "$(dirname "$0")/../.." && pwd -P)
source "$root/tools/macos/architecture.sh"
rocket_macos_architecture
cd "$root"
out=${ROCKET_MACOS_OUTPUT:?Set ROCKET_MACOS_OUTPUT to a new absolute result folder}
case "$out" in /*) ;; *) echo 'Output must be absolute.' >&2; exit 2;; esac
test ! -e "$out" && test ! -L "$out"
mkdir -p "$(dirname "$out")"
mkdir "$out"
exec > >(tee "$out/worktree-agents.log") 2>&1
export ROCKET_TEST_REPOSITORIES="$out/repositories"
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export MACOSX_DEPLOYMENT_TARGET=15.0
export ROCKET_LUA_WORKER_SHA256=
target="${CARGO_TARGET_DIR:-$root/target/macos-worktree-agents}"
case "$target" in /*) ;; *) echo 'Cargo target must be absolute.' >&2; exit 2;; esac
cargo_run() { rustup run 1.99.1 cargo "$@" --offline --locked --target-dir "$target"; }
# No ignored cases or discovery-only invocation: these run real Git operations.
cargo_run test -p rocket-git --test worktrees --test worktree_tracking --test worktree_maintenance
rustup run 1.99.1 cargo test --offline --locked --target-dir "$target" -p rocket-git --lib local_files::unix_tests::non_replacing_moves
rustup run 1.99.1 cargo test --offline --locked --target-dir "$target" -p rocket-git --lib local_files::unix_tests::filesystems_without_the_flag
rustup run 1.99.1 cargo test --offline --locked --target-dir "$target" -p rocket-ui -p rocket-mcp --lib agent -- --test-threads=1
# The native workflow reviews executable hashes repeatedly. Use the same
# optimized crypto/code path as the shipped app, not the large debug helper.
cargo_run build --release -p rocket-mcp
export ROCKET_AGENTS_EXE="$target/release/rocket-mcp"
export ROCKET_AGENTS_REPORT_ROOT="$out/native"
# Synthetic processes only; no AI account, browser sign-in or remote repository.
node tools/tests/test-mcp-agents.cjs
printf '%s\n' '{"passed":true,"scope":"macos-worktrees-and-agent-sessions"}' > "$out/result.json"
