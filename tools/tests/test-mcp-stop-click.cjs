'use strict';
// Local-only first-click acceptance: real Git waits on an owned ref lock.
// No hooks, replacement Git, credential helpers or repeated Stop clicks.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { create } = require('./mcp-github-fixture.cjs');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const workspace = path.resolve(__dirname, '../..');
const bundle = path.resolve(process.env.ROCKET_STOP_BUILD || path.join(workspace, 'target/release'));
const exe = path.join(bundle, 'rocket-mcp.exe');
const git = path.join(bundle, 'git/2.56.0.windows.4/cmd/git.exe');
const evidence = path.resolve(process.env.ROCKET_STOP_EVIDENCE || path.join(workspace, 'tools/maintenance/stop-first-click-20261031'));
assert(evidence.startsWith(path.join(workspace, 'tools/maintenance') + path.sep), 'Evidence must remain inside tools/maintenance');
const preflight = process.argv.includes('--lock-preflight');
const timing = process.argv.includes('--timing');
const externalOnly = process.argv.includes('--external-refresh-only');
const fetchPullOnly = process.argv.includes('--fetch-pull-only');
const singleRefresh = process.argv.includes('--expect-single-refresh');
const pullOnly = process.argv.includes('--pull-only');
const preparationOnly = process.argv.includes('--pull-preparation');
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const f = create('local', { gitExecutable: git });
const root = f.root, profile = path.join(root, 'stop-http'), output = path.join(root, 'stop-result.json');
const report = { passed: false, root, preflight, cases: [], executable_sha256: hash(exe), git_sha256: hash(git), runner_sha256: hash(__filename) };
fs.mkdirSync(evidence, { recursive: true });
fs.writeFileSync(path.join(evidence, preflight ? 'lock-preflight-latest.json' : 'native-latest.json'), JSON.stringify({ result: output }));
const save = () => fs.writeFileSync(output, JSON.stringify(report, null, 2));
let service, client, state, cursor = 0, held;
const records = [];
const ctl = id => state?.controls?.find(c => c.id === id && c.enabled);
const settled = s => !s.busy && !s.input_pending && !s.write && !s.dialog?.pull?.running && !s.tabs?.some(t => t.loading || t.search?.running);
async function until(test, label, timeout = 90000) {
  report.waiting_for = label; save();
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    assert(!state.startup_error, state.startup_error);
    if (test(state)) return;
    assert(performance.now() - start < timeout, label + ' timed out');
    await wait(20);
  }
}
async function click(id, finish = true) {
  await until(s => !s.input_pending && ctl(id), 'control ' + id);
  await client.tool('rocket_click', { target: id, expected_tab: state.active_tab });
  if (finish) await until(settled, 'settled after ' + id);
}
async function closeDialog() {
  if (state?.dialog) { await click(ctl('dialog-close') ? 'dialog-close' : 'dialog-cancel'); await until(s => !s.dialog, 'closed dialog'); }
}
async function logs() {
  for (;;) {
    const batch = await client.tool('rocket_logs', { after_seq: cursor, limit: 1000 });
    assert(!batch.retention_gap); cursor = batch.next_seq; records.push(...batch.records);
    if (!batch.more) return;
  }
}
async function capture(label) {
  const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!result.isError);
  fs.writeFileSync(path.join(root, label + '.png'), Buffer.from(result.content.find(c => c.type === 'image').data, 'base64'));
  fs.writeFileSync(path.join(root, label + '.json'), JSON.stringify(state, null, 2));
}
function lock(op) {
  assert(!held);
  const base = op === 'push' ? f.remote : path.join(f.client, '.git');
  const ref = op === 'push' ? f.ref : f.tracking;
  const file = path.join(base, ref + '.lock');
  assert(file.startsWith(root + path.sep));
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const contents = 'Rocket owned local Stop fixture ' + crypto.randomUUID() + '\n';
  fs.writeFileSync(file, contents, { flag: 'wx' });
  held = { file, contents };
}
function unlock() {
  if (!held) return;
  assert.equal(fs.readFileSync(held.file, 'utf8'), held.contents, 'Existing fixture lock was changed');
  fs.unlinkSync(held.file); held = null;
}
function prepare() {
  f.run('setup');
  f.config(f.client, 'core.askPass', '');
  f.git(['config', '--unset-all', 'http.sslBackend']);
  for (const key of Object.keys(f.env)) if (/^(GIT_ASKPASS|SSH_ASKPASS|GIT_SSH|GIT_SSH_COMMAND|GIT_SSH_VARIANT|GIT_PROXY_COMMAND|GIT_EXEC_PATH)$/i.test(key)) delete f.env[key];
  f.env.GIT_ALLOW_PROTOCOL = 'file';
  f.config(f.client, 'core.filesRefLockTimeout', preflight ? '2500' : '60000');
  f.git(['config', 'core.filesRefLockTimeout', preflight ? '2500' : '60000'], f.remote);
  f.r.remote_write_attempted = true; f.save();
  f.git(['push', '--set-upstream', 'origin', 'HEAD:' + f.ref]);
  f.r.expected_tip = f.r.base; f.save();
  f.git(['fetch', 'origin'], f.publisher);
  f.git(['switch', '-C', f.branch, f.tracking], f.publisher);
  f.r.expected_tip = f.commit(f.publisher, 'ROCKET_PRIVATE_incoming', 'Synthetic incoming change');
  f.git(['push', 'origin', 'HEAD:' + f.ref], f.publisher); f.save();
  f.assertRemote(f.r.expected_tip);
}
function preparePush() {
  f.git(['fetch', 'origin']);
  f.git(['merge', '--ff-only', f.tracking]);
  f.r.pending = f.commit(f.client, 'ROCKET_PRIVATE_pending', 'Synthetic outgoing change');
  f.save();
}
function lockPreflight(op) {
  lock(op);
  const start = performance.now();
  const args = op === 'push' ? ['push', '--porcelain', 'origin', 'HEAD:' + f.ref] : ['fetch', '--atomic', 'origin'];
  const result = cp.spawnSync(git, ['-C', f.client, ...args], { env: f.env, windowsHide: true, encoding: 'utf8', timeout: 15000 });
  const elapsed = Math.round(performance.now() - start);
  report.lock_probe = { operation: op, elapsed_ms: elapsed, exit: result.status, error: result.error?.code, stdout: result.stdout, stderr: result.stderr }; save();
  assert(!result.error && result.status !== 0, 'The owned lock must block the real Git update');
  assert.match(result.stderr + result.stdout, /cannot lock ref|unable to update local ref/i);
  assert(elapsed >= 2000 && elapsed < 15000, 'Git must wait for its configured lock timeout');
  unlock(); f.assertRemote(f.r.expected_tip);
  report.cases.push({ operation: op, lock_timeout_verified_ms: elapsed }); save();
}
async function start() {
  const storage = path.join(root, 'rocket-ui-state'); fs.mkdirSync(storage, { recursive: true });
  fs.writeFileSync(path.join(storage, 'git-selection'), 'ROCKET-GIT-2\nmanaged');
  cp.execFileSync(exe, ['--init-http', profile, root], { env: f.env, windowsHide: true, stdio: 'pipe', timeout: 30000 });
  const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
  const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
  fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
  const fd = fs.openSync(path.join(root, 'service.log'), 'w');
  service = cp.spawn(exe, ['--http-profile', profile], { env: f.env, windowsHide: true, stdio: ['ignore', 'ignore', fd] }); fs.closeSync(fd);
  client = new RocketHttpClient(profile);
  for (let n = 0;;n++) {
    try { if ((await client.raw('GET', '/health')).status === 200) break; } catch {}
    assert(n < 200 && service.exitCode === null, 'MCP service startup'); await wait(100);
  }
  await client.initialize(); await client.tool('rocket_open', { path: f.client });
  await until(s => s.tabs?.length && s.git_settings?.active_executable && settled(s), 'native repository ready');
  assert.equal(path.resolve(state.git_settings.active_executable.replace(/^\\\\\?\\/, '')).toLowerCase(), git.toLowerCase());
}
async function cancelCase(op, width, expanded) {
  report.stage = `${op}-${width}-${expanded ? 'activity' : 'status'}`; save();
  await closeDialog();
  await client.tool('rocket_resize', { width, height: width === 780 ? 620 : 960 });
  await until(s => s.viewport?.[0] === width && settled(s), 'resized native window');
  if (!state.console_visible) await click('toolbar-console');
  if (state.drawer_collapsed === expanded) await click(expanded ? 'drawer-expand' : 'drawer-collapse');
  await click('toolbar-' + op);
  await until(s => s.dialog?.kind === op && !s.input_pending, 'operation dialog');
  if (op === 'push') {
    await client.tool('rocket_type', { target: 'push-destination', text: f.branch, expected_tab: state.active_tab });
    await until(settled, 'push destination');
    if (state.dialog.flag) await click('push-upstream');
  }
  const snapshot = f.snapshot(), count = state.activities.length;
  lock(op); await logs(); const first = records.length;
  await click(op === 'pull' ? 'dialog-pull-fetch' : 'dialog-submit', false);
  const verb = op === 'pull' ? 'fetch' : op, deadline = Date.now() + 60000;
  let observed;
  for (;;) {
    state = await client.tool('rocket_state'); await logs();
    if (records.slice(first).some(r => r.event === 'git_start' && r.operation === verb)) {
      // git_start precedes spawn. Observe the owned app's actual child before
      // measuring process-stop latency; only numeric PIDs leave this query.
      const script = `@(Get-CimInstance Win32_Process -Filter 'ParentProcessId = ${service.pid}' | Where-Object { $_.Name -ieq 'git.exe' -and $_.CommandLine -match '(?:^|\\s)${verb}(?:\\s|$)' } | Select-Object -ExpandProperty ProcessId) | ConvertTo-Json -Compress`;
      const output = (process.env.ROCKET_PROCESS_PROBE
        ? cp.execFileSync(process.env.ROCKET_PROCESS_PROBE, [String(service.pid)], { encoding: 'utf8', windowsHide: true, timeout: 5000 })
        : cp.execFileSync(path.join(process.env.ProgramFiles, 'PowerShell/7/pwsh.exe'), ['-NoProfile', '-NonInteractive', '-Command', script], { encoding: 'utf8', windowsHide: true, timeout: 15000 })).trim();
      observed = output ? [JSON.parse(output)].flat() : [];
      if (observed.length) { state = await client.tool('rocket_state'); break; }
    }
    assert(Date.now() < deadline && state.activities.length === count, 'Git did not reach the held operation');
    await wait(10);
  }
  assert(state.write && !state.input_pending);
  const target = ctl('cancel-operation') ? 'cancel-operation' : 'status-cancel';
  assert(ctl(target), 'Visible Stop is required');
  const sample = { operation: op, width, expanded, target, click_count: 1, observed_git_pids: observed, rect: ctl(target).rect, progress_before: state.write.progress };
  report.active_case = sample; save();
  const pressed = performance.now();
  // Exactly one click, immediately after observing the real Git process. No stable-frame wait/retry.
  await client.tool('rocket_click', { target, expected_tab: state.active_tab });
  await until(s => s.write?.cancelling || (!s.write && s.activities.length === count + 1), 'first-click Stop activation', 2500);
  sample.feedback_ms = Math.round(performance.now() - pressed);
  if (state.write) assert.equal(state.write.cancel_enabled, false);
  await until(s => !s.write && !s.dialog?.pull?.running && s.activities.length === count + 1, 'cancelled operation completed', 15000);
  sample.operation_completed_ms = Math.round(performance.now() - pressed);
  sample.navigation_blocked_after_write = state.tabs.some(t => t.loading);
  sample.background_refresh_after_write = state.tabs.some(t => t.background_refresh);
  if (process.argv.includes('--expect-background')) assert(!sample.navigation_blocked_after_write, 'Follow-up reads must keep navigation available');
  assert(!state.activities.at(-1).command_succeeded);
  assert.match(JSON.stringify(state.activities.at(-1)), /cancel/i);
  await until(settled, 'background UI settled after cancellation');
  sample.settled_ms = Math.round(performance.now() - pressed);
  await logs();
  const ended = records.slice(first).filter(r => r.event === 'git_end' && r.operation === verb && r.outcome === 'cancelled');
  assert.equal(ended.length, 1, 'A single real Git process must report cancellation');
  assert(ended[0].cancel_to_stop_us > 0); sample.git_stop_us = ended[0].cancel_to_stop_us;
  const stoppedAt = records.indexOf(ended[0]);
  const afterStop = records.slice(stoppedAt + 1);
  sample.followup_requests = afterStop.filter(r => r.event === 'request_end').map(r => ({ operation: r.operation, elapsed_ms: r.elapsed_ms }));
  sample.followup_git_commands = afterStop.filter(r => r.event === 'git_end').map(r => ({
    operation: r.operation, elapsed_ms: r.elapsed_ms,
    phases_us: Object.fromEntries(['prepare_us', 'verify_us', 'spawn_us', 'contain_us', 'execute_us', 'drain_us'].filter(key => r[key] !== undefined).map(key => [key, r[key]])),
    unaccounted_ms: Math.max(0, r.elapsed_ms - ['prepare_us', 'verify_us', 'spawn_us', 'contain_us', 'execute_us', 'drain_us'].reduce((n, key) => n + (r[key] || 0), 0) / 1000),
  }));
  if (process.argv.includes('--expect-verify-timing')) {
    for (const command of sample.followup_git_commands) {
      assert(Number.isSafeInteger(command.phases_us.verify_us), 'Managed Git verification must remain visible before every launch');
      const sum = Object.values(command.phases_us).reduce((n, value) => n + value, 0);
      assert(sum <= command.elapsed_ms * 1000 + 999, 'Process timing phases must not overlap');
    }
  }
  if (singleRefresh && op !== 'push') {
    assert.equal(sample.followup_requests.filter(r => r.operation === 'auto_refresh').length, 1, 'One complete reconciliation snapshot');
    assert.equal(sample.followup_git_commands.length, 12, 'Ten snapshot reads plus two fresh selected-commit diff reads');
    assert(!afterStop.some(r => r.event === 'step_start' && r.operation === 'refresh' && ['status','branches','operation'].includes(r.step)), 'No discarded duplicate reconciliation reads');
  }
  assert.equal(records.slice(first).filter(r => r.event === 'request_start' && r.operation === (op === 'pull' ? 'pull-fetch' : op)).length, 1);
  assert.deepEqual(f.snapshot(), snapshot, 'Local checkout, index and file contents changed');
  f.assertRemote(f.r.expected_tip); unlock();
  sample.local_and_remote_unchanged = true;
  await capture(report.stage);
  // A completed Stop must not be delivered again or initiate any later write.
  const late = await client.request('tools/call', { name: 'rocket_click', arguments: { target, expected_tab: state.active_tab } });
  assert(late.isError, 'A missing/disabled Stop must be refused');
  sample.late_stop_refused = true;
  report.cases.push(sample); delete report.active_case; save();
  console.log('PASS ' + report.stage + ': first click, Git stopped, state preserved');
}
async function externalRefreshCase() {
  await closeDialog();
  const index = hash(path.join(f.client, '.git/index'));
  const head = f.git(['rev-parse', 'HEAD']);
  const include = path.join(root, 'ROCKET_PRIVATE_included.config');
  const file = path.join(f.client, 'ROCKET_PRIVATE_変更.rs');
  const rule = name => `[rocket "issuelink.${name}"]\npattern = ${name}-[0-9]+\nurl = https://example.invalid/{id}\n`;
  const count = state.tabs[0].refresh_count;
  fs.writeFileSync(include, rule('ONE'));
  f.git(['config', 'include.path', include]);
  fs.writeFileSync(file, 'External Unicode change\n');
  await until(s => s.tabs[0].refresh_count > count && s.tabs[0].issue_links?.rules === 1 && settled(s), 'external config and Unicode file refresh');
  const config = hash(path.join(f.client, '.git/config'));
  const next = state.tabs[0].refresh_count;
  fs.writeFileSync(include, rule('ONE') + rule('TWO'));
  fs.writeFileSync(file, 'Second external Unicode change\n');
  await until(s => s.tabs[0].refresh_count > next && s.tabs[0].issue_links?.rules === 2 && settled(s), 'included rules refreshed without touching repository config');
  assert.equal(hash(path.join(f.client, '.git/config')), config);
  await click('view-changes');
  const row = 'change-unstaged-' + Buffer.from('ROCKET_PRIVATE_変更.rs').toString('hex');
  // The model changes views before the next paint registers its native rows.
  await until(s => s.tabs[0].view === 'Changes' && ctl(row), 'external Unicode row rendered');
  assert(ctl(row), 'Unicode external edit must have a visible, enabled native row');
  await click(row);
  assert.equal(hash(path.join(f.client, '.git/index')), index);
  assert.equal(f.git(['rev-parse', 'HEAD']), head);
  assert.equal(fs.readFileSync(file, 'utf8'), 'Second external Unicode change\n');
  await capture('external-refresh');
  report.external_refresh = { passed: true, included_rules: 2, unicode_file: true, head_and_index_preserved: true };
  save(); console.log('PASS external refresh: included config, Unicode edits, HEAD/index preserved');
}

