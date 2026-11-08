'use strict';
// Test-only controller. The worker cannot create descendants through its guest.
// A dedicated lifeline closes on parent death; stdin EOF only ends the request.
const { spawn } = require('node:child_process');
const { performance } = require('node:perf_hooks');
const assert = require('node:assert/strict');

function worker(command, input, options = {}) {
  const { signal, timeoutMs = 5000, onReady, args = [], guarded = false } = options;
  assert(Buffer.isBuffer(input) && input.length <= 8 * 1024 * 1024 + 1);
  assert(Number.isSafeInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 5000);
  if (signal?.aborted) return Promise.reject(new Error('cancelled_before_spawn'));
  return new Promise((resolve, reject) => {
    const started = performance.now();
    const child = spawn(command, guarded ? [...args, '--parent-pid', String(process.pid)] : args,
      { windowsHide: true, stdio: guarded ? ['pipe', 'pipe', 'pipe', 'pipe'] : ['pipe', 'pipe', 'pipe'],
      // Mac's runtime supplies its own container HOME. No parent credentials,
      // proxy variables, loader overrides or source paths reach the child.
      env: { LANG: 'C.UTF-8' }, cwd: require('node:os').tmpdir() });
    let out = [], err = [], outSize = 0, errSize = 0, failure, stopped, ready = false;
    const stop = reason => {
      if (!failure) { failure = reason; stopped = performance.now(); child.kill('SIGKILL'); }
    };
    const abort = () => stop('cancelled');
    signal?.addEventListener('abort', abort, { once: true });
    const timer = setTimeout(() => stop('deadline'), timeoutMs);
    if (signal?.aborted) abort();
    child.stdout.on('data', bytes => {
      outSize += bytes.length;
      if (outSize > 1024 * 1024) { stop('output_limit'); return; }
      out.push(bytes);
    });
    child.stderr.on('data', bytes => {
      errSize += bytes.length;
      if (errSize > 4096) { stop('diagnostic_limit'); return; }
      err.push(bytes);
      if (!ready && Buffer.concat(err).toString('utf8').includes('{"event":"running"}\n')) {
        ready = true;
        try { onReady?.(); } catch { stop('ready_callback'); }
      }
    });
    child.stdin.on('error', error => { if (error.code !== 'EPIPE') stop('input_failed'); });
    if (guarded) child.stdio[3].on('error', () => stop('lifeline_failed'));
    child.on('error', () => { failure ||= 'spawn_failed'; });
    child.on('close', (code, exitSignal) => {
      clearTimeout(timer); signal?.removeEventListener('abort', abort);
      const result = { code, signal: exitSignal, ready, pid: child.pid,
        elapsed_ms: performance.now() - started, stop_ms: stopped === undefined ? null : performance.now() - stopped,
        stdout: Buffer.concat(out).toString('utf8'), stderr: Buffer.concat(err).toString('utf8') };
      if (failure) { const error = new Error(failure); error.result = result; reject(error); }
      else resolve(result);
    });
    child.stdin.end(input);
  });
}
module.exports = { worker };
