// Native Windows acceptance for 0.30.0 through authenticated localhost MCP: commit
// sequences with Ctrl/Shift selection, deleted-branch/Rocket-kept/unreachable recovery
// scopes and interactive edit/stop/exec. Synthetic, owned repositories only.
// Use --only=sequences|recovery|rebase to run one group.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root = path.resolve('work/mcp-fixtures/sequences-recovery-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/sequences-recovery';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const only = process.argv.find(a => a.startsWith('--only='))?.slice(7);
const report = { passed: false, version, root, only: only ?? null, cases: [] }, fixtures = [];
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');

function fixture(name) {
  const repo = path.join(root, name), hooks = path.join(root, name + '-hooks'); fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
  const write = (name, body) => { const p = path.join(repo, name); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  const commit = message => { git(['add', '-A']); git(['commit', '-m', message]); return git(['rev-parse', 'HEAD']); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket History Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  write('base.txt', 'base\n'); const base = commit('PRIVATE_BASELINE 中文'); fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  const f = { repo, hooks, git, write, commit, base, config: hash(path.join(repo, '.git/config')) }; fixtures.push(f); return f;
}

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
// Liveness bound for each awaited UI state. 45 s like the other runners; a heavily
// loaded machine may set ROCKET_MCP_WAIT_MS, which the report records.
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
report.wait_ms = WAIT_MS;
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog, write: state.write, notice: state.notice, activity: state.activities?.at(-1) }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running)); }
function active() { return state.tabs.find(t => t.id === state.active_tab); }
function control(id) { return state.controls.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target, modifiers) {
  await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_click', { target, expected_tab: state.active_tab, ...(modifiers ? { modifiers } : {}) }); await settle();
}
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
// A refused request must name its reason and leave the UI input queue empty.
async function refused(name, args, reason) {
  await stable(); let error = null;
  try { await client.tool(name, { expected_tab: state.active_tab, ...args }); } catch (e) { error = e; }
  assert(error, name + ' ' + JSON.stringify(args) + ' was not refused'); assert(error.message.includes(reason), error.message);
  await settle();
}
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function scroll(target, direction = 'down', pixels = 600) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); await stable(); }
async function menuItem(id) { await click('menu-repository'); await scroll('repository-menu-scroll', 'up', 2000); if (SUBMENU[id]) { await click(SUBMENU[id]); await stable(); } for (let i = 0; i < 8; i++) { const c = control(id); if (c && c.rect[3] - c.rect[1] >= 28) break; await scroll('repository-menu-scroll', 'down', 250); } await click(id); }
async function open(f) { if (state?.dialog) await click('dialog-cancel'); await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920); if (control('console-close')) await click('console-close'); await click('view-commits'); }
async function submit() { await click('dialog-submit'); await settle(); }
function latest() { return state.activities.at(-1); }
async function closeWindow() { if (state?.dialog && !state.write) await click('dialog-cancel'); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }

