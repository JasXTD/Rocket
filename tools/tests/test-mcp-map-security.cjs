// Native history-link budget acceptance. Synthetic local Git and isolated MCP only.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const fixtureBase = path.resolve('work/mcp-fixtures');
const root = path.join(fixtureBase, 'map-security-' + crypto.randomUUID());
const evidence = path.resolve(process.argv[2] || path.join('work', 'map-security-results-' + crypto.randomUUID()));
assert(!fs.existsSync(evidence), 'Choose a new evidence directory');
const profile = path.join(root, 'http-profile'), repo = path.join(root, 'history 資料');
const branch = 'rocket-ui-test/map-security';
const limitedBranch = 'rocket-ui-test/map-at-limit', denseBranch = 'rocket-ui-test/map-over-limit';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [], timings: {} };
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
for (const dir of [root, evidence, repo, path.join(root, 'empty-template')]) fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(path.join(root, 'empty.config'), '');
fs.writeFileSync(path.join(root, 'rocket-private-state'), 'synthetic-content-only-v1\n');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const git = (args, input) => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8', input, maxBuffer: 32 * 1024 * 1024 }).trimEnd();
let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const active = () => state.tabs.find(t => t.id === state.active_tab);
const map = () => active().repository_map;
const control = id => state.controls.find(c => c.id === id);
const pass = text => { report.cases.push(text); console.error('PASS ' + text); };
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return;
    assert(performance.now() - start < 30000, label + ' timed out');
    await delay(60);
  }
}
async function settle() { await until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running)); }
async function stable() {
  let previous = '', count = 0;
  await until(s => { const next = JSON.stringify(s.controls); count = previous === next ? count + 1 : 0; previous = next; return count >= 2; }, 'Stable controls');
}
async function click(target) {
  await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_click', { target, expected_tab: state.active_tab });
  await settle(); await stable();
}
async function type(target, text) {
  await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_type', { target, text, expected_tab: state.active_tab });
  await settle(); await stable();
}
async function finished() { await until(() => !map().running && !state.input_pending); assert(!map().error, map().error); await stable(); }
async function capture(name) {
  await stable();
  const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!result.isError);
  const image = result.content.find(c => c.type === 'image'); assert(image);
  fs.writeFileSync(path.join(evidence, name + '.png'), Buffer.from(image.data, 'base64'));
  fs.writeFileSync(path.join(evidence, name + '.json'), JSON.stringify(state, null, 2));
}
async function closeWindow() {
  await client.tool('rocket_close');
  await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit' && !s.input_pending);
  if (state.dialog?.kind === 'quit') {
    await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab });
    await until(s => ['closed', 'not_started'].includes(s.window));
  }
}
function fixture() {
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Map Fixture'], ['user.email', 'map@example.invalid'], ['commit.gpgsign', 'false'], ['core.hooksPath', path.join(root, 'empty-template')]]) git(['config', key, value]);
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  let input = '';
  for (let id = 1; id <= 657; id++) {
    const message = 'PRIVATE_MAP_HISTORY_' + id;
    input += `commit refs/heads/${branch}\nmark :${id}\ncommitter Map Fixture <map@example.invalid> ${1735689600 + id} +0000\ndata ${message.length}\n${message}\n`;
    if (id > 1) {
      input += `from :${id - 1}\n`;
      const parents = id < 156 ? id - 1 : id === 156 ? 65 : 1;
      for (let parent = 1; parent < parents; parent++) input += `merge :${parent}\n`;
    }
    input += '\n';
  }
  input += `reset refs/heads/${limitedBranch}\nfrom :156\n\nreset refs/heads/${denseBranch}\nfrom :157\n\ndone\n`;
  git(['fast-import', '--quiet', '--done'], input);
  git(['reset', '--hard', branch]);
  fs.writeFileSync(path.join(repo, 'local-untracked'), 'preserve fixture data');
}
function snapshot() { return { head: git(['rev-parse', 'HEAD']), index: hash(path.join(repo, '.git/index')), config: hash(path.join(repo, '.git/config')), local: hash(path.join(repo, 'local-untracked')) }; }
async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe');
    report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
    const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), `ROCKET-MCP-1\n${port}\n${root}\n`);
    const log = fs.openSync(path.join(evidence, 'service.log'), 'w');
    service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] });
    fs.closeSync(log); report.service_pid = service.pid; client = new RocketHttpClient(profile);
    for (let attempt = 0;; attempt++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (attempt === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    fixture(); const before = snapshot();
    await client.tool('rocket_open', { path: repo }); await settle();
    await client.tool('rocket_resize', { width: 1480, height: 920 }); await until(s => s.viewport[0] === 1480 && !s.input_pending);
    if (control('console-close')) await click('console-close');
    await click('view-map'); await click('map-view-history'); await finished();
    assert.equal(map().data.node_count, 500); assert.equal(map().data.link_count, 499); assert(map().can_more);
    const previous = JSON.stringify(map().data);
    const start = performance.now(); await click('map-more');
    await until(() => !map().running && !!map().error && !state.input_pending, 'Dense Load more refusal');
    report.timings.refused_more_ms = performance.now() - start;
    assert.match(map().error, /12,000 parent links/); assert.equal(JSON.stringify(map().data), previous); assert(map().can_more);
    await capture('load-more-refused');
    await type('map-search', 'PRIVATE_MAP_HISTORY_657'); assert(map().visible.length && map().visible.length < 500);
    await click('map-clear'); assert.equal(map().query, '');
    pass('Dense Load more is refused, previous pinned data is intact and search/clear remain responsive');
    await type('map-revision', denseBranch); await click('map-load');
    await until(() => !map().running && !!map().error && !state.input_pending, 'Dense initial read refusal');
    assert.match(map().error, /12,000 parent links/); assert.equal(JSON.stringify(map().data), previous);
    pass('Initial 12,001-link scope is refused without replacing the previous graph');
    await type('map-revision', limitedBranch); await click('map-load'); await finished();
    assert.equal(map().data.node_count, 156); assert.equal(map().data.link_count, 12000); assert(!map().can_more);
    await click('map-fit'); await capture('exact-budget-wide');
    await client.tool('rocket_resize', { width: 780, height: 620 }); await until(s => s.viewport[0] === 780 && !s.input_pending);
    await type('map-search', 'PRIVATE_MAP_HISTORY_156'); assert(map().visible.length > 0);
    await click('map-clear'); assert.equal(map().query, ''); await capture('exact-budget-compact');
    assert.deepEqual(snapshot(), before);
    pass('Exactly 12,000 links render at wide/compact sizes; local HEAD, index, configuration and edits are preserved');
    await closeWindow(); await client.raw('POST', '/shutdown');
    report.passed = true;
  } catch (e) {
    report.failure = e.stack; console.error(e.stack);
    if (client) { try { await until(() => true); await capture('failure'); } catch (captureError) { report.capture_error = String(captureError); } }
    process.exitCode = 1;
  } finally {
    if (service && service.exitCode === null) {
      try { state = await client.tool('rocket_state'); if (!['closed', 'not_started'].includes(state.window)) await closeWindow(); await client.raw('POST', '/shutdown'); } catch (e) { report.shutdown_note = String(e); }
      for (let n = 0; n < 50 && service.exitCode === null; n++) await delay(100);
      if (service.exitCode === null) { report.forced_service_stop = true; service.kill(); for (let n = 0; n < 50 && service.exitCode === null; n++) await delay(100); }
    }
    if (client) await client.finish();
    const token = path.join(profile, 'access-token'); if (fs.existsSync(token)) fs.unlinkSync(token);
    report.cleanup = { service_exit: service?.exitCode, token_removed: !fs.existsSync(token) };
    if (service?.exitCode !== 0 || report.forced_service_stop) { report.passed = false; process.exitCode = 1; }
    if (report.passed) {
      assert.equal(path.dirname(fs.realpathSync(root)), fs.realpathSync(fixtureBase));
      assert(!fs.lstatSync(root).isSymbolicLink());
      fs.rmSync(root, { recursive: true, force: false }); report.cleanup.fixture_removed = !fs.existsSync(root);
    }
    fs.writeFileSync(path.join(evidence, 'result.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  }
}
run().catch(e => { console.error(e.stack); process.exitCode = 1; });
