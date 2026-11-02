// Native Windows check of merge collapse/expand in the commit graph and the File history
// timeline, through authenticated localhost MCP with a synthetic owned repository (no
// network, no writes by Rocket). Collapse and expand use the merge's details button, the
// "+N" badge and View → Collapse/Expand All Merges (←/→ and the right-click menu need key
// and secondary-button input, which MCP does not send; in-process flow tests cover them).
// Usage: node tools/tests/test-mcp-merge-fold.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/merge-fold-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/merge-fold';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const FILE = '検索/notes 🐦.txt';

// main: base, M1 (merges side-a: A1, A2), M2 (merges side-b: B1), T. FILE changes in base,
// A1, B1 and T, on four different days, for the timeline.
function fixture() {
  const repo = path.join(root, 'PRIVATE_FOLD'), hooks = path.join(root, 'hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  let when = null;
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env: when ? { ...env, GIT_AUTHOR_DATE: when, GIT_COMMITTER_DATE: when } : env, windowsHide: true, encoding: 'utf8' }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  const commit = (message, date) => { when = date; git(['add', '-A']); git(['commit', '-m', message]); const id = git(['rev-parse', 'HEAD']); when = null; return id; };
  const merge = (side, message, date) => { when = date; git(['merge', '--no-ff', '-m', message, side]); const id = git(['rev-parse', 'HEAD']); when = null; return id; };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Fold Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  const ids = {};
  write(FILE, 'base\n'); ids.base = commit('PRIVATE_BASE 中文', '2024-01-10T09:00:00+01:00');
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  git(['switch', '-c', 'side-a']);
  write(FILE, 'A1\n'); ids.A1 = commit('PRIVATE_A1', '2024-03-05T09:00:00+01:00');
  write('a2.txt', 'A2\n'); ids.A2 = commit('PRIVATE_A2', '2024-03-06T09:00:00+01:00');
  git(['switch', branch]); ids.M1 = merge('side-a', 'PRIVATE_MERGE side-a', '2024-04-01T09:00:00+01:00');
  git(['switch', '-c', 'side-b']);
  write(FILE, 'B1\n'); ids.B1 = commit('PRIVATE_B1', '2024-06-15T09:00:00+01:00');
  git(['switch', branch]); ids.M2 = merge('side-b', 'PRIVATE_MERGE side-b', '2024-07-01T09:00:00+01:00');
  git(['branch', '-D', 'side-a', 'side-b']);
  write(FILE, 'T\n'); ids.T = commit('PRIVATE_TIP', '2025-01-20T09:00:00+01:00');
  return { repo, git, ids };
}

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < 45000, label + ' timed out; ' + JSON.stringify({ notice: state.notice, rows: active()?.history_ids?.length }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
const control = id => state.controls.find(c => c.id === id);
const active = () => state.tabs?.find(t => t.id === state.active_tab);
const rows = () => active().history_ids;
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeWindow() { await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
const collapsed = () => Object.fromEntries((active().collapsed_merges || []).map(m => [m.id, m.hidden]));

async function run() {
  try {
    const f = fixture(), ids = f.ids;
    const head = f.git(['rev-parse', 'HEAD']), index = hash(path.join(f.repo, '.git/index'));
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920);
    if (control('console-close')) await click('console-close');
    await click('view-commits');
    assert.equal(rows().length, 7);
    await capture('expanded');

    // One merge: its details button collapses it; the +2 badge expands it.
    await click('commit-' + ids.M1);
    await click('detail-merge-fold');
    assert.deepEqual(rows(), [ids.T, ids.M2, ids.B1, ids.M1, ids.base]);
    assert.deepEqual(collapsed(), { [ids.M1]: 2 });
    assert(control('commit-expand-' + ids.M1), 'No +2 badge');
    await capture('collapsed-one');
    await click('commit-expand-' + ids.M1);
    assert.equal(rows().length, 7);
    assert.deepEqual(collapsed(), {});
    passed('A merge collapses from its details (its side commits leave the list, "+2" shows) and its badge expands it again');

    // Every merge, then one expanded again, then all expanded.
    await click('menu-view'); await click('view-collapse-merges');
    assert.equal(active().all_merges_collapsed, true);
    assert.deepEqual(rows(), [ids.T, ids.M2, ids.M1, ids.base]);
    assert.deepEqual(collapsed(), { [ids.M2]: 1, [ids.M1]: 2 });
    await capture('collapsed-all');
    await click('commit-' + ids.M2); await click('detail-merge-fold');
    assert.deepEqual(rows(), [ids.T, ids.M2, ids.B1, ids.M1, ids.base]);
    await click('menu-view'); await click('view-expand-merges');
    assert.equal(active().all_merges_collapsed, false);
    assert.equal(rows().length, 7);
    passed('View → Collapse All Merges keeps the first-parent line; one merge reopens from its details; Expand All Merges restores every row');

    // Narrow window with a collapsed merge, then a selected side commit hidden by Collapse all.
    await click('commit-' + ids.A1);
    await click('menu-view'); await click('view-collapse-merges');
    assert.equal(active().selected_commit, ids.M1);
    await resize(780, 620);
    assert(control('commit-expand-' + ids.M1), 'No badge in the narrow window');
    await capture('collapsed-narrow');
    await resize(1480, 920);
    await click('menu-view'); await click('view-expand-merges');
    passed('Collapsing the merge that hides the selected commit selects that merge; the badge stays usable at 780x620');

    // File history timeline: four commits by date, oldest at the left; clicks select.
    await click('view-filehistory');
    await type('file-path', FILE); await click('file-load');
    await until(s => active().file_history.entries.length === 4 && !active().file_history.running, 'File history');
    const listed = active().file_history.entries.map(e => e.id);
    assert.deepEqual(listed, [ids.T, ids.B1, ids.A1, ids.base]);
    const x = id => { const c = control('file-timeline-' + id); assert(c, 'No timeline tick for ' + id); return (c.rect[0] + c.rect[2]) / 2; };
    assert(x(ids.base) < x(ids.A1) && x(ids.A1) < x(ids.B1) && x(ids.B1) < x(ids.T));
    // Gaps follow the dates: A1 to B1 (about 3 months) is shorter than B1 to T (about 7).
    assert(x(ids.B1) - x(ids.A1) < x(ids.T) - x(ids.B1));
    report.timeline_x = Object.fromEntries(['base', 'A1', 'B1', 'T'].map(k => [k, Math.round(x(ids[k]))]));
    await click('file-timeline-' + ids.base);
    await until(s => !active().file_history.running, 'Selected file version');
    assert.equal(active().file_history.selected, 3);
    await capture('timeline-oldest');
    await click('file-timeline-' + ids.B1);
    await until(s => !active().file_history.running, 'Selected file version');
    assert.equal(active().file_history.selected, 1);
    await resize(900, 700); await capture('timeline-narrow'); await resize(1480, 920);
    passed('File history shows a timeline: commits placed by date, oldest left; clicking a commit selects it');

    assert.equal(f.git(['rev-parse', 'HEAD']), head); assert.equal(hash(path.join(f.repo, '.git/index')), index);
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', '検索', 'notes']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    const starts = records.filter(r => r.event === 'request_start');
    for (const s of starts) assert(records.some(r => r.event === 'request_end' && r.session === s.session && r.request === s.request), 'Unpaired ' + s.operation);
    report.log_records = records.length; passed('HEAD and index unchanged; diagnostics paired with zero dropped events and no names, paths or messages');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); }
    fs.writeFileSync('work/merge-fold-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
  }
}
run().catch(() => { process.exitCode = 1; });
