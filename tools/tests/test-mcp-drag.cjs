// Native Windows acceptance for drag and drop (0.44.0) through authenticated
// localhost MCP and the rocket_drag tool: a branch dropped on the current branch,
// and a commit dropped on another commit, open only reviewed dialogs. Synthetic,
// owned repository; no network.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/drag-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/drag';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');

function fixture() {
  const repo = path.join(root, 'PRIVATE_DRAG'), hooks = path.join(root, 'hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
  const write = (file, body) => fs.writeFileSync(path.join(repo, file), body);
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Drag Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  const ids = [];
  for (let n = 1; n <= 4; n++) { write(`${n}.txt`, `PRIVATE_${n}\n`); git(['add', '-A']); git(['commit', '-m', `PRIVATE_COMMIT_${n}`]); ids.push(git(['rev-parse', 'HEAD'])); }
  git(['branch', 'feature', ids[1]]); git(['switch', '--quiet', 'feature']); write('f.txt', 'PRIVATE_FEATURE\n'); git(['add', '-A']); git(['commit', '-m', 'PRIVATE_FEATURE']);
  git(['switch', '--quiet', branch]); git(['branch', 'other', ids[0]]);
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
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, drop: state.drop, write: state.write, notice: state.notice }));
    await delay(80);
  }
}
const tab = () => state?.tabs?.find(t => t.id === state.active_tab);
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
function control(id) { return state.controls.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function drag(source, target) { await stable(); assert(control(source)?.enabled && control(target)?.enabled, 'Missing/disabled drag ends ' + source + ' ' + target); await client.tool('rocket_drag', { source, target, expected_tab: state.active_tab }); await settle(); await stable(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) await click('dialog-cancel'); assert(!state.dialog, 'Dialog still open'); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
function insideViewport(prefix) { for (const c of state.controls.filter(c => c.id.startsWith(prefix))) assert(c.rect[0] >= 0 && c.rect[2] <= state.viewport[0] + 1 && c.rect[3] <= state.viewport[1] + 1, 'Outside the window: ' + c.id); }
async function refused(source, target, pattern) { await assert.rejects(client.tool('rocket_drag', { source, target, expected_tab: state.active_tab }), pattern); await settle(); }

async function branches(f) {
  const head = f.git(['rev-parse', 'HEAD']);
  const feature = 'sidebar-branch-refs/heads/feature', current = 'sidebar-branch-refs/heads/' + branch, other = 'sidebar-branch-refs/heads/other';
  await drag(feature, current);
  assert.deepEqual(state.drop, { kind: 'branch', tab: state.active_tab, source: 'refs/heads/feature', target: 'refs/heads/' + branch });
  assert(control('drop-merge')?.enabled && control('drop-rebase')?.enabled);
  await capture('drop-branch-menu');
  await click('drop-rebase');
  assert.equal(state.drop, null); assert.equal(state.dialog.kind, 'rebase'); assert.equal(state.dialog.first, 'refs/heads/feature');
  await until(s => s.dialog?.rebase_preview_loaded, 'Rebase review');
  assert.deepEqual(state.dialog.rebase_preview.commits, [f.ids[2], f.ids[3]]);
  await capture('drop-branch-rebase-review'); await closeDialog();
  // Dragging the current branch onto another works too and offers the same choices.
  await drag(current, feature); await click('drop-merge');
  assert.equal(state.dialog.kind, 'merge'); assert.equal(state.dialog.first, 'refs/heads/feature');
  await until(s => s.dialog?.merge && !s.dialog.merge.loading, 'Merge review');
  await capture('drop-branch-merge-review'); await closeDialog();
  passed('Branch drops: a branch dropped on the current branch (or the reverse) offers Merge and Rebase, each opening its reviewed dialog prefilled; nothing is applied');
  // Two branches that are not checked out: only the reason and Cancel.
  await drag(feature, other);
  assert(state.drop && !control('drop-merge') && !control('drop-rebase'));
  await capture('drop-branch-unavailable'); await click('drop-cancel'); assert.equal(state.drop, null);
  await drag(feature, current); await client.tool('rocket_escape'); await settle(); assert.equal(state.drop, null);
  assert.equal(f.git(['rev-parse', 'HEAD']), head);
  passed('A drop between two branches that are not checked out explains why and offers only Cancel; Escape discards a drop');
}

async function commits(f) {
  const [c1, c2, c3, c4] = f.ids, head = f.git(['rev-parse', 'HEAD']);
  await click('view-commits'); await closeConsole();
  await drag('commit-' + c4, 'commit-' + c2);
  assert.deepEqual(state.drop, { kind: 'commit', tab: state.active_tab, source: c4, target: c2 });
  for (const id of ['drop-above', 'drop-below', 'drop-squash', 'drop-fixup', 'drop-cancel']) assert(control(id)?.enabled, id);
  await capture('drop-commit-menu');
  await resize(780, 620); await stable(); insideViewport('drop-'); await capture('drop-commit-menu-narrow'); await click('drop-cancel'); await resize(1480, 920); await closeConsole();
  await drag('commit-' + c4, 'commit-' + c2); await click('drop-below');
  assert.equal(state.dialog.kind, 'interactive-rebase'); assert.equal(state.dialog.first, c1);
  await until(s => s.dialog?.rebase_preview_loaded, 'Interactive rebase review');
  assert.deepEqual(state.dialog.rebase_plan.entries, [{ commit: c4, action: 'pick' }, { commit: c2, action: 'pick' }, { commit: c3, action: 'pick' }]);
  await capture('drop-commit-move-review'); await closeDialog();
  await drag('commit-' + c4, 'commit-' + c2); await click('drop-squash');
  await until(s => s.dialog?.rebase_preview_loaded, 'Squash review');
  assert.deepEqual(state.dialog.rebase_plan.entries.map(e => e.action), ['pick', 'squash', 'pick']);
  assert.deepEqual(state.dialog.rebase_plan.entries.map(e => e.commit), [c2, c4, c3]);
  await capture('drop-commit-squash-review'); await closeDialog();
  assert.equal(f.git(['rev-parse', 'HEAD']), head);
  passed('Commit drops: Move above/below, Squash and Fixup open the interactive rebase review with the arranged plan; HEAD is unchanged until it is applied');
  await refused('commit-' + c4, 'commit-' + c4, /onto another/);
  await refused('commit-' + c4, 'sidebar-branch-refs/heads/feature', /onto another/);
  await click('drop-cancel').catch(() => {});
  // Since 0.47.0 a drop that reaches the root commit plans a rebase from the root.
  await drag('commit-' + c2, 'commit-' + c1);
  // Above would keep the order (commit 2 already follows the root), so it stays unavailable.
  for (const id of ['drop-below', 'drop-squash', 'drop-fixup']) assert(control(id)?.enabled, id);
  assert(control('drop-above') && !control('drop-above').enabled);
  await capture('drop-commit-root'); await click('drop-below');
  await until(s => s.dialog?.rebase_preview_loaded, 'Root rebase review');
  assert.equal(state.dialog.first, ''); assert.equal(state.dialog.rebase_preview.onto, null);
  assert.deepEqual(state.dialog.rebase_plan.entries.map(e => e.commit), [c2, c1, c3, c4]);
  await capture('drop-commit-root-review'); await closeDialog();
  await drag('commit-' + c3, 'commit-' + c2); await click('drop-squash');
  await until(s => s.dialog && !s.input_pending, 'Dialog open');
  await refused('commit-' + c4, 'commit-' + c2, /dialog/i);
  await closeDialog();
  passed('A drop below the root commit opens the interactive review from the root (no destination, dropped order); rocket_drag refuses the same row, mixed row kinds and drags while a dialog is open');
}

async function run() {
  let f;
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    f = fixture();
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920); await closeConsole();
    await branches(f); await commits(f);
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'feature', f.ids[3]]) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    assert(!records.some(r => r.event === 'request_start' && /^(merge|rebase|interactive-rebase)$/.test(r.operation)), 'A drop started a write');
    report.log_records = records.length; passed('Diagnostics: zero dropped events, no names or commit IDs, and no merge or rebase write was started'); report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync('work/drag-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
run().catch(e => { console.error(e); process.exitCode = 1; });
