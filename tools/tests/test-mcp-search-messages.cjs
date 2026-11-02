// Native Windows acceptance for 0.38.0 through authenticated localhost MCP:
// history search by changed path and changed content, and the commit message
// helpers (commit.template prefill and guidance, recent messages, persistence).
// Synthetic, owned repositories only; no network.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/search-messages-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/search-messages';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const TEMPLATE = 'feat: \n\n# Why is this change needed?\n# PRIVATE_GUIDANCE 说明\n';

function fixture() {
  const repo = path.join(root, 'PRIVATE_SEARCH'), hooks = path.join(root, 'hooks'); fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  const commit = message => { git(['add', '-A']); git(['commit', '-m', message]); return git(['rev-parse', 'HEAD']); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Search Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  write('src/Main.rs', 'fn main() {}\n'); write('docs/说明.md', 'PRIVATE_NOTES\n');
  const ids = { root: commit('PRIVATE_ROOT entry point') };
  write('src/Main.rs', 'fn main() { needle(); }\n'); ids.call = commit('PRIVATE_CALL');
  git(['mv', 'docs/说明.md', 'docs/guide.md']); ids.rename = commit('PRIVATE_RENAME');
  write('README.md', 'Needle Text\n'); ids.docs = commit('PRIVATE_DOCS');
  fs.writeFileSync(path.join(repo, '.git/message-template.txt'), TEMPLATE);
  git(['config', 'commit.template', '.git/message-template.txt']);
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo, git, write, ids, config: hash(path.join(repo, '.git/config')) };
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
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, search: tab()?.search, composer: tab()?.composer, write: state.write, notice: state.notice }));
    await delay(80);
  }
}
const tab = () => state?.tabs?.find(t => t.id === state.active_tab);
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.scanning || t.composer?.loading || t.composer?.pending), 'Settled'); }
function control(id) { return state.controls.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function open(f) { await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920); await closeConsole(); }
async function closeWindow() { if (state?.dialog) await click('dialog-cancel'); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
const overlaps = (a, b) => a.rect[0] < b.rect[2] && b.rect[0] < a.rect[2] && a.rect[1] < b.rect[3] && b.rect[1] < a.rect[3];

async function searchFor(query, kind, expected) {
  if (tab().search.kind !== kind) { await click('search-kind'); await click('search-kind-' + kind); }
  await type('commit-filter', query);
  await until(s => { const t = s.tabs.find(x => x.id === s.active_tab); return t.search.query === query && t.search.kind === kind && t.search.complete && !t.search.scanning; }, 'Search ' + kind);
  assert.equal(tab().search.error, null);
  assert.deepEqual(tab().history_ids, expected, `${kind} search for ${query}`);
}

async function searches(f) {
  await open(f); await click('view-commits');
  // Switching kinds restarts the same text as a path search.
  await type('commit-filter', 'main.rs');
  await until(s => tab().search.complete && !tab().search.scanning, 'Text search');
  assert.deepEqual(tab().history_ids, []);
  await click('search-kind'); await capture('search-kind-menu'); await click('search-kind-path');
  await until(s => tab().search.kind === 'path' && tab().search.complete && !tab().search.scanning, 'Path search');
  assert.deepEqual(tab().history_ids, [f.ids.call, f.ids.root]);
  await searchFor('docs\\说明', 'path', [f.ids.rename, f.ids.root]);
  await capture('search-path-wide');
  passed('Path search: the kind selector restarts the query; matches are case-insensitive, accept \\ for /, include both sides of a rename and Unicode paths');
  await searchFor('NEEDLE', 'content', [f.ids.docs, f.ids.call]);
  await searchFor('needle ()', 'content', []);
  await resize(780, 620); await closeConsole(); await stable();
  for (const id of ['history-scope', 'search-kind', 'commit-filter']) assert(control(id)?.rect[2] <= 780, id + ' clipped');
  assert(!overlaps(control('search-kind'), control('commit-filter')) && !overlaps(control('history-scope'), control('search-kind')), 'Search bar controls overlap at 780x620');
  await capture('search-content-narrow'); await resize(1480, 920);
  passed('Content search finds commits that add or remove the text (ignoring case, literal), and the search bar fits at 780x620');
  await searchFor('PRIVATE_RENAME', 'text', [f.ids.rename]);
  await type('commit-filter', '');
  await until(s => tab().search.query === '' && tab().history_ids.length === 4, 'Full history');
  passed('Text search still matches messages; clearing the query shows the full history');
}

async function stageAndCommit(f, file) {
  f.write(file, crypto.randomUUID() + '\n');
  await until(s => tab().change_count > 0, 'Change visible');
  await click('stage-all'); await click('commit');
  await until(s => !s.write && tab().change_count === 0 && !tab().composer.loading, 'Commit');
  return f.git(['log', '-1', '--format=%B']);
}

async function composer(f) {
  await click('view-changes');
  await until(s => tab().composer?.template?.message_bytes === 5 && tab().draft_present, 'Template prefill');
  assert.equal(tab().composer.template.guidance_lines, 2); assert.equal(tab().composer.recent, 0);
  await capture('composer-template');
  assert.equal(await stageAndCommit(f, 'one.txt'), 'feat:');
  assert.equal(tab().composer.recent, 1); assert(tab().draft_present, 'The next draft starts from the template again');
  passed('An empty composer starts from commit.template (comment lines become guidance, never committed); after a commit the next draft starts from it again');
  await type('commit-message', 'fix: PRIVATE_TYPED 文');
  assert.equal(await stageAndCommit(f, 'two.txt'), 'fix: PRIVATE_TYPED 文');
  assert.equal(tab().composer.recent, 2);
  await click('composer-messages'); await capture('composer-messages-menu');
  await click('composer-recent-0');
  assert.equal(await stageAndCommit(f, 'three.txt'), 'fix: PRIVATE_TYPED 文');
  assert.equal(tab().composer.recent, 2, 'Reusing a message does not duplicate it');
  await click('composer-messages'); await click('composer-template');
  assert.equal(await stageAndCommit(f, 'four.txt'), 'feat:');
  passed('Messages menu: a recent message or the template replaces the draft; reuse keeps the list unique, newest first');
  await click('composer-messages'); await click('composer-recent-clear');
  assert.equal(tab().composer.recent, 0);
  await type('commit-message', 'chore: PRIVATE_PERSISTED');
  assert.equal(await stageAndCommit(f, 'five.txt'), 'chore: PRIVATE_PERSISTED');
  assert.equal(tab().composer.recent, 1);
  // Recent messages survive a restart; a broken template is reported, not committed.
  f.git(['config', 'commit.template', 'missing-template.txt']);
  await until(s => !s.state_pending, 'State saved');
  await closeWindow(); await open(f); await click('view-changes');
  await until(s => tab().composer && !tab().composer.loading && tab().composer.template_error, 'Template error after reopen');
  assert.equal(tab().composer.recent, 1); assert.equal(tab().composer.template, null);
  assert(tab().composer.template_error.includes('commit.template'), tab().composer.template_error);
  await click('composer-messages'); assert(control('composer-template') && !control('composer-template').enabled, 'Template action disabled');
  await capture('composer-template-error'); await click('composer-recent-0');
  assert.equal(await stageAndCommit(f, 'six.txt'), 'chore: PRIVATE_PERSISTED');
  passed('Recent messages persist across restarts; clearing empties the list; a missing template is shown as unavailable');
}

async function run() {
  let f;
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    f = fixture();
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await searches(f); await composer(f);
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE_', '说明', 'eedle', 'main.rs', 'message-template', 'Why is this change', 'feat:']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    for (const op of ['history-search', 'commit-template', 'commit']) { const starts = records.filter(r => r.event === 'request_start' && r.operation === op); assert(starts.length, 'Missing ' + op); for (const start of starts) assert(records.some(r => r.event === 'request_end' && r.session === start.session && r.request === start.request), 'Unpaired ' + op); }
    assert(records.some(r => r.event === 'git_start' && r.operation === 'diff-tree') && records.some(r => r.event === 'git_start' && r.operation === 'log'), 'Path/content search commands');
    report.log_records = records.length; passed('Paired history-search/commit-template diagnostics with fixed labels, zero dropped events, no queries, paths, templates or messages'); report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync('work/search-messages-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
run().catch(e => { console.error(e); process.exitCode = 1; });
