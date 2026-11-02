// Native Windows acceptance for automatic fetch and custom commands (0.44.0)
// through authenticated localhost MCP. Synthetic, owned repository whose remotes
// are bare repositories inside the fixture root; no network.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/commands-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/commands';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [], timings: {} };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
// The host runs automatic fetch only with this marker, here a 2-second test interval.
fs.writeFileSync(path.join(root, 'rocket-auto-fetch'), '2\n');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const hex = text => Buffer.from(text, 'utf8').toString('hex');

// Rocket runs the first git.exe on PATH, canonicalized; under MCP a custom command
// may use only that program.
function rocketGit() {
  for (const entry of (process.env.PATH || '').split(path.delimiter)) {
    const dir = entry.replace(/^"|"$/g, '');
    if (!path.isAbsolute(dir)) continue;
    const candidate = path.join(dir, 'git.exe');
    try { if (fs.statSync(candidate).isFile()) return '\\\\?\\' + fs.realpathSync.native(candidate); } catch {}
  }
  throw new Error('git.exe is not on PATH');
}

function fixture() {
  const repo = path.join(root, 'PRIVATE_COMMANDS'), hooks = path.join(root, 'hooks'), origin = path.join(root, 'origin.git');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const run = (cwd, args) => cp.execFileSync('git', args, { cwd, env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
  const git = args => run(repo, args), bare = args => run(root, ['--git-dir', origin, ...args]);
  const write = (file, body) => fs.writeFileSync(path.join(repo, file), body);
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Commands Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  write('a.txt', 'PRIVATE_1\n'); git(['add', '-A']); git(['commit', '-m', 'PRIVATE_COMMIT_1']);
  write('b.txt', 'PRIVATE_2\n'); git(['add', '-A']); git(['commit', '-m', 'PRIVATE_COMMIT_2']);
  run(root, ['init', '--quiet', '--bare', origin]);
  git(['remote', 'add', 'origin', origin]);
  git(['push', '--quiet', 'origin', branch, branch + ':refs/heads/gone']);
  git(['fetch', '--quiet', 'origin']);
  // Elsewhere, origin gains a branch and a tag and loses a branch. A configured
  // prune must still not delete the stale tracking branch.
  const head = git(['rev-parse', 'HEAD']);
  bare(['update-ref', 'refs/heads/arrived', head]); bare(['update-ref', '-d', 'refs/heads/gone']); bare(['tag', 'v-remote', head]);
  git(['config', 'fetch.prune', 'true']);
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo, origin, git, write, head };
}

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, auto_fetch: state.auto_fetch, write: state.write, notice: state.notice }));
    await delay(80);
  }
}
const tab = () => state?.tabs?.find(t => t.id === state.active_tab);
const fetchState = () => state.auto_fetch.tabs.find(t => t.tab === state.active_tab);
const tracked = name => tab().branches.some(b => b.reference === name);
async function settle() { return until(s => !s.busy && !s.input_pending && !s.write && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
function control(id) { return state.controls.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function scroll(target, direction, pixels) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); }
async function refusedClick(target, pattern) { await stable(); await assert.rejects(client.tool('rocket_click', { target, expected_tab: state.active_tab }), pattern); await settle(); }
async function refusedType(target, text, pattern) { await stable(); await assert.rejects(client.tool('rocket_type', { target, text, expected_tab: state.active_tab }), pattern); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) await click('dialog-cancel'); assert(!state.dialog, 'Dialog still open'); }
async function open(f) { await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920); await closeConsole(); }
async function menuItem(id) { await click('menu-repository'); await scroll('repository-menu-scroll', 'up', 2000); for (let i = 0; i < 8; i++) { const c = control(id); if (c && c.rect[3] - c.rect[1] >= 28) break; await scroll('repository-menu-scroll', 'down', 250); } await click(id); }
async function commandsMenu(id) { await menuItem('repository-custom-commands'); await click(id); }
async function preferences() { await click('menu-edit'); await click('preferences'); await stable(); assert(state.dialog && control('pref-auto-fetch'), 'Preferences did not open'); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
// Processes whose command line contains `text` (the slow upload-pack's unique sleep).
function processes(text) {
  const script = '@(Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($env:ROCKET_PROBE) -and $_.Name -ne "powershell.exe" } | ForEach-Object { $_.ProcessId }) -join ","';
  const out = cp.execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { env: { ...process.env, ROCKET_PROBE: text }, encoding: 'utf8', windowsHide: true }).trim();
  return out ? out.split(',').map(Number) : [];
}
async function untilProcesses(text, present, label) { const start = performance.now(); while ((processes(text).length > 0) !== present) { assert(performance.now() - start < WAIT_MS, label + ' timed out'); await delay(200); } }
async function activityAfter(count) { await until(s => !s.write && s.activities.length > count, 'Command activity'); return state.activities[state.activities.length - 1]; }

