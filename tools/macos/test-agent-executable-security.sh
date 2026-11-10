#!/bin/bash
# Native qualification of reviewed agent/setup executable binding, not a release.
set -euo pipefail
umask 077
root=$(cd "$(dirname "$0")/../.." && pwd -P)
source "$root/tools/macos/architecture.sh"
rocket_macos_architecture
test "$(id -u)" != 0
cd "$root"
out=${ROCKET_MACOS_OUTPUT:?Set ROCKET_MACOS_OUTPUT to a new absolute result folder}
case "$out" in /*) ;; *) echo 'Output must be absolute.' >&2; exit 2;; esac
test ! -e "$out" && test ! -L "$out"
mkdir -p "$(dirname "$out")"
mkdir "$out"
exec > >(tee "$out/agent-security.log") 2>&1
stage=preflight
commit=$(git rev-parse HEAD)
finish() {
    result=$?
    trap - EXIT
    node - "$out" "$stage" "$result" "$ROCKET_MAC_ARCH" "$commit" <<'JS'
const fs=require('node:fs'),path=require('node:path');
const [out,stage,code,architecture,commit]=process.argv.slice(2);
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify({passed:Number(code)===0,stage,exit_code:Number(code),architecture,commit,scope:'agent-executable-security',release_published:false},null,2));
JS
    exit "$result"
}
trap finish EXIT
identity=$(rustup run 1.99.1 rustc -vV)
printf '%s\n' "$identity" | grep -Fx "host: $ROCKET_MAC_TARGET" >/dev/null
printf '%s\n' "$identity" > "$out/compiler.log"
sw_vers > "$out/system.log"
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
export CARGO_PROFILE_RELEASE_DEBUG=0 MACOSX_DEPLOYMENT_TARGET=15.0
export ROCKET_LUA_WORKER_SHA256=
# A short, new native path avoids Darwin's short Unix-domain socket path limit.
export ROCKET_TEST_REPOSITORIES
ROCKET_TEST_REPOSITORIES=$(mktemp -d /private/tmp/rocket-agent-security.XXXXXXXX)
target="${CARGO_TARGET_DIR:-$root/target/macos-agent-security}"
case "$target" in /*) ;; *) echo 'Cargo target must be absolute.' >&2; exit 2;; esac
cargo_run() { rustup run 1.99.1 cargo "$@"; }
stage=locked-dependencies
cargo_run fetch --locked --target "$ROCKET_MAC_TARGET"
common=(--offline --locked --release --target-dir "$target")
stage=agents-regressions
cargo_run test "${common[@]}" -p rocket-ui --lib agents:: -- --test-threads=1 2>&1 | tee "$out/agents-tests.log"
stage=terminal-regressions
cargo_run test "${common[@]}" -p rocket-terminal --lib -- --test-threads=1 2>&1 | tee "$out/terminal-tests.log"
for check in custom_programs_run_by_their_full_path_and_missing_ones_say_so \
    a_shell_that_exits_reports_its_code_and_takes_its_background_jobs_with_it \
    dropping_a_session_stops_jobs_that_ignore_hangup_without_touching_other_sessions; do
    cargo_run test "${common[@]}" -p rocket-terminal --test unix_session "$check" -- --exact --test-threads=1 2>&1 | tee -a "$out/terminal-tests.log"
done
stage=build-mcp
cargo_run build "${common[@]}" -p rocket-mcp
stage=native-mcp
export ROCKET_AGENTS_EXE="$target/release/rocket-mcp"
export ROCKET_AGENTS_REPORT_ROOT="$out/mcp"
node tools/tests/test-mcp-agents.cjs
stage=verify-results
node - "$out" "$ROCKET_AGENTS_EXE" <<'JS'
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const [out,exe]=process.argv.slice(2),agents=fs.readFileSync(path.join(out,'agents-tests.log'),'utf8'),terminal=fs.readFileSync(path.join(out,'terminal-tests.log'),'utf8');
for(const name of ['executable_swap_after_recheck_never_runs_replacement','setup_handoff_rechecks_original_but_launches_reviewed_snapshots','snapshot_cancellation_and_oversize_leave_no_partial_executable','snapshot_is_private_bounded_and_removed_without_touching_other_files','unsafe_snapshot_parent_and_source_links_are_refused','native_snapshot_preserves_arguments_and_worktree_and_stop_releases_it']) assert(agents.includes(name+' ... ok'),'Missing native regression: '+name);
const counts=text=>[...text.matchAll(/test result: ok\. (\d+) passed; 0 failed;/g)].map(m=>Number(m[1]));
assert(counts(agents).some(n=>n>=13),'Agents suite did not run');
const terminalCounts=counts(terminal);assert(terminalCounts.length===4&&terminalCounts[0]>0&&terminalCounts.slice(1).every(n=>n===1),'Terminal library and three exact lifecycle regressions must all run');
const names=fs.readdirSync(path.join(out,'mcp'));assert.equal(names.length,1);
const report=JSON.parse(fs.readFileSync(path.join(out,'mcp',names[0],'result.json'),'utf8'));
const sha=crypto.createHash('sha256').update(fs.readFileSync(exe)).digest('hex');
assert(report.passed&&report.cases.length===18&&report.token_removed&&report.exe_sha256===sha,'Native MCP acceptance incomplete');
fs.writeFileSync(path.join(out,'verified.json'),JSON.stringify({passed:true,agents:counts(agents),terminal:terminalCounts,mcp_cases:report.cases.length,exe_sha256:sha},null,2));
JS
stage=complete
