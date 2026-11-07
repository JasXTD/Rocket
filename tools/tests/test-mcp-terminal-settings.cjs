// Native Windows acceptance for Preferences → Terminal and the known tools on External tools,
// through authenticated localhost MCP. An MCP window can't change the shell and always runs
// Windows PowerShell, so real Git Bash and Command Prompt are covered by rocket-terminal's
// tests. The tools "found" here are placeholder files in the fixture's install folders.
// Run from the repository root, under the build and MCP lock.
if (process.platform !== 'win32') { console.log('Terminal settings runner: Windows-only; skipped on this platform.'); process.exit(0); }
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/terminal-settings-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), shots = path.join(root, 'shots'), branch = 'rocket-ui-test/terminal-settings';
const report = { passed: false, root, cases: [], captures: [] };
fs.mkdirSync(shots, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
// Explicit local consent: this runner creates only synthetic shell data.
fs.writeFileSync(path.join(root, 'rocket-private-state'), 'synthetic-content-only-v1\n');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
const local = path.join(root, 'local'), programs = path.join(root, 'programs'), programs86 = path.join(root, 'programs-x86');
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_TERMINAL_PROMPT: '0', LOCALAPPDATA: local, ProgramFiles: programs, 'ProgramFiles(x86)': programs86 });
const placeholder = (folder, file) => { fs.mkdirSync(folder, { recursive: true }); const full = path.join(folder, file); fs.writeFileSync(full, 'placeholder, never run\n'); return full; };
const code = placeholder(path.join(local, 'Programs', 'Microsoft VS Code'), 'Code.exe');
fs.mkdirSync(programs86, { recursive: true });
const run = (cwd, args) => cp.execFileSync('git', args, { cwd, env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trim();
function fixture() {
  const repo = path.join(root, 'PRIVATE_SHELL'), hooks = path.join(root, 'hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  run(repo, ['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Shell Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) run(repo, ['config', key, value]);
  fs.writeFileSync(path.join(repo, 'base.txt'), 'base\n'); run(repo, ['add', '-A']); run(repo, ['commit', '-m', 'PRIVATE_COMMIT']);
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo };
}
let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
async function until(predicate, label) {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < 45000, 'Timed out: ' + label);
    await delay(50);
  }
}
const settle = () => until(s => !s.busy && !s.input_pending && !s.write && !(s.tabs || []).some(t => t.loading), 'Settled');
const control = id => state.controls.find(c => c.id === id);
const external = () => state.external_tools;
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3 && !s.input_pending; }, 'Stable controls'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const item = result.content.find(c => c.type === 'image'); assert(item); const file = path.join(shots, name + '.png'); fs.writeFileSync(file, Buffer.from(item.data, 'base64')); report.captures.push(file); }
async function preferences(page) { await click('menu-edit'); await click('preferences'); await until(() => !!control('pref-page-' + page), 'Preferences'); await click('pref-page-' + page); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) { await stable(); await click('dialog-close'); } assert(!state.dialog, 'Dialog closed'); }

async function lockedPage() {
  await preferences('terminal');
  assert.equal(state.terminal_settings.editable, false);
  assert.equal(state.terminal_settings.choice, 'windows-powershell');
  await stable();
  for (const id of ['shell-choice', 'shell-program', 'shell-arguments']) assert(control(id), 'Missing ' + id);
  assert.equal(control('shell-choice').enabled, false, 'The dropdown is off in an MCP window');
  await capture('terminal-page-locked');
  await assert.rejects(() => client.tool('rocket_click', { target: 'shell-choice', expected_tab: state.active_tab }), /user preference/);
  await assert.rejects(() => client.tool('rocket_click', { target: 'shell-save', expected_tab: state.active_tab }), /user preference/);
  await assert.rejects(() => client.tool('rocket_type', { target: 'shell-program', text: 'C:\\Windows\\System32\\notepad.exe', expected_tab: state.active_tab }), /user preference/);
  await settle();
  assert.equal(state.terminal_settings.dirty, false);
  assert.equal(state.terminal_settings.choice, 'windows-powershell');
  await closeDialog();
  passed('MCP sees the Terminal page locked and is refused every change to the shell');
}

