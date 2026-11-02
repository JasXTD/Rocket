'use strict';
// Compare the same synthetic repository through the real localhost MCP and native UI.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = process.env.ROCKET_PERFORMANCE_FIXTURE || fs.readFileSync('work/large-repo-fixture-path', 'utf8').trim();
assert(path.resolve(root).startsWith(path.resolve('work/mcp-fixtures') + path.sep));
const fixture = JSON.parse(fs.readFileSync(path.join(root, 'fixture.json'), 'utf8'));
const phase = process.argv[2] || 'baseline'; assert(/^[a-z0-9-]+$/.test(phase));
const runRoot = path.join(root, phase); fs.mkdirSync(runRoot);
const acceptanceOnly = process.argv[3] === 'acceptance';
const profile = path.join(runRoot, 'http-profile');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'hooks'), GIT_OPTIONAL_LOCKS: '0' });
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const git = args => cp.execFileSync('git', ['-C', fixture.repo, ...args], { env, windowsHide: true, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024, timeout: 60000 }).trimEnd();
const protectedFiles = ['.git/HEAD', '.git/index', '.git/config', 'src/perf.rs', fixture.unicode_file];
const before = protectedFiles.map(file => [file, hash(path.join(fixture.repo, file))]);
const exe = path.resolve(process.env.ROCKET_PERFORMANCE_EXE || 'target/release/rocket-mcp.exe');
const report = { passed: false, phase, root, fixture, executable_sha256: hash(exe), timings: [], checks: [] };
let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const active = () => state?.tabs?.find(t => t.id === state.active_tab);
const control = id => state?.controls?.find(c => c.id === id);
const originalLogs = new Set(fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')));
function records(strict = false) {
  return fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-') && !originalLogs.has(n)).sort().flatMap(n => fs.readdirSync(path.join(root, n)).filter(f => /^rocket(?:\.\d+)?\.jsonl$/.test(f)).sort().flatMap(f => fs.readFileSync(path.join(root, n, f), 'utf8').split(/\r?\n/).filter(Boolean).flatMap(line => { try { return [JSON.parse(line)]; } catch (e) { if (strict) throw e; return []; } })));
}
async function until(predicate, label = 'UI settles', timeout = 180000) {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    assert(!state.startup_error && !state.state_error, JSON.stringify({ startup: state.startup_error, state: state.state_error }));
    assert(!active()?.error, active()?.error);
    if (predicate(state)) return;
    assert(performance.now() - start < timeout, label + ' timed out');
    await delay(50);
  }
}
const settled = s => !s.busy && !s.input_pending && s.tabs.every(t => !t.loading && !t.background_refresh && !t.search.scanning && !t.composer?.loading && !t.composer?.pending);
const settle = () => until(settled);
async function deliver(name, args) {
  const response = await client.request('tools/call', { name, arguments: args });
  assert(!response.isError, response.content?.map(c => c.text || '').join('\n'));
  return response._meta?.rocket_request;
}
async function click(id) { assert(control(id)?.enabled, 'Missing/disabled ' + id); return deliver('rocket_click', { target: id, expected_tab: state.active_tab }); }
async function type(text) { return deliver('rocket_type', { target: 'commit-filter', text, expected_tab: state.active_tab }); }
async function measure(name, action, ready = settled) {
  const prior = new Set(records().map(r => JSON.stringify(r)));
  const start = performance.now(), receipt = await action();
  await until(ready, name);
  const elapsed_ms = +(performance.now() - start).toFixed(2);
  const added = records().filter(r => !prior.has(JSON.stringify(r)));
  const commands = added.filter(r => r.event === 'git_end').map(r => ({ operation: r.operation, ms: r.elapsed_ms }));
  const requests = added.filter(r => r.event === 'request_end').map(r => ({ operation: r.operation, ms: r.elapsed_ms, outcome: r.outcome }));
  const sample = { name, elapsed_ms, commands, requests, input: receipt, commits: active()?.commit_count, scanned: active()?.search.scanned };
  report.timings.push(sample); fs.writeFileSync(path.join(runRoot, 'result.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ name, elapsed_ms, command_count: commands.length, requests }));
}
async function capture(name) {
  const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError);
  fs.writeFileSync(path.join(runRoot, name + '.png'), Buffer.from(result.content.find(c => c.type === 'image').data, 'base64'));
}
async function close() {
  await client.tool('rocket_close');
  await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && control('dialog-submit')?.enabled && !s.input_pending), 'Close ready');
  if (state.dialog?.kind === 'quit') { await click('dialog-submit'); await until(s => ['closed', 'not_started'].includes(s.window)); }
  await client.raw('POST', '/shutdown');
  const start = performance.now();
  while (service.exitCode === null) { assert(performance.now() - start < 10000, 'Service shutdown timed out'); await delay(50); }
}
function passed(message) {
  report.checks.push(message);
  fs.writeFileSync(path.join(runRoot, 'result.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ check: message }));
}
async function acceptance() {
  assert(active().watching && state.preferences.auto_refresh, 'Automatic refresh required');
  // Real cancellation, after a Git child starts. No artificial backend delay.
  report.cancellations = [];
  for (let attempt = 0; attempt < 3; attempt++) {
    const prior = new Set(records().map(r => JSON.stringify(r)));
    await type('PRIVATE_CANCEL_ABSENT_' + attempt);
    await until(() => active().search.running && records().some(r => r.event === 'git_start' && !prior.has(JSON.stringify(r)) && !records().some(e => e.event === 'git_end' && e.session === r.session && e.id === r.id && e.request === r.request)), 'Active search process');
    const start = performance.now(), input = await click('history-stop');
    await until(() => !active().search.scanning && active().search.paused && !state.input_pending, 'Stopped search feedback');
    const feedback_ms = +(performance.now() - start).toFixed(2), ids = [...active().history_ids], scanned = active().search.scanned;
    await delay(500); await settle();
    assert.deepEqual(active().history_ids, ids); assert.equal(active().search.scanned, scanned);
    const ends = records().filter(r => !prior.has(JSON.stringify(r)) && r.event === 'git_end' && r.outcome === 'cancelled');
    const stopped = ends.filter(r => r.cancel_to_stop_us != null);
    report.cancellations.push({ feedback_ms, input, cancelled_commands: ends.map(r => r.operation), actual_stop_ms: stopped.map(r => r.cancel_to_stop_us / 1000) });
    if (stopped.length) break;
  }
  assert(report.cancellations.some(c => c.actual_stop_ms.length), 'No real Git cancellation observed');
  passed('Stop pauses promptly; actual Git stop measured separately; cancelled results never appear later');
  await type('PRIVATE_CANCEL_REPLACED'); await until(() => active().search.running);
  await type('PRIVATE_OLD_NEEDLE_性能'); await until(s => settled(s) && active().search.complete);
  assert.deepEqual(active().history_ids, [fixture.oldest_match]);
  passed('Changing the query cancels the previous scan and displays only the new results');
  await type(''); await settle(); await click('view-changes'); await settle();
  await deliver('rocket_type', { target: 'commit-message', text: 'PRIVATE_PERF_DRAFT 文', expected_tab: state.active_tab }); await settle();
  const target = path.join(fixture.repo, fixture.unicode_file), original = fs.readFileSync(target);
  try {
    await click('change-unstaged-' + Buffer.from(fixture.unicode_file).toString('hex')); await settle();
    const refresh = active().refresh_count;
    for (let n = 0; n < 20; n++) fs.writeFileSync(target, Buffer.concat([original, Buffer.from('PRIVATE_EXTERNAL_' + n + '\nextra\nthird\n')]));
    await until(() => active().background_refresh, 'External change starts refresh');
    const start = performance.now();
    const input = await deliver('rocket_type', { target: 'commit-message', text: 'PRIVATE_PERF_DRAFT updated while refreshing 文', expected_tab: state.active_tab });
    report.typing_during_refresh = { elapsed_ms: +(performance.now() - start).toFixed(2), input };
    assert(report.typing_during_refresh.elapsed_ms < 1500, 'Typing waited on repository refresh');
    await until(s => settled(s) && active().refresh_count > refresh && active().patch_added === 2003 && active().patch_removed === 2000);
    assert(active().draft_present); assert(!active().patch_error);
    git(['add', '--', fixture.unicode_file]);
    await until(s => settled(s) && control('change-staged-' + Buffer.from(fixture.unicode_file).toString('hex')));
    await click('change-staged-' + Buffer.from(fixture.unicode_file).toString('hex')); await settle();
    assert.equal(active().patch_added, 2003);
    await capture('external-edit-and-draft');
    git(['restore', '--staged', '--', fixture.unicode_file]);
  } finally { fs.writeFileSync(target, original); }
  await settle();
  passed('Burst edits and external staging refresh the selected Unicode diff; draft typing stays responsive and is preserved');
  // Use an isolated new file for the external commit; existing modified files stay untouched.
  const newFile = 'PRIVATE_EXTERNAL_性能.rs';
  assert(!fs.existsSync(path.join(fixture.repo, newFile)));
  fs.writeFileSync(path.join(fixture.repo, newFile), 'external commit\n');
  git(['add', '--', newFile]); git(['-c', 'commit.gpgsign=false', 'commit', '-m', 'PRIVATE_PERF_EXTERNAL_COMMIT']);
  const head = git(['rev-parse', 'HEAD']);
  await until(s => settled(s) && active().head.oid === head);
  assert(active().draft_present);
  await click('view-commits'); await settle(); await click('history-scope'); await settle(); await click('history-all'); await settle();
  await type('PRIVATE_NEW_BRANCH_ONLY'); await until(s => settled(s) && active().search.complete);
  assert.equal(active().commit_count, 0);
  const tree = git(['rev-parse', 'HEAD^{tree}']), side = git(['-c', 'commit.gpgsign=false', 'commit-tree', tree, '-p', head, '-m', 'PRIVATE_NEW_BRANCH_ONLY']);
  git(['update-ref', 'refs/heads/rocket-ui-test/external-性能', side]); git(['pack-refs', '--all', '--prune']);
  await until(s => settled(s) && active().search.complete && active().history_ids.includes(side), 'New packed branch restarts search');
  assert.equal(active().head.oid, head); assert.deepEqual(active().history_ids, [side]);
  git(['update-ref', '-d', 'refs/heads/rocket-ui-test/external-性能', side]);
  await until(s => settled(s) && active().search.complete && active().commit_count === 0, 'Deleted branch refreshes history');
  passed('External commit, packed branch creation and branch deletion refresh HEAD, combined history and the running search');
  // Errors must remain errors rather than a fabricated empty search.
  await type('x'.repeat(1025)); await until(s => settled(s) && active().search.error);
  assert.match(active().search.error, /1\.\.1024/); await capture('search-error');
  await type('PRIVATE_PERF_EXTERNAL_COMMIT'); await until(s => settled(s) && active().search.complete);
  assert.deepEqual(active().history_ids, [head]);
  passed('Invalid query shows a clear error; a corrected query recovers');
  assert.equal(hash(path.join(fixture.repo, 'src/perf.rs')), before.find(([p]) => p === 'src/perf.rs')[1]);
  assert.equal(hash(target), before.find(([p]) => p === fixture.unicode_file)[1]);
  assert.equal(hash(path.join(fixture.repo, '.git/config')), before.find(([p]) => p === '.git/config')[1]);
  assert.equal(git(['diff', '--cached', '--name-only']), '');
  await type(''); await settle();
  report.external_head = head;
}
function audit() {
  const logs = records(true); assert(logs.length);
  assert(logs.every(r => r.dropped_before === 0), 'Dropped diagnostic records');
  for (const kind of ['request', 'step', 'git']) {
    const starts = logs.filter(r => r.event === kind + '_start'), ends = logs.filter(r => r.event === kind + '_end');
    assert.equal(starts.length, ends.length, kind + ' unmatched spans');
    for (const r of starts) assert.equal(ends.filter(e => e.session === r.session && e.request === r.request && e.id === r.id).length, 1, kind + ' missing end');
  }
  for (const value of ['PRIVATE_', fixture.repo, fixture.unicode_file, fixture.head]) assert(!JSON.stringify(logs).includes(JSON.stringify(value).slice(1, -1)), 'Private payload in diagnostics');
  report.diagnostics = { records: logs.length, dropped: 0, spans_paired: true, private_payloads: false };
  passed('Diagnostics pair every request/step/Git span, retain cancellation and errors, and omit private payloads');
}
async function main() {
  try {
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true, stdio: 'ignore' });
    const socket = net.createServer(); await new Promise(resolve => socket.listen(0, '127.0.0.1', resolve)); const port = socket.address().port; await new Promise(resolve => socket.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(runRoot, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, detached: true, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    const start = performance.now(); for (;;) { try { const init = await client.initialize(); report.version = init.serverInfo.version; break; } catch (e) { if (e.code !== 'ECONNREFUSED') throw e; assert(performance.now() - start < 15000); await delay(60); } }
    await measure('open', () => deliver('rocket_open', { path: fixture.repo }), s => s.tabs.length === 1 && settled(s));
    await client.tool('rocket_resize', { width: 1480, height: 920 }); await settle();
    if (control('console-close')) { await click('console-close'); await settle(); }
    await click('view-commits'); await settle();
    if (active().search.query) { await type(''); await settle(); }
    await click('history-scope'); await settle(); await click('history-head'); await settle();
    if (process.argv[3] === 'diff-control') {
      await click('view-changes'); await settle();
      for (let i = 1; i <= 8; i++) {
        await measure('small-working-diff-' + i, () => click('change-unstaged-' + Buffer.from('src/perf.rs').toString('hex')));
        assert.equal(active().patch_added, 1); assert.equal(active().patch_removed, 1);
        await measure('large-unicode-diff-' + i, () => click('change-unstaged-' + Buffer.from(fixture.unicode_file).toString('hex')));
        assert.equal(active().patch_added, 2000); assert.equal(active().patch_removed, 2000);
      }
      assert.deepEqual(protectedFiles.map(file => [file, hash(path.join(fixture.repo, file))]), before);
      passed('Repeated isolated diff selections preserve exact contents and repository bytes');
      await close(); audit(); report.passed = true; return;
    }
    if (acceptanceOnly) {
      await acceptance(); await close(); audit(); report.passed = true;
      fs.writeFileSync('work/performance-acceptance-result', path.join(runRoot, 'result.json')); return;
    }
    for (let i = 1; i <= 3; i++) await measure('refresh-' + i, () => click('refresh'));
    const expected = git(['rev-list', '--topo-order', '--max-count=2000', fixture.head]).split(/\r?\n/);
    for (let i = 1; i <= 15; i++) {
      const old = active().commit_count;
      await measure('history-page-' + i, () => click('history-more'), s => settled(s) && active().commit_count === old + 200);
      assert.deepEqual(active().history_ids, expected.slice(0, Math.min(2000, active().commit_count)));
    }
    passed('History pages match independent Git ordering with no missing or duplicate IDs');
    await measure('search-old-match', () => type('PRIVATE_OLD_NEEDLE_性能'), s => settled(s) && active().search.complete);
    assert.deepEqual(active().history_ids, [fixture.oldest_match]); passed('Repository search finds a commit beyond loaded history');
    await measure('search-no-match', () => type('PRIVATE_ABSENT_NEEDLE'), s => settled(s) && active().search.complete);
    assert.equal(active().commit_count, 0);
    await type(''); await settle();
    await click('history-scope'); await settle(); await measure('all-branch-history', () => click('history-all'));
    const all = git(['rev-list', '--topo-order', '--max-count=200', '--branches', '--remotes', 'HEAD']).split(/\r?\n/);
    assert.deepEqual(active().history_ids.slice(0, 200), all); passed('All-branch history matches independent Git');
    await click('view-changes'); await settle();
    for (let i = 1; i <= 2; i++) {
      await measure('small-working-diff-' + i, () => click('change-unstaged-' + Buffer.from('src/perf.rs').toString('hex')));
      assert.equal(active().patch_added, 1); assert.equal(active().patch_removed, 1);
      await measure('large-unicode-diff-' + i, () => click('change-unstaged-' + Buffer.from(fixture.unicode_file).toString('hex')));
      assert.equal(active().patch_added, 2000); assert.equal(active().patch_removed, 2000);
    }
    await capture('large-repository-diff');
    assert.deepEqual(protectedFiles.map(file => [file, hash(path.join(fixture.repo, file))]), before);
    passed('Read operations preserve index, HEAD, configuration and working bytes');
    await close();
    audit();
    report.passed = true;
    if (report.version === '0.46.2') fs.writeFileSync('work/performance-optimized-result', path.join(runRoot, 'result.json'));
  } catch (e) { report.error = String(e); if (client) { try { await capture('failure'); } catch {} } throw e; }
  finally {
    fs.writeFileSync(path.join(runRoot, 'last-state.json'), JSON.stringify(state || null, null, 2));
    fs.writeFileSync(path.join(runRoot, 'result.json'), JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await close(); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
    console.log(JSON.stringify({ result: path.join(runRoot, 'result.json'), passed: report.passed, error: report.error }));
  }
}
main().catch(e => { console.error(e.message); process.exitCode = 1; });
