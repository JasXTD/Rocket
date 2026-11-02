// Focused native acceptance through authenticated localhost MCP. Disposable local
// repositories, isolated settings/config; no hosted remotes or real credentials.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const { gitRelay } = require('./git-relay.cjs');
const root = path.resolve('work/mcp-fixtures/guided-undo-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/guided-undo';
const FILE = 'src/資料 🚀 é.txt';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const remaining = process.argv.includes('--undo-failures-only');
const report = { passed: false, version, root, scope: remaining ? 'Undo failures/cancellation/logs' : 'all affected features', cases: [] };
fs.mkdirSync(root, { recursive: true });
fs.mkdirSync(path.join(root, 'empty-template'));
fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, {
  GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'),
  GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0',
  GIT_TEMPLATE_DIR: path.join(root, 'empty-template')
});
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const relay = gitRelay(root, env, 'tools/helpers/productivity/git-relay.rs'), { gate, marker } = relay;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = message => { report.cases.push(message); console.error('PASS ' + message); };
function fixture(name, unborn = false) {
  const repo = path.join(root, name), hooks = path.join(root, name + '-hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8' }).trimEnd();
  const write = (name, body) => { const p = path.join(repo, name); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Fixture'], ['user.email', 'rocket@example.invalid'],
    ['core.hooksPath', hooks], ['core.autocrlf', 'false'], ['commit.gpgsign', 'false']]) git(['config', key, value]);
  let base = null;
  if (!unborn) {
    write(FILE, 'baseline\n'); write('.gitignore', 'ignored/\n');
    git(['add', '.']); git(['commit', '-m', 'PRIVATE_BASE fixture']); base = git(['rev-parse', 'HEAD']);
  }
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo, hooks, git, write, base };
}
let client, service, state;
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < 45000, label + ' timed out: ' + JSON.stringify({ dialog: state.dialog, write: state.write, notice: state.notice }));
    await delay(80);
  }
}
const active = () => state.tabs.find(t => t.id === state.active_tab);
const control = id => state.controls.find(c => c.id === id);
const latest = () => state.activities.at(-1);
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running)); }
async function stable() {
  let last = '', count = 0;
  await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable control geometry');
}
async function click(target) {
  await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle();
}
async function type(target, text) {
  await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle();
}
async function scroll(target, direction = 'down', pixels = 650) {
  await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); await stable();
}
async function resize(width, height) {
  await client.tool('rocket_resize', { width, height });
  await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle();
}
async function capture(name) {
  await stable();
  const r = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!r.isError); const image = r.content.find(c => c.type === 'image'); assert(image);
  fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64'));
  fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2));
}
async function open(f) {
  if (state?.dialog) await click('dialog-cancel');
  // Removing a fixture lock schedules a watcher refresh. A refused open has
  // made no change; wait on that work instead of treating it as a feature error.
  for (let attempt = 0;; attempt++) {
    await settle();
    try { await client.tool('rocket_open', { path: f.repo }); break; }
    catch (error) {
      if (attempt >= 3 || !error.message.includes('Wait for the current UI operation and dismiss')) throw error;
      report.open_guard_waits = (report.open_guard_waits || 0) + 1;
    }
  }
  await settle(); relay.check(state); await resize(1480, 960);
  if (control('console-close')) await click('console-close');
  await click('view-changes');
}
async function refresh() { await click('refresh'); await settle(); }
async function commit(f, message = 'PRIVATE_COMMIT ordinary 中文') {
  await open(f); await click('stage-all'); await type('commit-message', message); await click('commit');
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().undo_available);
  f.tip = f.git(['rev-parse', 'HEAD']);
}
async function review() {
  await click('view-summary'); await click('summary-undo');
  assert.equal(state.dialog.kind, 'undo-commit');
  assert(state.dialog.undo.preview_loaded, JSON.stringify(state.dialog));
  return state.dialog.undo.preview.recovery_reference;
}
function localBytes(f) {
  return { index: hash(path.join(f.repo, '.git/index')), file: hash(path.join(f.repo, FILE)),
    config: hash(path.join(f.repo, '.git/config')), untracked: hash(path.join(f.repo, 'local 🚀')),
    ignored: hash(path.join(f.repo, 'ignored/cache')) };
}
function dirtyAfterCommit(f) { f.write(FILE, 'PRIVATE_UNSTAGED preserve\n'); f.write('local 🚀', 'PRIVATE_UNTRACKED preserve'); f.write('ignored/cache', 'PRIVATE_IGNORED preserve'); }
async function closeWindow() {
  if (state?.dialog && !state.write) await click('dialog-cancel');
  await client.tool('rocket_close');
  await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit' && !s.input_pending);
  if (state.dialog?.kind === 'quit') {
    assert.equal(state.state_error, null);
    await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab });
    await until(s => ['closed', 'not_started'].includes(s.window));
  }
}
async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe');
    report.executable_sha256 = hash(exe); report.relay_sha256 = hash(relay.exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer();
    await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
    const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    report.endpoint = 'http://127.0.0.1:' + port + '/mcp';
    const fd = fs.openSync(path.join(root, 'service.log'), 'w');
    service = cp.spawn(exe, ['--http-profile', profile], { env: relay.env, windowsHide: true, stdio: ['ignore', 'ignore', fd] }); fs.closeSync(fd);
    client = new RocketHttpClient(profile);
    for (let i = 0;; i++) {
      try { assert.equal((await client.raw('GET', '/health')).status, 200); break; }
      catch (e) { if (i === 100) throw e; await delay(100); }
    }
    assert.equal((await client.initialize()).serverInfo.version, version);

    if (!remaining) {
    const setup = fixture('PRIVATE_SETUP_資料');
    setup.git(['config', '--unset', 'user.name']); setup.git(['config', '--unset', 'user.email']);
    const configBefore = hash(path.join(setup.repo, '.git/config'));
    await open(setup); await click('view-summary'); await click('summary-setup');
    assert(state.guided_setup.open); assert(!state.guided_setup.identity_ready); await capture('guided-setup-wide');
    await click('setup-git'); assert(control('git-automatic')); await click('dialog-close');
    for (const [id, kind] of [['setup-open', 'open'], ['setup-new', 'init'], ['setup-clone', 'clone']]) {
      await click(id); assert.equal(state.dialog.kind, kind); await click('dialog-cancel');
    }
    assert.equal(hash(path.join(setup.repo, '.git/config')), configBefore);
    await click('setup-identity'); assert.equal(state.dialog.kind, 'identity');
    await type('dialog-identity-name', 'Rocket Fixture 中文'); await type('dialog-identity-email', 'rocket@example.invalid');
    await click('dialog-submit'); assert(latest().command_succeeded, JSON.stringify(latest()));
    await until(s => s.guided_setup.identity_ready && !s.busy);
    assert.equal(setup.git(['config', '--local', 'user.name']), 'Rocket Fixture 中文');
    assert.equal(fs.readFileSync(path.join(root, 'empty.config'), 'utf8'), '');
    await resize(780, 620); await click('menu-terminal'); await click('terminal-new'); await capture('guided-setup-compact-terminal');
    await scroll('setup-scroll'); await capture('guided-setup-compact-bottom'); await click('setup-summary');
    assert.equal(active().view, 'Summary'); assert(!state.guided_setup.open);
    await capture('summary-compact-terminal'); await resize(1480, 960);
    if (control('console-close')) await click('console-close');
    await click('menu-help'); await click('help-setup'); assert(state.guided_setup.open);
    await click('setup-summary');
    passed('Setup Guide routes Open/New/Clone without writes on Cancel, saves only fixture identity, and scrolls in a compact terminal-open window');

    setup.write(FILE, 'PRIVATE_STAGED\n'); setup.git(['add', '--', FILE]);
    setup.write(FILE, 'PRIVATE_UNSTAGED\n'); setup.write('local 🚀', 'PRIVATE_UNTRACKED');
    await refresh(); await click('view-summary');
    assert.deepEqual([active().summary.staged, active().summary.unstaged, active().summary.untracked], [1, 1, 1]);
    assert.equal(active().summary.remote, 'No upstream configured'); await capture('summary-dirty');
    setup.write('another', 'external');
    await until(() => active().summary?.untracked === 2, 'External edit automatic refresh');
    assert(active().summary.next.includes('Review staged'));
    await click('summary-changes'); assert.equal(active().view, 'Changes');
    passed('Summary distinguishes staged/unstaged/untracked data and follows external edits without another Git polling loop');

    for (const unborn of [false, true]) {
      const f = fixture(unborn ? 'PRIVATE_ROOT' : 'PRIVATE_ORDINARY', unborn);
      f.write(FILE, 'PRIVATE_COMMITTED\n'); if (unborn) f.write('.gitignore', 'ignored/\n');
      await commit(f); dirtyAfterCommit(f); await refresh();
      const before = localBytes(f);
      await review(); await capture(unborn ? 'undo-first-review' : 'undo-ordinary-review');
      await click('dialog-cancel'); assert.equal(f.git(['rev-parse', 'HEAD']), f.tip); assert.deepEqual(localBytes(f), before);
      const recovery = await review(); await click('dialog-submit');
      assert(latest().command_succeeded, JSON.stringify(latest())); assert(!active().undo_available);
      assert.equal(f.git(['rev-parse', recovery]), f.tip); assert.deepEqual(localBytes(f), before);
      if (unborn) {
        assert.equal(f.git(['symbolic-ref', 'HEAD']), 'refs/heads/' + branch);
        assert.notEqual(cp.spawnSync('git', ['-C', f.repo, 'rev-parse', '--verify', 'HEAD'], { env, windowsHide: true }).status, 0);
      }
      else assert.equal(f.git(['rev-parse', 'HEAD']), f.base);
      assert(active().summary.staged > 0);
      await capture(unborn ? 'undo-first-result' : 'undo-ordinary-result');
    }
    passed('Native ordinary/root commit Undo with reviewed cancellation, durable recovery refs and exact staged/working/untracked/ignored preservation');

    const stale = fixture('PRIVATE_STALE'); stale.write(FILE, 'PRIVATE_COMMITTED\n'); await commit(stale);
    dirtyAfterCommit(stale); await refresh(); await review(); stale.write(FILE, 'PRIVATE_CHANGED_AFTER_REVIEW\n');
    await click('dialog-submit'); assert(!latest().command_succeeded);
    assert(state.dialog.error.includes('undo_stale_preview'), JSON.stringify(state.dialog));
    assert.equal(stale.git(['rev-parse', 'HEAD']), stale.tip);
    assert.equal(fs.readFileSync(path.join(stale.repo, FILE), 'utf8'), 'PRIVATE_CHANGED_AFTER_REVIEW\n');
    await capture('undo-stale-error'); await click('dialog-undo-refresh');
    assert(state.dialog.undo.preview_loaded && !state.dialog.error);
    await click('dialog-submit'); assert(latest().command_succeeded, JSON.stringify(latest()));
    assert.equal(fs.readFileSync(path.join(stale.repo, FILE), 'utf8'), 'PRIVATE_CHANGED_AFTER_REVIEW\n');
    passed('Undo refuses an external edit after review, explains why and preserves that edit');
    }

    const locked = fixture('PRIVATE_LOCKED'); locked.write(FILE, 'PRIVATE_COMMITTED\n'); await commit(locked); await review();
    fs.writeFileSync(path.join(locked.repo, '.git/index.lock'), 'other writer');
    await click('dialog-submit'); assert(!latest().command_succeeded); assert(state.dialog.error.includes('undo_locked'));
    assert.equal(fs.readFileSync(path.join(locked.repo, '.git/index.lock'), 'utf8'), 'other writer');
    assert.equal(locked.git(['rev-parse', 'HEAD']), locked.tip); await capture('undo-lock-error'); await click('dialog-cancel');
    fs.unlinkSync(path.join(locked.repo, '.git/index.lock'));

    const shared = fixture('PRIVATE_SHARED'); shared.write(FILE, 'PRIVATE_COMMITTED\n'); await commit(shared);
    shared.git(['update-ref', 'refs/remotes/origin/' + branch, shared.tip]);
    await refresh(); await click('view-summary'); await click('summary-undo');
    assert(state.dialog.error.includes('undo_shared_commit')); assert(!control('dialog-submit') && control('dialog-undo-refresh'), 'only Check again after a refused check');
    assert.equal(shared.git(['rev-parse', 'HEAD']), shared.tip); await capture('undo-shared-error'); await click('dialog-cancel');
    passed('Existing Git lock and known published commit are rejected clearly, without overwriting lock files or refs');

    for (const phase of ['prepared', 'committed']) {
      const f = fixture('PRIVATE_CANCEL_' + phase); f.write(FILE, 'PRIVATE_COMMITTED\n'); await commit(f);
      dirtyAfterCommit(f); await refresh(); const before = localBytes(f), recovery = await review();
      await resize(780, 620); await capture('undo-compact-' + phase);
      // Match the final update's fixed reflog message, not a generic --stdin
      // argument also used by background attribute reads. Recovery is retained.
      // Hooks remain disabled by MCP's transport scope. No fake Git result.
      if (fs.existsSync(marker)) fs.unlinkSync(marker);
      fs.writeFileSync(gate, (phase === 'prepared' ? 'before' : 'after') + ':Rocket Undo commit; changes kept');
      await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab });
      await until(s => s.write && fs.existsSync(marker), 'Undo real-Git relay barrier');
      const pid = Number(fs.readFileSync(marker, 'utf8'));
      const alive = () => { try { process.kill(pid, 0); return true; } catch (e) { assert.equal(e.code, 'ESRCH'); return false; } };
      assert(alive());
      const start = performance.now();
      await client.tool('rocket_click', { target: 'cancel-operation', expected_tab: state.active_tab });
      while (alive()) { assert(performance.now() - start < 10000, 'Owned Git relay did not stop'); await delay(20); }
      report[phase + '_process_stop_ms'] = performance.now() - start;
      fs.unlinkSync(gate);
      await settle(); report[phase + '_cancel_ms'] = performance.now() - start;
      assert(report[phase + '_cancel_ms'] < 10000);
      assert.equal(latest().command_succeeded, phase === 'committed', JSON.stringify(latest()));
      assert.equal(f.git(['rev-parse', 'HEAD']), phase === 'prepared' ? f.tip : f.base);
      assert.equal(f.git(['rev-parse', recovery]), f.tip); assert.deepEqual(localBytes(f), before);
      await capture('undo-cancelled-' + phase); if (state.dialog) await click('dialog-cancel');
    }
    passed('MCP cancellation before/after branch movement reports the verified result and keeps local bytes plus recovery; compact review buttons remain reachable');

    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n =>
      fs.readdirSync(path.join(root, n)).filter(f => /^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f =>
        fs.readFileSync(path.join(root, n, f), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0));
    const payload = JSON.stringify(records);
    for (const privateText of ['PRIVATE_', '資料', 'rocket@example.invalid']) assert(!payload.includes(privateText), 'Diagnostic payload leak: ' + privateText);
    for (const op of ['undo-preview', 'undo-commit']) {
      const starts = records.filter(r => r.event === 'request_start' && r.operation === op); assert(starts.length, op);
      for (const start of starts) assert(records.some(r => r.event === 'request_end' && r.session === start.session && r.request === start.request), 'Unpaired ' + op);
    }
    for (const step of ['capture_undo', 'review_last_commit', 'revalidate_undo', 'save_undo_recovery', 'revalidate_before_undo', 'update_branch_keep_changes', 'verify_undo_result']) assert(payload.includes(step), 'Missing ' + step);
    for (const code of remaining ? ['undo_locked', 'undo_shared_commit'] : ['undo_stale_preview', 'undo_locked', 'undo_shared_commit']) assert(records.some(r => r.failure_code === code), 'Missing code ' + code);
    assert(records.some(r => r.outcome === 'cancelled'));
    report.log_records = records.length;
    passed('Paired diagnostic requests, fixed failure codes, cancellation records and no private paths/messages/identity payloads');
    report.passed = true;
  } catch (error) {
    if (client) try { await capture('failure'); } catch {}
    throw error;
  } finally {
    if (fs.existsSync(gate)) fs.unlinkSync(gate);
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2));
    fs.writeFileSync('work/guided-undo-last-result.txt', path.join(root, 'result.json'));
    console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) {
      try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {}
      await delay(150); if (service.exitCode === null) service.kill();
    }
    if (client) await client.finish();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