// ---- Commit sequences -------------------------------------------------------
function sequenceFixture(name) {
  const f = fixture(name);
  f.git(['switch', '-c', 'source']);
  f.write('一/a.txt', 'a\n'); f.s1 = f.commit('PRIVATE_S1 中文');
  f.write('base.txt', 'source\n'); f.s2 = f.commit('PRIVATE_S2 conflict');
  f.write('一/c 🚀.txt', 'c\n'); f.s3 = f.commit('PRIVATE_S3');
  f.git(['switch', branch]); f.write('base.txt', 'main\n'); f.main = f.commit('PRIVATE_MAIN');
  return f;
}
async function selectAll(ids) { await click('view-commits'); await click('history-scope'); await click('history-all'); await click('commit-' + ids[0]); for (const id of ids.slice(1)) await click('commit-' + id, 'ctrl'); }
async function commitPreview(operation) { await menuItem('commit-' + operation); await until(s => s.dialog && !s.dialog.commit_preview_loading && !s.busy, 'Commit preview'); await stable(); }
async function operation(kind) { await click('operation-' + kind); await until(s => s.dialog && !s.dialog.commit_preview_loading && !s.busy, 'Recovery preview'); await stable(); }
const journal = f => path.join(f.repo, '.git/rocket-commit-sequence');
async function sequences() {
  const f = sequenceFixture('PRIVATE_SEQUENCE');
  await open(f); await selectAll([f.s1, f.s2, f.s3]);
  assert.deepEqual([...active().selected_commits].sort(), [f.s1, f.s2, f.s3].sort());
  await capture('sequence-selection');
  await commitPreview('cherry-pick'); assert.equal(state.dialog.kind, 'cherry-pick');
  assert.deepEqual(state.dialog.commit_preview.commits, [f.s1, f.s2, f.s3]); assert(state.dialog.commit_preview.sequence);
  await capture('sequence-review'); await submit();
  assert.equal(latest().command_succeeded, false); assert(latest().details.some(d => d.includes('conflicts_remain')), JSON.stringify(latest()));
  assert(active().operation_state.sequencer); assert.deepEqual(active().sequence.remaining, [f.s2, f.s3]); assert(fs.existsSync(journal(f)));
  await click('view-commits'); await capture('sequence-conflict-banner');
  await operation('continue'); assert(state.dialog.error.includes('conflicts_remain'), JSON.stringify(state.dialog)); assert(!control('dialog-submit').enabled); await click('dialog-cancel');
  f.write('base.txt', 'resolved 中文\n'); f.git(['add', 'base.txt']); await click('refresh');
  await operation('continue'); assert(state.dialog.commit_preview.sequence); assert.deepEqual(state.dialog.commit_preview.remaining, [f.s2, f.s3]);
  await capture('sequence-continue-review'); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().operation_state.idle);
  assert.equal(f.git(['log', '--format=%s', f.main + '..HEAD']), 'PRIVATE_S3\nPRIVATE_S2 conflict\nPRIVATE_S1 中文');
  assert.equal(fs.readFileSync(path.join(f.repo, 'base.txt'), 'utf8'), 'resolved 中文\n'); assert(!fs.existsSync(journal(f)));
  passed('Ctrl-click selection, three-commit cherry-pick review in order, conflict pause with remaining commits, refused unresolved Continue, reviewed Continue finishing the sequence and journal cleanup');

  const skip = sequenceFixture('PRIVATE_SEQUENCE_SKIP'); await open(skip); await selectAll([skip.s1, skip.s2, skip.s3]); await commitPreview('cherry-pick'); await submit();
  assert(active().operation_state.sequencer); await operation('skip'); assert(state.dialog.commit_preview.sequence); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().operation_state.idle);
  assert.equal(skip.git(['log', '--format=%s', skip.main + '..HEAD']), 'PRIVATE_S3\nPRIVATE_S1 中文'); assert(!fs.existsSync(journal(skip)));

  const abort = sequenceFixture('PRIVATE_SEQUENCE_ABORT'); const before = abort.git(['rev-parse', 'HEAD']);
  await open(abort); await selectAll([abort.s1, abort.s2, abort.s3]); await commitPreview('cherry-pick'); await submit();
  const applied = abort.git(['rev-parse', 'HEAD']); assert.notEqual(applied, before);
  await operation('abort'); assert.equal(state.dialog.commit_preview.applied, 1); assert.equal(state.dialog.commit_preview.recovery_reference, 'refs/rocket/recovery/sequence/' + applied);
  await capture('sequence-abort-review'); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().operation_state.idle);
  assert.equal(abort.git(['rev-parse', 'HEAD']), before); assert.equal(abort.git(['rev-parse', 'refs/rocket/recovery/sequence/' + applied]), applied); assert(!fs.existsSync(journal(abort)));
  const undo = fixture('PRIVATE_REVERT_SKIP'); undo.write('a.txt', '1\n'); undo.write('b.txt', '1\n'); undo.commit('PRIVATE_U0');
  undo.write('a.txt', '2\n'); const u1 = undo.commit('PRIVATE_U1'); undo.write('b.txt', '2\n'); const u2 = undo.commit('PRIVATE_U2'); undo.write('b.txt', '3\n'); undo.commit('PRIVATE_U3');
  await open(undo); await click('commit-' + u2); await click('commit-' + u1, 'shift'); await commitPreview('revert'); assert.deepEqual(state.dialog.commit_preview.commits, [u2, u1]); await submit();
  assert(active().operation_state.revert); assert.equal(active().operation_state.step.at, 1); assert.equal(active().operation_state.step.total, 2); assert.equal(control('operation-skip').enabled, true);
  await operation('skip'); await capture('revert-skip-review'); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().operation_state.idle); assert.equal(undo.git(['log', '-1', '--format=%s']), 'Revert "PRIVATE_U1"');
  passed('Skip continues with the remaining commit; Abort returns to the starting commit and keeps the created commit at its recovery reference; a reverted range counts its commits and skips the same way');

  const range = fixture('PRIVATE_REVERT_RANGE');
  range.write('r/1.txt', '1\n'); const r1 = range.commit('PRIVATE_R1'); range.write('r/2.txt', '2\n'); const r2 = range.commit('PRIVATE_R2');
  range.write('r/1.txt', '1 changed\n'); const r3 = range.commit('PRIVATE_R3'); const baseTree = range.git(['rev-parse', range.base + '^{tree}']);
  await open(range); await click('commit-' + r3); await click('commit-' + r1, 'shift'); assert.deepEqual(active().selected_commits, [r3, r2, r1]);
  await commitPreview('revert'); assert.equal(state.dialog.kind, 'revert'); assert.deepEqual(state.dialog.commit_preview.commits, [r3, r2, r1]);
  await capture('revert-range-review'); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert.equal(range.git(['rev-parse', 'HEAD^{tree}']), baseTree); assert.equal(range.git(['rev-list', '--count', r3 + '..HEAD']), '3');
  await refused('rocket_click', { target: 'view-commits', modifiers: 'ctrl' }, 'Click modifiers apply only to commit or changed-file rows');

  const merge = fixture('PRIVATE_SEQUENCE_MERGE'); merge.git(['switch', '-c', 'side']); merge.write('side', 'side\n'); merge.commit('PRIVATE_SIDE');
  merge.git(['switch', branch]); merge.write('main', 'main\n'); const m1 = merge.commit('PRIVATE_M1'); merge.git(['merge', '--no-edit', 'side']); const merged = merge.git(['rev-parse', 'HEAD']);
  await open(merge); await click('commit-' + merged); await click('commit-' + m1, 'ctrl'); await commitPreview('revert');
  assert(state.dialog.error.includes('sequence_merge'), JSON.stringify(state.dialog)); assert(!control('dialog-submit').enabled); await capture('sequence-merge-refusal'); await click('dialog-cancel');
  assert.equal(merge.git(['rev-parse', 'HEAD']), merged);
  passed('Shift-click range reverts three commits newest first back to the base tree; modifiers are refused off commit rows; a merge in a sequence is refused before Git runs');
}

