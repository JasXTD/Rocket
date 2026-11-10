#!/bin/bash
# Diagnose the exact retained 0.92.3 Intel archive; no rebuild or personal profile.
set -euo pipefail
umask 077
test "${GITHUB_ACTIONS:-}" = true
test "${RUNNER_OS:-}" = macOS
test "${GITHUB_REPOSITORY:-}" = JasXTD/Rocket
test "${ROCKET_REQUESTED_MAC_ARCH:-}" = x64
test "$(uname -m)" = x86_64
out=${ROCKET_MACOS_OUTPUT:?}
test ! -e "$out"
mkdir -p "$out/downloads" "$out/mcp"
exec > >(tee "$out/retained-package.log") 2>&1
artifact=13805392743
run=42900875578
gh api "/repos/$GITHUB_REPOSITORY/actions/runs/$run" > "$out/source-run.json"
gh api "/repos/$GITHUB_REPOSITORY/actions/artifacts/$artifact" > "$out/source-artifact.json"
node - "$out" <<'JS'
const fs=require('node:fs'),assert=require('node:assert/strict'),p=process.argv[2];
const r=JSON.parse(fs.readFileSync(p+'/source-run.json')),a=JSON.parse(fs.readFileSync(p+'/source-artifact.json'));
assert.equal(r.id,42900875578);assert.equal(r.event,'workflow_dispatch');assert.equal(r.head_branch,'main');assert.equal(r.head_sha,'57e688e3c3372dcf7fac5e1a93e93f88f90aeb93');
assert.equal(r.path,'.github/workflows/macos-wasm-prototype.yml');assert.equal(r.head_repository.full_name,'JasXTD/Rocket');assert.equal(r.status,'completed');
assert.equal(a.id,13805392743);assert.equal(a.workflow_run.id,r.id);assert.equal(a.name,'lua-mac-x64-42900875578-1');assert(!a.expired);
assert.equal(a.digest,'sha256:56c4a456765950ba5d6cbcf2ce33b7ebe08bbef495cf6709c44c918e0193d3cb');
JS
gh api "/repos/$GITHUB_REPOSITORY/actions/artifacts/$artifact/zip" > "$out/downloads/artifact.zip"
unset GH_TOKEN GITHUB_TOKEN
test "$(shasum -a 256 "$out/downloads/artifact.zip" | cut -d ' ' -f1)" = 56c4a456765950ba5d6cbcf2ce33b7ebe08bbef495cf6709c44c918e0193d3cb
# Both ZIPs are exact, authenticated outputs of our pinned source run. They were
# independently path-checked in the local release workspace before this dispatch.
/usr/bin/ditto -x -k "$out/downloads/artifact.zip" "$out/downloads/artifact"
archive="$out/downloads/artifact/prototype-results/Rocket-0.92.3-macos-x64-preview.zip"
test "$(shasum -a 256 "$archive" | cut -d ' ' -f1)" = 077f91675937575be287913f7e81aa64191ff4acb71b8a950fa841de92876a6f
/usr/bin/ditto -x -k "$archive" "$out/extracted"
node tools/macos/bundle.cjs verify "$out/extracted/package"
export ROCKET_MCP_EXE="$out/extracted/package/Rocket.app/Contents/MacOS/rocket-mcp"
export ROCKET_MCP_RESULTS="$out/mcp" ROCKET_MACOS_SANDBOX_DISPOSABLE_USER=1
collect() {
  node - "$out" <<'JS'
const fs=require('node:fs'),path=require('node:path'),out=process.argv[2],root=out+'/mcp';
fs.mkdirSync(out+'/evidence',{recursive:true});
for(const name of fs.readdirSync(root)){const dir=path.join(root,name);if(!fs.statSync(dir).isDirectory())continue;
for(const f of fs.readdirSync(dir))if(f==='result.json'||f==='failure-state.json'||f.endsWith('.png'))fs.copyFileSync(path.join(dir,f),path.join(out,'evidence',name+'-'+f));}
JS
}
trap collect EXIT
# First run is deliberately cold: a diagnostic must not warm the service and
# accidentally hide the original failure. Its status determines the final result.
cold=0
node tools/tests/test-mcp-macos-package.cjs || cold=$?
xcrun clang -arch x86_64 -mmacosx-version-min=15.0 -fobjc-arc -fblocks -Wall -Wextra -Werror -framework AppKit tools/tests/macos-spelling-diagnostic.m crates/rocket-spell/src/macos.m -o "$out/spelling-probe"
"$out/spelling-probe" > "$out/spelling-native.json"
warm=0
if [ "$cold" != 0 ]; then node tools/tests/test-mcp-macos-package.cjs || warm=$?; fi
# Finish the previously unexecuted checks against these same extracted bytes.
export ROCKET_AGENTS_EXE="$ROCKET_MCP_EXE" ROCKET_AGENTS_REPORT_ROOT="$out/mcp"
node tools/tests/test-mcp-agents.cjs
export ROCKET_CLI_EXE="$out/extracted/package/Rocket.app/Contents/MacOS/rocket-cli" ROCKET_CLI_RESULTS="$out/mcp"
node tools/tests/test-cli-hosting-other-inline.cjs
node tools/macos/bundle.cjs verify "$out/extracted/package"
node - "$out" "$cold" "$warm" <<'JS'
const fs=require('node:fs');fs.writeFileSync(process.argv[2]+'/retained-result.json',JSON.stringify({source_run:42900875578,source_commit:'57e688e3c3372dcf7fac5e1a93e93f88f90aeb93',package_sha256:'077f91675937575be287913f7e81aa64191ff4acb71b8a950fa841de92876a6f',cold_exit:Number(process.argv[3]),warm_exit:Number(process.argv[4]),passed:process.argv[3]==='0'},null,2));
JS
exit "$cold"
