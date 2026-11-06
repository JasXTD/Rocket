// Native Windows acceptance through authenticated localhost MCP: Changed code in the
// commit search (git log -S / -G, match case, path) and Search in files (git grep at the
// checkout and a revision): results, opening matches, limits, Stop, special files, the
// hand-overs between them, the compact layout and diagnostics privacy.
// Synthetic, owned repositories only; no network. Screenshots go next to result.json.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/code-search-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/code-search';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const POINTER = 'version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 48211904\n';

function fixture(name) {
  const repo = path.join(root, name), hooks = path.join(root, name + '-hooks'); fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = (args, input) => cp.execFileSync('git', ['-C', repo, ...args], { env, input, windowsHide: true, encoding: 'utf8', maxBuffer: 1 << 28 }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  const commit = message => { git(['add', '-A']); git(['commit', '-q', '-m', message]); return git(['rev-parse', 'HEAD']); };
  git(['init', '-q', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Search Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  return { repo, git, write, commit };
}

function main_fixture() {
  const f = fixture('PRIVATE_CODE_SEARCH');
  const ids = {};
  f.write('src/net/retry.rs', 'pub const RETRY_LIMIT: u32 = 3;\n\npub fn retry() {\n    for _ in 0..RETRY_LIMIT {}\n}\n');
  ids.add = f.commit('PRIVATE_ADD retry loop');
  f.write('src/net/retry.rs', 'pub const RETRY_LIMIT: u32 = 5;\n\npub fn retry() {\n    for _ in 0..RETRY_LIMIT {}\n}\n');
  ids.raise = f.commit('PRIVATE_RAISE retry limit');
  f.write('src/net/fetch.rs', 'use super::retry::{retry, RETRY_LIMIT};\r\n// try RETRY_LIMIT times\r\n');
  f.write('docs/guide.md', 'Retry up to RETRY_LIMIT times.\n');
  f.write('legacy/latin1.txt', Buffer.from([...Buffer.from('caf'), 0xe9, ...Buffer.from(' RETRY_LIMIT = 3\n')]));
  f.write('assets/logo.png', Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0, 0, 0]), Buffer.from('RETRY_LIMIT'), Buffer.alloc(32, 0)]));
  f.write('assets/hero.psd', POINTER);
  f.write('data/huge.log', 'RETRY_LIMIT reached\n'.repeat(133_120));
  f.write('.gitignore', 'build/\n');
  ids.assets = f.commit('PRIVATE_ASSETS and data');
  for (let n = 0; n < 25; n++) f.write(`many/f${String(n).padStart(2, '0')}.txt`, 'needle\n'.repeat(120));
  ids.many = f.commit('PRIVATE_MANY needles');
  f.write('notes/new.txt', 'an untracked RETRY_LIMIT\n');
  f.write('build/out.txt', 'an ignored RETRY_LIMIT\n');
  fs.writeFileSync(path.join(f.repo, '.git/rocket-ui-fixture'), branch);
  return { ...f, ids };
}