// ---- Recovery scopes -------------------------------------------------------------
async function source(key, label) { await click('dialog-reflog-scope-' + key); await until(s => s.dialog?.recovery?.scope === label && s.dialog.recovery.entries && !s.dialog.recovery.loading && !s.busy, 'Recovery scope ' + label); await stable(); }
async function recoverySources() {
  const f = fixture('PRIVATE_RECOVERY_SOURCES'), feature = 'feature/削除-🚀';
  f.git(['switch', '-c', feature]); f.write('feature.txt', 'f\n'); const deletedTip = f.commit('PRIVATE_DELETED_TIP');
  f.git(['switch', branch]); f.git(['branch', '-D', feature]);
  const kept = f.git(['commit-tree', f.git(['rev-parse', 'HEAD^{tree}']), '-p', 'HEAD', '-m', 'PRIVATE_KEPT']); f.git(['update-ref', 'refs/rocket/recovery/reset/' + kept, kept]);
  f.write('base.txt', 'PRIVATE_STASHED\n'); f.git(['stash', 'push', '-m', 'PRIVATE_STASH']); const stash = f.git(['rev-parse', 'stash@{0}']); f.git(['stash', 'drop']);
  await open(f); await menuItem('repository-recovery'); await until(s => s.dialog?.recovery?.entries && !s.dialog.recovery.loading, 'Recovery dialog'); assert.equal(state.dialog.recovery.scope, 'Head');
  // Git removed the deleted branch's own log, so All local refs does not name it.
  await source('all', 'AllRefs'); assert(!state.dialog.recovery.entries.some(e => e.selector.includes(feature)), JSON.stringify(state.dialog.recovery.entries));
  await source('deleted', 'DeletedBranches'); let i = state.dialog.recovery.entries.findIndex(e => e.id === deletedTip);
  assert(i >= 0, JSON.stringify(state.dialog.recovery)); assert.equal(state.dialog.recovery_references[i], feature); await capture('recovery-deleted-branches');
  await click('dialog-reflog-row-' + i); await click('dialog-recovery-branch'); await until(s => s.dialog?.recovery?.preview_loaded || s.dialog?.error, 'Restore review');
  assert.equal(state.dialog.first, feature); assert(state.dialog.recovery.preview_loaded, JSON.stringify(state.dialog)); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert.equal(f.git(['rev-parse', 'refs/heads/' + feature]), deletedTip); assert.equal(active().head.branch, branch);
  await menuItem('repository-recovery'); await until(s => s.dialog?.recovery?.entries && !s.dialog.recovery.loading); await source('deleted', 'DeletedBranches');
  assert(!state.dialog.recovery.entries.some(e => e.id === deletedTip), 'A restored branch is still listed as deleted');
  await source('retained', 'Retained'); i = state.dialog.recovery.entries.findIndex(e => e.id === kept); assert(i >= 0); assert.equal(state.dialog.recovery_references[i], 'refs/rocket/recovery/reset/' + kept);
  await source('unreachable', 'Unreachable'); assert(state.dialog.recovery.entries.some(e => e.id === stash), JSON.stringify(state.dialog.recovery.entries)); await capture('recovery-unreachable');
  await source('head', 'Head'); await click('dialog-cancel');
  assert.equal(f.git(['rev-parse', 'HEAD']), f.base);
  passed('Recovery scopes: a deleted branch missing from All local refs is restored under its old name and then no longer listed; Rocket-kept refs and a dropped stash are recoverable');
}