async function automaticFetch(f) {
  await settle();
  assert.equal(state.auto_fetch.minutes, 0); assert.equal(state.auto_fetch.blocked, false); assert.deepEqual(state.auto_fetch.tabs, []);
  assert(!tracked('refs/remotes/origin/arrived') && tracked('refs/remotes/origin/gone'));
  await preferences(); await capture('auto-fetch-preferences-off');
  await click('pref-auto-fetch'); assert.equal(state.auto_fetch.minutes, 15);
  await click('pref-auto-fetch-5'); assert.equal(state.auto_fetch.minutes, 5);
  await capture('auto-fetch-preferences-on'); await click('dialog-submit');
  const started = performance.now();
  await until(s => { const t = s.auto_fetch.tabs.find(x => x.tab === s.active_tab); return t?.finished && s.tabs.find(x => x.id === s.active_tab).branches.some(b => b.reference === 'refs/remotes/origin/arrived'); }, 'First automatic fetch');
  report.timings.first_fetch_ms = Math.round(performance.now() - started);
  assert.equal(fetchState().failures, 0);
  assert.equal(f.git(['rev-parse', 'HEAD']), f.head); assert.equal(f.git(['rev-parse', 'refs/heads/' + branch]), f.head);
  assert.equal(f.git(['rev-parse', 'refs/remotes/origin/gone']), f.head, 'Automatic fetch pruned a branch');
  assert.equal(f.git(['tag', '--list', 'v-remote']), '', 'Automatic fetch fetched a tag');
  assert(fs.readFileSync(path.join(f.repo, '.git/FETCH_HEAD'), 'utf8').includes("'arrived'"), 'FETCH_HEAD was not written by the automatic fetch');
  await capture('auto-fetch-arrived');
  passed('Automatic fetch, turned on in Preferences, fetches on its interval and the new remote branch appears; nothing is merged, pruned (despite fetch.prune) or tag-fetched');

  // A remote that fails shows a quiet status-bar note and is retried.
  const broken = path.join(root, 'not-a-repository'); fs.mkdirSync(broken);
  f.git(['remote', 'add', 'broken', broken]); await click('refresh');
  await until(s => s.auto_fetch.tabs.find(x => x.tab === s.active_tab)?.failures >= 1 && s.controls.some(c => c.id === 'auto-fetch-failed'), 'Quiet failure');
  assert(tracked('refs/remotes/origin/arrived'));
  await capture('auto-fetch-failed');
  f.git(['remote', 'remove', 'broken']); await click('refresh');
  await until(s => s.auto_fetch.tabs.find(x => x.tab === s.active_tab)?.finished && s.auto_fetch.tabs.find(x => x.tab === s.active_tab).failures === 0 && !s.controls.some(c => c.id === 'auto-fetch-failed'), 'Recovered after the next interval');
  passed('A failing remote shows "Automatic fetch failed" in the status bar without a dialog, and the next interval clears it once the remote is fixed');

  // A network remote blocks automatic fetch on this host.
  f.git(['remote', 'add', 'network', 'https://example.invalid/rocket.git']); await click('refresh');
  await until(s => s.auto_fetch.blocked && s.auto_fetch.tabs.length === 0, 'Blocked for a network remote');
  await preferences(); await refusedClick('pref-auto-fetch', /marker and local fixture remotes/); await click('dialog-submit');
  f.git(['remote', 'remove', 'network']); await click('refresh');
  await until(s => !s.auto_fetch.blocked, 'Unblocked');
  passed('Under MCP a network remote in an open repository blocks automatic fetch and its Preferences controls');

  // A slow fetch: Stop ends it and its processes.
  const slow = 'sleep 29.' + crypto.randomInt(1000, 9999);
  f.git(['config', 'remote.origin.uploadpack', slow + '; git-upload-pack']);
  await until(s => s.auto_fetch.tabs.find(x => x.tab === s.active_tab)?.running, 'Slow fetch running');
  await untilProcesses(slow, true, 'Slow upload-pack started');
  await capture('auto-fetch-running');
  let stop = performance.now(); await click('auto-fetch-stop');
  await until(s => !s.auto_fetch.tabs.find(x => x.tab === s.active_tab)?.running, 'Stopped');
  await untilProcesses(slow, false, 'Slow upload-pack stopped');
  report.timings.stop_ms = Math.round(performance.now() - stop);
  passed('Stop in the status bar ends a running automatic fetch and the processes it started');

  // Starting an operation in the repository stops its fetch instead of waiting.
  await until(s => s.auto_fetch.tabs.find(x => x.tab === s.active_tab)?.running, 'Slow fetch running again');
  await untilProcesses(slow, true, 'Slow upload-pack started again');
  f.git(['config', '--unset', 'remote.origin.uploadpack']);
  f.write('d.txt', 'PRIVATE_STAGED\n'); await click('view-changes'); await click('refresh');
  await until(s => s.controls.some(c => c.id === 'change-unstaged-' + hex('d.txt')), 'Changed file listed');
  const activities = state.activities.length; stop = performance.now();
  await click('stage-all');
  const staged = await activityAfter(activities);
  report.timings.write_after_fetch_ms = Math.round(performance.now() - stop);
  assert(staged.command_succeeded, JSON.stringify(staged));
  assert(report.timings.write_after_fetch_ms < 20000, 'The write waited for the fetch');
  await untilProcesses(slow, false, 'Fetch stopped for the write');
  assert.equal(f.git(['diff', '--cached', '--name-only']), 'd.txt');
  passed('Starting an operation stops the repository\'s running automatic fetch and its processes; the operation then runs at once');

  await preferences(); await click('pref-auto-fetch'); assert.equal(state.auto_fetch.minutes, 0); await click('dialog-submit');
  await until(s => s.auto_fetch.tabs.length === 0, 'Turned off');
  passed('Turning automatic fetch off clears every schedule');
}

