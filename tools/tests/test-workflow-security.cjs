'use strict';
// Execute the real input guards locally; do not dispatch or emulate privileged CI.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const cp = require('node:child_process');
const root = path.resolve(__dirname, '../..');
const bash = process.env.ROCKET_TEST_BASH || (process.platform === 'win32'
  ? path.join(process.env.ProgramFiles, 'Git/bin/bash.exe') : 'bash');
const workflows = [
  ['macos-arm64.yml', ['followups', 'platform', 'sandbox', 'desktop-integration']],
  ['macos-wasm-prototype.yml', ['integration', 'prototype', 'complete-package', 'release-package']],
];

function runBlocks(text) {
  const lines = text.split(/\r?\n/), blocks = [];
  for (let i = 0; i < lines.length; i++) {
    const match = /^(\s*)run: \|$/.exec(lines[i]);
    if (!match) continue;
    const indent = match[1].length + 2, body = [];
    while (++i < lines.length && (!lines[i].trim() || /^\s*/.exec(lines[i])[0].length >= indent)) body.push(lines[i].slice(indent));
    i--;
    blocks.push(body.join('\n'));
  }
  return blocks;
}

for (const [name, modes] of workflows) {
  const source = fs.readFileSync(path.join(root, '.github/workflows', name), 'utf8');
  test(name + ': manual, read-only, pinned actions and no shell interpolation', () => {
    assert.match(source, /^on:\n  workflow_dispatch:/m);
    assert.doesNotMatch(source, /^\s*(?:pull_request(?:_target)?|push|workflow_run|schedule|repository_dispatch):/m);
    assert.doesNotMatch(source, /(?:write-all|:\s*write\b|secrets\.|self-hosted)/);
    const uses = [...source.matchAll(/^\s*-?\s*uses:\s*(\S+)/gm)].map(m => m[1]);
    assert(uses.length >= 2);
    for (const action of uses) assert.match(action, /^actions\/(?:checkout|upload-artifact)@[a-f0-9]{40}$/);
    const checkouts = uses.filter(x => x.startsWith('actions/checkout@')).length;
    assert.equal((source.match(/persist-credentials: false/g) || []).length, checkouts);
    const runs = runBlocks(source);
    assert(runs.length >= 2);
    for (const body of runs) assert(!body.includes('${{'), 'GitHub expressions must enter shell steps through quoted environment variables');
  });
  test(name + ': actual guard accepts known modes and refuses injected values', () => {
    const guard = runBlocks(source).find(b => b.includes('case "$ROCKET_MACOS_CHECK_SCOPE" in'));
    assert(guard, 'Validate inputs before checking out/building any source');
    assert(source.indexOf('- name: Validate requested checks') < source.indexOf('uses: actions/checkout@'));
    const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'rocket-ci-input-'));
    const marker = path.join(temp, 'must-not-exist');
    const env = { ...process.env };
    delete env.BASH_ENV; delete env.ENV;
    try {
      const run = (mode,arch='arm64') => cp.spawnSync(bash, ['--noprofile', '--norc', '-c', guard], {
        cwd: temp, env: { ...env, ROCKET_MACOS_CHECK_SCOPE: mode, ROCKET_REQUESTED_MAC_ARCH: arch }, encoding: 'utf8', windowsHide: true, timeout: 10000,
      });
      for (const arch of ['arm64','x64']) for (const mode of modes) {
        const result = run(mode,arch);
        assert.ifError(result.error); assert.equal(result.status, 0, mode);
      }
      for (const arch of ['', 'intel', 'self-hosted', '$(touch must-not-exist)', 'x64\ntouch must-not-exist']) {
        const result = run(modes[0],arch);
        assert.ifError(result.error); assert.equal(result.status, 2);
        assert.match(result.stderr, /Unsupported architecture selection/);
        assert(!fs.existsSync(marker), 'Architecture input must never execute as shell code');
      }
      for (const mode of ['', 'unknown', "'; touch must-not-exist; #", '$(touch must-not-exist)', 'integration\ntouch must-not-exist']) {
        const result = run(mode);
        assert.ifError(result.error); assert.equal(result.status, 2);
        assert.match(result.stderr, /Unsupported check selection/);
        assert(!fs.existsSync(marker), 'Input must never execute as shell code');
      }
      assert.deepEqual(fs.readdirSync(temp), []);
    } finally { fs.rmdirSync(temp); }
  });
}
