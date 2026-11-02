// Native Windows acceptance for the Preferences → General settings through authenticated
// localhost MCP: source code folder, tags in fetch, spaces in branch names, submodules on
// checkout, push after commit, the diff options and compact labels. Synthetic, owned
// repositories; every remote is a bare repository inside the fixture root; no network.
// Run from the repository root, under the build and MCP lock.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/general-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), shots = path.join(root, 'shots'), branch = 'rocket-ui-test/general';
const report = { passed: false, root, cases: [], captures: [] };
fs.mkdirSync(shots, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
// The host runs automatic fetch only with this marker, here a 2-second test interval.
fs.writeFileSync(path.join(root, 'rocket-auto-fetch'), '2\n');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_TERMINAL_PROMPT: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const run = (cwd, args) => cp.execFileSync('git', args, { cwd, env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trim();

function fixture() {
  const repo = path.join(root, 'PRIVATE_GENERAL'), child = path.join(root, 'child-source'), origin = path.join(root, 'origin.git'), code = path.join(root, 'code');
  for (const dir of [repo, child, code]) fs.mkdirSync(dir);
  const identity = cwd => { for (const [key, value] of [['user.name', 'Rocket General Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false']]) run(cwd, ['config', key, value]); };
  run(child, ['init', '--initial-branch=main']); identity(child);
  fs.writeFileSync(path.join(child, 'c.txt'), 'old\n'); run(child, ['add', '-A']); run(child, ['commit', '-m', 'child one']);
  const old = run(child, ['rev-parse', 'HEAD']);
  fs.writeFileSync(path.join(child, 'c.txt'), 'new\n'); run(child, ['add', '-A']); run(child, ['commit', '-m', 'child two']);
  const fresh = run(child, ['rev-parse', 'HEAD']);
  const git = args => run(repo, args);
  git(['init', '--initial-branch=' + branch]); identity(repo);
  fs.writeFileSync(path.join(repo, 'greeter.rs'), 'fn greet(name: &str) -> String {\n    format!("Hello {name}")\n}\n');
  git(['add', '-A']); git(['commit', '-m', 'PRIVATE_COMMIT_1']);
  git(['-c', 'protocol.file.allow=always', 'submodule', 'add', child, 'vendor/child']);
  git(['commit', '-m', 'PRIVATE_COMMIT_2']);
  git(['branch', 'rocket-ui-test/topic']);
  git(['-C', 'vendor/child', 'checkout', '--quiet', '--detach', old]);
  git(['update-index', '--cacheinfo', '160000,' + old + ',vendor/child']);
  git(['commit', '-m', 'PRIVATE_COMMIT_3']);
  run(root, ['init', '--quiet', '--bare', origin]);
  git(['remote', 'add', 'origin', origin]);
  git(['push', '--quiet', 'origin', branch, branch + ':refs/heads/rocket-ui-test/commands']);
  git(['fetch', '--quiet', '--no-tags', 'origin']);
  git(['branch', '--set-upstream-to=origin/' + branch, branch]);
  // The MCP host fetches only the owned fixture branch.
  git(['config', 'remote.origin.fetch', '+refs/heads/' + branch + ':refs/remotes/origin/' + branch]);
  run(root, ['--git-dir', origin, 'tag', 'v-remote', git(['rev-parse', 'HEAD'])]);
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo, git, code, old, fresh };
}

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < WAIT_MS, 'Timed out: ' + label);
    await delay(50);
  }
}
const tab = () => state?.tabs?.find(t => t.id === state.active_tab);
const settle = () => until(s => !s.busy && !s.input_pending && !s.write && !(s.tabs || []).some(t => t.loading), 'Settled');
const control = id => state.controls.find(c => c.id === id);
const width = id => { const [left, , right] = control(id).rect; return right - left; };
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 2 && !s.input_pending; }, 'Stable controls'); }
async function scroll(target, direction, pixels) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await stable(); }
async function reveal(target) { await stable(); if (control(target)) return; if (control('prefs-scroll')) { await scroll('prefs-scroll', 'up', 2000); for (let n = 0; !control(target) && n < 14; n++) await scroll('prefs-scroll', 'down', 200); } }
async function click(target) { await reveal(target); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await reveal(target); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const item = result.content.find(c => c.type === 'image'); const file = path.join(shots, name + '.png'); fs.writeFileSync(file, Buffer.from(item.data, 'base64')); report.captures.push(file); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) { await stable(); await click(control('dialog-close') ? 'dialog-close' : 'dialog-cancel'); } assert(!state.dialog, 'Dialog closed'); }
async function preferences() { await click('menu-edit'); await click('preferences'); await stable(); assert(control('pref-page-general'), 'Preferences is open'); await click('pref-page-general'); }
async function open(f) { await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920); await closeConsole(); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Window closed'); } }

