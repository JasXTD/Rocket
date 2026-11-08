'use strict';
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const { worker } = require('./controller.cjs');
const { createHash } = require('node:crypto');
const normalized = value => Array.isArray(value) ? value.map(normalized) : value && typeof value === 'object'
  ? Object.fromEntries(Object.keys(value).sort().map(k => [k, normalized(value[k])])) : value;
async function main() {
  const [exe, output, mode, referencePath] = process.argv.slice(2);
  assert(mode === undefined || mode === 'pulley' || mode === 'native-reference');
  const native = mode === 'native-reference';
  const guarded = !native && (process.platform === 'darwin' || process.env.ROCKET_TEST_LIFELINE === '1');
  const reference = referencePath ? JSON.parse(fs.readFileSync(referencePath, 'utf8')) : null;
  if (reference) assert(reference.compatibility_passed && reference.mode === 'native-reference');
  assert(exe && output && path.isAbsolute(exe) && path.isAbsolute(output));
  const report = { production_ready: false, compatibility_passed: false, mode: mode || 'pulley', platform: process.platform, arch: process.arch, checks: [] };
  // Never overwrite an earlier run or include script/response bodies in reports.
  const fd = fs.openSync(output, 'wx', 0o600);
  const save = () => { const data = Buffer.from(JSON.stringify(report, null, 2) + '\n'); fs.writeSync(fd, data, 0, data.length, 0); fs.ftruncateSync(fd, data.length); };
  const record = row => { report.checks.push(row); save(); console.log((row.passed ? 'PASS ' : 'FAIL ') + row.name); };
  try {
    for (const fixture of require('./cases.json')) {
      let result, value;
      try {
        result = await worker(exe, Buffer.from(JSON.stringify({ api: 1, source: fixture.source, context: {} })), { guarded });
        value = JSON.parse(result.stdout);
        assert.equal(result.code, 0, fixture.name);
        assert.equal(value.kind, fixture.kind, fixture.name);
        if (fixture.code) assert.equal(value.code, fixture.code, fixture.name);
        if (fixture.title) assert.equal(value.view.title, fixture.title, fixture.name);
        assert(!result.stdout.includes('PRIVATE_TEST_SENTINEL_DO_NOT_LOG'));
        let peak_bytes;
        if (guarded) {
          assert.match(result.stderr, /^\{"event":"running"\}\n\{"event":"guard","peak_bytes":\d+\}\n$/);
          peak_bytes = JSON.parse(result.stderr.trim().split('\n')[1]).peak_bytes;
          assert(peak_bytes <= 192 * 1024 * 1024);
          if (process.platform === 'darwin') assert(peak_bytes > 0);
        } else assert.equal(result.stderr, native ? '' : '{"event":"running"}\n');
        const result_sha256 = createHash('sha256').update(JSON.stringify(normalized(value))).digest('hex');
        if (reference) assert.equal(result_sha256, reference.checks.find(c => c.name === fixture.name)?.result_sha256, 'Native output differs');
        record({ name: fixture.name, passed: true, elapsed_ms: result.elapsed_ms, result_sha256, peak_bytes });
      } catch (error) {
        record({ name: fixture.name, passed: false, expected_kind: fixture.kind, expected_code: fixture.code,
          observed_kind: value?.kind, observed_code: value?.code, exit_code: result?.code,
          supervisor: error.result ? error.message : null, elapsed_ms: (result || error.result)?.elapsed_ms });
      }
    }
    if (native) { report.compatibility_passed = report.checks.every(c => c.passed); save(); assert(report.compatibility_passed, 'Native comparison failed'); return; }
    const oversized = await worker(exe, Buffer.alloc(8 * 1024 * 1024 + 1, 32), { guarded });
    assert.equal(oversized.code, 2); assert.equal(JSON.parse(oversized.stdout).code, 'wasm_boundary');
    record({ name: 'oversized_input_refused', passed: true, elapsed_ms: oversized.elapsed_ms });
    const controller = new AbortController();
    try {
      await worker(exe, Buffer.from(JSON.stringify({ api: 1, source: 'return function() while true do end end', context: {} })), {
        signal: controller.signal, onReady: () => controller.abort(), guarded,
      });
      assert.fail('Worker finished before cancellation was observed');
    } catch (error) {
      assert.equal(error.message, 'cancelled'); assert(error.result.ready); assert(error.result.stop_ms < 1500);
      assert.throws(() => process.kill(error.result.pid, 0), { code: 'ESRCH' });
      record({ name: 'cancel_running_worker', passed: true, stop_ms: error.result.stop_ms });
    }
    report.compatibility_passed = report.checks.every(c => c.passed); save();
    assert(report.compatibility_passed, 'Lua/Pulley compatibility failed; production remains disabled');
  } finally { fs.closeSync(fd); }
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
