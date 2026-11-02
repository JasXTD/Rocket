const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { randomUUID } = require('node:crypto');
const { spawnSync } = require('node:child_process');
const { RocketMcpClient } = require('../mcp/mcp-client.cjs');

const id = randomUUID().replaceAll('-', '');
const root = path.resolve('work/mcp-fixtures', 'lifecycle-' + id);
const repo = path.join(root, 'client');
const hooks = path.join(root, 'hooks');
const executable = path.join(__dirname, '..', '..', 'target', 'release', 'rocket-mcp.exe');
for (const dir of [repo, hooks, path.join(root, 'home')]) fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|GCM_|ROCKET_|SSH_|CURL_|SSL_CERT_)|_PROXY$|^NETRC$/i.test(key)) delete env[key];
Object.assign(env, { HOME: path.join(root, 'home'), GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'),
  GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_TEMPLATE_DIR: hooks,
  GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_OPTIONAL_LOCKS: '0', GCM_INTERACTIVE: '0', LC_ALL: 'C' });
function git(args) {
  const r = spawnSync('git.exe', ['-C', repo, ...args], { env, encoding: 'utf8', windowsHide: true, timeout: 15000 });
  assert.equal(r.status, 0, 'Fixture Git failed: ' + r.stderr); return r.stdout;
}
const branch = 'rocket-ui-test/lifecycle-' + id;
git(['init', '--initial-branch=' + branch]);
for (const [key, value] of [['user.name', 'Rocket MCP Test'], ['user.email', 'rocket@example.invalid'],
  ['core.hooksPath', hooks], ['core.autocrlf', 'false'], ['commit.gpgsign', 'false']]) git(['config', key, value]);
fs.writeFileSync(path.join(repo, '.git', 'rocket-ui-fixture'), branch);
fs.writeFileSync(path.join(repo, 'file'), 'base\n'); git(['add', '--', 'file']); git(['commit', '-m', 'Synthetic base']);
fs.writeFileSync(path.join(repo, 'file'), 'staged\n'); git(['add', '--', 'file']);
fs.writeFileSync(path.join(repo, 'file'), 'working\n');
const before = { head: git(['rev-parse', 'HEAD']), entries: git(['ls-files', '--stage', '-z']), bytes: fs.readFileSync(path.join(repo, 'file'), 'utf8') };
const heartbeat = path.join(root, 'hook-heartbeat');
const release = path.join(root, 'hook-release');
const quote = s => "'" + s.replaceAll('\\', '/').replaceAll("'", "'\"'\"'") + "'";
fs.writeFileSync(path.join(hooks, 'pre-commit'), `#!/bin/sh\nwhile test ! -f ${quote(release)}; do printf x >> ${quote(heartbeat)}; sleep 0.1; done\n`);
const report = { passed: false, root, cases: [] };
let mcp;
(async () => {
  try {
    mcp = new RocketMcpClient(executable, root, env);
    await mcp.initialize(); assert.equal((await mcp.tool('rocket_state')).window_count, 0);
    await mcp.finish(); report.cases.push('EOF before opening exits without creating a window');
    mcp = new RocketMcpClient(executable, root, env);
    await mcp.initialize(); await mcp.tool('rocket_open', { path: repo });
    let state; const deadline = Date.now() + 30000;
    do { state = await mcp.tool('rocket_wait', { timeout_ms: 10000 }); } while (state.busy && Date.now() < deadline);
    assert.equal(state.busy, false);
    const expected_tab = state.active_tab;
    await mcp.tool('rocket_click', { target: 'view-changes', expected_tab });
    await mcp.tool('rocket_type', { target: 'commit-message', text: 'Disconnect test draft', expected_tab });
    await mcp.tool('rocket_click', { target: 'commit', expected_tab });
    const hookDeadline = Date.now() + 10000;
    while (!fs.existsSync(heartbeat) && Date.now() < hookDeadline) await new Promise(r => setTimeout(r, 50));
    assert(fs.existsSync(heartbeat), 'Commit did not reach the owned blocking hook');
    assert((await mcp.tool('rocket_state')).busy);
    await mcp.finish();
    assert.deepEqual({ head: git(['rev-parse', 'HEAD']), entries: git(['ls-files', '--stage', '-z']), bytes: fs.readFileSync(path.join(repo, 'file'), 'utf8') }, before);
    assert(!fs.existsSync(path.join(repo, '.git', 'index.lock')), 'Owned commit left an index lock');
    const lastBeat = fs.statSync(heartbeat).size;
    await new Promise(r => setTimeout(r, 350)); // Observe beyond several hook heartbeat intervals after process exit.
    assert.equal(fs.statSync(heartbeat).size, lastBeat, 'Owned hook is still running after server exit');
    const logDirectory = fs.readdirSync(root).find(n => n.startsWith('mcp-logs-'));
    const records = fs.readFileSync(path.join(root, logDirectory, 'rocket.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
    assert(records.some(r => r.event === 'git_end' && r.outcome === 'cancelled'));
    assert(records.every(r => r.dropped_before === 0));
    report.cases.push('EOF during a blocked commit cancels workers and hook, preserves data and exits cleanly');
    report.passed = true;
  } catch (error) { report.error = error.message; console.error(error.stack); }
  finally {
    fs.writeFileSync(release, 'release owned fixture hook');
    if (mcp) { try { await mcp.finish(); } catch (error) { report.shutdown_error = error.message; report.passed = false; mcp.terminateOwnedProcess(); } }
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    for (const test of report.cases) console.log('PASS ' + test);
    console.log('MCP lifecycle results: ' + path.join(root, 'result.json'));
    if (!report.passed) process.exitCode = 1;
  }
})();
