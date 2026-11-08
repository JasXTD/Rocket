'use strict';
const test = require('node:test'), assert = require('node:assert/strict');
const { worker } = require('./controller.cjs');
const launch = (script, options = {}) => worker(process.execPath, Buffer.from('{}'), { ...options, args: ['-e', script] });
test('Stop kills the running owned child and waits for closure', async () => {
  const controller = new AbortController();
  await assert.rejects(launch('console.error(JSON.stringify({event:"running"}));setInterval(()=>{},1000)', {
    signal: controller.signal, onReady: () => controller.abort(),
  }), error => error.message === 'cancelled' && error.result.ready && error.result.stop_ms < 1500);
});
test('deadline kills a child that never signals readiness', async () => {
  await assert.rejects(launch('setInterval(()=>{},1000)', { timeoutMs: 150 }),
    error => error.message === 'deadline' && error.result.elapsed_ms < 2000);
});
test('streaming stdout is capped', async () => {
  await assert.rejects(launch('setInterval(()=>process.stdout.write("x".repeat(32768)),1)'),
    error => error.message === 'output_limit' && error.result.stdout.length <= 1024 * 1024);
});
test('streaming diagnostics are capped', async () => {
  await assert.rejects(launch('setInterval(()=>process.stderr.write("x".repeat(8192)),1)'),
    error => error.message === 'diagnostic_limit' && error.result.stderr.length <= 4096);
});
test('pre-cancel cannot launch a child', async () => {
  const controller = new AbortController(); controller.abort();
  await assert.rejects(worker('does-not-exist', Buffer.from('{}'), { signal: controller.signal }), /cancelled_before_spawn/);
});
test('missing executable reports spawn failure', async () => {
  await assert.rejects(worker('rocket-absent-prototype', Buffer.from('{}')), /spawn_failed/);
});
test('child gets neither ambient environment nor lost exit status', async () => {
  process.env.ROCKET_PRIVATE_FIXTURE = 'synthetic';
  try {
    const r = await launch('process.stdout.write(JSON.stringify({absent:!process.env.ROCKET_PRIVATE_FIXTURE}));process.exitCode=7');
    assert.equal(r.code, 7); assert.equal(JSON.parse(r.stdout).absent, true);
  } finally { delete process.env.ROCKET_PRIVATE_FIXTURE; }
});
