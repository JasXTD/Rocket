'use strict';
const test = require('node:test'), assert = require('node:assert/strict');
const fs = require('node:fs'), os = require('node:os'), path = require('node:path');
const { bounded, qualification, newOutput } = require('./test-macos-sandbox.cjs');

test('Probe Stop waits for the live process to exit', async () => {
  const control = new AbortController(); let ready = '';
  await assert.rejects(bounded(process.execPath, ['-e', 'console.log("ready");setInterval(()=>{},1000)'], {
    signal: control.signal, onStdout: bytes => { ready += bytes; if (ready.includes('ready')) control.abort(); },
  }), error => error.message === 'cancelled' && error.result.stop_ms < 2000 && error.result.pid > 0);
});
test('Probe deadline terminates a hung child', async () => {
  await assert.rejects(bounded(process.execPath, ['-e', 'setInterval(()=>{},1000)'], { timeoutMs: 200 }),
    error => error.message === 'deadline' && error.result.elapsed_ms < 3000);
});
test('Probe output limit stops streaming output', async () => {
  await assert.rejects(bounded(process.execPath, ['-e', 'setInterval(()=>process.stdout.write("x".repeat(4096)),1)'], { cap: 512 }),
    error => error.message === 'output_limit' && error.result.stdout.length <= 512);
});
test('Pre-cancelled probe never spawns a process', async () => {
  const control = new AbortController(); control.abort();
  await assert.rejects(bounded('absent-program', [], { signal: control.signal }), /cancelled_before_spawn/);
});
test('Spawn failure is explicit', async () => {
  await assert.rejects(bounded(path.join(os.tmpdir(), 'rocket-nonexistent-sandbox-probe'), []), /spawn_failed/);
});
test('Successful output retains native exit status', async () => {
  const result = await bounded(process.execPath, ['-e', 'process.stdout.write("value");process.exitCode=7']);
  assert.equal(result.stdout, 'value'); assert.equal(result.code, 7); assert.equal(result.stop_ms, null);
});
test('A measured sandbox gap cannot be reported as production ready', () => {
  const result = qualification({ container_write: { success: true }, container_read: { success: true },
    fork: { created: true }, memory: [{ limit: { set: true, read_back: true }, allocated: true }] });
  assert.equal(result.production_ready, false);
  assert.deepEqual(result.blockers, ['persistent_container_between_processes',
    'app_sandbox_alone_allows_child_processes', '128_mib_native_memory_limit_not_established']);
});
test('Passing the limited probes cannot bypass remaining production gates', () => {
  const result = qualification({ container_write: { success: false }, container_read: { success: false },
    fork: { created: false }, memory: [{ limit: { set: true, read_back: true, requested: 134217728,
      soft: '134217728', hard: '134217728' }, allocated: false }] });
  assert.equal(result.production_ready, false); assert(result.pending.includes('actual_lua_worker'));
  assert.deepEqual(result.blockers, []);
});
test('Rounded or excessive native limits cannot establish memory protection', () => {
  for (const value of [9223372036854776000, '9223372036854775807', '134217729', 'unavailable']) {
    const result = qualification({ container_write: { success: false }, container_read: { success: false },
      fork: { created: false }, memory: [{ limit: { set: true, read_back: true, requested: 134217728,
        soft: value, hard: value }, allocated: false }] });
    assert.deepEqual(result.blockers, ['128_mib_native_memory_limit_not_established']);
  }
});
test('Existing output and files are preserved', () => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'rocket-sandbox-harness-')));
  const file = path.join(root, 'keep'); fs.writeFileSync(file, 'synthetic');
  assert.throws(() => newOutput(root)); assert.throws(() => newOutput(file));
  assert.equal(fs.readFileSync(file, 'utf8'), 'synthetic');
  const out = newOutput(path.join(root, 'new')); assert(fs.statSync(out).isDirectory());
  fs.unlinkSync(file); fs.rmdirSync(out); fs.rmdirSync(root);
});
