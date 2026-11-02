'use strict';
// Affected live acceptance only. UI fetch/pull/push use localhost MCP; the
// publisher is an independent setup/verification actor on one owned branch.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { create, Fixture } = require('./mcp-github-fixture.cjs');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const workspace = path.resolve(__dirname, '../..');
const exe = path.join(workspace, 'target/release/rocket-mcp.exe');
const git = path.join(workspace, 'target/release/git/2.56.0.windows.4/cmd/git.exe');
const authHolderScript = path.join(__dirname, 'managed-git-auth-holder.ps1');
const evidence = path.join(workspace, 'tools/maintenance/managed-git-github-20261030');
const version = fs.readFileSync(path.join(workspace, 'Cargo.toml'), 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const wait = ms => new Promise(r => setTimeout(r, ms));
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const normal = p => path.resolve(p.replace(/^\\\\\?\\/, '')).toLowerCase();
const unlink = p => { if (fs.existsSync(p)) fs.unlinkSync(p); };
const args = process.argv.slice(2);
const resume = args[0] === '--resume' ? JSON.parse(fs.readFileSync(path.resolve(args[1]), 'utf8')) : null;
const modes = resume ? [resume.mode] : args;
assert(modes.length && modes.every(m => ['https', 'ssh'].includes(m)), 'Pass explicit https and/or ssh');
fs.mkdirSync(evidence, { recursive: true });
function userSnapshot() {
  const files = ['.gitconfig', '.ssh/config', '.ssh/known_hosts', '.ssh/id_ed25519_github.pub'];
  const hashes = files.map(p => [p, fs.existsSync(path.join(process.env.USERPROFILE, p)) ? hash(path.join(process.env.USERPROFILE, p)) : null]);
  const dir = path.join(process.env.USERPROFILE, '.ssh');
  // Check key-file metadata without reading private-key contents.
  const entries = fs.existsSync(dir) ? fs.readdirSync(dir).sort().map(name => {
    const s = fs.statSync(path.join(dir, name));
    return [name, s.size, s.mtimeMs, s.isDirectory()];
  }) : [];
  return { hashes, entries };
}
async function run(mode) {
  const savedUser = userSnapshot();
  if (resume) {
    assert.equal(resume.stage, 'push-cancelled');
    assert.equal(resume.cleanup, 'confirmed-absent');
    assert(resume.user_keys_and_configs_unchanged && resume.temporary_credential_removed);
    assert.equal(resume.executable_sha256, hash(exe));
    assert.equal(resume.git_sha256, hash(git));
    assert(resume.cancellations.some(c => c.operation === 'fetch') && resume.cancellations.some(c => c.operation === 'pull'));
  }
  const f = resume ? new Fixture(resume.root, { gitExecutable: git }) : create(mode, { gitExecutable: git });
  const root = f.root, profile = path.join(root, 'bundled-http');
  const output = path.join(root, 'bundled-result.json');
  const report = {
    passed: false, mode, root, version, branch: f.branch, runtime: '2.56.0.windows.4',
    cases: [], cancellations: [], captures: [], phases: [], cleanup: 'pending',
    executable_sha256: hash(exe), git_sha256: hash(git),
    lua_worker_sha256: hash(path.join(workspace, 'target/release/rocket-lua-host.exe')),
  };
  if (resume) {
    const previous = path.join(root, 'partial-before-push-resume.json');
    assert(!fs.existsSync(previous), 'Do not replay a resumed write blindly');
    fs.copyFileSync(output, previous);
    report.cases = resume.cases.slice();
    report.phases = resume.phases.slice();
    report.cancellations = resume.cancellations.slice();
    report.prior_result = previous;
    report.prior_failure = resume.failure;
  }
  const save = () => fs.writeFileSync(output, JSON.stringify(report, null, 2));
  fs.writeFileSync(path.join(evidence, mode + '-latest.json'), JSON.stringify({ result: output }));
  save();
  let client, service, state, closed = false, cursor = 0, authHolder, token = '', encodedAuthorization = '';
  const authFile = path.join(root, 'private-auth/git-auth.config');
  const records = [];
  const control = id => state?.controls?.find(c => c.id === id && c.enabled);
  const latest = () => state.activities.at(-1);
  const pass = label => { report.cases.push(label); save(); console.log('PASS ' + mode + ' ' + label); };
  async function until(test, label, timeout = 90000) {
    const start = performance.now();
    for (;;) {
      state = await client.tool('rocket_state');
      assert(!state.startup_error, 'Native startup failed: ' + state.startup_error);
      if (test(state)) return;
      assert(performance.now() - start < timeout, label + ' timed out');
      await wait(35);
    }
  }
  const isSettled = s => !s.busy && !s.input_pending && !s.write && !s.git_settings?.busy && !s.dialog?.pull?.running && !s.tabs?.some(t => t.loading || t.search?.running);
  async function settle() { await until(isSettled, 'native workers settled'); }
  async function stable(id) {
    let previous = '', count = 0;
    await until(s => {
      const now = JSON.stringify(s.controls);
      count = now === previous ? count + 1 : 0; previous = now;
      return !s.input_pending && count >= 1 && (!id || control(id));
    }, 'ready control ' + (id || ''));
  }
  async function click(id, finish = true) {
    await stable(id);
    await client.tool('rocket_click', { target: id, expected_tab: state.active_tab });
    if (finish) await settle();
  }
  async function type(id, text) {
    await stable(id);
    await client.tool('rocket_type', { target: id, text, expected_tab: state.active_tab });
    await settle();
  }
  async function closeDialog() {
    if (state?.dialog) {
      await stable();
      await click(control('dialog-close') ? 'dialog-close' : 'dialog-cancel');
      await until(s => !s.dialog && !s.input_pending, 'dialog closed');
    }
  }
  async function readLogs() {
    for (;;) {
      const b = await client.tool('rocket_logs', { after_seq: cursor, limit: 1000 });
      assert(!b.retention_gap, 'Diagnostic retention gap');
      assert(b.next_seq >= cursor);
      cursor = b.next_seq; records.push(...b.records);
      if (!b.more) break;
    }
  }
  async function capture(label) {
    await stable();
    const r = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
    assert(!r.isError);
    const block = r.content.find(c => c.type === 'image');
    assert(block, 'Missing native capture');
    fs.writeFileSync(path.join(root, label + '.png'), Buffer.from(block.data, 'base64'));
    fs.writeFileSync(path.join(root, label + '.json'), JSON.stringify(state, null, 2));
    report.captures.push(label);
  }
  async function openOperation(operation, upstream = false) {
    await closeDialog(); await settle();
    await click('toolbar-' + operation);
    await until(s => s.dialog?.kind === operation && !s.input_pending, operation + ' dialog');
    if (operation === 'push') {
      await type('push-destination', f.branch);
      if (state.dialog.flag !== upstream) await click('push-upstream');
      assert.equal(state.dialog.second, f.branch);
      assert.equal(state.dialog.flag, upstream);
    }
  }
  async function result(operation, beforeCount, expected, logStart, label) {
    await until(s => s.activities.length === beforeCount + 1 && isSettled(s), label + ' result');
    const a = latest(), details = a.details.join('\n');
    assert.equal(a.operation, operation);
    assert(!/github_pat_|gh[pousr]_[A-Za-z0-9]{16,}|Authorization:\s*(Basic|Bearer)\s+\S+|PRIVATE KEY/i.test(details), 'Credential-shaped UI details omitted');
    const code = details.match(/\[([a-z_]+)\]/)?.[1] || '';
    if (expected) {
      assert(!a.command_succeeded && new RegExp('^(?:' + expected + ')$').test(code), label + ': expected ' + expected + ', got ' + code);
      assert.match(details, /Next: /, 'Actionable error guidance is missing');
    } else assert(a.command_succeeded, label + ' failed: ' + a.summary + '; ' + details);
    assert.equal(state.dropped_events, 0);
    assert(!state.tabs.find(t => t.id === state.active_tab)?.error, 'Repository refresh error');
    await readLogs();
    const relevant = records.slice(logStart);
    assert.equal(relevant.filter(r => r.event === 'request_start' && r.operation === operation).length, 1, 'Missing or repeated operation request');
    if (code) assert(relevant.some(r => r.failure_code === code), 'UI failure missing from diagnostics');
    report.phases.push({ label, operation, command_succeeded: a.command_succeeded, failure_code: code, elapsed_ms: a.elapsed_ms, summary: a.summary, details: a.details });
    save();
    return relevant;
  }
  async function operation(label, op, expected = '', upstream = false) {
    if (op === 'pull') return pullOperation(label, expected);
    report.stage = label; save();
    await openOperation(op, upstream);
    await readLogs();
    const count = state.activities.length, first = records.length;
    await click('dialog-submit', false);
    await result(op, count, expected, first, label);
    if (expected) await capture(label);
    await closeDialog();
    pass(label);
  }
  async function pullOperation(label, expected) {
    report.stage = label; save();
    await openOperation('pull');
    assert(!control('dialog-submit'), 'Pull must require a fetched review');
    const baseline = f.snapshot();
    await readLogs();
    let count = state.activities.length, first = records.length;
    await click('dialog-pull-fetch', false);
    await result('pull-fetch', count, expected === 'pull_diverged' ? '' : expected, first, label + '-fetch');
    assert.deepEqual(f.snapshot(), baseline, 'Fetch/review changed local checkout or index');
    if (expected && expected !== 'pull_diverged') {
      assert(!state.dialog.pull.fetched && !control('dialog-submit'));
      await capture(label); await closeDialog(); pass(label);
      return;
    }
    assert(state.dialog.pull.fetched && state.dialog.pull.preview, 'Missing fetched pull review');
    if (expected === 'pull_diverged') {
      assert.equal(state.dialog.pull.preview.relation, 'Diverged');
      assert(!state.dialog.pull.preview.ready && !control('dialog-submit'));
      report.phases.push({ label, operation: 'pull-preview', guard: 'diverged-fast-forward-disabled', command_succeeded: false, failure_code: '' });
      await capture(label); await closeDialog();
      pass(label + ': diverged preview explains choices and disables unsafe fast-forward');
      return;
    }
    assert(state.dialog.pull.preview.ready, 'Reviewed pull cannot apply');
    if (label === 'pull-fast-forward') assert.equal(state.dialog.pull.preview.relation, 'FastForward');
    if (label === 'pull-up-to-date') assert.equal(state.dialog.pull.preview.relation, 'UpToDate');
    await capture(label + '-preview');
    count = state.activities.length; first = records.length;
    await click('dialog-submit', false);
    await result('pull-apply', count, '', first, label + '-apply');
    await closeDialog(); pass(label);
  }
  function credential() {
    const r = cp.spawnSync(git, ['credential-manager', 'get'], {
      env: { ...f.env, GCM_INTERACTIVE: 'never', GCM_GUI_PROMPT: '0' },
      input: 'protocol=https\nhost=github.com\nusername=JasXTD\n\n',
      encoding: 'utf8', windowsHide: true, timeout: 30000, maxBuffer: 128 * 1024,
    });
    const value = (r.stdout || '').split(/\r?\n/).find(line => line.startsWith('password='))?.slice(9);
    assert(r.status === 0 && value, 'Existing noninteractive GitHub credential unavailable; no login mutation attempted');
    return value;
  }
  function clientAuth(enabled) {
    assert.equal(mode, 'https');
    if (enabled) f.config(f.client, 'include.path', authFile.replaceAll('\\', '/'));
    else f.git(['config', '--local', '--unset-all', 'include.path']);
  }
  async function prepareAuth() {
    for (const key of ['core.askPass', 'credential.helper', 'core.sshCommand']) f.config(f.client, key, '');
    if (f.git(['config', '--local', '--name-only', '--list']).split(/\r?\n/).includes('http.sslbackend')) {
      f.git(['config', '--local', '--unset-all', 'http.sslBackend']);
    }
    if (mode !== 'https') return;
    if (resume) {
      assert.deepEqual(fs.readdirSync(path.dirname(authFile)), [], 'Unexpected private-auth contents');
      fs.rmdirSync(path.dirname(authFile));
    }
    token = credential();
    encodedAuthorization = Buffer.from('x-access-token:' + token).toString('base64');
    const config = '[http "' + f.remote + '"]\n\textraHeader = Authorization: Basic ' + encodedAuthorization + '\n';
    const powershell = path.join(process.env.ProgramFiles, 'PowerShell/7/pwsh.exe');
    authHolder = cp.spawn(powershell, ['-NoProfile', '-NonInteractive', '-File', authHolderScript, '-Directory', path.dirname(authFile)], {
      env: f.env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
    });
    const ready = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('Private credential holder did not become ready')), 30000);
      let seen = '';
      authHolder.stdout.on('data', data => {
        seen += data.toString('utf8');
        if (seen.length > 128) { clearTimeout(timer); reject(Error('Unexpected holder output')); return; }
        if (seen.includes('READY')) { clearTimeout(timer); resolve(); }
      });
      authHolder.on('error', () => { clearTimeout(timer); reject(Error('Private credential holder could not start')); });
      authHolder.on('exit', () => { clearTimeout(timer); if (!seen.includes('READY')) reject(Error('Private credential holder failed before ready')); });
      authHolder.stderr.resume();
    });
    authHolder.stdin.write(Buffer.from(config).toString('base64') + '\n');
    await ready; clientAuth(true);
    assert(f.git(['config', '--file', authFile, '--name-only', '--list']).includes('extraheader'));
    report.https_credential = 'bundled GCM get only; owner-only temporary config removed on holder EOF/finally';
  }
  async function cancellation(op) {
    const label = op + '-cancelled';
    report.stage = label; save();
    if (mode === 'ssh' && op === 'push') {
      await closeDialog();
      await click('drawer-activity');
      if (control('drawer-expand')) await click('drawer-expand');
      assert(control('drawer-collapse'), 'Activity drawer must be expanded for this Stop check');
    }
    await openOperation(op);
    const baseline = f.snapshot();
    await readLogs();
    const count = state.activities.length, first = records.length;
    await click(op === 'pull' ? 'dialog-pull-fetch' : 'dialog-submit', false);
    // Observe the real transport process. Do not inject fake Git, proxies or hooks.
    const verb = op === 'pull' ? 'fetch' : op;
    const deadline = Date.now() + 30000;
    for (;;) {
      state = await client.tool('rocket_state');
      await readLogs();
      if (records.slice(first).some(r => r.event === 'git_start' && r.operation === verb)) break;
      assert(Date.now() < deadline && state.activities.length === count, 'Transport did not start before completion');
      await wait(10);
    }
    assert(state.write && !state.input_pending, 'Operation completed before cancel; not counted as a cancellation');
    const start = performance.now();
    const attempts = [];
    report.cancel_attempt = { operation: op, attempts, requested_at: Date.now() }; save();
    for (let n = 0; n < 3; n++) {
      const stop = control('dialog-pull-cancel') ? 'dialog-pull-cancel' : control('cancel-operation') ? 'cancel-operation' : 'status-cancel';
      assert(control(stop), 'Visible Stop is unavailable');
      const attempt = { target: stop, rect: control(stop).rect, at: Date.now() };
      attempts.push(attempt); save();
      await client.tool('rocket_click', { target: stop, expected_tab: state.active_tab });
      await until(s => !s.input_pending, 'Stop input finished');
      attempt.cancelling = !!state.write?.cancelling;
      attempt.still_running = !!state.write;
      attempt.control_after = control(stop)?.rect;
      attempt.elapsed_ms = Math.round(performance.now() - start); save();
      if (state.write?.cancelling || !state.write) break;
      await capture(label + '-stop-not-activated-' + n);
    }
    await until(s => s.write?.cancelling || (!s.write && s.activities.length === count + 1), 'cancellation feedback');
    const feedback = Math.round(performance.now() - start);
    const feedbackState = state.write ? state.write.progress : 'completed before next state observation';
    if (state.write) assert.equal(state.write.cancel_enabled, false);
    await until(s => s.activities.length === count + 1 && isSettled(s), 'cancelled operation settled', 15000);
    const completion = Math.round(performance.now() - start);
    report.cancel_attempt.feedback_ms = feedback;
    report.cancel_attempt.completion_ms = completion;
    report.cancel_attempt.activity_success = latest().command_succeeded; save();
    assert(!latest().command_succeeded, 'Stop did not cancel this push; inspect the recorded input and process timing');
    assert.match(JSON.stringify(latest()), /cancel/i);
    assert(feedback < 1500, 'Cancellation feedback exceeded 1.5 seconds');
    assert(completion < 10000, 'Cancellation completion exceeded 10 seconds');
    assert.deepEqual(f.snapshot(), baseline, 'Cancelled command changed local checkout/index/files');
    await readLogs();
    const stopped = records.slice(first).filter(r => r.event === 'git_end' && r.outcome === 'cancelled');
    assert(stopped.some(r => r.cancel_to_stop_us > 0), 'Actual Git stop timing missing');
    assert.equal(records.slice(first).filter(r => r.event === 'request_start' && r.operation === (op === 'pull' ? 'pull-fetch' : op)).length, 1, 'Cancellation retried the operation');
    const actual = f.tip(f.remoteState());
    const beforeTip = f.r.expected_tip;
    assert(actual === beforeTip || (op === 'push' && actual === f.r.pending), 'Unexpected concurrent remote writer; preserved');
    if (actual !== beforeTip) { f.r.expected_tip = actual; f.save(); }
    report.cancellations.push({ operation: op, phase: 'actual ' + verb + ' process started', input_attempts: attempts.length, feedback_ms: feedback, feedback_state: feedbackState, completion_ms: completion, git_cancel_to_stop_us: stopped.map(r => r.cancel_to_stop_us), remote_result: actual === beforeTip ? 'unchanged' : 'applied_before_stop' });
    await capture(label + '-result'); await closeDialog();
    f.assertRemote(f.r.expected_tip);
    pass(label + ': immediate feedback, measured Git stop and independently checked remote/local state');
  }
  async function closeWindow() {
    if (!client) return;
    await settle(); await closeDialog();
    await client.tool('rocket_close');
    await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'window close');
    if (state.dialog?.kind === 'quit') {
      await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab });
      await until(s => ['closed', 'not_started'].includes(s.window), 'window closed');
    }
    closed = true;
  }
  try {
    console.log('START ' + mode + ' bundled Git fixture; no user configuration changes');
    // No system Git executable is available to the native app or its GCM helper.
    for (const key of Object.keys(f.env)) if (key.toLowerCase() === 'path') {
      f.env[key] = f.env[key].split(path.delimiter).filter(p => path.isAbsolute(p) && !/[\\/]Program Files[\\/]Git(?:[\\/]|$)/i.test(p) && !fs.existsSync(path.join(p, 'git.exe'))).join(path.delimiter);
    }
    f.r.git_executable = git; f.save();
    if (resume) f.assertRemote('');
    else f.run('setup');
    await prepareAuth();
    if (resume) {
      f.r.remote_write_attempted = true; f.save();
      f.git(['push', '--porcelain', '--no-follow-tags', '--no-prune', '--no-recurse-submodules',
        '--force-with-lease=' + f.ref + ':', '--', 'origin', f.r.expected_tip + ':' + f.ref]);
      f.assertRemote(f.r.expected_tip);
    }
    report.auth_preflight = 'existing credential/key authenticated read-only; no new sign-in';
    const storage = path.join(root, 'rocket-ui-state');
    fs.mkdirSync(storage, { recursive: true });
    fs.writeFileSync(path.join(storage, 'git-selection'), 'ROCKET-GIT-2\nmanaged');
    cp.execFileSync(exe, ['--init-http', profile, root], { env: f.env, windowsHide: true, stdio: 'pipe', timeout: 30000 });
    const listener = net.createServer();
    await new Promise(r => listener.listen(0, '127.0.0.1', r));
    const port = listener.address().port; await new Promise(r => listener.close(r));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    report.port = port;
    const fd = fs.openSync(path.join(root, 'service.log'), 'w');
    const serviceEnv = { ...f.env };
    for (const name of ['GIT_ASKPASS', 'SSH_ASKPASS', 'GIT_SSH', 'GIT_SSH_COMMAND', 'GIT_SSH_VARIANT', 'GIT_PROXY_COMMAND', 'GIT_EXEC_PATH']) delete serviceEnv[name];
    const options = ['--http-profile', profile];
    if (mode === 'ssh') options.push('--windows-ssh-agent', path.join(root, 'identity.pub'));
    service = cp.spawn(exe, options, { env: serviceEnv, windowsHide: true, stdio: ['ignore', 'ignore', fd] });
    fs.closeSync(fd); client = new RocketHttpClient(profile);
    for (let n = 0; ; n++) {
      try { if ((await client.raw('GET', '/health')).status === 200) break; } catch {}
      assert(n < 150 && service.exitCode === null, 'Owned MCP service did not start'); await wait(100);
    }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: f.client });
    await until(s => s.tabs?.length && s.git_settings?.active_executable && isSettled(s), 'bundled Git repository startup');
    await client.tool('rocket_resize', { width: 1460, height: 960 }); await settle();
    await click('menu-edit'); await click('preferences'); await click('pref-page-git');
    await until(s => s.git_settings.loaded && isSettled(s), 'Git preference verification');
    assert.equal(normal(state.git_settings.active_executable), normal(git));
    assert(state.git_settings.managed && state.git_settings.bundled.verified, 'Managed Git was not selected and verified');
    assert.equal(state.git_settings.bundled.version, report.runtime);
    report.native_git = state.git_settings.active_executable;
    report.native_runtime_verified = true;
    await capture('bundled-startup');
    await closeDialog();
    if (control('console-close')) await click('console-close');
    if (!resume) pass('Native MCP uses the verified bundled runtime with system Git absent from PATH');
    if (!resume) {
    f.run('before-push');
    await operation('push-new-branch', 'push', '', true); f.run('verify-new-push');
    if (mode === 'https') {
      clientAuth(false);
      for (const op of ['fetch', 'pull', 'push']) {
        await operation(op + '-authentication-unavailable', op, 'credentials_unavailable|authentication_rejected');
        f.run('verify-unchanged');
      }
      clientAuth(true);
    }
    await operation('fetch-up-to-date', 'fetch'); f.run('verify-unchanged');
    f.run('publish-advance');
    await operation('fetch-new-commit', 'fetch'); f.run('verify-fetch');
    await operation('pull-fast-forward', 'pull'); f.run('verify-pull');
    await operation('pull-up-to-date', 'pull'); f.run('verify-unchanged');
    f.run('local-advance'); await click('refresh');
    await operation('push-fast-forward', 'push'); f.run('verify-local-push');
    await operation('push-up-to-date', 'push'); f.run('verify-unchanged');
    const pending = f.commit(f.client, 'ROCKET_PRIVATE_cancel_pending', 'Synthetic unpushed cancellation sentinel');
    f.r.pending = pending; f.save();
    await click('refresh');
    for (const op of ['fetch', 'pull', 'push']) await cancellation(op);
    } else await cancellation('push');
    const pending = f.r.pending;
    await operation('push-after-cancellation', 'push');
    f.assertRemote(pending); f.r.expected_tip = pending; f.save();
    pass('Existing authentication still works after all three cancellations');
    f.run('diverge'); await click('refresh');
    await operation('push-fetch-first', 'push', 'push_fetch_first'); f.run('verify-unchanged');
    await operation('fetch-diverged', 'fetch'); f.run('verify-diverged-fetch');
    await operation('push-non-fast-forward', 'push', 'push_non_fast_forward'); f.run('verify-unchanged');
    await operation('pull-diverged', 'pull', 'pull_diverged'); f.run('verify-unchanged');
    f.run('dirty'); await click('refresh');
    await operation('pull-dirty-worktree', 'pull', 'pull_dirty_worktree'); f.run('verify-unchanged');
    await operation('fetch-with-local-edits', 'fetch'); f.run('verify-unchanged');
    await capture('final-local-changes-preserved');
    await readLogs();
    await closeWindow();
    await client.raw('POST', '/shutdown');
    const end = Date.now() + 10000;
    while (service.exitCode === null && Date.now() < end) await wait(25);
    assert.equal(service.exitCode, 0, 'MCP service failed to stop');
    const disk = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(p => /^rocket(?:\.\d+)?\.jsonl$/.test(p)).flatMap(p => fs.readFileSync(path.join(root, n, p), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length && disk.length, 'Diagnostics missing');
    assert(disk.every(r => r.dropped_before === 0), 'Dropped diagnostic events');
    const payload = JSON.stringify(disk);
    for (const value of ['ROCKET_PRIVATE', f.branch, f.remote, 'rocket-auth-test', 'rocket@example.invalid', ...f.r.owned, 'Authorization:', 'PRIVATE KEY', 'credential-manager', token, encodedAuthorization].filter(Boolean)) assert(!payload.includes(value), 'Private payload found in diagnostics; details omitted');
    assert(!/github_pat_|gh[pousr]_[A-Za-z0-9]{16,}/.test(payload), 'Credential-shaped diagnostic value');
    for (const start of disk.filter(r => ['request_start', 'step_start', 'git_start'].includes(r.event))) {
      assert.equal(disk.filter(r => r.event === start.event.replace('_start', '_end') && r.id === start.id && r.session === start.session).length, 1, 'Unpaired diagnostic span');
    }
    for (const phase of report.phases.filter(p => p.failure_code)) assert(disk.some(r => r.failure_code === phase.failure_code), 'Failure absent from persisted logs');
    report.logs = { via_localhost_mcp: records.length, persisted: disk.length, dropped: 0, paired: true, privacy_scan_passed: true };
    fs.writeFileSync(path.join(root, 'mcp-diagnostics.json'), JSON.stringify(records, null, 2));
    assert.equal(hash(exe), report.executable_sha256, 'Native executable changed during tests');
    assert.equal(hash(git), report.git_sha256, 'Bundled Git changed during tests');
    pass('Typed errors and cancellation timing are visible in paired logs with no tested private payloads');
    report.passed = true;
  } catch (error) {
    report.failure = error.message;
    if (client) { try { await capture('failure'); } catch {} }
    console.error('FAIL ' + mode + ': ' + error.message);
  } finally {
    // Stop the writer before remote cleanup or removal of credential data.
    if (client && service?.exitCode === null) {
      try {
        state = await client.tool('rocket_state');
        if (state.write) await click('cancel-operation', false);
        await closeWindow(); await client.raw('POST', '/shutdown');
        const end = Date.now() + 10000;
        while (service.exitCode === null && Date.now() < end) await wait(30);
        assert.equal(service.exitCode, 0);
      } catch (e) { report.shutdown_error = e.message; report.passed = false; }
    }
    if (client) await client.finish();
    if (!service || service.exitCode !== null) {
      if (f.r.baseline && f.r.remote_write_attempted) {
        try {
          f.run('cleanup');
          report.cleanup = f.r.cleanup;
          report.original_refs_and_default_head_unchanged = f.r.original_refs_and_default_head_unchanged;
        } catch (e) { report.cleanup_error = e.message; report.passed = false; }
      } else report.cleanup = 'no-remote-write';
      unlink(path.join(profile, 'access-token'));
    } else { report.cleanup = 'deferred-until-owned-worker-stops'; report.passed = false; }
    if (authHolder) {
      authHolder.stdin.end();
      const end = Date.now() + 10000;
      while (authHolder.exitCode === null && Date.now() < end) await wait(25);
      if (authHolder.exitCode === null) {
        authHolder.kill();
        const stopped = Date.now() + 5000;
        while (authHolder.exitCode === null && Date.now() < stopped) await wait(25);
      }
      if (authHolder.exitCode !== null) unlink(authFile);
      report.temporary_credential_removed = !fs.existsSync(authFile);
      if (!report.temporary_credential_removed) report.passed = false;
    }
    if (mode === 'https' && token) {
      try { report.existing_login_unchanged = credential() === token; }
      catch { report.existing_login_unchanged = false; }
      if (!report.existing_login_unchanged) report.passed = false;
    }
    try { assert.deepEqual(userSnapshot(), savedUser); report.user_keys_and_configs_unchanged = true; }
    catch { report.user_keys_and_configs_unchanged = false; report.passed = false; }
    report.tool_calls = client?.calls || 0; save();
    console.log(JSON.stringify({ mode, passed: report.passed, result: output, cleanup: report.cleanup, cases: report.cases.length, failure: report.failure }));
  }
  assert(report.passed, mode + ' acceptance failed; inspect the saved result before another run');
  return report;
}
(async () => {
  for (const mode of modes) await run(mode);
})().catch(error => { console.error(error.message); process.exitCode = 1; });
