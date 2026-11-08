'use strict';
// Deliberately disposable intermediate parent. The test kills this process,
// leaving only the actual worker's own lifeline/deadline to stop execution.
const { spawn } = require('node:child_process');
const [exe, scenario, kind] = process.argv.slice(2);
const args = kind === 'worker' ? ['--parent-pid', String(process.pid)] : ['guarded', String(process.pid), 'wait'];
const child = spawn(exe, args, { env: { LANG: 'C.UTF-8' }, cwd: require('node:os').tmpdir(), stdio: ['pipe', 'pipe', 'pipe', 'pipe'] });
let stderr = '', stdout = '', ready = false;
const send = message => { if (process.connected) process.send(message); };
child.on('spawn', () => {
  if (scenario === 'startup') { process.send({ pid: child.pid }, () => process.exit(0)); return; }
  send({ pid: child.pid });
  if (scenario !== 'before-input') child.stdin.end(JSON.stringify({ api: 1, source: 'return function() while true do end end', context: {} }));
});
child.stdin.on('error', () => {});
child.stdio[3].on('error', () => {});
function checkReady() {
  if (!ready && (stderr.includes('{"event":"running"}') || stdout.includes('{"ready":true}'))) { ready = true; send({ ready: true }); }
  if (stderr.length + stdout.length > 8192) { child.kill('SIGKILL'); process.exitCode = 1; }
}
child.stdout.on('data', b => { stdout += b; checkReady(); });
child.stderr.on('data', b => { stderr += b; checkReady(); });
child.on('close', (code, signal) => { send({ exit: true, code, signal, stderr }); process.disconnect(); });
child.on('error', () => { send({ error: 'spawn' }); process.disconnect(); });
process.on('message', message => {
  if (message === 'close-lifeline') child.stdio[3].destroy();
  if (message === 'stop') child.kill('SIGKILL');
});
process.on('disconnect', () => { child.stdio[3].destroy(); });
