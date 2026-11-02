// Native Windows acceptance of Git Flow through authenticated localhost MCP: Initialize, Start and
// Finish (feature fast-forward, release with tag and back-merge, hotfix stopped by a conflict and
// resumed), a stale review, cancellation inside a merge hook, refusals and log privacy. A disposable
// repository owns only branches under its marked namespace. No network, nothing pushed.
// Usage: node tools/tests/test-mcp-gitflow.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/gitflow-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), ns = 'rocket-ui-test/gf-', branch = ns + 'main';
const names = { develop: ns + 'develop', feature: ns + 'feature/', release: ns + 'release/', hotfix: ns + 'hotfix/', support: ns + 'support/' };
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const gate = path.join(root, 'hook-gate'), marker = path.join(root, 'hook-started');

function fixture() {
  const repo = path.join(root, 'PRIVATE_FLOW'), hooks = path.join(root, 'hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8' }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  const commit = (file, body, message) => { write(file, body); git(['add', '--', file]); git(['commit', '-m', message]); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Flow Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['tag.gpgSign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  commit('app.txt', 'base\n', 'PRIVATE_BASE');
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  // The runner owns every branch under this prefix; Git Flow switches between them.
  fs.writeFileSync(path.join(repo, '.git/rocket-gitflow-fixture'), ns);
  return { repo, hooks, git, write, commit };
}

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
async function until(predicate, label = 'UI state', limit = WAIT_MS) {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < limit, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, error: state.dialog?.error, gitflow: state.dialog?.gitflow?.preview, write: state.write, notice: state.notice }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !s.write && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
const control = id => state.controls.find(c => c.id === id);
const tab = () => state.tabs?.find(t => t.id === state.active_tab);
const flow = () => state.dialog?.gitflow;
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function scroll(target, direction, pixels) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); }
async function refusedClick(target, pattern) { await stable(); await assert.rejects(client.tool('rocket_click', { target, expected_tab: state.active_tab }), pattern); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) await click('dialog-cancel'); assert(!state.dialog, 'Dialog still open'); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
async function menuItem(id) { await click('menu-repository'); await scroll('repository-menu-scroll', 'up', 2000); for (let i = 0; i < 8; i++) { const c = control(id); if (c && c.rect[3] - c.rect[1] >= 28) break; await scroll('repository-menu-scroll', 'down', 250); } await click(id); }
/// Repository > Git Flow > item, then the dialog's own read.
async function gitflow(item) { await menuItem('repository-gitflow'); await click(item); await until(s => s.dialog?.kind === 'gitflow' && !s.dialog.gitflow.loading, 'Git Flow dialog'); }
async function review() { await click('dialog-preview-refresh'); await until(s => flow() && !flow().reviewing && (flow().preview || s.dialog.error), 'Git Flow review'); assert(flow().preview, 'Review failed: ' + state.dialog.error); return flow().preview; }
async function submit() { const count = state.activities.length; await click('dialog-submit'); await until(s => !s.write && s.activities.length > count, 'Git Flow write'); return state.activities.at(-1); }
const head = () => tab().head?.branch;

