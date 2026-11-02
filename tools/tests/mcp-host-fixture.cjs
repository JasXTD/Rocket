// Fixture preparation and independent verification for direct Codex MCP calls.
// This helper never starts a second Rocket/MCP process.
const fs = require('node:fs');
const path = require('node:path');
const { randomUUID, createHash } = require('node:crypto');
const { spawnSync } = require('node:child_process');
const assert = require('node:assert/strict');
const action = process.argv[2];
fs.mkdirSync(path.resolve('work/mcp-fixtures'), {recursive:true});
const parent = fs.realpathSync(path.resolve('work/mcp-fixtures'));
let root;
if (action === 'setup') {
  root = path.join(parent, 'host-' + randomUUID().replaceAll('-', ''));
  fs.mkdirSync(root);
} else {
  root = fs.realpathSync(process.argv[3]);
  assert.equal(path.dirname(root), parent);
  assert(/^host-[0-9a-f]{32}$/.test(path.basename(root)));
}
const repo = path.join(root, 'client');
const publisher = path.join(root, 'publisher');
const remote = path.join(root, 'upstream.git');
const hooks = path.join(root, 'hooks');
const branch = 'rocket-ui-test/' + path.basename(root);
const ref = 'refs/heads/' + branch;
const tracking = 'refs/remotes/origin/' + branch;
const evidence = path.join(root, 'fixture.json');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|GCM_|ROCKET_|SSH_|CURL_|SSL_CERT_)|_PROXY$|^NETRC$/i.test(key)) delete env[key];
Object.assign(env, { HOME: path.join(root, 'home'), GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'),
  GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_TEMPLATE_DIR: hooks,
  GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_OPTIONAL_LOCKS: '0', GCM_INTERACTIVE: '0', LC_ALL: 'C' });
