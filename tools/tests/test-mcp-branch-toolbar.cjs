// Affected-feature acceptance: native Branch menu and virtual history row targeting.
// All Git operations use disposable local repositories and isolated configuration.
const fs = require('node:fs');
const path = require('node:path');
const cp = require('node:child_process');
const crypto = require('node:crypto');
const net = require('node:net');
const assert = require('node:assert/strict');
const {RocketHttpClient} = require('../mcp/mcp-http-client.cjs');
const {gitRelay} = require('./git-relay.cjs');

const baseline = process.argv.includes('--baseline');
const root = path.resolve('work/mcp-fixtures/branch-toolbar-' + crypto.randomUUID());
const profile = path.join(root, 'http-profile');
const exe = path.resolve(baseline ? 'tools/releases/0.81.1/rocket-mcp.exe' : 'target/release/rocket-mcp.exe');
fs.mkdirSync(root, {recursive: true});
fs.mkdirSync(path.join(root, 'template'));
fs.writeFileSync(path.join(root, 'global.config'), '# isolated synthetic fixture\n');
fs.writeFileSync(path.join(root, 'system.config'), '');
fs.writeFileSync(path.join(root, 'rocket-private-state'), 'synthetic-content-only-v1\n');
const env = {...process.env};
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, {
  GIT_CONFIG_GLOBAL: path.join(root, 'global.config'), GIT_CONFIG_SYSTEM: path.join(root, 'system.config'),
  GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0',
  GIT_OPTIONAL_LOCKS: '0', GIT_TEMPLATE_DIR: path.join(root, 'template'),
});
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
function git(repo, args, input) {
  return cp.execFileSync('git', ['-C', repo, ...args], {env, input, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe']}).trimEnd();
}
function write(repo, file, text) {
  const dest = path.join(repo, file); fs.mkdirSync(path.dirname(dest), {recursive: true}); fs.writeFileSync(dest, text);
}
function init(name) {
  const repo = path.join(root, name + ' 文'), branch = 'rocket-ui-test/' + name;
  fs.mkdirSync(repo); git(repo, ['init', '-b', branch]);
  for (const [key, value] of [['user.name', 'Branch Fixture'], ['user.email', 'fixture@example.invalid'], ['core.autocrlf', 'false'], ['commit.gpgsign', 'false']]) git(repo, ['config', key, value]);
  write(repo, '.git/rocket-ui-fixture', branch);
  return {repo, branch, base: branch + '-base'};
}
const history = init('history');
let stream = '';
for (let i = 0; i < 160; i++) {
  const subject = `History ${String(i).padStart(3, '0')} · 日本語`, body = `line ${i}\n`;
  stream += `commit refs/heads/${history.branch}\ncommitter Branch Fixture <fixture@example.invalid> ${1700000000 + i} +0000\ndata ${Buffer.byteLength(subject)}\n${subject}\nM 100644 inline example.txt\ndata ${Buffer.byteLength(body)}\n${body}\n`;
}
git(history.repo, ['fast-import', '--quiet'], stream); git(history.repo, ['reset', '--hard', 'HEAD']);

function commit(repo, message) { git(repo, ['add', '.']); git(repo, ['commit', '-m', message]); return git(repo, ['rev-parse', 'HEAD']); }
function workflow(name, conflict = false, diverged = true) {
  const f = init(name);
  write(f.repo, '共同 文.txt', 'original\n'); write(f.repo, 'draft.txt', 'original draft\n'); commit(f.repo, 'base'); git(f.repo, ['branch', f.base]);
  write(f.repo, conflict ? '共同 文.txt' : 'feature.txt', 'feature\n'); f.tip = commit(f.repo, 'feature');
  if (diverged) { git(f.repo, ['switch', f.base]); write(f.repo, conflict ? '共同 文.txt' : 'base.txt', 'base advance\n'); commit(f.repo, 'base advance'); git(f.repo, ['switch', f.branch]); }
  f.baseTip = git(f.repo, ['rev-parse', f.base]); return f;
}
const fixtures = baseline ? {} : {
  sync: workflow('sync'), stale: workflow('stale'), cancel: workflow('cancel'), conflict: workflow('conflict', true),
  finish: workflow('finish', false, false), fast: workflow('fast', false, false),
};
if (!baseline) {
  const side = history.branch + '-side'; git(history.repo, ['branch', side]);
  write(history.repo, 'main.txt', 'main\n'); commit(history.repo, 'main advance');
  git(history.repo, ['switch', side]); write(history.repo, 'side.txt', 'side\n'); commit(history.repo, 'side advance');
  git(history.repo, ['switch', history.branch]); git(history.repo, ['merge', '--no-ff', side, '-m', 'Merge side']);
  history.merge = git(history.repo, ['rev-parse', 'HEAD']);
}
for (const f of Object.values(fixtures).filter(f => !['finish', 'fast'].some(name => f.branch.endsWith('/' + name)))) {
  write(f.repo, 'draft.txt', 'staged PRIVATE_BRANCH_PAYLOAD\n'); git(f.repo, ['add', '--', 'draft.txt']);
  write(f.repo, 'draft.txt', 'working PRIVATE_BRANCH_PAYLOAD\n'); write(f.repo, 'keep 日本語.txt', 'untracked PRIVATE_BRANCH_PAYLOAD\n');
}
const fingerprint = repo => ({
  head: git(repo, ['rev-parse', 'HEAD']), branch: git(repo, ['branch', '--show-current']),
  index: git(repo, ['ls-files', '--stage', '-z']), status: git(repo, ['status', '--porcelain=v2', '-z']),
  draft: fs.existsSync(path.join(repo, 'draft.txt')) ? hash(path.join(repo, 'draft.txt')) : null,
});
const globalHash = hash(path.join(root, 'global.config'));
const relay = baseline ? null : gitRelay(root, env, 'tools/helpers/file-history-preview/git-relay.rs'), gate = relay?.gate, marker = relay?.marker;

let service, client, state;
const report = {passed: false, baseline, root, cases: [], exe_hash: hash(exe)};
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const control = id => state.controls?.find(c => c.id === id);
const rows = () => state.controls.filter(c => /^commit-[0-9a-f]{40,64}$/.test(c.id)).sort((a,b) => a.rect[1] - b.rect[1]);
const tab = () => state.tabs.find(t => t.id === state.active_tab);
const pass = label => { report.cases.push(label); console.log('PASS ' + label); };
async function until(predicate, label = 'state', timeout = 60000) {
  const deadline = performance.now() + timeout;
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return;
    assert(performance.now() < deadline, label + ' timeout'); await delay(60);
  }
}
async function settled() {
  await until(s => !s.busy && !s.input_pending && !s.dialog?.workflow?.loading && !s.dialog?.workflow?.applying && !s.tabs?.some(t => t.loading), 'settled');
}
async function stable() {
  let previous = '', same = 0;
  await until(s => { const next = JSON.stringify(s.controls); same = next === previous ? same + 1 : 0; previous = next; return same >= 2; }, 'stable layout');
}
async function click(target, extra = {}) {
  await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_click', {target, expected_tab: state.active_tab, ...extra}); await settled();
}
async function type(target, text) {
  await stable(); await client.tool('rocket_type', {target, text, expected_tab: state.active_tab}); await settled();
}
async function resize(width, height) {
  await client.tool('rocket_resize', {width, height});
  await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending, 'resize');
  await settled(); await stable();
}
async function scroll(target, direction, pixels = 600) {
  await client.tool('rocket_scroll', {target, direction, pixels, expected_tab: state.active_tab}); await settled(); await stable();
}
async function escape() { await client.tool('rocket_escape'); await settled(); await stable(); }
async function rejectClick(target, extra = {}) {
  await stable(); const selected = tab().selected_commit;
  await assert.rejects(client.tool('rocket_click', {target, expected_tab: state.active_tab, ...extra}), /Commit row is not visible.*history-scroll/);
  await settled(); assert.equal(tab().selected_commit, selected); assert.equal(state.dialog, null);
}
async function capture(name) {
  await stable(); const result = await client.request('tools/call', {name: 'rocket_capture', arguments: {}});
  assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image);
  fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64'));
  fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2));
}
async function open(fixture) {
  if (state?.dialog) await click(control('dialog-cancel') ? 'dialog-cancel' : 'dialog-close');
  await settled(); await client.tool('rocket_open', {path: fixture.repo}); await settled(); await stable();
}
async function start() {
  cp.execFileSync(exe, ['--init-http', profile, root], {env, windowsHide: true, stdio: 'pipe'});
  const probe = net.createServer(); await new Promise(resolve => probe.listen(0, '127.0.0.1', resolve));
  const port = probe.address().port; await new Promise(resolve => probe.close(resolve));
  fs.writeFileSync(path.join(profile, 'settings'), `ROCKET-MCP-1\n${port}\n${root}\n`);
  const log = fs.openSync(path.join(root, 'service.log'), 'w');
  service = cp.spawn(exe, ['--http-profile', profile], {env: relay?.env ?? env, windowsHide: true, stdio: ['ignore', 'ignore', log]}); fs.closeSync(log);
  client = new RocketHttpClient(profile);
  for (let i = 0;; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i >= 120) throw e; await delay(100); } }
  report.version = (await client.initialize()).serverInfo.version;
  await client.tool('rocket_open', {path: history.repo}); await settled(); relay?.check(state);
}
async function checkBaseline() {
  await resize(780, 620); await click('view-commits'); await stable(); await capture('baseline-before');
  const target = rows().at(-1).id, selected = tab().selected_commit;
  await click(target); await capture('baseline-after');
  report.offscreen_click = {target, before: selected, after: tab().selected_commit, accepted: true};
  assert.notEqual(tab().selected_commit, target.slice(7), 'Baseline did not reproduce; inspect row bounds');
  pass('Reproduced: MCP accepted an overscan row click, but the native list did not select that commit');
}
function checkBounds() {
  const viewport = control('history-scroll'); assert(viewport?.enabled); assert(rows().length >= 3);
  for (const c of state.controls.filter(c => /^commit-(?:expand-)?[0-9a-f]{40,64}$/.test(c.id))) {
    assert(c.rect[2] > c.rect[0] && c.rect[3] > c.rect[1], 'Positive visible bounds');
    assert(c.rect[0] >= viewport.rect[0] && c.rect[1] >= viewport.rect[1] && c.rect[2] <= viewport.rect[2] && c.rect[3] <= viewport.rect[3], JSON.stringify({c, viewport}));
  }
}
async function checkHistory() {
  const before = fingerprint(history.repo);
  await resize(1440, 1000); await click('view-commits'); await stable(); checkBounds();
  await click('commit-' + history.merge, {button: 'right'}); await click('commit-menu-fold');
  const expand = 'commit-expand-' + history.merge; assert(control(expand)); checkBounds();
  await scroll('history-scroll', 'down', 17); checkBounds();
  assert(control(expand), 'Merge expansion pill is partly visible at this offset');
  await click(expand); assert(!control(expand));
  await scroll('history-scroll', 'up', 2000);
  pass('Partly clipped merge-expansion pill remains inside the history viewport and expands through native input');
  const wideBottom = rows().at(-1).id;
  await resize(780, 620); checkBounds(); assert(!control(wideBottom));
  await rejectClick(wideBottom); await rejectClick(wideBottom, {button: 'right'}); await rejectClick(wideBottom, {modifiers: 'ctrl'});
  pass('Resize hides old rows; left/right/modifier clicks reject stale targets without changing selection');
  const edge = rows().at(-1); report.partial_row_height = edge.rect[3] - edge.rect[1];
  await click(edge.id); assert.equal(tab().selected_commit, edge.id.slice(7)); await capture('history-compact-edge');
  pass('Partially visible compact row selects the exact commit through native input');
  const first = rows()[0].id; await scroll('history-scroll', 'down', 900); checkBounds(); assert(!control(first)); await rejectClick(first);
  const older = rows()[2].id; await click(older); assert.equal(tab().selected_commit, older.slice(7)); await capture('history-scrolled');
  pass('History wheel scrolling reveals older rows; hidden IDs reject with scroll/filter guidance');
  await click('toolbar-console'); await stable(); checkBounds(); await capture('history-compact-drawer');
  await click(rows()[1].id); assert.equal(state.console_visible, true);
  await click('toolbar-console'); await scroll('history-scroll', 'up', 2000);
  await type('commit-filter', 'History 040'); await until(() => rows().length === 1, 'history filter');
  await click(rows()[0].id); assert.equal(tab().selected_commit, rows()[0].id.slice(7));
  await type('commit-filter', ''); await settled(); await stable();
  assert.deepEqual(fingerprint(history.repo), before);
  pass('Compact drawer and history filter preserve exact row selection and repository contents');
}
async function branchMenu() { await click('toolbar-branch'); await until(() => !!control('toolbar-branch-sync')); }
async function review(f, mode, save = false, fast = false) {
  await open(f); await branchMenu(); await click(mode === 6 ? 'toolbar-branch-sync' : 'toolbar-branch-finish');
  assert.equal(state.dialog.workflow.mode, mode);
  await type('dialog-first', f.base); if (save) await click('workflow-save-changes'); if (fast) await click('workflow-finish-ff');
  await click('workflow-review');
}
const activity = () => state.activities.at(-1);
async function checkToolbar() {
  await open(history); await resize(1440, 1000); await branchMenu(); await capture('branch-menu-desktop');
  await escape(); assert(!control('toolbar-branch-new')); assert.equal(state.dialog, null);
  await resize(780, 620); await branchMenu();
  for (const id of ['toolbar-branch-new', 'toolbar-branch-sync', 'toolbar-branch-finish']) {
    const c = control(id); assert(c?.enabled); assert(c.rect[0] >= 0 && c.rect[1] >= 0 && c.rect[2] <= 780 && c.rect[3] <= 620);
  }
  await capture('branch-menu-compact'); await click('toolbar-branch-new');
  await type('dialog-first', history.branch + '-new'); await click('dialog-cancel');
  assert.equal(git(history.repo, ['branch', '--list', history.branch + '-new']), '');
  await branchMenu(); await click('toolbar-branch-new'); assert.equal(state.dialog.kind, 'new-branch');
  await type('dialog-first', 'outside-fixture');
  await assert.rejects(client.tool('rocket_click', {target: 'dialog-submit', expected_tab: state.active_tab}), /marked branch namespace/);
  assert.equal(git(history.repo, ['branch', '--list', 'outside-fixture']), '');
  await type('dialog-first', history.branch + '-new'); await click('dialog-submit');
  assert.equal(git(history.repo, ['rev-parse', history.branch + '-new']), git(history.repo, ['rev-parse', history.branch]));
  assert.equal(git(history.repo, ['branch', '--show-current']), history.branch);
  pass('Desktop/compact dropdown, Escape, reviewed New Branch and cancellation preserve checkout');

  const f = fixtures.sync, before = fingerprint(f.repo);
  await review(f, 6); assert(state.dialog.error); assert(!state.dialog.workflow.preview);
  pass('Sync opened from toolbar refuses dirty checkout until saving is explicitly selected');
  await click('workflow-save-changes'); await click('workflow-review'); assert.equal(state.dialog.workflow.preview.saved_files, 1);
  await capture('sync-preview-compact'); await click('dialog-cancel'); assert.deepEqual(fingerprint(f.repo), before);
  await review(f, 6, true); await click('dialog-submit'); assert.equal(activity().command_succeeded, true);
  assert.equal(git(f.repo, ['show', ':draft.txt']), 'staged PRIVATE_BRANCH_PAYLOAD');
  assert.equal(fs.readFileSync(path.join(f.repo, 'draft.txt'), 'utf8'), 'working PRIVATE_BRANCH_PAYLOAD\n');
  assert.equal(fs.readFileSync(path.join(f.repo, 'keep 日本語.txt'), 'utf8'), 'untracked PRIVATE_BRANCH_PAYLOAD\n');
  assert(git(f.repo, ['stash', 'list'])); assert(activity().details.some(s => /recovery stash retained/i.test(s)));
  pass('Sync review cancellation preserves state; confirmed Sync restores staged/unstaged edits and retains recovery stash');

  const stale = fixtures.stale; await review(stale, 6, true); write(stale.repo, 'draft.txt', 'external PRIVATE_BRANCH_PAYLOAD\n');
  await click('dialog-submit'); assert.equal(activity().command_succeeded, false); assert.equal(git(stale.repo, ['rev-parse', 'HEAD']), stale.tip);
  assert.equal(git(stale.repo, ['stash', 'list']), ''); assert.equal(fs.readFileSync(path.join(stale.repo, 'draft.txt'), 'utf8'), 'external PRIVATE_BRANCH_PAYLOAD\n');
  await capture('sync-external-edit'); pass('External edit after review refuses Sync and preserves the new bytes');

  const cancel = fixtures.cancel; await review(cancel, 6, true); await stable(); fs.writeFileSync(gate, 'rebase');
  await client.tool('rocket_click', {target: 'dialog-submit', expected_tab: state.active_tab});
  await until(() => fs.existsSync(marker) && control('workflow-stop')?.enabled, 'gated rebase');
  const pids = fs.readFileSync(marker, 'utf8').trim().split('\n').map(Number), started = performance.now();
  await client.tool('rocket_click', {target: 'workflow-stop', expected_tab: state.active_tab});
  await until(s => !s.input_pending && (s.dialog?.workflow?.stopping || !s.busy), 'stop feedback');
  report.cancel_feedback_ms = Math.round(performance.now() - started);
  await until(() => pids.every(pid => { try { process.kill(pid, 0); return false; } catch { return true; } }), 'owned process exit');
  report.cancel_process_stop_ms = Math.round(performance.now() - started); fs.unlinkSync(gate); await settled();
  assert.equal(activity().command_succeeded, false); assert(activity().details.some(s => /stash retained/i.test(s)));
  assert.equal(git(cancel.repo, ['show', 'stash@{0}:draft.txt']), 'working PRIVATE_BRANCH_PAYLOAD');
  await capture('sync-cancelled'); pass('Cancel stops the owned Sync process and reports the retained recovery stash');

  const conflict = fixtures.conflict; await review(conflict, 6, true); await click('dialog-submit');
  assert.equal(activity().command_succeeded, false); assert(fs.existsSync(path.join(conflict.repo, '.git/rebase-merge')));
  assert(git(conflict.repo, ['stash', 'list'])); await capture('sync-conflict'); await click('dialog-cancel');
  await branchMenu(); assert.equal(control('toolbar-branch-sync').enabled, false); assert.equal(control('toolbar-branch-finish').enabled, false);
  await capture('branch-menu-recovery'); await escape(); await click('view-changes'); await capture('recovery-compact');
  await click('operation-abort'); await until(s => s.dialog?.rebase_preview_loaded, 'abort review'); await click('dialog-submit');
  assert.equal(git(conflict.repo, ['rev-parse', 'HEAD']), conflict.tip); assert.equal(git(conflict.repo, ['branch', '--show-current']), conflict.branch);
  git(conflict.repo, ['stash', 'apply', '--index', 'stash@{0}']);
  assert.equal(git(conflict.repo, ['show', ':draft.txt']), 'staged PRIVATE_BRANCH_PAYLOAD');
  assert.equal(fs.readFileSync(path.join(conflict.repo, 'draft.txt'), 'utf8'), 'working PRIVATE_BRANCH_PAYLOAD\n');
  assert(git(conflict.repo, ['stash', 'list']));
  pass('Rebase conflict disables new integrations; reviewed UI Abort restores checkout and retained stash restores edits');

  for (const [f, fast] of [[fixtures.finish, false], [fixtures.fast, true]]) {
    await review(f, 7, false, fast); assert.equal(state.dialog.workflow.preview.allow_fast_forward, fast);
    const before = fingerprint(f.repo); await capture(fast ? 'finish-fast-preview' : 'finish-merge-preview');
    await click('dialog-cancel'); assert.deepEqual(fingerprint(f.repo), before);
    await review(f, 7, false, fast); await click('dialog-submit'); assert.equal(activity().command_succeeded, true);
    assert.equal(git(f.repo, ['branch', '--show-current']), f.base); assert.equal(git(f.repo, ['rev-parse', f.branch]), f.tip);
    if (fast) assert.equal(git(f.repo, ['rev-parse', 'HEAD']), f.tip);
    else { assert.equal(git(f.repo, ['rev-parse', 'HEAD^1']), f.baseTip); assert.equal(git(f.repo, ['rev-parse', 'HEAD^2']), f.tip); }
    pass('Finish ' + (fast ? 'opt-in fast-forward' : 'default merge commit') + ' uses reviewed base and keeps source branch');
  }
}
async function checkLogs() {
  await until(s => !s.busy && !s.input_pending); report.log_directory = state.log_directory;
  const records = fs.readdirSync(state.log_directory).filter(n => n.endsWith('.jsonl')).flatMap(n => fs.readFileSync(path.join(state.log_directory, n), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse));
  assert(records.length > 0); assert(records.every(r => !r.dropped_before)); assert.equal(state.dropped_events, 0);
  assert(!JSON.stringify(records).includes('PRIVATE_BRANCH_PAYLOAD')); assert.equal(hash(path.join(root, 'global.config')), globalHash);
  assert(records.some(r => /mcp/.test(r.operation || r.event)));
  report.log_records = records.length;
  pass('Diagnostics record MCP/backend steps with zero dropped events and no fixture file payloads; isolated Git config unchanged');
}
async function run() {
  try {
    await start();
    if (baseline) await checkBaseline();
    else { assert.equal(report.version, fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1]); await checkHistory(); await checkToolbar(); await checkLogs(); }
    report.passed = true;
  } catch (error) {
    report.error = error.stack; console.error(error.stack); process.exitCode = 1;
    if (client) try { await capture('failure'); } catch {}
  } finally {
    if (fs.existsSync(gate)) fs.unlinkSync(gate);
    if (client) {
      try {
        if (state?.dialog) await click(control('dialog-cancel') ? 'dialog-cancel' : 'dialog-close');
        await client.tool('rocket_close');
        await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit');
        if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', {target: 'dialog-submit', expected_tab: state.active_tab}); await until(s => ['closed', 'not_started'].includes(s.window)); }
        await client.raw('POST', '/shutdown');
      } catch (error) { report.cleanup_error = error.message; }
      await client.finish();
    }
    if (service) {
      for (let i = 0; i < 100 && service.exitCode === null; i++) await delay(100);
      if (service.exitCode === null) { report.forced_cleanup = true; cp.spawnSync('taskkill', ['/PID', String(service.pid), '/T', '/F'], {windowsHide: true}); }
    }
    const token = path.join(profile, 'access-token'); if (fs.existsSync(token)) fs.unlinkSync(token);
    report.cleanup = {service_exit: service?.exitCode, token_removed: !fs.existsSync(token)};
    if (report.cleanup_error || report.forced_cleanup || service?.exitCode !== 0) { report.passed = false; process.exitCode = 1; }
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    const results = path.resolve('work/results/branch-toolbar-0.82'); fs.mkdirSync(results, {recursive: true});
    fs.writeFileSync(path.join(results, baseline ? 'baseline.json' : 'native-latest.json'), JSON.stringify(report, null, 2));
    console.log('Evidence: ' + root);
  }
}
run();