async function networkRefreshSuccessCases() {
  if (pullOnly) {
    // Match the preparation-only baseline after the compact Stop case.
    await closeDialog();
    await client.tool('rocket_resize', { width: 1480, height: 920 });
    await until(s => s.viewport?.[0] === 1480 && s.viewport?.[1] === 920 && settled(s), 'baseline-sized pull window');
    if (state.console_visible) await click('toolbar-console');
  }
  report.success_cases = [];
  for (const op of (pullOnly ? ['pull'] : ['fetch', 'pull'])) {
    await closeDialog();
    await click('toolbar-' + op);
    await logs(); const first = records.length;
    const original = f.snapshot(), count = state.activities.length, started = performance.now();
    await click(op === 'pull' ? 'dialog-pull-fetch' : 'dialog-submit', false);
    await until(s => settled(s) && s.activities.length > count, 'successful ' + op + ' refresh');
    const settledMs = Math.round(performance.now() - started);
    assert(state.activities.at(-1).command_succeeded, 'Successful local transport must be reported');
    assert.equal(f.git(['rev-parse', f.tracking]), f.r.expected_tip);
    assert.deepEqual(f.snapshot(), original, 'Fetch and pull review keep checkout/index/edits');
    if (op === 'pull') {
      assert(state.dialog?.pull?.fetched && state.dialog.pull.preview, 'Fresh pull review is ready');
      assert.equal(state.dialog.pull.preview.relation, 'FastForward');
    }
    await logs();
    const followup = records.slice(first);
    assert.equal(followup.filter(r => r.event === 'request_end' && r.operation === 'auto_refresh').length, 1);
    if (singleRefresh) assert(!followup.some(r => r.event === 'step_start' && r.operation === 'refresh' && ['status','branches','operation'].includes(r.step)));
    report.success_cases.push({ operation: op, passed: true, settled_ms: settledMs,
      refresh_ms: followup.find(r => r.event === 'request_end' && r.operation === 'auto_refresh').elapsed_ms,
      preparation: followup.filter(r => r.event === 'request_end' && ['pull-fetch','pull-preview'].includes(r.operation)).map(r => ({
        operation: r.operation, elapsed_ms: r.elapsed_ms,
        git_commands: followup.filter(g => g.event === 'git_end' && g.request === r.request).length,
        steps: followup.filter(g => g.event === 'step_end' && g.request === r.request).map(g => ({ step:g.step, operation:g.operation, elapsed_ms:g.elapsed_ms })),
      })) });
    await capture('success-' + op);
    if (op === 'pull') {
      if (process.argv.includes('--pull-review-guards')) await pullReviewGuards();
      // Exercise the affected consumer: a fresh review can still integrate the
      // pinned upstream. This is a synthetic fast-forward, preserving untracked data.
      const keep = fs.readFileSync(path.join(f.client, 'ROCKET_PRIVATE_untracked'));
      await click('dialog-submit');
      assert(state.activities.at(-1).command_succeeded);
      assert.equal(f.git(['rev-parse', 'HEAD']), f.r.expected_tip);
      assert.deepEqual(fs.readFileSync(path.join(f.client, 'ROCKET_PRIVATE_untracked')), keep);
      report.pull_applied = true;
    }
  }
  await closeDialog();
  const original = f.snapshot();
  const unavailable = path.join(root, 'ROCKET_PRIVATE_missing.git');
  fs.mkdirSync(unavailable); // In-scope directory, deliberately not a Git repository.
  f.config(f.client, 'remote.origin.url', unavailable);
  await click('refresh'); await click('toolbar-fetch');
  await click('dialog-submit');
  assert(!state.activities.at(-1).command_succeeded);
  assert.match(JSON.stringify(state.activities.at(-1)), /repository|fetch.*fail|unavailable/i);
  assert.deepEqual(f.snapshot(), original);
  assert(!state.write && !state.tabs[0].loading);
  report.transport_failure = { passed: true, local_state_preserved: true };
  await capture('fetch-error'); await closeDialog();
  f.config(f.client, 'remote.origin.url', f.remote); await click('refresh');
  save(); console.log('PASS successful fetch, pull review/apply and unavailable transport');
}
async function pullReviewGuards() {
  const before = f.snapshot();
  await logs(); const first = records.length;
  await click('dialog-pull-merge', false);
  await until(s => s.dialog?.pull?.running && !s.dialog.pull.fetching && !s.input_pending, 'live local pull preview');
  await click('dialog-pull-cancel');
  assert(!state.dialog.pull.preview && /cancel/i.test(state.dialog.pull.notice));
  assert.deepEqual(f.snapshot(), before);
  await click('dialog-preview-refresh'); assert(state.dialog.pull.preview);
  await logs(); const next = records.length;
  await click('dialog-pull-ff', false);
  const started = performance.now();
  for (;;) {
    await logs();
    const request = records.slice(next).find(r => r.event === 'request_start' && r.operation === 'pull-preview');
    if (request && records.slice(next).some(r => r.request === request.request && r.event === 'git_start' && r.operation === 'merge-base')) break;
    assert(performance.now() - started < 20000, 'Pull ancestry phase was not observed');
    await wait(15);
  }
  // Change effective configuration after initial validation, during ancestry work.
  f.config(f.client, 'remote.origin.url', f.publisher);
  try {
    await until(s => settled(s) && !s.dialog?.pull?.running, 'externally changed pull review refused');
    assert(!state.dialog.pull.preview && state.dialog.error);
  } finally {
    if (f.git(['config','--get','remote.origin.url']) === f.publisher)
      f.config(f.client, 'remote.origin.url', f.remote);
  }
  assert.deepEqual(f.snapshot(), before);
  await capture('pull-config-changed-during-review');
  await click('dialog-preview-refresh'); assert(state.dialog.pull.preview);
  await logs();
  assert(!records.slice(first).some(r => r.event === 'git_start' && r.operation === 'fetch'), 'Local review must never refetch');

  const tracked = f.git(['ls-files','-z']).split('\0').find(Boolean);
  const file = path.join(f.client, tracked), contents = fs.readFileSync(file);
  fs.appendFileSync(file, '\nROCKET_PRIVATE external edit\n');
  const dirty = f.snapshot();
  await click('dialog-submit');
  assert(!state.activities.at(-1).command_succeeded && state.dialog.error);
  assert.deepEqual(f.snapshot(), dirty);
  await capture('pull-dirty-after-review');
  await closeDialog();
  await click('toolbar-pull');
  await logs(); const dirtyFirst = records.length;
  await click('dialog-pull-fetch');
  assert(!state.dialog.pull.fetched && state.dialog.error);
  assert.deepEqual(f.snapshot(), dirty);
  await logs();
  assert(!records.slice(dirtyFirst).some(r => r.event === 'git_start' && r.operation === 'fetch'), 'Dirty checkout must fail before transport');
  fs.writeFileSync(file, contents);
  await click('dialog-pull-fetch'); assert(state.dialog.pull.preview);
  assert.deepEqual(f.snapshot(), before);
  report.pull_guards = { cancellation:true, config_changed_during_review:true, dirty_before_fetch:true, dirty_before_apply:true, no_implicit_refetch:true, local_state_preserved:true };
  save(); console.log('PASS pull preview cancellation, changed config during review and dirty-state guards');
}
async function verificationTimingView() {
  await closeDialog();
  await click('menu-view'); await click('activity-open-menu');
  await until(s => s.activity_history?.open && !s.activity_history.loading, 'diagnostics loaded');
  const row = state.activity_history.rows.find(r => r.operation === 'fetch');
  assert(row, 'The native diagnostics page must contain the measured fetch');
  await click('activity-row-' + row.index);
  await until(s => s.activity_history.selected?.request === row.request, 'fetch diagnostics selected');
  assert(state.activity_history.selected.records.some(r => Number.isSafeInteger(r.verify_us)), 'Verification phase survives native diagnostics projection');
  if (ctl('activity-tab-details')) await click('activity-tab-details');
  await click('activity-view-records');
  report.verification_views = [];
  for (const width of [1200, 780]) {
    await client.tool('rocket_resize', { width, height: 820 });
    await until(s => !s.input_pending && s.viewport?.[0] === width, 'diagnostics resize');
    if (ctl('activity-tab-details')) await click('activity-tab-details');
    await until(s => s.activity_history.records_view, 'phase timeline rendered');
    const label = 'verification-phases-' + width;
    await capture(label);
    report.verification_views.push({ width, capture: label + '.png', passed: true });
  }
  await click('activity-close');
  save(); console.log('PASS native verification timing projection and wide/compact captures');
}
async function stop() {
  if (!client || service.exitCode !== null) return;
  state = await client.tool('rocket_state');
  if (state.write) {
    report.cleanup_stop_required = true;
    const target = ctl('cancel-operation') ? 'cancel-operation' : 'status-cancel';
    await client.tool('rocket_click', { target, expected_tab: state.active_tab });
  }
  await until(settled, 'cleanup workers settled', 35000); await closeDialog(); await logs();
  await client.tool('rocket_close');
  await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit', 'close');
  if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'closed'); }
  await client.raw('POST', '/shutdown');
  const deadline = Date.now() + 10000; while (service.exitCode === null && Date.now() < deadline) await wait(25);
  assert.equal(service.exitCode, 0); await client.finish();
  fs.unlinkSync(path.join(profile, 'access-token'));
}
(async () => {
  try {
    prepare();
    if (preflight) { lockPreflight('fetch'); preparePush(); lockPreflight('push'); }
    else {
      await start();
      if (preparationOnly || pullOnly) fs.writeFileSync(path.join(f.client, 'ROCKET_PRIVATE_untracked'), 'Preserve this synthetic local edit\n');
      for (const op of (externalOnly || preparationOnly ? [] : pullOnly ? ['pull'] : fetchPullOnly ? ['pull', 'fetch'] : ['pull', 'fetch', 'push'])) {
        if (op === 'push') preparePush();
        if (op !== 'pull') fs.writeFileSync(path.join(f.client, 'ROCKET_PRIVATE_untracked'), 'Preserve this synthetic local edit\n');
        for (const width of [1460, 780]) for (const expanded of (timing ? [width === 780] : [false, true])) await cancelCase(op, width, expanded);
      }
      if (process.argv.includes('--success-refresh')) await networkRefreshSuccessCases();
      if (externalOnly || process.argv.includes('--external-refresh')) await externalRefreshCase();
      if (process.argv.includes('--expect-verify-timing')) await verificationTimingView();
      await stop();
      assert(records.length);
      assert(records.every(r => r.dropped_before === 0));
      const text = JSON.stringify(records);
      for (const value of ['ROCKET_PRIVATE', f.branch, f.remote, 'rocket@example.invalid', ...f.r.owned]) assert(!text.includes(value), 'Private fixture payload in diagnostics');
      for (const start of records.filter(r => ['request_start', 'step_start', 'git_start'].includes(r.event))) assert.equal(records.filter(r => r.session === start.session && r.id === start.id && r.event === start.event.replace('_start', '_end')).length, 1, 'Unpaired diagnostic span');
      report.logs = { records: records.length, dropped: 0, paired: true, privacy: true };
      fs.writeFileSync(path.join(root, 'diagnostics.json'), JSON.stringify(records, null, 2));
    }
    assert.equal(hash(exe), report.executable_sha256, 'Tested executable changed');
    report.passed = true;
  } catch (e) {
    report.failure = e.message; console.error(e.message);
    if (client) { try { state = await client.tool('rocket_state'); await capture('failure'); } catch {} }
  } finally {
    try { await stop(); } catch (e) { report.shutdown_error = e.message; report.passed = false; }
    if (!service || service.exitCode !== null) {
      try { unlock(); f.run('cleanup'); report.cleanup = f.r.cleanup; report.original_refs_unchanged = f.r.original_refs_and_default_head_unchanged; }
      catch (e) { report.cleanup_error = e.message; report.passed = false; }
    } else { report.cleanup = 'deferred-until-owned-service-stops'; report.passed = false; }
    if (records.length) fs.writeFileSync(path.join(root, 'diagnostics.json'), JSON.stringify(records, null, 2));
    save(); console.log(JSON.stringify({ passed: report.passed, cases: report.cases.length, result: output, cleanup: report.cleanup }));
  }
  if (!report.passed) process.exitCode = 1;
})();
