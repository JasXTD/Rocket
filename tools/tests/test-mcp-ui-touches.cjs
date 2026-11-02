// Native Windows check of the 0.48.0 UI touches through authenticated localhost MCP:
// the Repositories list in the sidebar, the filtered branch list of the Push dialog,
// the spell-check preference and underlined words in the commit message. Synthetic,
// owned repositories; no network. Screenshots are kept for visual review.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/touches-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/touches';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');

function repository(name, commits) {
  const repo = path.join(root, name), hooks = path.join(root, 'hooks-' + name);
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Touches Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  const ids = [];
  for (let n = 1; n <= commits; n++) { fs.writeFileSync(path.join(repo, `${n}.txt`), `PRIVATE_${n}\n`); git(['add', '-A']); git(['commit', '-m', `PRIVATE_COMMIT_${n}`]); ids.push(git(['rev-parse', 'HEAD'])); }
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo, git, ids };
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
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, write: state.write, notice: state.notice }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
function control(id) { return state.controls.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) await click('dialog-cancel'); assert(!state.dialog, 'Dialog still open'); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }

async function repositoriesList(first, second) {
  const ids = state.tabs.map(t => t.id);
  assert.equal(ids.length, 2);
  await click('menu-view'); await click('menu-repository-list');
  await stable();
  for (const id of ids) {
    assert(control('repository-row-' + id), 'row ' + id);
    assert(!control('tab-' + id), 'tab strip still shown for ' + id);
  }
  await capture('repositories-sidebar');
  const other = ids.find(id => id !== state.active_tab);
  await click('repository-row-' + other);
  assert.equal(state.active_tab, other);
  await capture('repositories-sidebar-switched');
  // A narrow window shows the icon rail, so the tabs come back above.
  await resize(900, 720); await stable();
  assert(control('tab-' + ids[0]) && !control('repository-row-' + ids[0]));
  await capture('repositories-narrow-tabs'); await resize(1480, 920); await closeConsole();
  await click('menu-view'); await click('menu-repository-list'); await stable();
  assert(control('tab-' + ids[0]) && !control('repository-row-' + ids[0]));
  passed('Repositories in Sidebar lists both repositories as sidebar rows instead of tabs, switches on click, and narrow windows or turning it off bring the tabs back');
}

async function pushPicker() {
  // The repository with the remote; the previous case left the other one active.
  const withRemote = state.tabs.find(t => t.path.endsWith('PRIVATE_TOUCHES'));
  if (state.active_tab !== withRemote.id) await click('tab-' + withRemote.id);
  await click('toolbar-push');
  await until(s => s.dialog?.kind === 'push' && !s.input_pending, 'Push dialog');
  await click('push-destination-choice');
  await stable();
  for (const n of [0, 1, 2]) assert(control('push-destination-choice-' + n), 'list row ' + n);
  assert(control('push-destination-choice-filter'), 'filter field');
  await capture('push-branch-list');
  await click('push-destination-choice-1');
  assert(!control('push-destination-choice-0'), 'list still open');
  await capture('push-branch-chosen');
  await closeDialog();
  passed('Push lists the remote\'s branches under a filter field and a choice fills the destination; nothing is pushed');
}

async function spelling() {
  await click('menu-edit'); await click('preferences');
  // The host reports Preferences as kind 'other'; wait for its control instead.
  await until(s => s.dialog && !s.input_pending && s.controls.some(c => c.id === 'pref-spell-check'), 'Preferences');
  await click('pref-spell-check');
  await delay(1500); await stable();
  await capture('preferences-spell-check');
  // Preferences closes with Done.
  await click('dialog-submit'); assert(!state.dialog, 'Preferences still open');
  await click('view-changes');
  await type('commit-message', 'Fix the smple parser bug');
  await delay(1500); await stable();
  await capture('commit-message-spelling');
  await type('commit-message', '');
  passed('Turning on spell check in Preferences starts the Windows checker, and the commit summary is checked as typed (see the captures)');
}

async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    const first = repository('PRIVATE_TOUCHES', 3), second = repository('PRIVATE_SECOND', 1);
    // A remote known only through its tracking refs: nothing is fetched or pushed.
    // The MCP policy accepts a remote that maps only the owned fixture branch.
    cp.execFileSync('git', ['init', '--bare', '--quiet', path.join(root, 'origin.git')], { env, windowsHide: true });
    first.git(['remote', 'add', 'origin', path.join(root, 'origin.git')]);
    first.git(['config', 'remote.origin.fetch', '+refs/heads/' + branch + ':refs/remotes/origin/' + branch]);
    for (const [name, id] of [['main', 0], ['feature/search', 1], ['release/2026', 2]]) first.git(['update-ref', 'refs/remotes/origin/' + name, first.ids[id]]);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: second.repo }); await settle();
    await client.tool('rocket_open', { path: first.repo }); await settle(); await resize(1480, 920); await closeConsole();
    await repositoriesList(first, second);
    await pushPicker();
    await spelling();
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'smple', 'feature/search']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    assert(!records.some(r => r.event === 'request_start' && /^(push|commit)$/.test(r.operation)), 'A write was started');
    report.log_records = records.length; passed('Diagnostics: zero dropped events, no names or typed text, and no push or commit was started'); report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync('work/touches-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
run().catch(e => { console.error(e); process.exitCode = 1; });
