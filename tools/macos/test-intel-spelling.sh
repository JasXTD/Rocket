#!/bin/bash
# Diagnose the native editor on a disposable standard Intel runner.
set -euo pipefail
trap 'echo "::error::Intel spelling preparation or native check failed at line $LINENO."' ERR
umask 077
[[ ${GITHUB_ACTIONS:-} == true && ${RUNNER_OS:-} == macOS && ${GITHUB_REPOSITORY:-} == JasXTD/Rocket ]]
[[ $(uname -m) == x86_64 && -n ${RUNNER_TEMP:-} ]]
out="$RUNNER_TEMP/rocket-spelling"
mkdir -m 700 "$out"

# The release job's Defender scan approved this exact Git source. Verify the
# immutable run, its successful scan and the pinned bytes before building it.
# No Windows job or new software version is needed for the editor-only check.
node - "$out/git-scan.json" <<'JS'
const fs=require('node:fs'),assert=require('node:assert/strict');
(async()=>{
 const api='https://api.github.com/repos/JasXTD/Rocket';
 async function read(p){const r=await fetch(api+p,{redirect:'error',signal:AbortSignal.timeout(30000)});assert(r.ok);return r.json()}
 const repo=await read(''),run=await read('/actions/runs/45755297542');
 assert.equal(repo.private,false);assert.equal(run.head_repository.full_name,'JasXTD/Rocket');
 assert.equal(run.head_sha,'66f034ee6bf29b20bf3b596957538edb9d25ec66');assert.equal(run.path,'.github/workflows/macos-wasm-prototype.yml');
 const {jobs}=await read('/actions/runs/45755297542/attempts/1/jobs?per_page=100');
 const scan=jobs.find(j=>j.name==='scan_git');assert.equal(scan.conclusion,'success');
 assert(scan.steps.some(s=>s.name==='Verify and scan the pinned Git source with Microsoft Defender'&&s.conclusion==='success'));
 fs.writeFileSync(process.argv[2],JSON.stringify({source:run.head_sha,run:run.id,scan:scan.id,passed:true}));
})().catch(e=>{console.error(e.message);process.exitCode=1});
JS
read -r git_sha archive < tools/macos/release-git.sha256
[[ $git_sha == fd0c0495427dc9ac445fdced8e36bb5eb6f233a14da5db3c6dfd8dcfa7f85761 && $archive == git-2.56.1.tar.gz ]]
curl --proto '=https' --proto-redir '=https' -fsSL --max-time 120 \
  "https://www.kernel.org/pub/software/scm/git/$archive" -o "$out/$archive"
printf '%s  %s\n' "$git_sha" "$out/$archive" | shasum -a 256 --check --status
tar -xzf "$out/$archive" -C "$out"
make -C "$out/git-2.56.1" -j3 prefix="$out/git" NO_GETTEXT=YesPlease NO_TCLTK=YesPlease install > "$out/git-build.log"
export PATH="$out/git/bin:$PATH"
[[ $(git --version) == 'git version 2.56.1' ]]

# Establish the original packaged behavior before running the same fixture
# against the current source. Neither run installs Rocket on the runner.
archive=Rocket-0.92.7-macos-x64-preview.zip
curl --proto '=https' --proto-redir '=https' -fsSL --max-time 180 \
  "https://github.com/JasXTD/Rocket/releases/download/v0.92.7/$archive" -o "$out/$archive"
printf '%s  %s\n' 24dbb2b3efbbe75e5208320beace9d7360749f1925d1eae56e4872d3a47bc936 "$out/$archive" | shasum -a 256 --check --status
ditto -x -k "$out/$archive" "$out/released"
export ROCKET_MCP_EXE="$out/released/package/Rocket.app/Contents/MacOS/rocket-mcp"
codesign --verify --strict "$ROCKET_MCP_EXE"
export ROCKET_MCP_RESULTS="$out/mcp"
export ROCKET_SPELL_OBSERVER="$out/spelling-observer.dylib"
xcrun clang -arch x86_64 -fobjc-arc -fblocks -Wall -Wextra -Werror -dynamiclib -framework AppKit \
  tools/tests/macos-spelling-observer.m -o "$ROCKET_SPELL_OBSERVER"
codesign --force --sign - "$ROCKET_SPELL_OBSERVER"

node tools/tests/test-mcp-spelling.cjs released-failure

# Use the same pinned toolchain as the native workflow. Only build the MCP
# executable which hosts the real editor; spelling needs no Lua worker or ZIP.
rustup toolchain install 1.99.1 --profile minimal --no-self-update
export CARGO_BUILD_JOBS=3
if ! rustup run 1.99.1 cargo build --locked -p rocket-mcp --bin rocket-mcp 2>&1 | tee "$out/build.log"; then
  node - "$out" <<'JS'
const fs=require('node:fs'),path=require('node:path'),out=process.argv[2];
const errors=fs.readFileSync(path.join(out,'build.log'),'utf8').split('\n').filter(l=>/error[:\[]|^\s*-->/i.test(l)).slice(-20).map(l=>l.slice(0,400));
fs.writeFileSync(path.join(out,'build-errors.json'),JSON.stringify({errors}));
JS
  exit 1
fi
export ROCKET_MCP_EXE="$PWD/target/debug/rocket-mcp"
result=0
for layout in service-recovery release-layout immediate ready-first before-enable; do
  node tools/tests/test-mcp-spelling.cjs "$layout" || result=1
done
exit "$result"
