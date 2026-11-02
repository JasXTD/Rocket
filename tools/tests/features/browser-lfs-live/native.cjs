// Focused live acceptance. The fixture setup does not publish any refs or LFS objects.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../../../mcp/mcp-http-client.cjs');
const { gitRelay } = require('../../git-relay.cjs');
const {SUBMENU}=require('../../menu-paths.cjs');
const { Fixture, hash, snapshot, personalSnapshot, credentialFingerprint, download, cacheAt, payload, oid } = require('./fixture.cjs');
const f = new Fixture(path.resolve(process.argv[2]));
const browser = process.argv.includes('--browser');
const resumePath = process.argv.find(a => a.startsWith('--browser-only='))?.slice(15);
const prior = resumePath ? JSON.parse(fs.readFileSync(resumePath, 'utf8')) : null;
assert(!browser || f.mode === 'https');
const root = f.root, output = path.join(root, 'prune-run-' + Date.now());
fs.mkdirSync(output);
const profile = path.join(output, 'http-profile');
const exe = path.resolve('target/release/rocket-mcp.exe');
const relay = gitRelay(output, f.env, 'tools/helpers/file-history-preview/git-relay.rs', root), { gate, marker } = relay;
const reportPath = path.join(output, 'result.json');
const report = { passed: false, root, output, mode: f.mode, cases: [], timings: {}, uploads: 0, remote_writes: 0 };
const earlierLogs = new Set(fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')));
const delay = ms => new Promise(r => setTimeout(r, ms));
let service, client, state, savedError, credentialBefore;
const personalBefore = personalSnapshot();
const control = id => state.controls?.find(c => c.id === id);
const workflow = () => state.dialog?.workflow;
function save() { report.log_directories = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-') && (prior || !earlierLogs.has(n))); fs.writeFileSync(reportPath, JSON.stringify(report, null, 2)); }
function pass(label) { report.cases.push(label); save(); console.log('PASS ' + f.mode + ' ' + label); }
async function until(test, label = 'state', timeout = 120000) {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (test(state)) return;
    assert(performance.now() - start < timeout, label + ' timed out');
    await delay(100);
  }
}
async function settle() {
  await until(s => !s.busy && !s.input_pending && !s.hosting?.busy && !s.dialog?.workflow?.loading
    && !s.dialog?.workflow?.applying && !(s.tabs || []).some(t => t.loading), 'operation settled');
}
async function stable() {
  let last = '', same = 0;
  await until(s => { const now = JSON.stringify(s.controls); same = now === last ? same + 1 : 0; last = now; return !s.input_pending && same >= 2; }, 'stable controls');
}
async function scroll(region, direction, pixels) {
  for (let remaining = pixels; remaining > 0; remaining -= 1000) {
    await client.tool('rocket_scroll', { target: region, direction, pixels: Math.min(remaining, 1000), expected_tab: state.active_tab });
    await until(s => !s.input_pending); await stable();
  }
}
async function reveal(id) {
  await stable(); const visible = () => control(id) && control(id).rect[3] - control(id).rect[1] >= 18;
  if (visible()) return;
  const region = id.startsWith('repository-') ? 'repository-menu-scroll' : id.startsWith('hosting-') ? 'hosting-scroll' : 'dialog-workflow-scroll';
  if (control(region)) { await scroll(region, 'up', 3000); for (let i = 0; i < 25 && !visible(); i++) await scroll(region, 'down', 220); }
  assert(visible(), 'Unreachable control: ' + id);
}
async function click(id, wait = true) {
  await reveal(id); assert(control(id)?.enabled, 'Disabled control: ' + id);
  await client.tool('rocket_click', { target: id, expected_tab: state.active_tab }); if (wait) await settle();
}
async function menu(id) { await click('menu-repository'); if (SUBMENU[id]) { await click(SUBMENU[id]); await stable(); } await click(id); }
async function review() {
  await click('workflow-review'); assert(workflow()?.preview, state.dialog?.error || 'Missing LFS preview');
  assert.equal(workflow().preview.kind, 'lfs'); assert(!state.dialog.error, state.dialog.error);
}
async function capture(name) {
  await stable(); const r = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!r.isError); fs.writeFileSync(path.join(output, name + '.png'), Buffer.from(r.content.find(c => c.type === 'image').data, 'base64'));
}
async function resize(width, height) {
  await client.tool('rocket_resize', { width, height });
  await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle();
}
async function cancelAt(start, name) {
  await reveal(start); fs.rmSync(marker, { force: true }); fs.writeFileSync(gate, 'worktree');
  await client.tool('rocket_click', { target: start, expected_tab: state.active_tab });
  // The relay creates the file before writing both IDs. Never interpret an
  // observable empty file as PID 0 (which would probe this Node process group).
  let pids;
  await until(s => {
    if (s.input_pending || !fs.existsSync(marker)) return false;
    const text = fs.readFileSync(marker, 'utf8').trim();
    if (!/^[1-9]\d*\r?\n[1-9]\d*$/.test(text)) return false;
    pids = text.split('\n').map(Number);
    return pids.every(p => Number.isSafeInteger(p) && p > 0 && p !== process.pid);
  }, 'complete cancellation gate');
  report.timings[name + '_pids'] = pids;
  await reveal('workflow-stop'); const at = performance.now();
  await client.tool('rocket_click', { target: 'workflow-stop', expected_tab: state.active_tab });
  await until(s => !s.input_pending); report.timings[name + '_feedback_ms'] = Math.round(performance.now() - at);
  await until(() => pids.every(p => { try { process.kill(p, 0); return false; } catch { return true; } }), 'owned Git stopped');
  report.timings[name + '_process_stop_ms'] = Math.round(performance.now() - at);
  fs.rmSync(gate, { force: true }); await settle();
  report.timings[name + '_settled_ms'] = Math.round(performance.now() - at);
  assert.equal(hash(cacheAt(f.client)), oid);
}
function logs() {
  return fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-') && (prior || !earlierLogs.has(n))).flatMap(d => fs.readdirSync(path.join(root, d))
    .filter(n => /^rocket(?:\.\d+)?\.jsonl$/.test(n)).flatMap(n => fs.readFileSync(path.join(root, d, n), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
}
async function start() {
  const realGit = cp.execFileSync('where.exe', ['git.exe'], { encoding: 'utf8', windowsHide: true }).trim().split(/\r?\n/)[0];
  const lfsExe = ['../ucrt64/bin/git-lfs.exe', '../mingw64/bin/git-lfs.exe', '../libexec/git-core/git-lfs.exe']
    .map(p => path.resolve(path.dirname(realGit), p)).find(fs.existsSync);
  assert(lfsExe); fs.copyFileSync(lfsExe, path.join(relay.folder, 'git-lfs.exe')); assert.equal(hash(lfsExe), hash(path.join(relay.folder, 'git-lfs.exe')));
  report.executable_sha256 = hash(exe); report.lfs_sha256 = hash(lfsExe);
  if (browser) fs.writeFileSync(path.join(root, 'rocket-github-browser-check'), 'ROCKET-GITHUB-BROWSER-CHECK-1\nhttps://github.com\nJasXTD/rocket-auth-test\n');
  cp.execFileSync(exe, ['--init-http', profile, root], { env: f.env, windowsHide: true, stdio: 'pipe' });
  const probe = net.createServer(); await new Promise(r => probe.listen(0, '127.0.0.1', r)); const port = probe.address().port; await new Promise(r => probe.close(r));
  fs.writeFileSync(path.join(profile, 'settings'), `ROCKET-MCP-1\n${port}\n${root}\n`);
  const env = relay.env, pathKey = Object.keys(f.env).find(k => k.toUpperCase() === 'PATH');
  env[pathKey] = [relay.folder, path.resolve(path.dirname(realGit), '../usr/bin'), path.dirname(lfsExe), f.env[pathKey]].join(path.delimiter);
  const err = fs.openSync(path.join(output, 'service.log'), 'w');
  service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', err] }); fs.closeSync(err);
  client = new RocketHttpClient(profile);
  for (let i = 0;; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
  report.version = (await client.initialize()).serverInfo.version;
  await client.tool('rocket_open', { path: f.client }); await settle(); relay.check(state); await resize(1480, 920);
  if (control('console-close')) await click('console-close');
}
async function browserCheck() {
  credentialBefore = credentialFingerprint(f);
  if (state.dialog) await click('dialog-cancel');
  await menu('repository-hosting'); await click('hosting-tab-accounts');
  assert.equal(state.hosting.accounts_count, 0); assert.equal(state.hosting.remember, false);
  // No test API, personal account store or token is exposed to MCP.
  for (const target of ['hosting-remember', 'hosting-provider-gitlab', 'hosting-connect']) {
    await reveal(target);
    await assert.rejects(client.tool('rocket_click', { target, expected_tab: state.active_tab }), /Live browser acceptance/);
  }
  await reveal('hosting-browser'); await capture('browser-ready');
  fs.writeFileSync(path.join(output, 'browser-ready'), 'Ready for one user-controlled browser sign-in\n');
  console.log('READY_BROWSER ' + output); save();
  const start = Date.now(); while (!fs.existsSync(path.join(output, 'browser-go'))) { assert(Date.now() - start < 20 * 60 * 1000, 'Browser consent not started'); await delay(500); }
  await click('hosting-browser', false); await until(s => !s.input_pending); console.log('BROWSER_STARTED');
  await until(s => !s.hosting.busy && !s.input_pending, 'browser sign-in', 330000);
  assert(state.hosting.connected, state.hosting.error || state.hosting.notice || 'Browser sign-in did not connect');
  assert(!state.hosting.remember && state.hosting.accounts_count === 0);
  await reveal('hosting-tab-create');
  await assert.rejects(client.tool('rocket_click', { target: 'hosting-tab-create', expected_tab: state.active_tab }), /Live browser acceptance/);
  await click('hosting-tab-accounts'); await capture('browser-connected');
  await click('hosting-disconnect'); assert(!state.hosting.connected);
  report.existing_credential_unchanged = credentialBefore === credentialFingerprint(f);
  assert(report.existing_credential_unchanged, 'Existing GCM credential differs after browser sign-in');
  download(f); // Existing get-only GCM credential still authenticates, after sign-in/disconnect.
  pass('Real GCM browser sign-in and /user verification; disconnect; existing HTTPS credential unchanged and still usable');
}
async function run() {
  try {
    f.assertScope();
    const before = snapshot(f); await start();
    if (prior) {
      assert(browser && prior.root === root && prior.mode === 'https');
      assert.equal(prior.executable_sha256, report.executable_sha256); assert.equal(prior.version, report.version);
      assert.equal(prior.cases.length, 5); assert(prior.personal_files_unchanged);
      assert.match(prior.failure, /Control is not present in this UI state/);
      assert(!fs.existsSync(cacheAt(f.client))); f.assertRemote('');
      report.resumed_from = path.resolve(resumePath); report.cases = [...prior.cases]; report.timings = { ...prior.timings }; save();
    } else {
    assert.equal(hash(cacheAt(f.client)), oid);
    await menu('repository-workflows'); await click('workflow-mode-5');
    await cancelAt('workflow-review', 'review_cancel'); assert(!workflow().preview); assert.match(workflow().notice, /cancelled/i);
    pass('Cancelled preview stops owned Git and preserves the known cache object');
    await review(); assert.deepEqual(workflow().preview.objects, [{ oid, size: payload.length }]); assert.equal(workflow().preview.bytes, payload.length);
    await capture('lfs-review'); await resize(780, 620); await capture('lfs-review-compact'); await resize(1480, 920);
    await cancelAt('dialog-submit', 'apply_cancel'); assert(state.dialog.error || workflow().notice);
    pass('Live remote-verified byte preview; cancelled apply keeps cache and checkout');
    await review(); const file = path.join(f.client, 'ROCKET_PRIVATE_file'), original = fs.readFileSync(file);
    fs.writeFileSync(file, 'Synthetic external edit; preserve me\n'); await click('dialog-submit');
    assert(state.dialog.error); assert.equal(fs.readFileSync(file, 'utf8'), 'Synthetic external edit; preserve me\n'); assert.equal(hash(cacheAt(f.client)), oid);
    await capture('lfs-external-edit'); fs.writeFileSync(file, original);
    pass('External edit rejects stale apply and preserves edited file/cache');
    f.auth(false); await click('workflow-review'); assert(!workflow().preview); assert.match(state.dialog.error, /lfs_prune_verification/);
    assert.equal(hash(cacheAt(f.client)), oid); await capture('lfs-auth-rejected'); f.auth(true);
    pass('Fixture-only authentication refusal is clear and preserves cache; personal login not revoked');
    await review(); await click('dialog-submit'); assert(!state.dialog.error, state.dialog.error); assert.match(workflow().notice, /cleanup finished/);
    assert(!fs.existsSync(cacheAt(f.client))); assert.deepEqual(snapshot(f), before); await capture('lfs-pruned');
    download(f); f.assertRemote(''); pass('Exactly one 72-byte object pruned locally; downloaded identical bytes again; HEAD/index/files and remote refs preserved');
    }
    if (browser) await browserCheck();
    assert.deepEqual(personalSnapshot(), personalBefore); report.personal_files_unchanged = true;
    await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit');
    if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window)); }
    await client.raw('POST', '/shutdown'); await delay(700);
    const events = logs(); assert(events.length > 0); assert(events.every(r => r.dropped_before === 0));
    for (const r of events.filter(r => r.event === 'request_start')) assert(events.some(e => e.event === 'request_end' && e.session === r.session && e.request === r.request));
    assert(events.some(r => r.failure_code === 'lfs_prune_verification'));
    assert(events.some(r => r.operation === 'lfs-prune' || r.operation === 'lfs-prune-preview'));
    const serialized = JSON.stringify(events);
    for (const privateValue of ['ROCKET_PRIVATE', 'rocket-auth-test', 'JasXTD', '保留', oid, 'gho_', 'ghp_', 'github_pat_', 'Authorization:']) assert(!serialized.includes(privateValue), 'Diagnostic privacy: ' + privateValue);
    report.log_records = events.length;
    pass('Diagnostics have paired requests, typed failure, zero dropped events and no tested private payloads');
    report.passed = true;
  } catch (e) {
    savedError = e; report.failure = e.stack; console.error(e.message); process.exitCode = 1;
    if (client) { try { state = await client.tool('rocket_state'); fs.writeFileSync(path.join(output, 'failure-state.json'), JSON.stringify(state, null, 2)); } catch {} }
  } finally {
    fs.rmSync(gate, { force: true });
    try { f.auth(true); } catch (e) { report.cleanup_error = e.message; process.exitCode = 1; report.passed = false; }
    if (client) { try { await client.raw('POST', '/shutdown'); } catch {} await client.finish(); }
    if (service && service.exitCode === null) { await delay(800); if (service.exitCode === null) service.kill(); }
    report.personal_files_unchanged = JSON.stringify(personalSnapshot()) === JSON.stringify(personalBefore);
    if (!report.personal_files_unchanged) { report.passed = false; process.exitCode = 1; }
    save(); console.log('RESULT ' + reportPath);
  }
}
run();