function git(args, at = repo) {
  const r = spawnSync('git.exe', ['-C', at, '-c', 'gc.auto=0', '-c', 'maintenance.auto=false', ...args],
    { env, encoding: 'utf8', windowsHide: true, timeout: 15000, maxBuffer: 1024 * 1024 });
  assert.equal(r.status, 0, 'Fixture Git ' + args[0] + ' failed: ' + r.stderr); return r.stdout.trimEnd();
}
function hash(file) { return createHash('sha256').update(fs.readFileSync(file)).digest('hex'); }
function snapshot() {
  return { head: git(['rev-parse', 'HEAD']), index: hash(path.join(repo, '.git', 'index')),
    entries: git(['ls-files', '--stage', '-z']),
    files: fs.readdirSync(repo, { withFileTypes: true }).filter(f => f.isFile()).map(f => [f.name, hash(path.join(repo, f.name))]) };
}
function tip() { return git(['ls-remote', '--refs', 'origin', ref]).split('\t')[0]; }
let report = action === 'setup' ? { root, repo, publisher, remote, branch, checks: [] } : JSON.parse(fs.readFileSync(evidence, 'utf8'));
if (action === 'setup') {
  for (const dir of [repo, publisher, hooks, path.join(root, 'home')]) fs.mkdirSync(dir);
  fs.writeFileSync(path.join(root, 'empty.config'), '');
  for (const at of [repo, publisher]) {
    git(['init', '--initial-branch=' + branch], at);
    const settings = [['user.name', 'Rocket MCP Test'], ['user.email', 'rocket@example.invalid'], ['core.hooksPath', hooks],
      ['core.attributesFile', path.join(root, 'empty.config')], ['core.autocrlf', 'false'], ['commit.gpgsign', 'false'],
      ['credential.helper', ''], ['protocol.allow', 'never'], ['protocol.file.allow', 'always'], ['gc.auto', '0'], ['maintenance.auto', 'false']];
    for (const protocol of ['http', 'https', 'ssh', 'git', 'ext']) settings.push(['protocol.' + protocol + '.allow', 'never']);
    for (const [key, value] of settings) git(['config', key, value], at);
    git(['remote', 'add', 'origin', remote], at);
    git(['config', 'remote.origin.fetch', `+${ref}:${tracking}`], at);
  }
  git(['init', '--bare', '--initial-branch=main', remote]);
  fs.writeFileSync(path.join(repo, '.git', 'rocket-ui-fixture'), branch);
  fs.writeFileSync(path.join(repo, 'hello.txt'), 'base\n'); git(['add', '--', 'hello.txt']); git(['commit', '-m', 'Synthetic fixture base']);
  report.base = git(['rev-parse', 'HEAD']);
  fs.writeFileSync(path.join(repo, 'hello.txt'), 'edited by the direct MCP fixture\n');
  report.initial = snapshot();
} else if (action === 'verify-stage') {
  assert.equal(git(['diff', '--cached', '--name-only']), 'hello.txt');
  assert.equal(git(['diff', '--name-only']), '');
  assert.equal(git(['rev-parse', 'HEAD']), report.base);
  assert.deepEqual(snapshot().files, report.initial.files);
  report.checks.push('Stage selected fixture contents through the actual UI');
} else if (action === 'verify-unstage') {
  assert.equal(git(['diff', '--cached', '--name-only']), '');
  assert.equal(git(['diff', '--name-only']), 'hello.txt');
  assert.deepEqual(snapshot().files, report.initial.files);
  report.checks.push('Unstage preserves working file bytes');
} else if (action === 'verify-commit') {
  report.committed = git(['rev-parse', 'HEAD']); assert.notEqual(report.committed, report.base);
  assert.equal(git(['log', '-1', '--format=%s']), 'MCP connection verified');
  assert.equal(git(['status', '--porcelain']), '');
  report.checks.push('Typed commit creates the expected commit and clean worktree');
} else if (action === 'verify-push-and-advance') {
  assert.equal(tip(), report.committed);
  assert.equal(git(['rev-parse', '--symbolic-full-name', '@{upstream}']), tracking);
  git(['fetch', '--no-tags', 'origin', `${ref}:${tracking}`], publisher);
  git(['switch', '--create', branch, '--no-track', tracking], publisher);
  fs.writeFileSync(path.join(publisher, 'hello.txt'), 'remote advance from a local fixture\n');
  git(['add', '--', 'hello.txt'], publisher); git(['commit', '-m', 'Synthetic remote advance'], publisher);
  report.remoteAdvance = git(['rev-parse', 'HEAD'], publisher);
  git(['push', '--no-follow-tags', 'origin', `HEAD:${ref}`], publisher);
  assert.equal(tip(), report.remoteAdvance);
  fs.writeFileSync(path.join(repo, 'sentinel.txt'), 'keep this untracked fixture\n');
  report.beforeFetch = snapshot();
  report.checks.push('UI push updates only the owned local-remote branch and sets upstream');
} else if (action === 'verify-fetch') {
  assert.deepEqual(snapshot(), report.beforeFetch);
  assert.equal(git(['rev-parse', tracking]), report.remoteAdvance);
  report.checks.push('UI fetch updates tracking while preserving HEAD, index and files');
} else if (action === 'verify-pull-and-dirty') {
  assert.equal(git(['rev-parse', 'HEAD']), report.remoteAdvance);
  assert.equal(fs.readFileSync(path.join(repo, 'hello.txt'), 'utf8'), 'remote advance from a local fixture\n');
  assert.equal(fs.readFileSync(path.join(repo, 'sentinel.txt'), 'utf8'), 'keep this untracked fixture\n');
  fs.writeFileSync(path.join(repo, 'hello.txt'), 'dirty fixture must be preserved\n');
  report.beforeFailure = snapshot();
  report.checks.push('UI pull fast-forwards HEAD and file bytes, preserving the untracked sentinel');
} else if (action === 'verify-failure') {
  assert.deepEqual(snapshot(), report.beforeFailure);
  assert.equal(tip(), report.remoteAdvance);
  report.checks.push('Dirty-worktree pull rejection preserves all local and remote state');
} else if (action === 'verify-input-only') {
  assert.equal(report.cleanup, 'confirmed-absent');
  assert.deepEqual(snapshot(), report.beforeFailure);
  assert.equal(tip(), '');
  report.checks.push('MCP input regression leaves HEAD, index and working files unchanged; remote branch remains absent');
} else if (action === 'cleanup') {
  const current = tip(); assert.equal(current, report.remoteAdvance);
  git(['push', `--force-with-lease=${ref}:${current}`, 'origin', `:${ref}`], publisher);
  assert.equal(tip(), ''); assert.equal(git(['ls-remote', '--refs', 'origin']), '');
  report.cleanup = 'confirmed-absent';
} else { throw Error('Unknown fixture operation'); }
fs.writeFileSync(evidence, JSON.stringify(report, null, 2));
console.log(JSON.stringify({ root, repo, branch, action, passed: true, checks: report.checks, cleanup: report.cleanup }));