async function sourceFolder(f) {
  assert.deepEqual(state.general, { source_folder: null, fetch_tags: 'none', branch_space: 'none', submodules_on_checkout: false, push_after_commit: false, diff_marks: true, diff_font: 13, diff_syntax: true, compact_labels: false });
  passed('Every General setting starts at today\'s behaviour');
  await capture('01-general-top');
  await type('pref-source-folder', 'Code');
  assert.equal(state.general.source_folder, null);
  await capture('02-folder-partial');
  await click('pref-source-browse'); await until(s => s.controls.some(c => c.id === 'picker-choose'), 'folder picker'); await type('picker-filter', 'code'); await until(s => s.controls.some(c => c.id === 'picker-row-0') && !s.controls.some(c => c.id === 'picker-row-1'), 'only the code folder listed'); await click('picker-row-0'); await click('picker-choose');
  await until(s => s.general.source_folder?.endsWith('code'), 'Browse chose the folder');
  passed('A partial path waits; Browse sets the source code folder');
  await capture('03-folder-set');
  await click('pref-source-clear'); assert.equal(state.general.source_folder, null);
  await type('pref-source-folder', path.join(root, 'gone')); await delay(400); await scroll('prefs-scroll', 'down', 120); await capture('04-folder-missing');
  await click('pref-source-clear');
  fs.writeFileSync(path.join(root, 'a-file.txt'), 'file'); await type('pref-source-folder', path.join(root, 'a-file.txt')); await delay(400); await capture('05-folder-file');
  await click('pref-source-clear');
  await type('pref-source-folder', f.code); assert.equal(state.general.source_folder, f.code);
  passed('The folder row warns about a missing folder and a file, and clears');
}

async function settings() {
  await click('pref-fetch-tags-all'); assert.equal(state.general.fetch_tags, 'all');
  await click('pref-branch-space-hyphen'); assert.equal(state.general.branch_space, 'hyphen');
  await click('pref-submodules-on-checkout'); assert.equal(state.general.submodules_on_checkout, true);
  await click('pref-push-after-commit'); assert.equal(state.general.push_after_commit, true);
  await capture('06-repositories-card');
  await click('pref-diff-font-16'); assert.equal(state.general.diff_font, 16);
  await capture('07-diff-font-16');
  await click('pref-diff-marks'); assert.equal(state.general.diff_marks, false);
  await click('pref-diff-syntax'); assert.equal(state.general.diff_syntax, false);
  await capture('08-diff-plain');
  await click('pref-diff-font-13'); await click('pref-diff-syntax'); assert.equal(state.general.diff_syntax, true);
  await click('pref-diff-font-15');
  await capture('09-commit-list-standard');
  await click('pref-compact-labels'); assert.equal(state.general.compact_labels, true);
  await capture('10-commit-list-compact');
  passed('Each setting answers its control');
}

async function dialogs(f) {
  await click('toolbar-fetch'); await stable();
  assert.equal(state.dialog.fetch_tags, 'all');
  await capture('11-fetch-all-tags'); await closeDialog();
  passed('Fetch starts at the chosen tags');
  await click('menu-file'); await click('file-clone'); await stable();
  await type('dialog-clone-source', path.join(root, 'child-source')); await stable();
  assert(state.dialog.second.startsWith(f.code + path.sep), 'Clone starts in the source code folder: ' + state.dialog.second);
  await capture('12-clone'); await closeDialog();
  await click('menu-file'); await click('repository-new'); await stable();
  assert.equal(state.dialog.first, f.code + path.sep);
  await delay(400); await stable();
  assert(!control('dialog-submit').enabled, 'Create stays off for the folder itself');
  await capture('13-new-repository');
  await type('dialog-init-path', path.join(f.code, 'fresh')); await delay(400); await stable();
  assert(control('dialog-submit').enabled, 'Create turns on with a name');
  await closeDialog();
  passed('Clone and New repository start in the source code folder');
  await click('toolbar-branch'); await click('toolbar-branch-new'); await stable();
  await type('dialog-first', 'fix login page');
  assert.equal(state.dialog.first, 'fix-login-page');
  await capture('14-new-branch'); await closeDialog();
  passed('Spaces become hyphens as a branch name is typed');
}