// ---- Interactive edit, stops and commands ----------------------------------------
async function ensure(target) { await settle(); if (control(target)?.enabled) return; await scroll('dialog-rebase-scroll', 'up', 2000); if (control(target)?.enabled) return; for (let i = 0; i < 14; i++) { await scroll('dialog-rebase-scroll', 'down', 240); if (control(target)?.enabled) return; } throw Error('Control not visible: ' + target); }
async function plan() { await menuItem('repository-interactive-rebase'); await type('dialog-rebase-target', 'main'); await click('dialog-preview-refresh'); await until(s => s.dialog?.rebase_preview_loaded || s.dialog?.error); assert(state.dialog.rebase_preview_loaded, JSON.stringify(state.dialog)); await stable(); }
async function action(i, name) { await ensure('dialog-rebase-action-' + i); await click('dialog-rebase-action-' + i); await click('dialog-rebase-action-' + i + '-' + name); assert.equal(state.dialog.rebase_plan.entries[i].action, name); }
async function row(i) { await ensure('dialog-rebase-row-' + i); await click('dialog-rebase-row-' + i); assert.equal(state.dialog.rebase_plan.selected, i); }
async function recover(kind) { await click('operation-' + kind); await until(s => s.dialog?.rebase_preview_loaded || s.dialog?.error); await stable(); }
function rebaseFixture(name, names) { const f = fixture(name); f.git(['branch', 'main']); for (const n of names) { f.write('検索/' + n + '.txt', n + '\n'); f.commit('PRIVATE_SOURCE ' + n); } return f; }
async function rebaseStops() {
  const f = rebaseFixture('PRIVATE_STOPS', ['A', 'B', 'C']);
  await open(f); await plan(); await action(1, 'edit');
  await row(0); await ensure('dialog-rebase-stop'); await click('dialog-rebase-stop'); assert.equal(state.dialog.rebase_plan_extras[0].stop_after, true);
  await row(2); await ensure('dialog-rebase-exec');
  await refused('rocket_type', { target: 'dialog-rebase-exec', text: 'echo PRIVATE_COMMAND' }, 'fixed planned rebase commands');
  await type('dialog-rebase-exec', 'false'); assert.equal(state.dialog.rebase_plan_extras[2].exec, 'false');
  await capture('stop-exec-plan'); await submit(); assert(state.dialog.rebase_plan.reviewed); await capture('stop-exec-review'); await submit();
  assert(active().operation_state.rebase); assert(latest().summary.includes('planned break'), JSON.stringify(latest()));
  await recover('continue'); assert.deepEqual(state.dialog.rebase_planned_stop, { kind: 'break' }); await capture('break-continue-review'); await submit();
  assert(active().operation_state.rebase); assert(latest().summary.includes('Stopped to edit'), JSON.stringify(latest()));
  f.write('検索/B.txt', 'B amended 中文\n'); await click('refresh'); await click('view-changes'); await click('stage-all');
  await recover('continue'); assert.equal(state.dialog.rebase_planned_stop.kind, 'edit'); await capture('edit-continue-review'); await submit();
  assert(active().operation_state.rebase); assert(latest().details.some(d => d.includes('[planned_command_stopped]')), JSON.stringify(latest()));
  await recover('continue'); assert.deepEqual(state.dialog.rebase_planned_stop, { kind: 'command', command: 'false' }); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().operation_state.idle);
  assert.equal(f.git(['log', '--reverse', '--format=%s', 'main..HEAD']), 'PRIVATE_SOURCE A\nPRIVATE_SOURCE B\nPRIVATE_SOURCE C');
  assert.equal(f.git(['show', 'HEAD~1:検索/B.txt']), 'B amended 中文'); assert(!fs.existsSync(path.join(f.repo, '.git/rocket-interactive-plan')));
  passed('Interactive plan with edit, a planned break and a failing planned command: arbitrary commands refused by automation, each stop reported and reviewed, edit amended and the plan completed');

  const d = rebaseFixture('PRIVATE_DISCARD', ['A', 'B']);
  await open(d); await plan(); await row(0); await ensure('dialog-rebase-stop'); await click('dialog-rebase-stop'); await submit(); await submit();
  assert(active().operation_state.rebase); d.write('検索/A.txt', 'PRIVATE_DISCARDED\n'); await click('refresh');
  await recover('skip'); assert.deepEqual(state.dialog.rebase_planned_stop, { kind: 'break' }); await capture('discard-and-continue-review'); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().operation_state.idle);
  assert.equal(fs.readFileSync(path.join(d.repo, '検索/A.txt'), 'utf8'), 'A\n'); assert.equal(d.git(['rev-list', '--count', 'main..HEAD']), '2');
  passed('At a planned break, Skip discards tracked edits made since the stop and keeps both replayed commits');
}