async function main() {
  try {
    const f = fixture();
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920);
    if (control('console-close')) await click('console-close');

    // 1. Start before Initialize explains; Initialize with git-flow's defaults is refused by
    //    the namespace policy, then the namespaced settings are written.
    await gitflow('gitflow-start-feature');
    assert.equal(flow().initialized, false); assert(!control('dialog-preview-refresh').enabled);
    await capture('gitflow-not-initialized'); await closeDialog();
    await gitflow('gitflow-init');
    assert.equal(flow().config.master, branch); assert.equal(flow().config.develop, 'develop');
    let preview = await review();
    assert.equal(preview.changes.length, 7); assert.equal(preview.develop.source, branch);
    await refusedClick('dialog-submit', /marked namespace/);
    assert.throws(() => f.git(['config', 'gitflow.branch.master']));
    for (const [key, value] of [['develop', names.develop], ['feature', names.feature], ['release', names.release], ['hotfix', names.hotfix], ['support', names.support], ['version-tag', 'v']]) await type('dialog-gitflow-' + key, value);
    assert.equal(flow().preview, null, 'Editing drops the review');
    preview = await review(); await capture('gitflow-init-review');
    let activity = await submit();
    assert(activity.command_succeeded, JSON.stringify(activity));
    assert.equal(f.git(['config', 'gitflow.branch.develop']), names.develop); assert.equal(f.git(['config', 'gitflow.prefix.versiontag']), 'v');
    assert.equal(f.git(['rev-parse', names.develop]), f.git(['rev-parse', branch])); assert.equal(head(), branch);
    passed('Initialize: Start explains first; git-flow defaults outside the owned namespace are refused at submit; namespaced settings are written to the repository config and develop is created without switching');

    // 2. Feature with one commit: develop fast-forwards, the branch is deleted, HEAD ends on develop.
    await gitflow('gitflow-start-feature');
    await type('dialog-gitflow-name', 'login');
    preview = await review();
    assert.equal(preview.branch, names.feature + 'login'); assert.equal(preview.base, names.develop);
    await capture('gitflow-start-review');
    activity = await submit(); assert(activity.command_succeeded, JSON.stringify(activity));
    await until(() => head() === names.feature + 'login', 'Switched to the feature');
    f.commit('login.txt', 'login\n', 'PRIVATE_LOGIN');
    const featureTip = f.git(['rev-parse', 'HEAD']);
    await gitflow('gitflow-finish');
    assert.equal(flow().branch, names.feature + 'login');
    preview = await review();
    assert.deepEqual(preview.steps.map(s => s.step), ['switch', 'merge', 'delete']);
    assert.equal(preview.steps[1].mode, 'AllowFastForward'); assert.equal(preview.steps[1].fast_forward, true);
    assert.equal(preview.ends_on, names.develop);
    await capture('gitflow-finish-feature-review');
    activity = await submit(); assert(activity.command_succeeded, JSON.stringify(activity));
    assert.equal(f.git(['rev-parse', names.develop]), featureTip);
    assert.equal(f.git(['branch', '--list', names.feature + 'login']), '');
    await until(() => head() === names.develop, 'Ended on develop');
    passed('Feature: one commit fast-forwards develop as git-flow does; the branch is deleted with git branch -d and HEAD ends on develop');

    // 3. Release: --no-ff into main, annotated tag, --no-ff back into develop, deleted.
    await gitflow('gitflow-start-release');
    await type('dialog-gitflow-name', '1.0');
    preview = await review(); assert.equal(preview.tag, 'v1.0');
    activity = await submit(); assert(activity.command_succeeded, JSON.stringify(activity));
    await until(() => head() === names.release + '1.0', 'Switched to the release');
    f.commit('app.txt', 'release 1.0\n', 'PRIVATE_RELEASE');
    const releaseTip = f.git(['rev-parse', 'HEAD']);
    await gitflow('gitflow-finish');
    assert.equal(flow().branch, names.release + '1.0');
    await type('dialog-gitflow-message', 'Release 1.0 文');
    preview = await review();
    assert.deepEqual(preview.steps.map(s => s.step), ['switch', 'merge', 'tag', 'switch', 'merge', 'delete']);
    assert(preview.steps.filter(s => s.step === 'merge').every(s => s.mode === 'NoFastForward' && !s.fast_forward));
    assert.equal(preview.steps[2].name, 'v1.0');
    await capture('gitflow-finish-release-review');
    activity = await submit(); assert(activity.command_succeeded, JSON.stringify(activity));
    assert.equal(f.git(['rev-parse', branch + '^2']), releaseTip);
    assert.equal(f.git(['cat-file', '-t', 'v1.0']), 'tag');
    assert.equal(f.git(['rev-parse', 'v1.0^{commit}']), f.git(['rev-parse', branch]));
    assert.equal(f.git(['tag', '-l', '--format=%(contents)', 'v1.0']), 'Release 1.0 文');
    assert.equal(f.git(['rev-parse', names.develop + '^2']), releaseTip);
    assert.equal(f.git(['log', '-1', '--format=%s', branch]), `Merge branch '${names.release}1.0' into ${branch}`);
    passed('Release: reviewed steps run in order: --no-ff into main, annotated tag v1.0 on that merge, --no-ff back into develop, branch deleted; nothing pushed');

    // 4. Hotfix stopped by a conflict on develop, resolved through Rocket, then finished again.
    f.commit('app.txt', 'develop change\n', 'PRIVATE_DEVELOP');
    await gitflow('gitflow-start-hotfix');
    await type('dialog-gitflow-name', '1.0.1');
    preview = await review(); assert.equal(preview.base, branch);
    activity = await submit(); assert(activity.command_succeeded, JSON.stringify(activity));
    await until(() => head() === names.hotfix + '1.0.1', 'Switched to the hotfix');
    f.commit('app.txt', 'hotfix change\n', 'PRIVATE_HOTFIX');
    await gitflow('gitflow-finish');
    await type('dialog-gitflow-message', 'Hotfix 1.0.1');
    await review();
    activity = await submit();
    assert(!activity.command_succeeded && activity.recovery_pending, JSON.stringify(activity));
    assert(/conflicts/.test(activity.summary), activity.summary);
    assert(activity.details.some(d => d.includes('gitflow_conflicts')), JSON.stringify(activity.details));
    await until(s => !s.dialog && tab().view === 'Changes' && tab().operation_state && !tab().operation_state.idle, 'Merge stopped in Changes');
    assert.equal(f.git(['cat-file', '-t', 'v1.0.1']), 'tag', 'The steps before the conflict are done');
    await capture('gitflow-hotfix-conflict');
    f.write('app.txt', 'resolved\n'); f.git(['add', 'app.txt']);
    await type('commit-message', 'Merge hotfix 1.0.1 into develop');
    await until(s => !tab().loading && control('commit')?.enabled, 'Resolution staged; Continue merge enabled');
    await click('commit');
    await until(s => tab().operation_state?.idle, 'Merge continued');
    await gitflow('gitflow-finish');
    assert.equal(flow().branch, names.hotfix + '1.0.1');
    preview = await review();
    assert.deepEqual(preview.steps.map(s => s.step), ['delete']);
    assert.deepEqual(preview.notes.map(n => n.note).sort(), ['merged', 'merged', 'tagged']);
    activity = await submit(); assert(activity.command_succeeded, JSON.stringify(activity));
    assert.equal(f.git(['branch', '--list', names.hotfix + '1.0.1']), '');
    passed('Hotfix: a conflict stops the run after main, its merge and tag v1.0.1; Changes resolves and continues the merge; Finish again skips the done steps and only deletes the branch');

    // 5. A review spanning an external commit writes nothing; cancellation inside a merge hook
    //    stops the run and leaves recovery to the merge flow.
    await gitflow('gitflow-start-feature');
    await type('dialog-gitflow-name', 'orbit');
    await review(); activity = await submit(); assert(activity.command_succeeded);
    await until(() => head() === names.feature + 'orbit', 'Switched to the feature');
    f.commit('orbit.txt', 'one\n', 'PRIVATE_ORBIT_ONE'); f.commit('orbit.txt', 'two\n', 'PRIVATE_ORBIT_TWO');
    await gitflow('gitflow-finish'); await review();
    // develop moves outside Rocket, without touching this checkout.
    f.git(['update-ref', 'refs/heads/' + names.develop, f.git(['commit-tree', '-p', names.develop, '-m', 'PRIVATE_EXTERNAL', names.develop + '^{tree}'])]);
    const developBefore = f.git(['rev-parse', names.develop]);
    activity = await submit();
    assert(!activity.command_succeeded && activity.details.some(d => d.includes('gitflow_stale_preview')), JSON.stringify(activity));
    assert.equal(f.git(['rev-parse', names.develop]), developBefore); assert.equal(head(), names.feature + 'orbit');
    await until(s => s.dialog?.kind === 'gitflow' && /stale/.test(s.dialog.error || ''), 'Stale review reported');
    await closeDialog();
    fs.writeFileSync(gate, 'hold');
    fs.writeFileSync(path.join(f.hooks, 'pre-merge-commit'), `#!/bin/sh\necho started > "${marker.replaceAll('\\', '/')}"\nwhile [ -f "${gate.replaceAll('\\', '/')}" ]; do sleep 0.1; done\n`);
    await gitflow('gitflow-finish'); await review();
    const count = state.activities.length;
    await stable(); await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab });
    for (let n = 0; !fs.existsSync(marker); n++) { assert(n < 400, 'Merge hook did not start'); await delay(50); }
    await until(s => s.write && control('cancel-operation')?.enabled, 'Cancellable write');
    const stop = performance.now();
    await client.tool('rocket_click', { target: 'cancel-operation', expected_tab: state.active_tab });
    await until(s => !s.write && s.activities.length > count, 'Cancelled finish');
    report.cancel_ms = Math.round(performance.now() - stop);
    activity = state.activities.at(-1);
    assert(!activity.command_succeeded, JSON.stringify(activity));
    assert(activity.details.some(d => d.startsWith('Not run: delete')), JSON.stringify(activity.details));
    fs.rmSync(gate); fs.rmSync(path.join(f.hooks, 'pre-merge-commit'));
    // Cancellation is not rollback: Git was stopped inside the merge, before its commit.
    assert.equal(f.git(['rev-parse', names.develop]), developBefore, 'No merge commit was made');
    assert.notEqual(f.git(['branch', '--list', names.feature + 'orbit']), '', 'The branch was not deleted');
    await settle();
    report.cancel_leftover = { merge_in_progress: !tab().operation_state?.idle, changes: tab().change_count };
    await capture('gitflow-cancelled');
    passed(`A review spanning an external develop commit is refused as stale with nothing written; Cancel inside a blocking merge hook stops in ${report.cancel_ms} ms, no merge commit is made, later steps do not run and the leftover state is shown for review`);

    // 6. Logs: fixed labels and paired requests only.
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'login', 'orbit', 'gf-', 'Release 1.0', 'v1.0']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    for (const op of ['gitflow-read', 'gitflow-init-preview', 'gitflow-init', 'gitflow-start-preview', 'gitflow-start', 'gitflow-finish-preview', 'gitflow-finish']) {
      const starts = records.filter(r => r.event === 'request_start' && r.operation === op);
      assert(starts.length, 'Missing ' + op);
      for (const s of starts) assert(records.some(r => r.event === 'request_end' && r.session === s.session && r.request === s.request), 'Unpaired ' + op);
    }
    assert(records.some(r => r.event === 'request_end' && r.operation === 'gitflow-finish' && r.failure_code === 'gitflow_stale_preview'), 'Stale refusal code');
    report.log_records = records.length;
    passed('Diagnostics: every gitflow-* request paired, the stale refusal logged by its code, zero dropped events, no branch names, versions, messages or paths');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    try { fs.rmSync(gate, { force: true }); } catch {}
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); }
    fs.writeFileSync('work/gitflow-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
  }
}
main().catch(() => { process.exitCode = 1; });
