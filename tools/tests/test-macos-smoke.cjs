'use strict';
// Native CI smoke, also runnable against an existing Windows/Linux CLI to verify
// this harness. Only this run's freshly created fixture is ever modified/deleted.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {spawnSync} = require('node:child_process');

const [binaryDirectory, evidenceDirectory] = process.argv.slice(2);
assert(binaryDirectory && evidenceDirectory && process.argv.length === 4,
  'usage: node tools/tests/test-macos-smoke.cjs BINARY_DIRECTORY NEW_EVIDENCE_DIRECTORY');
const cli = path.resolve(binaryDirectory, process.platform === 'win32' ? 'rocket-cli.exe' : 'rocket-cli');
assert(fs.statSync(cli).isFile(), 'CLI must exist');
const evidence = path.resolve(evidenceDirectory);
fs.mkdirSync(evidence, {recursive: true});
const resultPath = path.join(evidence, 'cli-smoke.json');
assert(!fs.existsSync(resultPath), 'Existing evidence is preserved; choose a new directory');
const tempParent = fs.realpathSync(os.tmpdir());
const fixture = fs.realpathSync(fs.mkdtempSync(path.join(tempParent, 'rocket-macos-smoke-')));
const repo = path.join(fixture, 'repo 資料');
const logs = path.join(fixture, 'logs');
const env = {...process.env};
for (const key of Object.keys(env)) {
  if (/^(GIT_|GCM_|SSH_|ROCKET_)/i.test(key)) delete env[key];
}
const emptyConfig = path.join(fixture, 'empty.gitconfig');
fs.writeFileSync(emptyConfig, '');
Object.assign(env, {
  GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: emptyConfig,
  GIT_TERMINAL_PROMPT: '0', GCM_INTERACTIVE: 'never',
  XDG_STATE_HOME: path.join(fixture, 'state'),
  XDG_CONFIG_HOME: path.join(fixture, 'config'),
  // macOS uses HOME/Library/Application Support; never touch the runner's profile.
  HOME: path.join(fixture, 'home'),
});
fs.mkdirSync(env.HOME, {mode: 0o700});

function run(program, args, expected = 0) {
  const result = spawnSync(program, args, {
    cwd: fixture, env, encoding: 'utf8', timeout: 60000, maxBuffer: 2 * 1024 * 1024,
    windowsHide: true,
  });
  if (result.error) throw result.error;
  assert.equal(result.status, expected, `${path.basename(program)}: ${result.stderr}`);
  return result.stdout;
}
function rocket(args, expected = 0) {
  const result = JSON.parse(run(cli, ['--repo', repo, '--log-dir', logs, '--json', ...args], expected));
  assert.equal(result.schema_version, 1);
  assert.equal(result.ok, expected === 0);
  if (result.diagnostics) assert.equal(result.diagnostics.dropped_events, 0);
  return result;
}

let passed = false;
try {
  fs.mkdirSync(repo);
  run('git', ['-C', repo, 'init', '--initial-branch=main', '--template=']);
  const name = '資料-🚀.md';
  const contents = 'Synthetic macOS preparation fixture.\n';
  fs.writeFileSync(path.join(repo, name), contents);
  const before = run('git', ['--no-optional-locks', '-C', repo, 'status', '--porcelain=v2', '-z']);
  const version = rocket(['--version']);
  assert.match(version.data.version, /^\d+\.\d+\.\d+/);
  const status = rocket(['status']);
  assert(status.data.entries.some(entry => entry.path.text === name && entry.kind === 'untracked'));
  const cancelled = rocket(['--cancel-after-ms', '0', 'status'], 130);
  assert.equal(cancelled.error.code, 'cancelled');
  const missing = JSON.parse(run(cli, [
    '--repo', path.join(fixture, 'absent'), '--log-dir', logs, '--json', 'status',
  ], 1));
  assert.equal(missing.ok, false);
  assert(missing.error.code && missing.error.message);
  assert.equal(run('git', ['--no-optional-locks', '-C', repo, 'status', '--porcelain=v2', '-z']), before);
  assert.equal(fs.readFileSync(path.join(repo, name), 'utf8'), contents);
  assert(!fs.existsSync(path.join(repo, '.git', 'index')), 'Read-only smoke must not stage files');
  const journal = fs.readFileSync(path.join(logs, 'rocket.jsonl'), 'utf8')
    .trim().split(/\r?\n/).map(line => JSON.parse(line));
  assert(journal.length > 0, 'Diagnostic journal must contain events');
  fs.writeFileSync(resultPath, JSON.stringify({
    passed: true, platform: process.platform, arch: process.arch,
    version: version.data.version,
    checks: ['version-json', 'unicode-status', 'pre-cancellation', 'missing-repository', 'repository-preserved', 'diagnostic-json'],
    diagnosticEvents: journal.length,
    limitations: 'No desktop/MCP, authentication, remote write or active-process cancellation test.',
  }, null, 2) + '\n', {flag: 'wx'});
  passed = true;
  console.log('PASS CLI version/Unicode status/pre-cancellation/errors/preservation/diagnostics');
} finally {
  if (passed) {
    // Bound the resolved path before recursive removal; never accept a caller's
    // repository or evidence directory as the cleanup target.
    assert.equal(path.dirname(fixture), tempParent);
    assert(path.basename(fixture).startsWith('rocket-macos-smoke-'));
    assert(!fs.lstatSync(fixture).isSymbolicLink());
    fs.rmSync(fixture, {recursive: true});
  } else {
    console.error(`Failed fixture preserved: ${fixture}`);
  }
}