async function powershellWindow() {
  await click('menu-terminal'); await click('terminal-new');
  await until(s => s.terminal?.running && s.terminal.tail.includes('PS '), 'PowerShell prompt');
  assert.equal(state.terminal.shell, 'Windows PowerShell');
  await capture('terminal-windows-powershell');
  await click('terminal-end'); await until(s => s.terminal.closed, 'Session ended');
  passed('An MCP window runs Windows PowerShell and names it in its state');
}

async function addList() {
  await preferences('tools');
  await click('tools-kind-2');
  await click('tools-profile-add');
  await until(() => !!control('tools-known-vscode'), 'Add list');
  await stable();
  for (const id of ['tools-known-beyond-compare', 'tools-known-winmerge', 'tools-known-kdiff3', 'tools-known-meld', 'tools-known-p4merge', 'tools-known-tortoisegitmerge', 'tools-known-sublime-merge', 'tools-known-araxis-merge', 'tools-known-empty']) assert(control(id), 'Missing ' + id);
  assert(!control('tools-known-notepad-plus-plus'), 'An editor is not listed for merges');
  await capture('tools-add-list');
  await click('tools-known-vscode');
  await stable();
  assert.deepEqual(external().profiles, ['VS Code'], 'The empty Default took it');
  assert.deepEqual(external().form, { name: 'VS Code', executable: code, arguments: '-n\n--wait\n--merge\n{theirs}\n{ours}\n{base}\n{result}' });
  assert.equal(external().executable, '', 'The saved profile stays as it was');
  assert.equal(external().settings_dirty, true, 'Nothing is saved until Save profiles');
  assert.equal(external().running, false, 'Choosing a tool runs nothing');
  assert.equal(external().error, null);
  await capture('tools-filled');
  await click('tools-save-settings');
  assert.equal(external().settings_dirty, false);
  assert.equal(external().executable, code);
  passed('Add lists the known tools, and choosing one only fills in the form');

  await click('tools-profile-add');
  await until(() => !!control('tools-known-winmerge'), 'Add list again');
  await click('tools-known-winmerge');
  await stable();
  assert.deepEqual(external().profiles, ['VS Code', 'WinMerge'], 'A profile in use stays as it is');
  assert.equal(external().selected_profile, 1);
  assert.equal(external().default_profile, 0, 'The default stays');
  assert.deepEqual(external().form, { name: 'WinMerge', executable: '', arguments: '/e\n/u\n/s-\n{theirs}\n{base}\n{ours}\n/o\n{result}' }, 'Not found leaves Program for Browse');
  await capture('tools-not-found');
  await click('tools-profile-add');
  await until(() => !!control('tools-known-vscode'), 'Add list a third time');
  await capture('tools-already-added');
  await click('tools-known-vscode');
  await stable();
  assert.deepEqual(external().profiles, ['VS Code', 'WinMerge'], 'A tool is never added twice');
  assert.equal(external().selected_profile, 0);
  await click('tools-discard-settings');
  await stable();
  assert.deepEqual(external().profiles, ['VS Code']);
  await click('tools-profile-add');
  await until(() => !!control('tools-known-empty'), 'Add list a fourth time');
  await click('tools-known-empty');
  await stable();
  assert.equal(external().profiles.length, 2);
  assert.equal(external().form.executable, '');
  assert.equal(external().form.arguments, '{base}\n{ours}\n{theirs}\n{result}');
  await click('tools-discard-settings');
  await closeDialog();
  passed('Not found fills the rest, a tool already here selects its profile, and Empty profile is the plain form');
}

(async () => {
  const exe = path.resolve(process.env.ROCKET_MCP_EXE || 'target/release/rocket-mcp.exe');
  const f = fixture();
  cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
  const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
  fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
  const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log);
  client = new RocketHttpClient(profile);
  for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
  await client.initialize();
  try {
    await client.tool('rocket_open', { path: f.repo }); await settle();
    await client.tool('rocket_resize', { width: 1480, height: 920 }); await delay(300); await settle();
    await lockedPage();
    await powershellWindow();
    await addList();
    await client.tool('rocket_close');
    report.passed = true;
  } catch (error) {
    report.failure = error.stack; try { await capture('failure'); } catch {} throw error;
  } finally {
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    fs.writeFileSync('work/terminal-settings-last-result.txt', path.join(root, 'result.json'));
    fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2));
    console.log(JSON.stringify(report, null, 2));
    await delay(500);
    if (service && service.exitCode === null) service.kill();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