/// 30,000 checked-out files, so a search runs long enough to stop it.
function slow_fixture() {
  const f = fixture('PRIVATE_SLOW_SEARCH');
  const lines = [];
  lines.push(`commit refs/heads/${branch}\ncommitter Rocket Search Fixture <rocket@example.invalid> 1700000000 +0000\ndata 4\nslow\n`);
  const body = Array.from({ length: 40 }, (_, i) => `    let value_${i} = compute(${i}, "lorem ipsum dolor sit amet");`).join('\n') + '\n';
  for (let i = 0; i < 30000; i++) lines.push(`M 100644 inline s${i % 60}/f${i}.rs\ndata ${Buffer.byteLength(body)}\n${body}\n`);
  f.git(['fast-import', '--quiet'], lines.join('') + '\n');
  f.git(['reset', '--hard', '-q']);
  fs.writeFileSync(path.join(f.repo, '.git/rocket-ui-fixture'), branch);
  return f;
}

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 60000);
async function until(predicate, label = 'UI state', ms = WAIT_MS) {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < ms, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, search: tab()?.search, files: tab()?.file_search && { ...tab().file_search, files: undefined }, notice: state.notice }));
    await delay(80);
  }
}
const tab = () => state?.tabs?.find(t => t.id === state.active_tab);
const fsearch = () => tab().file_search;
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.scanning || t.repository_tree?.running || t.file_search?.running || t.file_search?.opening), 'Settled'); }
function control(id) { return state.controls.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target, button) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab, ...(button ? { button } : {}) }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); }
async function scroll(target, direction = 'down', pixels = 200) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); await stable(); }
// The results list draws only the rows on screen, so scroll a row into view first.
async function reveal(id) { for (let i = 0; i < 12 && !control(id); i++) await scroll('files-results-scroll'); assert(control(id), 'Not revealed ' + id); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending, 'Resize ' + width); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function open(repo) { if (state?.dialog) await client.tool('rocket_escape'); await client.tool('rocket_open', { path: repo }); await settle(); await resize(1480, 920); await closeConsole(); }
async function closeWindow() { await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
const overlaps = (a, b) => a.rect[0] < b.rect[2] && b.rect[0] < a.rect[2] && a.rect[1] < b.rect[3] && b.rect[1] < a.rect[3];

async function codeSearch(query) {
  await type('commit-filter', query);
  await until(s => { const t = s.tabs.find(x => x.id === s.active_tab); return t.search.query === query && !t.search.scanning; }, 'Commit search ' + query);
  return tab().history_ids;
}
async function setToggle(id, on, key) { if (tab().search[key] !== on) { await click(id); await until(s => tab().search[key] === on, id); } }

async function commits(f) {
  await open(f.repo); await click('view-commits');
  await click('search-kind'); await capture('code-kind-menu'); await click('search-kind-content');
  assert.equal(tab().search.kind, 'content');
  // Exact text (-S): changing 3 to 5 keeps the count, so PRIVATE_RAISE is missing.
  assert.deepEqual(await codeSearch('RETRY_LIMIT'), [f.ids.assets, f.ids.add]);
  assert.equal(tab().search.diff_find, 'RETRY_LIMIT', 'Exact text fills the diff Find');
  await setToggle('search-regex', true, 'regex');
  await until(s => !tab().search.scanning && tab().search.complete, 'Regex search');
  assert.deepEqual(tab().history_ids, [f.ids.assets, f.ids.raise, f.ids.add]);
  assert.equal(tab().search.diff_find, '', 'A regex search clears the Find it filled');
  await type('commit-search-path', 'src\\net\\retry.rs');
  await until(s => tab().search.path === 'src\\net\\retry.rs' && !tab().search.scanning && tab().search.complete, 'Path search');
  assert.deepEqual(tab().history_ids, [f.ids.raise, f.ids.add]);
  await capture('code-regex-path-wide');
  passed('Changed code: exact text uses -S, .* uses -G and finds the edited line, In path keeps one file (\\ read as /), exact text fills the diff Find');
  await setToggle('search-case', true, 'match_case');
  assert.deepEqual(await codeSearch('retry_limit'), []);
  assert(state.controls.some(c => c.id === 'history-empty'), 'No-match line');
  await setToggle('search-case', false, 'match_case');
  assert.deepEqual(await codeSearch('retry_limit'), [f.ids.raise, f.ids.add]);
  await codeSearch('RETRY_(');
  assert(tab().search.bad_pattern && /^Not a valid regular expression: /.test(tab().search.error), JSON.stringify(tab().search));
  assert(control('history-search-error'), 'Bad expression strip');
  await capture('code-bad-regex');
  passed('Match case narrows the search; a bad expression shows Git\'s reason and runs nothing');
  await resize(780, 620); await closeConsole(); await stable();
  for (const id of ['search-kind', 'commit-filter', 'search-regex', 'commit-search-path']) assert(control(id)?.rect[2] <= 780, id + ' clipped');
  assert(!overlaps(control('commit-filter'), control('commit-search-path')), 'Field and path overlap at 780x620');
  assert(control('commit-search-path').rect[1] > control('commit-filter').rect[1], 'The path takes its own row at 780x620');
  await capture('code-narrow'); await resize(1480, 920);
  await type('commit-search-path', ''); await setToggle('search-regex', false, 'regex'); await codeSearch('');
  passed('At 780x620 the path moves to its own row and nothing is clipped');
}

async function filesSearch(text) {
  await type('files-search', text);
  await click('files-search-go');
  await until(s => !tab().file_search.running && (tab().file_search.ended || tab().file_search.error), 'Files search ' + text);
  return fsearch().files.map(g => g.path);
}

async function files(f) {
  // The shortcut opens Files with the search field ready.
  await client.tool('rocket_shortcut', { binding: 'Ctrl+Shift+F', expected_tab: state.active_tab }); await settle();
  await until(s => tab().view === 'Tree' && tab().file_search.results_pane, 'Ctrl+Shift+F opens Search in files');
  await capture('files-before');
  assert.deepEqual(await filesSearch('retry_limit'), ['data/huge.log', 'docs/guide.md', 'legacy/latin1.txt', 'notes/new.txt', 'src/net/fetch.rs', 'src/net/retry.rs']);
  let s = fsearch();
  assert.equal(s.ended, 'Complete'); assert.equal(s.matches, 100 + 1 + 1 + 1 + 2 + 2);
  const huge = s.files[0], latin = s.files[2];
  assert(huge.capped && huge.size === 20 * 133120 && !huge.open, 'Large file: capped, sized, collapsed');
  assert(latin.not_utf8 && latin.lines[0].text.startsWith('caf� '), 'Latin-1 line shown with a replacement mark');
  assert.deepEqual(s.files[4].lines.map(l => l.number), [1, 2]);
  await capture('files-results');
  passed('Search in files at the checkout: untracked included, ignored and binary skipped, grouped by file, large files capped and collapsed, Latin-1 marked');
  await click('files-match-5-1');
  await until(s => fsearch().open?.content, 'Open retry.rs');
  s = fsearch();
  assert.equal(s.open.number, 4); assert.equal(s.open.content.focus_text, '    for _ in 0..RETRY_LIMIT {}'); assert.equal(s.open.content.marked_lines, 2);
  await capture('files-open-match');
  await click('files-match-previous');
  assert.equal(fsearch().open.number, 1); assert.deepEqual(fsearch().selected, [5, 0]);
  await click('files-result-0'); await stable();
  assert(fsearch().files[0].open, 'Clicking a file row expands it');
  await click('files-match-0-10');
  await until(s => fsearch().open?.content, 'Open huge.log');
  s = fsearch().open.content;
  assert(s.window && s.first === 1 && s.line_count === 211 && s.focus_text === 'RETRY_LIMIT reached', JSON.stringify(s));
  await capture('files-large-window');
  // Collapse it again so the later files' rows are on screen.
  await click('files-result-0'); await stable();
  await click('files-match-2-0');
  await until(s => fsearch().open?.content?.lossy, 'Open latin1.txt');
  passed('A match opens its file at the line with matches marked; arrows step through matches; a 2.6 MB file opens 200 lines around the match; non-UTF-8 text opens with replacement marks');
  // The revision shown: no untracked file, and an LFS pointer is labelled.
  await click('files-search-scope'); await click('files-search-scope-revision');
  assert.deepEqual(await filesSearch('RETRY_LIMIT'), ['data/huge.log', 'docs/guide.md', 'legacy/latin1.txt', 'src/net/fetch.rs', 'src/net/retry.rs']);
  assert.match(fsearch().searched, /^[0-9a-f]{7}$/);
  assert.deepEqual(await filesSearch('4d7a2146'), ['assets/hero.psd']);
  assert(fsearch().files[0].lfs_pointer, 'LFS pointer labelled');
  assert.deepEqual(await filesSearch('RETRY_LIMT'), []);
  assert(control('files-search-empty'), 'No-match message');
  await capture('files-none');
  passed('The revision shown has no untracked file; a match in a Git LFS pointer is labelled; no matches says where it looked');
  assert.equal((await filesSearch('needle')).length, 20);
  assert.equal(fsearch().ended, 'TooMany'); assert.equal(fsearch().matches, 2000);
  assert(control('files-search-message'), 'Too many strip');
  await capture('files-too-many');
  await click('files-search-regex');
  await filesSearch('retry(');
  assert(fsearch().bad_pattern && /^Not a valid regular expression: /.test(fsearch().error), JSON.stringify(fsearch().error));
  assert.deepEqual(fsearch().files, []);
  await capture('files-bad-regex');
  await click('files-search-regex');
  passed('Too many results stop at 2,000 and say so; a bad expression shows Git\'s reason and clears the results');
  // Find in commits from an opened match.
  await click('files-search-scope'); await click('files-search-scope-checkout');
  await filesSearch('RETRY_LIMIT');
  await click('files-match-5-0'); await until(s => fsearch().open?.content, 'Open retry.rs again');
  await click('files-find-in-commits');
  await until(s => tab().view === 'Commits' && tab().search.kind === 'content' && tab().search.complete && !tab().search.scanning, 'Find in commits');
  assert.equal(tab().search.path, 'src/net/retry.rs'); assert.deepEqual(tab().history_ids, [f.ids.add]);
  passed('Find in commits opens Changed code for the same text in that file');
  // A commit's right-click menu searches files at that commit.
  await codeSearch('');
  const row = state.controls.find(c => /^commit-[0-9a-f]{40}$/.test(c.id) && c.enabled);
  await click(row.id, 'right'); await click('commit-menu-search-files');
  await until(s => tab().view === 'Tree' && tab().file_search.scope === 'Revision' && tab().repository_tree.tip, 'Search files at this commit');
  passed('Search files at this commit opens Files with the revision chosen');
  await resize(780, 620); await closeConsole();
  await filesSearch('RETRY_LIMIT');
  await capture('files-narrow-results');
  for (const id of ['files-search', 'files-search-scope', 'files-search-path', 'files-search-go']) assert(control(id)?.rect[2] <= 780, id + ' clipped');
  await reveal('files-match-4-0');
  await click('files-match-4-0'); await until(s => fsearch().open?.content && fsearch().show_file, 'Narrow open');
  assert(control('files-show-results'), 'Show results in a narrow window');
  await capture('files-narrow-file');
  await click('files-show-results'); assert(!fsearch().show_file);
  await resize(1480, 920);
  passed('At 780x620 the search row wraps, results and the file show one at a time');
}

async function stop(slow) {
  await open(slow.repo);
  await client.tool('rocket_shortcut', { binding: 'Ctrl+Shift+F', expected_tab: state.active_tab }); await settle();
  await type('files-search', 'value_\\d+ = never');
  report.stop_attempts = 0;
  for (let attempt = 0; attempt < 4; attempt++) {
    report.stop_attempts++;
    await stable();
    await client.tool('rocket_click', { target: 'files-search-go', expected_tab: state.active_tab });
    await until(s => !s.input_pending, 'Search started');
    state = await client.tool('rocket_state');
    if (control('files-search-stop')?.enabled) {
      const asked = performance.now();
      await client.tool('rocket_click', { target: 'files-search-stop', expected_tab: state.active_tab });
      await until(s => !tab().file_search.running, 'Stopped', 15000);
      report.stop_ms = Math.round(performance.now() - asked);
      if (fsearch().ended === 'Stopped') break;
    }
    await until(s => !tab().file_search.running, 'Search ended');
  }
  assert.equal(fsearch().ended, 'Stopped', 'Stop ended a running search');
  assert(control('files-search-message'), 'Stopped strip');
  await capture('files-stopped');
  passed(`Stop ends a running search at once (${report.stop_ms} ms through MCP) and keeps what was found`);
}

async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    const f = main_fixture(), slow = slow_fixture();
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await commits(f); await files(f); await stop(slow);
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE_', 'RETRY', 'retry_limit', 'needle', '4d7a2146', 'latin1', 'huge.log', 'value_']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    for (const op of ['history-search', 'file-search', 'file-search-open']) { const starts = records.filter(r => r.event === 'request_start' && r.operation === op); assert(starts.length, 'Missing ' + op); for (const start of starts) assert(records.some(r => r.event === 'request_end' && r.session === start.session && r.request === start.request), 'Unpaired ' + op); }
    assert(records.some(r => r.event === 'git_start' && r.operation === 'grep'), 'git grep is labelled');
    report.log_records = records.length; passed('Paired history-search, file-search and file-search-open diagnostics with fixed labels (grep), zero dropped events, no queries, paths or file text'); report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
run().catch(e => { console.error(e); process.exitCode = 1; });