async function customCommands(f, git) {
  await commandsMenu('command-edit');
  assert.equal(state.preferences.page, 'commands'); assert.equal(state.dialog.command_editor.count, 0);
  await click('dialog-commands-new'); await type('dialog-commands-name', 'Show commit');
  await refusedType('dialog-commands-program', 'C:\\Windows\\System32\\cmd.exe', /fixed Git test commands/);
  await type('dialog-commands-program', git);
  await refusedType('dialog-commands-arguments', 'status', /fixed Git test commands/);
  await type('dialog-commands-arguments', 'log\n-1\n--format=%H\n{commit}');
  for (const [name, args] of [['Missing ref', 'rev-parse\n--verify\nrefs/heads/rocket-missing'], ['List file', 'ls-files\n--\n{file}']]) {
    await click('dialog-commands-new'); await type('dialog-commands-name', name); await type('dialog-commands-program', git); await type('dialog-commands-arguments', args);
  }
  assert.equal(state.dialog.command_editor.count, 3); assert.equal(state.dialog.command_editor.problem, null);
  await capture('commands-editor'); await click('dialog-commands-save'); await click('dialog-close');
  assert.equal(state.dialog, null);
  assert.deepEqual(state.commands.map(c => [c.name, c.test_command]), [['Show commit', true], ['Missing ref', true], ['List file', true]]);
  passed('Custom commands are edited and saved in Preferences; under MCP any program but Rocket\'s Git, or arguments outside the fixed test list, are refused');

  await click('view-commits'); await click('commit-' + f.head);
  await commandsMenu('command-run-0');
  assert.equal(state.dialog.kind, 'run-command');
  assert.deepEqual(state.dialog.run_command, { ready: true, action: 'program', name: 'Show commit', historical_file: false, arguments: ['log', '-1', '--format=%H', f.head] });
  await capture('command-confirm');
  let count = state.activities.length; await click('dialog-submit');
  let activity = await activityAfter(count);
  assert.equal(activity.summary, 'Show commit finished.'); assert(activity.command_succeeded);
  assert(activity.details.includes(f.head), JSON.stringify(activity.details));
  await capture('command-output');
  passed('Running a command shows the exact program and arguments first, runs only after Run, and reports its output in Activity');

  await commandsMenu('command-run-1');
  count = state.activities.length; await click('dialog-submit');
  activity = await activityAfter(count);
  assert.equal(activity.summary, 'Missing ref exited with code 128.'); assert(!activity.command_succeeded);
  assert(activity.details.includes('Errors:'), JSON.stringify(activity.details));
  passed('A command that fails reports its exit code and error output');

  await commandsMenu('command-run-2');
  assert.deepEqual(state.dialog.run_command, { ready: false }); assert(!control('dialog-submit')?.enabled);
  await capture('command-unavailable'); await closeDialog();
  await click('view-changes'); await click('change-staged-' + hex('d.txt'));
  await commandsMenu('command-run-2');
  const listed = state.dialog.run_command;
  assert(listed.ready && listed.name === 'List file'); assert.deepEqual(listed.arguments.slice(0, 2), ['ls-files', '--']);
  // A plain path, not Rocket's canonical \\?\ form, which many programs cannot open.
  assert.equal(listed.arguments[2].toLowerCase(), path.join(f.repo, 'd.txt').toLowerCase());
  count = state.activities.length; await click('dialog-submit');
  activity = await activityAfter(count);
  assert(activity.command_succeeded && activity.details.includes('d.txt'), JSON.stringify(activity));
  passed('A command whose placeholder has no value cannot run; selecting a file makes {file} its full path');
}

async function run() {
  let f;
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    f = fixture(); const git = rocketGit(); report.git = git;
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await open(f);
    await automaticFetch(f); await customCommands(f, git);
    // Both settings are saved with Rocket's state.
    await closeWindow(); await open(f);
    assert.equal(state.auto_fetch.minutes, 0); assert.deepEqual(state.commands.map(c => c.name), ['Show commit', 'Missing ref', 'List file']);
    passed('Custom commands and the automatic fetch setting survive closing and reopening the window');
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'Show commit', 'Missing ref', 'rocket-missing', f.head]) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    const operations = new Set(records.filter(r => r.event === 'request_start').map(r => r.operation));
    for (const operation of ['auto-fetch', 'custom-command']) assert(operations.has(operation), 'No ' + operation + ' request in diagnostics');
    report.log_records = records.length; passed('Diagnostics: zero dropped events, automatic fetch and custom command requests recorded without names, arguments or commit IDs'); report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync('work/commands-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
