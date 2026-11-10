'use strict';
// Native App Sandbox characterization only. No production sandbox is enabled by
// this runner. Requires a disposable OS user because macOS creates a container.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const cp = require('node:child_process');
const crypto = require('node:crypto');
const net = require('node:net');
const { performance } = require('node:perf_hooks');

// Bound output while streaming, and wait for the actual child close after Stop.
// Native probes cannot spawn long-lived descendants; their only fork exits at once.
function bounded(command, args, options = {}) {
  const { timeoutMs = 8000, cap = 16384, signal, onStdout, ...spawnOptions } = options;
  assert(Number.isInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 120000);
  assert(Number.isInteger(cap) && cap > 0 && cap <= 1024 * 1024);
  if (signal?.aborted) return Promise.reject(new Error('cancelled_before_spawn'));
  return new Promise((resolve, reject) => {
    const start = performance.now();
    const child = cp.spawn(command, args, { ...spawnOptions, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let out = [], err = [], size = 0, failure = null, stoppedAt = null;
    const stop = reason => {
      if (!failure) {
        failure = reason;
        stoppedAt = performance.now();
        child.kill('SIGKILL');
      }
    };
    const abort = () => stop('cancelled');
    const timer = setTimeout(() => stop('deadline'), timeoutMs);
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
    function receive(bytes, destination, stdout) {
      size += bytes.length;
      if (size > cap) { stop('output_limit'); return; }
      destination.push(bytes);
      if (stdout && !failure && onStdout) {
        try { onStdout(bytes); } catch { stop('output_callback'); }
      }
    }
    child.stdout.on('data', bytes => receive(bytes, out, true));
    child.stderr.on('data', bytes => receive(bytes, err, false));
    child.on('error', () => { failure = failure || 'spawn_failed'; });
    child.on('close', (code, exitSignal) => {
      clearTimeout(timer); signal?.removeEventListener('abort', abort);
      const result = { code, signal: exitSignal, pid: child.pid, elapsed_ms: performance.now() - start,
        stop_ms: stoppedAt === null ? null : performance.now() - stoppedAt,
        stdout: Buffer.concat(out).toString('utf8'), stderr: Buffer.concat(err).toString('utf8') };
      if (failure) { const error = new Error(failure); error.result = result; reject(error); }
      else resolve(result);
    });
  });
}

function verifiedLimit(limit) {
  return limit.set === true && limit.read_back === true &&
    Number.isSafeInteger(limit.requested) && limit.requested >= 0 &&
    [limit.soft, limit.hard].every(value => typeof value === 'string' && /^\d{1,20}$/.test(value) &&
      BigInt(value) <= BigInt(limit.requested));
}

function qualification(observed) {
  const blockers = [];
  if (observed.container_write.success && observed.container_read.success) blockers.push('persistent_container_between_processes');
  if (observed.fork.created) blockers.push('app_sandbox_alone_allows_child_processes');
  if (observed.memory.some(item => !verifiedLimit(item.limit) || item.allocated !== false)) {
    blockers.push('128_mib_native_memory_limit_not_established');
  }
  // Even an unexpectedly restrictive OS cannot enable production from these few
  // probes: XPC identity, IPC, parent death and actual Lua remain separate gates.
  return { production_ready: false, blockers, pending: [
    'broker_and_helper_identity', 'mach_ipc_and_credentials', 'parent_and_supervisor_death',
    'fresh_invocation_and_inherited_handles', 'actual_lua_worker', 'packaged_worker_binding',
  ] };
}

function newOutput(output) {
  assert(path.isAbsolute(output), 'Output must be absolute');
  const parent = fs.realpathSync(path.dirname(output));
  assert.equal(path.dirname(path.resolve(output)), parent, 'Output parent must be canonical');
  fs.mkdirSync(output, { mode: 0o700 }); // Fails for existing files/directories/links.
  assert.equal(fs.realpathSync(output), path.resolve(output));
  return output;
}

async function native(output) {
  assert.equal(process.platform, 'darwin', 'Native macOS required');
  assert(['arm64', 'x64'].includes(process.arch), 'Native Apple Silicon or Intel Mac required');
  assert.notEqual(process.getuid(), 0, 'Use an ordinary disposable OS user');
  assert.equal(process.env.ROCKET_MACOS_SANDBOX_DISPOSABLE_USER, '1', 'Disposable OS-user opt-in required');
  const root = newOutput(output), tmp = path.join(root, 'tmp'); fs.mkdirSync(tmp);
  const env = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', HOME: os.homedir(), TMPDIR: tmp + '/', LANG: 'en_US.UTF-8' };
  const report = { kind: 'app_sandbox_characterization', production_ready: false, checks: [], observations: {} };
  const save = () => fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2) + '\n');
  const run = async (command, args, extra = {}) => {
    const result = await bounded(command, args, { env, cwd: root, ...extra });
    assert.equal(result.code, 0, 'Native command failed: ' + path.basename(command) + '\n' + result.stderr);
    return result.stdout;
  };
  const check = (name, body) => { body(); report.checks.push(name); save(); console.log('PASS ' + name); };
  const id = 'dev.rocket.sandbox-probe.' + crypto.randomBytes(12).toString('hex');
  const app = path.join(root, 'Probe.app'), contents = path.join(app, 'Contents'), bin = path.join(contents, 'MacOS');
  fs.mkdirSync(bin, { recursive: true });
  const executable = path.join(bin, 'probe'), control = path.join(root, 'control');
  let server;
  try {
    report.os = (await run('/usr/bin/sw_vers', ['-productVersion'])).trim();
    report.sdk = (await run('/usr/bin/xcrun', ['--sdk', 'macosx', '--show-sdk-version'])).trim();
    const source = path.join(__dirname, 'macos-sandbox-probe.m');
    await run('/usr/bin/xcrun', ['--sdk', 'macosx', 'clang', '-std=c11', '-Wall', '-Wextra', '-Werror',
      '-fobjc-arc', '-framework', 'Foundation', source, '-o', control], { timeoutMs: 120000 });
    fs.copyFileSync(control, executable, fs.constants.COPYFILE_EXCL);
    const info = `<?xml version="1.0"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>${id}</string><key>CFBundleExecutable</key><string>probe</string>
<key>CFBundleName</key><string>Rocket Sandbox Probe</string><key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string><key>LSUIElement</key><true/></dict></plist>`;
    fs.writeFileSync(path.join(contents, 'Info.plist'), info, { flag: 'wx' });
    const entitlements = path.join(root, 'sandbox.plist');
    fs.writeFileSync(entitlements, '<?xml version="1.0"?><plist version="1.0"><dict><key>com.apple.security.app-sandbox</key><true/></dict></plist>', { flag: 'wx' });
    await run('/usr/bin/codesign', ['--force', '--sign', '-', '--timestamp=none', '--options', 'runtime', control]);
    await run('/usr/bin/codesign', ['--force', '--sign', '-', '--timestamp=none', '--options', 'runtime', '--identifier', id, '--entitlements', entitlements, app]);
    await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', app]);
    const signed = await run('/usr/bin/codesign', ['--display', '--entitlements', ':-', app]);
    fs.writeFileSync(path.join(root, 'signed-entitlements.plist'), signed, { flag: 'wx' });
    const actual = JSON.parse(await run('/usr/bin/plutil', ['-convert', 'json', '-o', '-', path.join(root, 'signed-entitlements.plist')]));
    check('exact_signed_sandbox_entitlement', () => assert.deepEqual(actual, { 'com.apple.security.app-sandbox': true }));
    report.probe_sha256 = crypto.createHash('sha256').update(fs.readFileSync(executable)).digest('hex');
    const probe = async (operation, args = [], program = executable) => JSON.parse(await run(program, [operation, ...args]));
    const identity = await probe('identity');
    check('container_identity_and_unprivileged_launch', () => {
      assert.equal(identity.protocol, 1); assert.equal(identity.uid, process.getuid()); assert.equal(identity.bundle, id);
      assert.equal(identity.home, path.join(os.homedir(), 'Library', 'Containers', id, 'Data'));
    });
    const fixture = path.join(root, 'synthetic-private-file'), protectedOutput = path.join(root, 'must-not-write');
    const marker = Buffer.from('rocket-sandbox-synthetic\0'); fs.writeFileSync(fixture, marker, { flag: 'wx', mode: 0o600 });
    const controlRead = await probe('read', [fixture], control);
    const controlWrite = await probe('write', [path.join(root, 'control-write')], control);
    check('unsandboxed_file_positive_controls', () => { assert(controlRead.success); assert(controlWrite.success); });
    const read = await probe('read', [fixture]), write = await probe('write', [protectedOutput]);
    check('outside_container_files_denied', () => {
      assert.equal(read.opened, false); assert([1, 13].includes(read.errno));
      assert.equal(write.opened, false); assert([1, 13].includes(write.errno));
      assert.equal(fs.existsSync(protectedOutput), false); assert.deepEqual(fs.readFileSync(fixture), marker);
    });
    server = net.createServer(socket => socket.end());
    await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
    const port = String(server.address().port);
    const controlNetwork = await probe('network', [port], control), network = await probe('network', [port]);
    check('loopback_denied_with_reachable_positive_control', () => {
      assert(controlNetwork.connected); assert.equal(network.connected, false); assert([1, 13].includes(network.errno));
    });
    report.observations.container_write = await probe('container-write');
    report.observations.container_read = await probe('container-read');
    report.observations.fork = await probe('fork');
    report.observations.limited_fork = await probe('limited-fork');
    report.observations.memory = [];
    for (const operation of ['as-private', 'as-shared', 'as-mach', 'data-private', 'data-shared', 'data-mach']) {
      const result = await probe(operation);
      report.observations.memory.push({ operation, ...result }); save();
    }
    const controller = new AbortController(); let ready = '';
    try {
      await bounded(executable, ['wait'], { env, cwd: root, signal: controller.signal,
        onStdout: bytes => { ready += bytes; if (ready.includes('\n')) controller.abort(); } });
      assert.fail('Stop must cancel the live probe');
    } catch (error) {
      check('stop_reaps_owned_probe', () => {
        assert.equal(error.message, 'cancelled'); assert.equal(JSON.parse(ready).ready, true);
        assert.equal(error.result.signal, 'SIGKILL'); assert(error.result.stop_ms < 1000);
      });
      report.stop_ms = error.result.stop_ms;
    }
    report.qualification = qualification(report.observations);
    report.completed = true; save();
    console.log('CHARACTERIZATION COMPLETE. Production Lua remains disabled.');
    console.log(JSON.stringify(report.qualification));
  } finally {
    if (server) await new Promise(resolve => server.close(resolve));
    save();
    // The fresh app container is intentionally left to the disposable OS-user
    // teardown. Never delete Library/Containers entries on a developer's Mac.
  }
}

module.exports = { bounded, qualification, newOutput };
if (require.main === module) native(process.argv[2]).catch(error => { console.error(error.message); process.exitCode = 1; });
