// Focused native terminal acceptance over authenticated numeric loopback HTTP.
// Disposable repo/history only; no network Git remotes or user shell commands.
'use strict';
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/terminal-interrupt-' + crypto.randomUUID().replaceAll('-', ''));
const repo = path.join(root, 'client-中文'), profile = path.join(root, 'http-profile');
const branch = 'rocket-ui-test/terminal-interrupt';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [], prompt_return_ms: {} };
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const optionalHash = file => fs.existsSync(file) ? hash(file) : null;
const ordinaryHistory = path.join(process.env.APPDATA, 'Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt');
const ordinaryBefore = optionalHash(ordinaryHistory);
fs.mkdirSync(repo, { recursive: true }); fs.mkdirSync(path.join(root, 'hooks')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'hooks') });
const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8' }).trimEnd();
git(['init', '--initial-branch=' + branch]);
for (const [key, value] of [['user.name', 'Rocket Terminal Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', path.join(root, 'hooks')]]) git(['config', key, value]);
for (const file of ['baseline.txt', 'rocket-completion-中文.txt']) fs.writeFileSync(path.join(repo, file), 'unchanged synthetic fixture\n');
git(['add', '.']); git(['commit', '-m', 'Synthetic terminal interrupt fixture']); fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
const unchanged = ['.git/HEAD', '.git/index', '.git/config', 'baseline.txt', 'rocket-completion-中文.txt'].map(file => [file, hash(path.join(repo, file))]);
let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
const tail = () => state.terminal?.tail || '';
const count = marker => tail().split(/\r?\n/).filter(line => line.trim() === marker).length;
const prompt = () => tail().trimEnd().endsWith('\n>');
const line = () => tail().trimEnd().split('\n').at(-1);
function alive(pid) { try { process.kill(pid, 0); return true; } catch (e) { if (e.code === 'ESRCH') return false; throw e; } }
async function until(predicate, label = 'UI condition', timeout = 15000) {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state'); assert(!state.terminal?.error, state.terminal?.error);
    if (predicate(state)) return;
    assert(performance.now() - start < timeout, label + ' timed out: ' + JSON.stringify({ terminal: state.terminal && { pid: state.terminal.pid, running: state.terminal.running, tail: tail().slice(-2200) }, dialog: state.dialog?.kind }));
    await delay(50);
  }
}
async function settle() { await until(s => !s.busy && !s.input_pending && !s.tabs.some(t => t.loading || t.search.running)); }
async function click(target) { await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function probe(name) { await client.tool('rocket_terminal_probe', { probe: name, expected_tab: state.active_tab }); await settle(); }
async function command(text) { await client.tool('rocket_type', { target: 'terminal-command', text, expected_tab: state.active_tab }); await settle(); await client.tool('rocket_terminal_enter', { expected_tab: state.active_tab }); }
async function capture(name) {
  const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError);
  const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64'));
}
async function sameShell() {
  const before = count('ROCKET_SHELL_True'); await probe('interrupt-status');
  await until(() => count('ROCKET_SHELL_True') > before && count('ROCKET_STATE_42') > 0 && prompt(), 'same PowerShell PID and state'); assert.equal(state.terminal.pid, report.host_pid);
}
async function interrupt(name, button = false) {
  const start = performance.now(); if (button) await click('terminal-interrupt'); else await probe('cancel-line');
  await until(() => prompt(), name + ' prompt return'); report.prompt_return_ms[name] = Math.round((performance.now() - start) * 100) / 100;
  assert(report.prompt_return_ms[name] < 5000, name + ' was too slow'); assert.equal(count('ROCKET_INTERRUPT_UNEXPECTED'), 0); assert.equal(count('ROCKET_NATIVE_UNEXPECTED'), 0);
}
async function closeWindow() {
  await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit');
  if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window)); }
}
async function main() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe); report.helper_sha256 = hash(path.resolve('target/release/rocket-terminal-host.exe'));
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true, stdio: 'ignore' });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n'); report.endpoint = 'http://127.0.0.1:' + port + '/mcp';
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, detached: true, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    const starting = performance.now(); for (;;) { try { await client.initialize(); break; } catch (e) { if (e.code !== 'ECONNREFUSED') throw e; assert(performance.now() - starting < 15000, 'MCP startup timeout'); await delay(60); } }
    await client.tool('rocket_open', { path: repo }); await settle(); await click('view-changes'); await click('menu-terminal'); await click('terminal-new');
    await until(s => s.terminal?.running && prompt(), 'initial prompt'); report.host_pid = state.terminal.pid;
    await probe('interrupt-start'); await until(() => count('ROCKET_INTERRUPT_READY') === 1, 'sleep started'); report.shell_pid = Number(tail().match(/^ROCKET_SHELL_PID_(\d+)$/m)[1]);
    await interrupt('button', true); await sameShell(); passed('Interrupt button stops a running PowerShell command and retains actual shell PID and variables');
    await probe('interrupt-start'); await until(() => count('ROCKET_INTERRUPT_READY') === 2, 'second sleep started');
    await interrupt('native_ctrl_c'); await sameShell(); passed('Native Ctrl+C Copy event stops the command without replacing the shell');
    await probe('interrupt-native'); await until(() => count('ROCKET_NATIVE_READY') === 1, 'external command started'); report.foreground_pid = Number(tail().match(/^ROCKET_CHILD_PID_(\d+)$/m)[1]); assert(alive(report.foreground_pid));
    await interrupt('external_command'); assert(!alive(report.foreground_pid)); await sameShell(); passed('Ctrl+C ends the actual foreground child process and returns to the existing shell');
    await probe('resize-draft'); await until(() => line().includes('ROCKET_RESIZE_'), 'idle draft'); await interrupt('idle_line'); assert.equal(count('ROCKET_RESIZE_中文_🚀_é_OX'), 0); await sameShell();
    await probe('typing'); await until(() => count('ROCKET_TYPING_OK') === 1 && prompt(), 'typing/editing after interrupt');
    await probe('history-seed'); await until(() => count('ROCKET_HISTORY_SECOND') === 1 && prompt()); await probe('history-previous'); await until(() => line().includes("Write-Output 'ROCKET_HISTORY_FIRST'"), 'history recall');
    await probe('cancel-line'); await until(() => prompt()); await probe('completion-command'); await until(() => line().endsWith('Write-Output'), 'command completion');
    await probe('completion-argument'); await until(() => count('ROCKET_COMPLETION_OK') === 1 && prompt()); await probe('completion-path'); await until(() => line().includes('rocket-completion-中文.txt'), 'Unicode path completion');
    await probe('cancel-line'); await until(() => prompt()); await probe('unicode'); await until(() => count('ROCKET_UNICODE_中文 🚀 é') === 1 && prompt());
    passed('Idle-line cancellation, typing, history, command completion and Unicode path completion still work'); await capture('terminal-after-interrupt');
    await command("Write-Error 'ROCKET_EXPECTED_ERROR'"); await until(() => tail().includes('WriteErrorException') && prompt()); await sameShell(); await capture('terminal-error-recovery');
    await click('terminal-end'); await until(s => s.terminal.closed); assert(!alive(report.host_pid)); assert(!alive(report.shell_pid));
    await click('menu-terminal'); await click('terminal-new'); await until(s => s.terminal.running && prompt()); const secondHost = state.terminal.pid; assert.notEqual(secondHost, report.host_pid);
    await command('exit'); await until(s => s.terminal.closed); assert.equal(state.terminal.exit_code, 0); assert(!alive(secondHost));
    passed('Errors stay visible; End session reaps host and shell; a fresh session exits normally');
    for (const [file, before] of unchanged) assert.equal(hash(path.join(repo, file)), before, file + ' changed'); assert.equal(optionalHash(ordinaryHistory), ordinaryBefore, 'User history changed');
    await closeWindow(); await client.raw('POST', '/shutdown'); const stopped = performance.now(); while (service.exitCode === null) { assert(performance.now() - stopped < 10000, 'MCP did not shut down'); await delay(50); }
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(f => /^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f => fs.readFileSync(path.join(root, n, f), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); for (const operation of ['terminal_start', 'terminal_interrupt', 'terminal_exit']) assert(records.some(r => r.operation === operation), 'Missing log ' + operation);
    for (const text of ['ROCKET_INTERRUPT_', 'ROCKET_NATIVE_', '$global:', 'ROCKET_EXPECTED_ERROR', repo]) assert(!JSON.stringify(records).includes(JSON.stringify(text).slice(1, -1)), 'Private terminal payload in diagnostics');
    report.log_records = records.length; passed('Lifecycle/interrupt logs are present with zero dropped events and no commands/output; repository and user history unchanged'); report.passed = true;
  } catch (error) { report.error = String(error); if (client && state) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); fs.writeFileSync('work/terminal-interrupt-last-result', path.join(root, 'result.json')); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); } if (client) await client.finish();
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
