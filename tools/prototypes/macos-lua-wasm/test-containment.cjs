'use strict';
const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const cp = require('node:child_process'), net = require('node:net'), crypto = require('node:crypto');
const assert = require('node:assert/strict');
const { performance } = require('node:perf_hooks');
const { bounded } = require('../../tests/test-macos-sandbox.cjs');
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

async function signed(source, root, name, entitlements = { 'com.apple.security.app-sandbox': true }) {
  const app = path.join(root, name + '.app'), contents = path.join(app, 'Contents');
  fs.mkdirSync(path.join(contents, 'MacOS'), { recursive: true });
  const exe = path.join(contents, 'MacOS', 'worker');
  fs.copyFileSync(source, exe, fs.constants.COPYFILE_EXCL);
  const id = 'dev.rocket.lua-prototype.' + crypto.randomBytes(12).toString('hex');
  fs.writeFileSync(path.join(contents, 'Info.plist'), `<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>${id}</string><key>CFBundleExecutable</key><string>worker</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleVersion</key><string>1</string><key>LSUIElement</key><true/></dict></plist>`, { flag: 'wx' });
  const plist = path.join(root, name + '.plist');
  fs.writeFileSync(plist, '<?xml version="1.0"?><plist version="1.0"><dict>' + Object.entries(entitlements).map(([key, value]) => { assert.equal(value, true); return `<key>${key}</key><true/>`; }).join('') + '</dict></plist>', { flag: 'wx' });
  const run = async (cmd, args) => {
    const r = await bounded(cmd, args, { timeoutMs: 30000, env: { PATH: '/usr/bin:/bin', HOME: os.homedir(), LANG: 'en_US.UTF-8' } });
    assert.equal(r.code, 0, r.stderr); return r.stdout;
  };
  await run('/usr/bin/codesign', ['--force', '--sign', '-', '--timestamp=none', '--options', 'runtime', '--identifier', id, '--entitlements', plist, app]);
  await run('/usr/bin/codesign', ['--verify', '--strict', '--deep', app]);
  const actual = path.join(root, name + '-signed.plist');
  fs.writeFileSync(actual, await run('/usr/bin/codesign', ['--display', '--entitlements', ':-', app]), { flag: 'wx' });
  assert.deepEqual(JSON.parse(await run('/usr/bin/plutil', ['-convert', 'json', '-o', '-', actual])), entitlements);
  return exe;
}

// The test supervisor allows 7 seconds so the worker's own 5-second deadline is
// what must fire. It still kills/reaps every directly owned child on failure.
function launch(exe, args, { extraFd, parentArgument = false } = {}) {
  const stdio = ['pipe', 'pipe', 'pipe', 'pipe'];
  if (extraFd !== undefined) { while (stdio.length < 128) stdio.push('ignore'); stdio.push(extraFd); }
  const child = cp.spawn(exe, parentArgument ? ['--parent-pid', String(process.pid)] : args,
    { stdio, env: { LANG: 'C.UTF-8' }, cwd: os.tmpdir() });
  let stdout = '', stderr = '', failure, ready = false;
  const completed = new Promise((resolve, reject) => {
    const timer = setTimeout(() => { failure = 'supervisor_deadline'; child.kill('SIGKILL'); }, 7000);
    child.on('error', () => { failure = 'spawn_failed'; });
    child.stdin.on('error', () => {}); child.stdio[3].on('error', () => {});
    const collect = (name, data) => {
      if (name === 'stdout') stdout += data; else stderr += data;
      ready ||= stdout.includes('{"ready":true}') || stderr.includes('{"event":"running"}');
      if (stdout.length + stderr.length > 16384) { failure = 'output_limit'; child.kill('SIGKILL'); }
    };
    child.stdout.on('data', b => collect('stdout', b)); child.stderr.on('data', b => collect('stderr', b));
    child.on('close', (code, signal) => { clearTimeout(timer); const r = { code, signal, stdout, stderr, ready }; failure ? reject(Error(failure)) : resolve(r); });
  });
  child.stdin.end();
  return { child, completed, isReady: () => ready };
}

function executionState(pid) {
  const r = cp.spawnSync('/bin/ps', ['-p', String(pid), '-o', 'stat='], { encoding: 'utf8', timeout: 1000, maxBuffer: 1024 });
  assert(!r.error && [0, 1].includes(r.status), 'Cannot inspect fixture process');
  const state = r.stdout.trim();
  return !state ? 'gone' : state.startsWith('Z') ? 'zombie' : 'running';
}