async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n'); report.endpoint = 'http://127.0.0.1:' + port + '/mcp';
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    const cases = { sequences, recovery: recoverySources, rebase: rebaseStops };
    for (const [name, test] of Object.entries(cases)) if (!only || only === name) await test();
    for (const f of fixtures) assert.equal(hash(path.join(f.repo, '.git/config')), f.config, 'Repository config changed');
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(f => /^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f => fs.readFileSync(path.join(root, n, f), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE_', '一/', '削除', '検索']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    const expect = {
      sequences: { ops: ['cherry-pick', 'cherry-pick-continue', 'cherry-pick-skip', 'cherry-pick-abort', 'revert', 'revert-skip'], steps: ['read_sequence'], codes: ['conflicts_remain', 'sequence_merge'] },
      recovery: { ops: ['reflog', 'recover-branch'], steps: ['read_reflog', 'find_deleted_branches', 'read_retained_refs', 'scan_lost_objects'], codes: [] },
      rebase: { ops: ['interactive-rebase', 'rebase-continue', 'rebase-skip'], steps: [], codes: ['planned_command_stopped'] },
    };
    for (const [name, e] of Object.entries(expect)) {
      if (only && only !== name) continue;
      for (const op of e.ops) { const starts = records.filter(r => r.event === 'request_start' && r.operation === op); assert(starts.length, 'Missing ' + op); for (const start of starts) assert(records.some(r => r.event === 'request_end' && r.session === start.session && r.request === start.request), 'Unpaired ' + op); }
      for (const step of e.steps) assert(payload.includes('"' + step + '"'), 'Missing step ' + step);
      for (const code of e.codes) assert(records.some(r => r.failure_code === code), 'Missing failure ' + code);
    }
    report.log_records = records.length; passed('Paired diagnostic spans, new steps and fixed failure codes, zero dropped events, no private paths or messages'); report.passed = true;
  } catch (error) { if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync('work/sequences-recovery-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
run().catch(e => { console.error(e); process.exitCode = 1; });
