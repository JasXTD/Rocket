// Native Windows check of 0.53 rebase --update-refs through authenticated localhost MCP, with
// synthetic owned repositories (no network). A stacked branch points into the replayed
// commits: the rebase review lists it and moves it only when the option is ticked (ordinary
// and interactive, after a reorder, squash and drop); a branch checked out in another worktree
// stays; a branch moved after the review makes Apply stale; a conflict keeps the planned
// moves for Continue. Diagnostics stay paired and free of branch names and messages.
// Usage: node tools/tests/test-mcp-update-refs.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root = path.resolve('work/mcp-fixtures/update-refs-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/update-refs';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] }, fixtures = [];
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const FILE = '検索/結果 🐦.txt', STACK = 'PRIVATE-stack-1', LOWER = 'PRIVATE-stack-b';

// main gets a new commit; the fixture branch has A, B, C on the old base. STACK points at A.
// With `conflict`, main also writes B's file, so replaying B stops.
function fixture(name, conflict = false) {
  const repo = path.join(root, name), hooks = path.join(root, name + '-hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8' }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  const commit = message => { git(['add', '-A']); git(['commit', '-m', message]); return git(['rev-parse', 'HEAD']); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Stack Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  write(FILE, 'base\n'); commit('PRIVATE_BASE 中文');
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  git(['branch', 'main']);
  const ids = {};
  for (const letter of ['A', 'B', 'C']) {
    write(letter === 'B' ? FILE : `stack/${letter}.txt`, `${letter}\n`);
    ids[letter] = commit(`PRIVATE_${letter} 中文`);
    if (letter === 'A') git(['branch', STACK]);
  }
  git(['switch', 'main']); write(conflict ? FILE : 'main.txt', 'main\n'); commit('PRIVATE_DESTINATION'); git(['switch', branch]);
  const f = { repo, git, write, ids, config: hash(path.join(repo, '.git/config')) };
  fixtures.push(f);
  return f;
}
const rev = (f, name) => f.git(['rev-parse', name]);
const subjects = (f, range) => f.git(['log', '--reverse', '--format=%s', range]);

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < 45000, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, error: state.dialog?.error, notice: state.notice }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
const control = id => state.controls.find(c => c.id === id);
const active = () => state.tabs.find(t => t.id === state.active_tab);
const latest = () => state.activities.at(-1);
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function scroll(target, direction, pixels) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); await stable(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function menuItem(id) { await click('menu-repository'); await scroll('repository-menu-scroll', 'up', 2000); if (SUBMENU[id]) { await click(SUBMENU[id]); await stable(); } for (let i = 0; i < 8; i++) { const c = control(id); if (c && c.rect[3] - c.rect[1] >= 28) break; await scroll('repository-menu-scroll', 'down', 250); } await click(id); }
async function present(target) { await settle(); const seen = () => { const c = control(target); return c && c.rect[3] - c.rect[1] >= 12; }; if (seen()) return; await scroll('dialog-rebase-scroll', 'up', 2000); for (let i = 0; i < 14 && !seen(); i++) await scroll('dialog-rebase-scroll', 'down', 240); assert(seen(), 'Control not visible: ' + target); }
async function ensure(target) { await settle(); if (control(target)?.enabled) return; await scroll('dialog-rebase-scroll', 'up', 2000); if (control(target)?.enabled) return; for (let i = 0; i < 14; i++) { await scroll('dialog-rebase-scroll', 'down', 240); if (control(target)?.enabled) return; } throw Error('Control not visible: ' + target); }
async function open(f) { if (state?.dialog) await click('dialog-cancel'); await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920); if (control('console-close')) await click('console-close'); await click('view-commits'); }
async function plan(menu) { await menuItem(menu); await type('dialog-rebase-target', 'main'); await click('dialog-preview-refresh'); await until(s => s.dialog?.rebase_preview_loaded || s.dialog?.error, 'Rebase review'); assert(state.dialog.rebase_preview_loaded, JSON.stringify(state.dialog)); await stable(); }
async function recover(action) { await click('operation-' + action); await until(s => s.dialog?.rebase_preview_loaded || s.dialog?.error, 'Recovery review'); await stable(); }
async function submit() { await click('dialog-submit'); await settle(); }
const updates = () => state.dialog.rebase_update_refs;
const listed = () => updates().branches.map(b => `${b.branch}@${b.target.slice(0, 9)}:${b.moves ? 'moves' : 'stays'}`).sort().join(' ');
async function tick() { await ensure('dialog-rebase-update-refs'); await click('dialog-rebase-update-refs'); assert.equal(updates().choice, true); }
async function closeWindow() { if (state?.dialog && !state.write) await click('dialog-cancel'); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }

