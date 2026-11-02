// Native Windows check of 0.50.0 through authenticated localhost MCP: issue-tracker links in
// commit messages (Repository Settings → Issue links, a global rule, opening links, an
// external config edit) and Test connection in Add remote, Edit remote URL and Clone
// (success, failure, Stop and the 20-second limit, the last two held by the Git relay gate).
// Synthetic owned repositories and local paths only; links are recorded, never opened.
// Usage: node tools/tests/test-mcp-issue-links.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const { gitRelay } = require('./git-relay.cjs');
const root = path.resolve('work/mcp-fixtures/links-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/links';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template'));
const globalConfig = path.join(root, 'global.config');
// A shared rule from global config; repository rules of the same name would replace it.
fs.writeFileSync(globalConfig, '[rocket "issuelink.ops"]\n\tpattern = \\\\bOPS-(\\\\d+)\\\\b\n\turl = https://ops.example.com/t/{1}\n');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: globalConfig, GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const run = (file, args) => cp.execFileSync(file, args, { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();

const relay = gitRelay(root, env, 'tools/helpers/file-history-preview/git-relay.rs'), { gate, marker } = relay;
const repo = path.join(root, 'PRIVATE_LINKS 文'), hooks = path.join(root, 'hooks');
const git = args => run('git', ['-C', repo, ...args]);
fs.mkdirSync(repo); fs.mkdirSync(hooks);
git(['init', '--initial-branch=' + branch]);
for (const [key, value] of [['user.name', 'Rocket Links Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
fs.writeFileSync(path.join(repo, 'a.txt'), 'PRIVATE_A\n'); git(['add', '-A']); git(['commit', '-m', 'PRIVATE first']);
fs.writeFileSync(path.join(repo, 'b.txt'), 'PRIVATE_B\n'); git(['add', '-A']);
git(['commit', '-m', 'PROJ-12: fix the PRIVATE crash', '-m', 'Refs #7 and OPS-3.']);
const linked = git(['rev-parse', 'HEAD']);
const bare = path.join(root, 'origin 文.git');
run('git', ['clone', '--bare', '--quiet', repo, bare]);
run('git', ['-C', bare, 'branch', 'feature/x']);
git(['remote', 'add', 'origin', bare]);
git(['config', 'remote.origin.fetch', '+refs/heads/' + branch + ':refs/remotes/origin/' + branch]);
const notRepository = path.join(root, 'not a repository'); fs.mkdirSync(notRepository);
fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
async function until(predicate, label = 'UI state', limit = WAIT_MS) {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < limit, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, connection: state.dialog?.connection, notice: state.notice }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
const control = id => state.controls.find(c => c.id === id);
const tab = () => state.tabs.find(t => t.id === state.active_tab);
const latest = () => state.activities.at(-1);
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target, wait = true) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); if (wait) await settle(); else await until(s => !s.input_pending, 'Delivered ' + target); }
async function type(target, text) { await stable(); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function refused(name, args, pattern) { let error; try { await client.tool(name, { ...args, expected_tab: state.active_tab }); } catch (e) { error = String(e); } assert(error && pattern.test(error), 'Expected refusal ' + pattern + ', got ' + error); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) await click('dialog-cancel'); assert(!state.dialog, 'Dialog still open'); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
async function settings() {
  await click('menu-repository');
  await click('repository-settings-menu');await stable();
  const visible = () => { const c = control('repository-settings'); return c && c.rect[3] - c.rect[1] >= 28; };
  for (let i = 0; i < 8 && !visible(); i++) { await client.tool('rocket_scroll', { target: 'repository-menu-scroll', direction: 'down', pixels: 250, expected_tab: state.active_tab }); await settle(); }
  await click('repository-settings'); assert.equal(state.dialog.kind, 'identity');
  await click('dialog-settings-issue-links');
  await until(s => s.dialog?.issue_links?.loaded && s.dialog.issue_links.shown, 'Issue links page');
}
const links = () => state.dialog.issue_links;
const opened = () => state.github.opened_urls;
const connection = () => state.dialog?.connection;

async function editor() {
  await settings();
  assert.deepEqual(links().inherited, [{ scope: 'Global', name: 'ops' }]);
  assert.equal(links().rules.length, 0);
  await capture('issue-links-empty');
  await click('dialog-issue-add');
  await type('dialog-issue-name-0', 'jira');
  await type('dialog-issue-pattern-0', '\\b[A-Z][A-Z0-9]+-\\d+\\b');
  await type('dialog-issue-url-0', 'http://jira.example.com/browse/{id}');
  assert.equal(links().rules[0].valid, false); assert(!control('dialog-submit').enabled, 'An http link must not be savable');
  await type('dialog-issue-url-0', 'https://jira.example.com/browse/{id}');
  await click('dialog-issue-add');
  await type('dialog-issue-name-1', 'redmine');
  await type('dialog-issue-pattern-1', '#(\\d+)');
  await type('dialog-issue-url-1', 'https://redmine.example.com/issues/{1}');
  assert(links().rules.every(r => r.valid)); assert.equal(links().problem, null);
  await type('dialog-issue-sample', 'PROJ-12 fixes #7 and OPS-3');
  await capture('issue-links-editor');
  const before = hash(path.join(repo, '.git/config'));
  await click('dialog-submit');
  assert(!state.dialog && latest().command_succeeded, JSON.stringify(latest()));
  assert.notEqual(hash(path.join(repo, '.git/config')), before);
  assert.equal(git(['config', 'rocket.issuelink.jira.url']), 'https://jira.example.com/browse/{id}');
  assert.equal(git(['config', 'rocket.issuelink.redmine.pattern']), '#(\\d+)');
  await until(s => s.tabs[0].issue_links.rules === 3 && !s.tabs[0].issue_links.loading, 'Rules applied');
  passed('Repository Settings → Issue links lists the global rule, refuses an http link, tries a sample and saves two rules to the repository config');
}

async function details() {
  await click('view-commits');
  await click('commit-' + linked);
  for (const n of [0, 1, 2]) assert(control('issue-link-' + n), 'issue-link-' + n);
  assert(!control('issue-link-3'));
  await capture('issue-links-details');
  for (const n of [0, 1, 2]) await click('issue-link-' + n);
  assert.deepEqual(opened(), ['https://jira.example.com/browse/PROJ-12', 'https://redmine.example.com/issues/7', 'https://jira.example.com/browse/OPS-3']);
  passed('Commit details link PROJ-12 in the subject and #7 and OPS-3 in the body; the repository rule wins over the global one; links open as validated https URLs');
  // An edit made outside Rocket applies on a manual refresh.
  git(['config', 'rocket.issuelink.jira.url', 'https://tracker.example.com/{id}']);
  await click('refresh');
  await until(s => !s.tabs[0].issue_links.loading && !s.busy, 'Reloaded rules');
  await click('commit-' + linked);
  await click('issue-link-0');
  assert.equal(opened().at(-1), 'https://tracker.example.com/PROJ-12');
  passed('A rule changed with git config takes effect after a refresh');
  // Removing every repository rule leaves only the global one.
  await settings();
  await click('dialog-issue-remove-1'); await click('dialog-issue-remove-0');
  await click('dialog-submit');
  assert(latest().command_succeeded);
  assert(!fs.readFileSync(path.join(repo, '.git/config'), 'utf8').includes('issuelink'));
  await until(s => s.tabs[0].issue_links.rules === 1 && !s.tabs[0].issue_links.loading, 'Global rule only');
  await click('commit-' + linked);
  assert(control('issue-link-0') && !control('issue-link-1'));
  await click('issue-link-0');
  assert.equal(opened().at(-1), 'https://ops.example.com/t/3');
  passed('Removing the repository rules keeps only the global OPS rule, and the global config file is unchanged');
}

async function connections() {
  await click('view-remotes');
  await click('remote-add');
  await type('dialog-remote-name', 'second');
  await refused('rocket_type', { target: 'dialog-remote-url', text: 'https://example.invalid/r.git' }, /fixture|outside|local/i);
  await type('dialog-remote-url', bare);
  await click('dialog-test-connection');
  await until(s => s.dialog?.connection?.result, 'Test result');
  assert.deepEqual([connection().result.connected, connection().result.branches, connection().result.default_branch], [true, 2, branch]);
  await capture('test-connection-ok');
  await type('dialog-remote-url', notRepository);
  assert.equal(connection(), null);
  await click('dialog-test-connection');
  await until(s => s.dialog?.connection?.result, 'Failure result');
  assert.equal(connection().result.connected, false);
  assert(!['cancelled', 'timed_out'].includes(connection().result.code), connection().result.code);
  await capture('test-connection-failed');
  // Stop: the relay holds ls-remote until the gate goes away.
  fs.writeFileSync(gate, 'ls-remote');
  await type('dialog-remote-url', bare);
  await click('dialog-test-connection', false);
  await until(s => s.dialog?.connection?.running && fs.existsSync(marker), 'Held test');
  await capture('test-connection-running');
  const stopped = performance.now();
  await click('dialog-test-connection-stop', false);
  await until(s => s.dialog?.connection?.result, 'Stopped test');
  report.stop_ms = Math.round(performance.now() - stopped);
  assert.equal(connection().result.code, 'cancelled');
  assert(report.stop_ms < 10000, 'Stop took ' + report.stop_ms);
  fs.unlinkSync(marker);
  // The time limit: nothing answers for 20 seconds.
  await type('dialog-remote-url', bare + path.sep);
  const started = performance.now();
  await click('dialog-test-connection', false);
  await until(s => s.dialog?.connection?.result, 'Timed-out test', 45000);
  report.limit_ms = Math.round(performance.now() - started);
  fs.unlinkSync(gate);
  assert.equal(connection().result.code, 'timed_out');
  assert(report.limit_ms >= 19000 && report.limit_ms < 35000, 'Limit after ' + report.limit_ms);
  await capture('test-connection-timed-out');
  await closeDialog();
  assert.equal(git(['remote']), 'origin');
  passed('Add remote tests the typed path without saving it: connected with 2 branches, a failure code for a non-repository, Stop cancels a held test, and a silent remote stops at 20 seconds');
  await click('remote-edit-fetch-0');
  await click('dialog-test-connection');
  await until(s => s.dialog?.connection?.result, 'Edit remote result');
  assert.equal(connection().result.connected, true);
  await closeDialog();
  await click('menu-file'); await click('file-clone');
  assert.equal(state.dialog.kind, 'clone');
  await refused('rocket_type', { target: 'dialog-clone-source', text: 'https://example.invalid/r.git' }, /fixture|outside|local/i);
  await type('dialog-clone-source', bare);
  await click('dialog-test-connection');
  await until(s => s.dialog?.connection?.result, 'Clone source result');
  assert.equal(connection().result.connected, true);
  await capture('test-connection-clone');
  await closeDialog();
  passed('Edit remote URL and Clone test their sources too; automation refuses non-fixture URLs before any connection');
}

async function main() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    // The existing Git relay pauses a command whose arguments contain the gate's text.
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env: relay.env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: repo }); await settle(); relay.check(state); await resize(1480, 920); await closeConsole();
    assert.equal(tab().issue_links.rules, 1, 'The global rule applies before any repository rule');
    await editor();
    await details();
    await connections();
    await resize(780, 620); await settings(); await capture('issue-links-narrow'); await closeDialog(); await resize(1480, 920);
    const globalAfter = fs.readFileSync(globalConfig, 'utf8'); assert(globalAfter.includes('issuelink.ops') && !globalAfter.includes('jira'));
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'PROJ-12', 'example.com', 'origin 文', 'not a repository']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    for (const op of ['issue-links-read', 'issue-links-save', 'issue-link-open', 'remote-check']) {
      const starts = records.filter(r => r.event === 'request_start' && r.operation === op);
      assert(starts.length, 'Missing ' + op);
      for (const s of starts) assert(records.some(r => r.event === 'request_end' && r.session === s.session && r.request === s.request), 'Unpaired ' + op);
    }
    assert(records.some(r => r.operation === 'remote-check' && r.outcome === 'cancelled'), 'No cancelled remote-check outcome');
    assert(records.some(r => r.operation === 'remote-check' && r.outcome === 'timed_out'), 'No timed-out remote-check outcome');
    report.log_records = records.length; passed('Diagnostics: paired requests, cancelled and timed-out outcomes, zero dropped events, no paths, IDs or URLs');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    if (fs.existsSync(gate)) fs.unlinkSync(gate);
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); }
    fs.writeFileSync('work/issue-links-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
  }
}
main().catch(() => { process.exitCode = 1; });