async function parentExit(exe, scenario, kind) {
  const parent = cp.fork(path.join(__dirname, 'containment-parent.cjs'), [exe, scenario, kind], { stdio: ['ignore', 'ignore', 'pipe', 'ipc'], env: { LANG: 'C.UTF-8' } });
  let pid, ready = false, exit, parentClosed = false, bytes = 0;
  parent.stderr.on('data', b => { bytes += b.length; if (bytes > 4096) parent.kill('SIGKILL'); });
  parent.on('message', m => { if (m.pid) pid = m.pid; if (m.ready) ready = true; if (m.exit) exit = m; if (m.error) exit = m; });
  const parentDone = new Promise(resolve => parent.on('close', () => { parentClosed = true; resolve(); }));
  const started = performance.now();
  try {
    while (!pid || scenario !== 'startup' && scenario !== 'before-input' && !ready) {
      assert(performance.now() - started < 4000 && !exit, 'Worker failed before test trigger: ' + JSON.stringify(exit));
      await pause(5);
    }
    // Positive control: a waiting guarded worker stays alive with parent/lifeline.
    if (scenario === 'lifeline' && kind === 'probe') await pause(300);
    if (scenario === 'before-input') { await pause(100); assert(!exit && executionState(pid) === 'running'); }
    if (scenario !== 'startup') assert(!exit && executionState(pid) === 'running', 'Worker completed before trigger');
    const trigger = performance.now();
    if (scenario === 'lifeline') parent.send('close-lifeline');
    else if (scenario !== 'startup') parent.kill('SIGKILL');
    let state;
    do { state = executionState(pid); if (state === 'running') await pause(10); }
    while (state === 'running' && performance.now() - trigger < 1200);
    const stop_ms = performance.now() - trigger;
    assert.notEqual(state, 'running', 'Worker outlived parent/lifeline; not a deadline pass');
    assert(stop_ms < 1200);
    if (scenario === 'lifeline') {
      for (let n = 0; n < 30 && !exit; ++n) await pause(5);
      assert.equal(exit?.code, 90, JSON.stringify(exit));
    }
    return { stop_ms, state, ready, exit_code: exit?.code ?? null };
  } finally {
    // The owner fixture, never any unrelated process. No name-based process kill.
    if (pid && executionState(pid) === 'running') { try { process.kill(pid, 'SIGKILL'); } catch (e) { if (e.code !== 'ESRCH') throw e; } }
    if (!parentClosed) parent.kill('SIGKILL');
    await parentDone;
  }
}