async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);

    // Ordinary rebase: the branch is listed; without the option it stays.
    const off = fixture('PRIVATE_OFF');
    await open(off); await plan('repository-rebase');
    assert.equal(listed(), `refs/heads/${STACK}@${off.ids.A.slice(0, 9)}:moves`);
    assert.equal(updates().choice, false); assert.equal(updates().preview, false);
    await capture('ordinary-review-off');
    await submit();
    assert(latest().command_succeeded, JSON.stringify(latest()));
    assert.equal(subjects(off, 'main..HEAD'), 'PRIVATE_A 中文\nPRIVATE_B 中文\nPRIVATE_C 中文');
    assert.equal(rev(off, STACK), off.ids.A);
    passed('Ordinary rebase review lists the branch pointing into the replayed commits; without the option it stays at its commit');

    // Ordinary rebase with the option: the branch moves to the rewritten A; a branch moved
    // after the review first makes Apply stale.
    const on = fixture('PRIVATE_ON');
    await open(on); await plan('repository-rebase'); await tick();
    await capture('ordinary-review-on');
    on.git(['branch', '-f', STACK, on.ids.B]);
    const before = rev(on, 'HEAD');
    await submit();
    assert(state.dialog?.error?.includes('stale_preview'), JSON.stringify(state.dialog?.error));
    assert.equal(rev(on, 'HEAD'), before); assert.equal(rev(on, STACK), on.ids.B);
    await capture('ordinary-stale');
    on.git(['branch', '-f', STACK, on.ids.A]);
    await click('dialog-preview-refresh'); await until(s => s.dialog?.rebase_preview_loaded && !s.dialog?.error, 'Fresh review');
    if (!updates().choice) await tick();
    await submit();
    assert(latest().command_succeeded, JSON.stringify(latest()));
    assert.equal(rev(on, STACK), rev(on, 'HEAD~2')); assert.equal(rev(on, STACK + '~1'), rev(on, 'main'));
    assert(latest().details.some(d => d.startsWith(`${STACK} moved with the rebase: ${on.ids.A.slice(0, 9)} → `)), JSON.stringify(latest().details));
    assert.equal(rev(on, 'refs/rocket/recovery/rebase/' + before), before);
    await capture('ordinary-moved');
    passed('With "Move them with the rebase" the branch moves to its rewritten commit; a branch moved after the review makes Apply stale and changes nothing');

    // Interactive: C moves up and is squashed into A, B is dropped. STACK (at A) gets the
    // combined commit; LOWER (at the dropped B) gets the commit before it.
    const inter = fixture('PRIVATE_INTERACTIVE');
    inter.git(['branch', LOWER, inter.ids.B]);
    await open(inter); await plan('repository-interactive-rebase');
    assert.equal(listed(), [`refs/heads/${LOWER}@${inter.ids.B.slice(0, 9)}:moves`, `refs/heads/${STACK}@${inter.ids.A.slice(0, 9)}:moves`].sort().join(' '));
    await tick();
    await ensure('dialog-rebase-up-2'); await click('dialog-rebase-up-2');
    await ensure('dialog-rebase-action-1'); await click('dialog-rebase-action-1'); await click('dialog-rebase-action-1-squash');
    await ensure('dialog-rebase-row-1'); await click('dialog-rebase-row-1'); await ensure('dialog-rebase-message'); await type('dialog-rebase-message', 'PRIVATE_A and C 中文');
    await ensure('dialog-rebase-action-2'); await click('dialog-rebase-action-2'); await click('dialog-rebase-action-2-drop');
    await submit(); assert(state.dialog.rebase_plan.reviewed);
    await capture('interactive-review');
    await resize(780, 620); await capture('interactive-review-narrow'); await resize(1480, 920);
    await submit();
    assert(latest().command_succeeded, JSON.stringify(latest()));
    assert.equal(subjects(inter, 'main..HEAD'), 'PRIVATE_A and C 中文');
    assert.equal(rev(inter, STACK), rev(inter, 'HEAD')); assert.equal(rev(inter, LOWER), rev(inter, 'HEAD'));
    assert.equal(fs.readFileSync(path.join(inter.repo, FILE), 'utf8'), 'base\n');
    passed('Interactive rebase with the option: after a reorder, squash and drop, both branches point at the right rewritten commits');

    // A branch checked out in another worktree is listed as staying; alone it cannot be moved.
    const wt = fixture('PRIVATE_WORKTREE');
    const elsewhere = path.join(root, 'PRIVATE_WORKTREE checkout');
    wt.git(['worktree', 'add', elsewhere, STACK]);
    await open(wt); await plan('repository-rebase');
    assert.equal(listed(), `refs/heads/${STACK}@${wt.ids.A.slice(0, 9)}:stays`);
    await present('dialog-rebase-update-refs'); assert.equal(control('dialog-rebase-update-refs').enabled, false);
    await capture('worktree-only-stays');
    await click('dialog-cancel');
    wt.git(['branch', LOWER, wt.ids.B]);
    await plan('repository-rebase'); await tick(); await submit();
    assert(latest().command_succeeded, JSON.stringify(latest()));
    assert.equal(rev(wt, STACK), wt.ids.A); assert.equal(rev(wt, LOWER), rev(wt, 'HEAD~1'));
    assert(latest().details.some(d => d.startsWith(`${STACK} stays at ${wt.ids.A.slice(0, 9)}: it is checked out in `)), JSON.stringify(latest().details));
    passed('A branch checked out in another worktree is shown as staying and stays; the option is unavailable when it is the only one');

    // A conflict: the rebase stops; Continue's review lists the branch it will move.
    const stop = fixture('PRIVATE_CONFLICT', true);
    await open(stop); await plan('repository-rebase'); await tick(); await submit();
    assert(active().operation_state.rebase, JSON.stringify(active().operation_state));
    assert(latest().details.some(d => d === `${STACK} moves when the rebase finishes (--update-refs).`), JSON.stringify(latest().details));
    stop.write(FILE, 'resolved\n'); stop.git(['add', '--', FILE]);
    await recover('continue');
    assert.equal(updates().preview, true); assert.equal(listed(), `refs/heads/${STACK}@${stop.ids.A.slice(0, 9)}:moves`);
    await capture('continue-review');
    await submit();
    assert(latest().command_succeeded, JSON.stringify(latest()));
    assert(!active().operation_state.rebase);
    assert.equal(rev(stop, STACK), rev(stop, 'HEAD~2')); assert.notEqual(rev(stop, STACK), stop.ids.A);
    passed('After a conflict, the Continue review lists the branch the rebase will move, and finishing moves it');

    for (const f of fixtures) assert.equal(hash(path.join(f.repo, '.git/config')), f.config, 'Repository config changed');
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'stack-1', 'stack-b', '検索']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    const starts = records.filter(r => r.event === 'request_start');
    for (const s of starts) assert(records.some(r => r.event === 'request_end' && r.session === s.session && r.request === s.request), 'Unpaired ' + s.operation);
    report.operations = [...new Set(starts.map(s => s.operation))].filter(o => /rebase/.test(o));
    assert(records.some(r => r.failure_code === 'stale_preview'));
    report.log_records = records.length; passed('Diagnostics: paired rebase requests, the stale refusal, zero dropped events, no branch names, messages or paths');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); }
    fs.writeFileSync('work/update-refs-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
  }
}
run().catch(() => { process.exitCode = 1; });