async function changes(f) {
  await click('view-commits'); await settle(); await capture('15-commits-compact');
  fs.appendFileSync(path.join(f.repo, 'greeter.rs'), '// PRIVATE_EDIT\n'); f.git(['add', 'greeter.rs']);
  await click('view-changes');
  await click('refresh');
  await until(s => s.controls.some(c => c.id === 'commit-message'), 'Commit box');
  await type('commit-message', 'PRIVATE_MESSAGE'); await stable();
  await capture('16-commit-box');
  const head = f.git(['rev-parse', 'HEAD']);
  assert(width('commit-push') > width('commit'), 'Commit & Push is the main button');
  await click('commit-push');
  assert.equal(state.dialog?.kind, 'workflows', 'The main button opens the review');
  await capture('17-review'); await closeDialog();
  assert.equal(f.git(['rev-parse', 'HEAD']), head, 'Leaving the review commits nothing');
  passed('Push after commit swaps the buttons, and the review still comes first');
}

async function submodules(f) {
  await type('commit-message', ''); f.git(['restore', '--staged', 'greeter.rs']); f.git(['checkout', '--', 'greeter.rs']);
  await click('refresh');
  const child = path.join(f.repo, 'vendor/child'), marker = path.join(f.repo, '.git/rocket-ui-fixture');
  assert.equal(run(child, ['rev-parse', 'HEAD']), f.old);
  const switchTo = async name => {
    await click('view-branches');
    const row = tab().branches.find(b => b.reference === 'refs/heads/' + name); assert(row, name);
    const count = state.activities.length;
    await click('branch-row-' + row.index); await click('branch-switch');
    await until(s => s.activities.length >= count + 2 && !s.busy && !s.write, 'Switch and update');
    return state.activities.at(-1);
  };
  let update = await switchTo('rocket-ui-test/topic');
  assert.equal(update.operation, 'submodule-checkout'); assert(update.command_succeeded, JSON.stringify(update));
  assert.equal(update.summary, '1 submodule updated');
  assert.equal(run(child, ['rev-parse', 'HEAD']), f.fresh);
  passed('Switching branch updates the clean submodule');
  fs.writeFileSync(path.join(child, 'c.txt'), 'PRIVATE_LOCAL\n');
  fs.writeFileSync(marker, 'rocket-ui-test/topic');
  update = await switchTo(branch);
  fs.writeFileSync(marker, branch);
  assert.equal(update.summary, 'vendor/child left alone, local changes');
  assert.equal(run(child, ['rev-parse', 'HEAD']), f.fresh, 'A changed submodule stays put');
  assert.equal(fs.readFileSync(path.join(child, 'c.txt'), 'utf8'), 'PRIVATE_LOCAL\n');
  await capture('18-activity-left-alone');
  passed('A submodule with local changes is left alone and named');
  run(child, ['checkout', '--', 'c.txt']);
}

async function automaticTags(f) {
  assert.equal(f.git(['tag', '--list', 'v-remote']), '');
  await preferences(); await click('pref-auto-fetch'); await closeDialog();
  const start = performance.now();
  while (f.git(['tag', '--list', 'v-remote']) !== 'v-remote') { assert(performance.now() - start < WAIT_MS, 'The tag arrived'); await delay(200); }
  await preferences(); await click('pref-auto-fetch'); await closeDialog();
  passed('Automatic fetch brings the chosen tags');
}

async function main() {
  const exe = path.resolve(process.env.ROCKET_MCP_EXE || 'target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
  const f = fixture();
  cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
  const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
  fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
  const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log);
  client = new RocketHttpClient(profile);
  for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
  await client.initialize();
  try {
    await open(f);
    await preferences();
    await sourceFolder(f);
    await settings();
    await closeDialog();
    await dialogs(f);
    await changes(f);
    await submodules(f);
    await automaticTags(f);
    const saved = { ...state.general };
    await closeWindow(); await open(f);
    assert.deepEqual(state.general, saved);
    passed('The settings survive closing and reopening the window');
    await closeWindow();
    report.passed = true;
  } catch (error) {
    report.failure = error.stack; try { await capture('failure'); } catch {} throw error;
  } finally {
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    fs.writeFileSync('work/general-settings-last-result.txt', path.join(root, 'result.json'));
    fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2));
    if (service && service.exitCode === null) { try { await client.tool('rocket_close'); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