async function main() {
  let [worker, probe, output] = process.argv.slice(2);
  for (const p of [worker, probe, output]) assert(path.isAbsolute(p));
  const mac = process.platform === 'darwin'; assert(mac || process.platform === 'linux');
  if (mac) { assert.equal(process.arch, 'arm64'); assert.notEqual(process.getuid(), 0); assert.equal(process.env.ROCKET_MACOS_SANDBOX_DISPOSABLE_USER, '1'); }
  fs.mkdirSync(output, { mode: 0o700 });
  const report = { production_ready: false, platform: process.platform, arch: process.arch, checks: [], native_threshold_bytes: 192 * 1024 * 1024 };
  const save = () => fs.writeFileSync(path.join(output, 'containment.json'), JSON.stringify(report, null, 2) + '\n');
  const check = async (name, action) => {
    try { const details = await action(); report.checks.push({ name, passed: true, ...details }); console.log('PASS ' + name); }
    catch (error) { report.checks.push({ name, passed: false, error: error.message.slice(0, 600) }); throw error; }
    finally { save(); }
  };
  const control = probe, rawWorker = worker;
  if (mac) { worker = await signed(worker, output, 'Lua'); probe = await signed(probe, output, 'Probe'); }
  report.worker = worker; // Only the disposable local path, never user repo data.
  save();
  await check('actual_worker_rejects_wrong_parent_before_input', async () => {
    const r = await launch(worker, ['--parent-pid', '1']).completed;
    assert.equal(r.code, 90); assert.equal(r.stdout, ''); assert(!r.stderr.includes('running'));
  });
  await check('actual_worker_rejects_missing_lifeline', async () => {
    const r = await bounded(worker, ['--parent-pid', String(process.pid)], { timeoutMs: 2000, env: { LANG: 'C.UTF-8' }, cwd: os.tmpdir() });
    assert.equal(r.code, 90); assert.equal(r.stdout, ''); assert(!r.stderr.includes('running'));
  });
  const run = async (operation, args = [], { exe = probe, mode = 'guarded', extraFd } = {}) => {
    const task = launch(exe, [mode, String(process.pid), operation, ...args], { extraFd });
    const result = await task.completed;
    assert.equal(result.code, 0, `${operation}: ${result.stderr} / ${result.signal} / ${result.code}`);
    return JSON.parse(result.stdout);
  };
  if (mac) {
    await check('unsigned_actual_worker_refused', async () => {
      const r = await launch(rawWorker, [], { parentArgument: true }).completed;
      assert.equal(r.code, 93); assert(r.stderr.includes('sandbox_identity')); assert.equal(r.stdout, '');
    });
    await check('extra_entitlement_actual_worker_refused', async () => {
      const extra = await signed(rawWorker, output, 'Extra', { 'com.apple.security.app-sandbox': true, 'com.apple.security.network.client': true });
      const r = await launch(extra, [], { parentArgument: true }).completed;
      assert.equal(r.code, 93); assert(r.stderr.includes('sandbox_identity')); assert.equal(r.stdout, '');
    });
    const outside = path.join(output, 'synthetic-private'), link = path.join(output, 'link');
    fs.writeFileSync(outside, 'fixture only', { flag: 'wx', mode: 0o600 }); fs.symlinkSync(outside, link);
    await check('outside_files_and_symlink_denied_with_controls', async () => {
      for (const file of [outside, link]) { assert((await run('read', [file], { exe: control, mode: 'control' })).success); assert.equal((await run('read', [file])).success, false); }
      assert((await run('write', [path.join(output, 'control-write')], { exe: control, mode: 'control' })).success);
      const denied = path.join(output, 'denied-write'); assert.equal((await run('write', [denied])).success, false); assert(!fs.existsSync(denied)); assert.equal(fs.readFileSync(outside, 'utf8'), 'fixture only');
    });
    for (const kind of ['tcp', 'unix']) await check(kind + '_denied_with_control', async () => {
      const server = net.createServer(s => s.end()); const socket = path.join(os.tmpdir(), 'rocket-lua-' + crypto.randomBytes(8).toString('hex') + '.sock');
      try {
        await new Promise((resolve, reject) => { server.once('error', reject); kind === 'tcp' ? server.listen(0, '127.0.0.1', resolve) : server.listen(socket, resolve); });
        const arg = kind === 'tcp' ? String(server.address().port) : socket;
        assert((await run(kind, [arg], { exe: control, mode: 'control' })).success);
        assert.equal((await run(kind, [arg])).success, false);
      } finally { await new Promise(resolve => server.close(resolve)); }
    });
    await check('inherited_file_fd_above_limit_closed', async () => {
      const fd = fs.openSync(outside, 'r');
      try {
        assert((await run('fd', ['128'], { exe: control, mode: 'control', extraFd: fd })).success);
        const r = await run('fd', ['128'], { extraFd: fd }); assert(!r.success && r.closed);
      } finally { fs.closeSync(fd); }
    });
    await check('kernel_process_limit_prevents_fork', async () => {
      assert((await run('fork', [], { exe: control, mode: 'control' })).success);
      assert.equal((await run('fork')).success, false);
    });
    for (const kind of ['private', 'shared']) await check(kind + '_native_pressure_stops', async () => {
      const start = performance.now(), r = await launch(probe, ['guarded', String(process.pid), kind]).completed;
      assert.equal(r.code, 92, r.stderr); assert.equal(r.signal, null);
      const samples = r.stdout.trim().split('\n').map(s => JSON.parse(s)).filter(s => s.sample_bytes);
      assert(samples.length > 0); assert(!r.stdout.includes('unexpected_completion'));
      const peak_sample_bytes = Math.max(...samples.map(s => s.sample_bytes));
      assert(peak_sample_bytes > 128 * 1024 * 1024 && peak_sample_bytes < 224 * 1024 * 1024);
      return { elapsed_ms: performance.now() - start, peak_sample_bytes, allocated_bytes: Math.max(...samples.map(s => s.allocated_bytes)) };
    });
  }
  for (const scenario of ['startup', 'before-input', 'during-lua']) await check('actual_worker_parent_exit_' + scenario, () => parentExit(worker, scenario, 'worker'));
  await check('dedicated_lifeline_closes_while_parent_is_alive', () => parentExit(probe, 'lifeline', 'probe'));
  // Require the guard's distinct exit status from the actual running Lua worker;
  // natural EOF, fuel or the Lua instruction limit cannot satisfy this check.
  await check('actual_running_lua_reports_lifeline_exit', () => parentExit(worker, 'lifeline', 'worker'));
  await check('parent_crash_during_native_wait', () => parentExit(probe, 'during-wait', 'probe'));
  await check('worker_deadline_without_controller_timer', async () => {
    const start = performance.now(), r = await launch(probe, ['guarded', String(process.pid), 'wait']).completed;
    assert.equal(r.code, 91, r.stderr); assert(r.ready); const elapsed_ms = performance.now() - start;
    assert(elapsed_ms >= 4900 && elapsed_ms < 6500); return { elapsed_ms };
  });
  report.passed = report.checks.every(c => c.passed); save();
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
