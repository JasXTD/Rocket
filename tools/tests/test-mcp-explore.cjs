// Native Windows acceptance for Explore and repository previews through authenticated
// localhost MCP: a loopback fake GitHub search API (signed out, then signed in with a
// fake credential helper), paging, sort, rate limit, refused and empty searches, a
// pasted local address, and previews of local filter-enabled sources: the bare clone
// in the test root's preview folder, Files and a commit diff that downloads an older
// file version, read-only refusals, the size limit and "Continue without a limit",
// failure cards, Discard, Keep into a new folder, and cleanup of every preview folder.
// Links are recorded by the MCP host, never opened.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), http = require('node:http'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
// Outside any repository: Clone runs Git in a folder beside its destination, and Git
// would otherwise read the enclosing checkout's settings (its credential helper), which
// the MCP transport scope refuses.
const root = path.join(require('node:os').tmpdir(), 'rocket-mcp-explore-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/explore';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template'));
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0', GIT_NO_LAZY_FETCH: '1' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const TOKEN = 'gho_rocketExploreFake' + crypto.randomBytes(8).toString('hex');
const calls = path.join(root, 'helper-calls');
const LIMIT = 1024 * 1024; // the runner's preview limit: 1 MB
const previews = path.join(root, 'rocket-previews'), stale = path.join(previews, 'p1-dead-0');

// ---- Sources -----------------------------------------------------------------------
const git = (dir, args, input) => cp.execFileSync('git', ['-C', dir, ...args], { env, windowsHide: true, encoding: 'utf8', input, stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
function repository(dir, initial) {
  fs.mkdirSync(dir, { recursive: true });
  git(dir, ['init', '--initial-branch=' + initial]);
  for (const [key, value] of [['user.name', 'Rocket Explore Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['uploadpack.allowFilter', 'true']]) git(dir, ['config', key, value]);
  return (files, message) => {
    for (const [name, body] of Object.entries(files)) { fs.mkdirSync(path.dirname(path.join(dir, name)), { recursive: true }); fs.writeFileSync(path.join(dir, name), body); }
    git(dir, ['add', '-A']); git(dir, ['commit', '-m', message]); return git(dir, ['rev-parse', 'HEAD']);
  };
}
const fuzzy = path.join(root, 'sources', 'fuzzy-finder'), huge = path.join(root, 'sources', 'huge-assets'), empty = path.join(root, 'sources', 'not-a-repo');
const commits = {};
function sources() {
  const commit = repository(fuzzy, 'main');
  commits.start = commit({ 'README.md': '# fuzzy-finder\n\nFind files by typing a few letters.\n', 'src/main.rs': 'fn main() {\n    println!("finder");\n}\n' }, 'Start the finder');
  commits.match = commit({ 'src/matcher.rs': 'pub fn score(query: &str, path: &str) -> u32 {\n    path.matches(query).count() as u32\n}\n' }, 'Add matching');
  commits.rank = commit({ 'src/matcher.rs': 'pub fn score(query: &str, path: &str) -> u32 {\n    let hits = path.matches(query).count() as u32;\n    hits * 10 - path.len().min(9) as u32\n}\n' }, 'Rank results');
  commits.docs = commit({ 'README.md': '# fuzzy-finder\n\nFind files by typing a few letters.\n\nSee docs/usage.md.\n', 'docs/usage.md': '# Usage\n\nType part of a name.\n' }, 'Document usage');
  // Two more branches whose files the preview downloads only when a view or Keep needs them.
  git(fuzzy, ['switch', '-c', 'dev']); commits.dev = commit({ 'src/dev.rs': 'pub fn dev() {}\n' }, 'Try a dev idea');
  git(fuzzy, ['switch', '-c', 'notes', 'main']); commits.notes = commit({ 'notes.md': '# Notes\n' }, 'Add notes');
  git(fuzzy, ['switch', 'main']);
  const big = repository(huge, 'main');
  big({ 'assets/big.bin': crypto.randomBytes(3 * LIMIT), 'README.md': '# huge-assets\n' }, 'Add assets');
  fs.mkdirSync(empty, { recursive: true });
}
function home() {
  const dir = path.join(root, 'home'); const commit = repository(dir, branch);
  commit({ 'notes.txt': 'home\n' }, 'Home fixture');
  fs.writeFileSync(path.join(dir, '.git/rocket-ui-fixture'), branch);
  return { repo: dir, config: hash(path.join(dir, '.git/config')) };
}
const plain = p => p.replace(/^\\\\\?\\/, '');
const present = (dir, oid) => { try { git(dir, ['--no-lazy-fetch', 'cat-file', '-e', oid]); return true; } catch { return false; } };
// The source's objects moved away: it still looks like a folder to MCP, but nothing
// can be downloaded from it until they're back.
const breakSource = () => fs.renameSync(path.join(fuzzy, '.git', 'objects'), path.join(fuzzy, '.git', 'objects-away'));
const fixSource = () => fs.renameSync(path.join(fuzzy, '.git', 'objects-away'), path.join(fuzzy, '.git', 'objects'));
const missing = (dir, revision = '--all') => git(dir, ['--no-lazy-fetch', 'rev-list', '--objects', '--missing=print', '--no-object-names', revision]).split('\n').filter(l => l.startsWith('?')).length;

// ---- Fake api.github.com search --------------------------------------------------------
let mode = 'ok', port = 0; const requests = [];
const item = (name, extra = {}) => ({ name, owner: { login: 'rocket-fixtures' }, private: false, fork: false, archived: false, default_branch: 'main',
  html_url: 'https://github.com/rocket-fixtures/' + name, description: null, stargazers_count: 3, forks_count: 0, open_issues_count: 0, language: 'Rust',
  license: { spdx_id: 'MIT' }, topics: [], size: 12, pushed_at: '2026-10-20T08:09:10Z', updated_at: '2026-10-21T08:09:10Z', ...extra });
const fillers = (from, count) => Array.from({ length: count }, (_, i) => item('filler-' + (from + i), { stargazers_count: 100 - from - i, description: 'Filler ' + (from + i) }));
function results(page) {
  if (page === '2') return fillers(26, 15);
  return [
    item('fuzzy-finder', { clone_url: fuzzy, stargazers_count: 18214, forks_count: 812, open_issues_count: 37, size: 40, description: 'A fast fuzzy file finder for the terminal', topics: ['cli', 'search', 'fuzzy'] }),
    item('huge-assets', { clone_url: huge, stargazers_count: 950, size: 600, language: 'C', description: 'Assets that are bigger than GitHub says' }),
    item('big-media', { clone_url: huge, stargazers_count: 420, size: 4000, language: null, license: null, description: 'Bigger than the preview limit' }),
    item('not-a-repo', { clone_url: empty, stargazers_count: 7, size: 1, description: 'Not a repository' }),
    ...fillers(0, 26),
  ];
}
const server = http.createServer((req, res) => {
  const auth = req.headers.authorization;
  requests.push({ url: req.url, method: req.method, auth: auth === 'Bearer ' + TOKEN, anonymous: auth === undefined, accept: req.headers.accept, version: req.headers['x-github-api-version'], agent: req.headers['user-agent'] });
  const send = (status, body, headers = {}) => { res.writeHead(status, { 'Content-Type': 'application/json', ...headers }); res.end(JSON.stringify(body)); };
  if (req.method !== 'GET' || (auth !== undefined && auth !== 'Bearer ' + TOKEN)) return send(401, { message: 'Bad credentials' });
  const [route, query = ''] = req.url.split('?'), q = new URLSearchParams(query);
  if (route === '/user') return auth ? send(200, { login: 'rocket-fake', name: 'Rocket Fake', html_url: 'https://github.com/rocket-fake' }) : send(401, { message: 'Requires authentication' });
  if (route !== '/search/repositories' || q.get('per_page') !== '30') return send(404, { message: 'Not Found' });
  const remaining = { 'X-RateLimit-Remaining': auth ? '29' : '4', 'X-RateLimit-Reset': String(Math.floor(Date.now() / 1000) + 60) };
  if (mode === 'rate') return send(403, { message: 'API rate limit exceeded' }, { 'X-RateLimit-Remaining': '0', 'Retry-After': '3' });
  if (mode === 'refused') return send(422, { message: 'Validation Failed' });
  if (q.get('q') === 'nothing matches') return send(200, { total_count: 0, incomplete_results: false, items: [] }, remaining);
  send(200, { total_count: 45, incomplete_results: false, items: results(q.get('page')) }, remaining);
});
/// Git's credential helper: nothing for a silent read, the fake token when interactive.
function fakeHelper() {
  const script = `!f() { cat >/dev/null; [ \\"$1\\" = get ] || exit 0; echo \\"$GCM_INTERACTIVE\\" >> '${calls.replaceAll('\\', '/')}'; [ \\"$GCM_INTERACTIVE\\" = auto ] || exit 0; echo username=rocket-fake; echo password=${TOKEN}; }; f`;
  fs.writeFileSync(path.join(root, 'empty.config'), `[credential]\n\thelper = "${script}"\n`);
}
const helperCalls = () => fs.existsSync(calls) ? fs.readFileSync(calls, 'utf8').trim().split(/\r?\n/) : [];
const searches = () => requests.filter(r => r.url.startsWith('/search/repositories'));

// ---- MCP helpers ---------------------------------------------------------------------
let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 60000);
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, error: state.dialog?.error, notice: state.notice, explore: state.explore, scope: state.scope_error, preview: active()?.preview }));
    await delay(80);
  }
}
const active = () => state?.tabs?.find(t => t.id === state.active_tab);
const explore = () => state.explore, preview = () => active()?.preview;
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading) && !s.explore?.running, 'Settled'); }
function control(id) { return state.controls?.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target, args = {}) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab, ...args }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function refused(name, args) { const r = await client.request('tools/call', { name, arguments: { expected_tab: state.active_tab, ...args } }); assert(r.isError, name + ' should be refused: ' + JSON.stringify(args)); await settle(); return r.content[0].text; }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function scroll(target, direction = 'down', pixels = 600) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); await stable(); }
async function reveal(target, region) { for (let i = 0; i < 12 && !(control(target)?.rect[3] - control(target)?.rect[1] >= 20); i++) await scroll(region, 'down', 400); assert(control(target)?.enabled, target + ' unreachable'); }
async function closeWindow() { if (state?.dialog && !state.write && !state.scope_error) try { await click('dialog-cancel'); } catch {} await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
async function openExplore() { await click('menu-file'); await click('file-explore'); await until(s => s.explore?.open, 'Explore page'); }
// Typing clicks the box, then sends the text a frame later; if the window lost focus in
// between (other input on the desktop), the text never arrives. Check, retry, and count it.
async function typeQuery(text) {
  for (let i = 0; i < 3 && explore().query !== text; i++) {
    if (i > 0) report.type_retries = (report.type_retries || 0) + 1;
    await type('explore-query', text);
  }
  assert.equal(explore().query, text, 'Explore box');
}
async function search(text) { await typeQuery(text); await click('explore-search'); await until(s => !s.explore.running, 'Search finished'); }
async function ready(label) { await until(s => active()?.preview && active().preview.state !== 'opening' && !active().loading && !s.busy, label); return preview(); }
const slots = () => fs.existsSync(previews) ? fs.readdirSync(previews).filter(n => n.startsWith('p')) : [];

// ---- Cases ---------------------------------------------------------------------------
async function searching(f) {
  await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920);
  if (control('console-close')) await click('console-close');
  await openExplore();
  assert.equal(explore().result_count, 0); assert.equal(searches().length, 0);
  await capture('01-explore-empty');
  await typeQuery('fuzzy finder');
  assert.equal(searches().length, 0, 'Typing alone sends nothing');
  await click('explore-search'); await until(s => !s.explore.running && s.explore.result_count > 0, 'First results');
  assert.deepEqual(helperCalls(), ['never'], 'One silent sign-in attempt, then signed out');
  assert(explore().anonymous && !explore().signed_in); assert.equal(explore().remaining, 4);
  assert.equal(explore().result_count, 30); assert.equal(explore().total, 45); assert(explore().more); assert.equal(explore().selected, 0);
  assert.equal(explore().results[0].name, 'rocket-fixtures/fuzzy-finder'); assert.equal(explore().results[0].clone_url, fuzzy);
  assert(searches().every(r => r.anonymous && r.accept === 'application/vnd.github+json' && r.version === '2022-11-28' && /^Rocket\/\d+\.\d+\.\d+$/.test(r.agent)));
  await capture('02-explore-results');
  await reveal('explore-more', 'explore-results-scroll'); await click('explore-more');
  await until(s => !s.explore.running && s.explore.result_count === 45, 'Second page');
  assert(!explore().more); assert(searches().at(-1).url.includes('page=2'));
  passed('Search runs only on Search: one silent sign-in attempt, then signed-out results with the searches left, 30 a page and Load more to 45');

  await click('explore-sort-stars'); await until(s => !s.explore.running, 'Sorted');
  assert(searches().at(-1).url.includes('sort=stars&order=desc')); assert.equal(explore().result_count, 30);
  await click('explore-sort-best'); await until(s => !s.explore.running, 'Best match');
  mode = 'rate'; await click('explore-search'); await until(s => !s.explore.running && s.explore.wait_seconds > 0, 'Rate limited');
  assert.equal(explore().result_count, 30, 'Results stay while limited'); assert.equal(explore().remaining, 0);
  await capture('03-explore-rate-limited');
  const limited = searches().length;
  await until(s => s.explore.wait_seconds === null, 'Countdown over'); mode = 'ok';
  assert.equal(searches().length, limited, 'Nothing is sent while waiting');
  await click('explore-search'); await until(s => !s.explore.running && s.explore.wait_seconds === null && s.explore.remaining === 4, 'Searching again');
  mode = 'refused'; await click('explore-search'); await until(s => !s.explore.running && s.explore.problem === 'refused', 'Refused');
  assert.equal(explore().result_count, 30); mode = 'ok';
  await search('nothing matches'); assert.equal(explore().problem, 'no_results'); assert.equal(explore().result_count, 0);
  await capture('04-explore-no-results');
  passed('Sort by stars, a rate limit with its countdown keeping results and sending nothing, a refused query keeping results, and an empty search');

  await click('explore-sign-in'); await until(s => !s.explore.running && s.explore.signed_in, 'Signed in');
  assert.deepEqual(helperCalls(), ['never', 'auto']); assert(searches().at(-1).auth);
  await search('fuzzy finder'); assert.equal(explore().remaining, 29); assert(searches().at(-1).auth);
  passed('Sign in asks Git\'s credential helper interactively, then searches send the token and show 29 left');
  // The MCP transport scope refuses clones and fetches while any credential helper is
  // configured; the signed-in session stays in memory.
  fs.writeFileSync(path.join(root, 'empty.config'), '');

  const text = await refused('rocket_type', { target: 'explore-query', text: 'https://gitlab.com/acme/app' });
  assert(text.includes('local paths'), text); assert.equal(explore().query, 'fuzzy finder');
  await typeQuery(fuzzy);
  assert.deepEqual(explore().address, { name: 'fuzzy-finder', host: 'this computer', url: fuzzy });
  await capture('05-explore-address');
  passed('A pasted local repository shows its address card; MCP refuses a non-local address before typing');
  await search('fuzzy finder');
}

async function previewing() {
  fs.mkdirSync(stale, { recursive: true }); fs.writeFileSync(path.join(stale, 'rocket-preview.lock'), ''); fs.writeFileSync(path.join(stale, 'junk.bin'), 'x');
  await click('explore-result-0'); const before = state.tabs.length;
  await click('explore-preview');
  await until(s => s.tabs.length === before + 1 && !s.explore.open, 'Preview tab');
  const p = await ready('Preview ready');
  assert.equal(p.state, 'ready', JSON.stringify(p)); assert.equal(p.branch, 'main'); assert.equal(p.limit, LIMIT);
  assert(p.size > 0); assert.equal(p.name, 'rocket-fixtures/fuzzy-finder'); assert.equal(p.url, fuzzy);
  const repo = plain(active().path); report.preview = repo;
  assert(repo.startsWith(previews), repo);
  assert.equal(git(repo, ['rev-parse', '--is-bare-repository']), 'true'); assert(!fs.existsSync(path.join(repo, 'index')));
  assert.equal(missing(repo, 'HEAD^{tree}'), 0, 'The tip\'s files are here'); const before2 = missing(repo); assert(before2 > 0, 'Older versions stay on the server');
  assert.equal(active().head?.branch, 'main'); assert.equal(active().commit_count, 4);
  await until2(() => !fs.existsSync(stale), 'Stale preview folder removed');
  await capture('06-preview-ready');
  passed('Preview opens a bare clone in the test root\'s preview folder with the tip\'s files only, shows main\'s 4 commits, and removes a folder a crashed Rocket left');

  await click('view-tree'); await until(s => active().repository_tree.entries?.length, 'Files');
  const names = active().repository_tree.entries.map(e => e.name); assert(['README.md', 'docs', 'src'].every(n => names.includes(n)), JSON.stringify(names));
  await click('tree-entry-' + active().repository_tree.entries.findIndex(e => e.name === 'README.md'));
  await until(s => active().repository_tree.content?.line_count, 'README contents');
  await capture('07-preview-files');
  await click('view-commits'); await until(s => active().history_ids?.length === 4, 'Commits');
  const old = git(fuzzy, ['rev-parse', commits.match + ':src/matcher.rs']);
  assert(!present(repo, old), 'The older version of src/matcher.rs is not downloaded yet');
  await click('commit-' + commits.rank);
  await until(s => active().selected_commit === commits.rank && !active().loading && (active().patch_added > 0 || active().patch_error), 'Rank results diff');
  assert.equal(active().patch_error ?? null, null); assert(active().patch_removed > 0);
  assert(present(repo, old), 'The diff downloaded the older version it needs');
  const after = missing(repo); assert(after < before2, `Fewer versions missing (${before2} -> ${after})`);
  await capture('08-preview-diff');
  passed(`Files lists the tip and opens README; an older commit's diff downloads the file version it needs (${before2} -> ${after} missing)`);

  await click('commit-' + commits.rank, { button: 'right' });
  await until(s => s.controls.some(c => c.id === 'commit-menu-search-files'), 'Commit menu'); await stable();
  assert(control('commit-menu-search-files')?.enabled, 'Reading items stay in the commit menu');
  for (const id of ['commit-menu-revert', 'commit-menu-cherry-pick', 'commit-menu-series']) assert(!control(id), id + ' is hidden in a preview');
  await capture('09-preview-commit-menu');
  await client.tool('rocket_escape'); await settle();
  assert(!control('commit-open-review')?.enabled, 'No review window for a preview');
  assert.equal(git(repo, ['rev-parse', 'HEAD']), commits.docs);
  await click('view-statistics'); await click('stats-section-treemap');
  await until(s => s.dialog?.statistics && !s.dialog.statistics.treemap_running && (s.dialog.statistics.treemap || s.dialog.statistics.treemap_error), 'Treemap');
  assert.equal(state.dialog.statistics.treemap_error, null); assert(state.dialog.statistics.treemap.folder.size > 0);
  await capture('10-preview-statistics');
  await click('view-commits');
  passed('The commit menu keeps only reading items, there is no review window, nothing changes, and the Statistics treemap reads the tip\'s real sizes');

  // Viewing branch: the preview's branches; dev's newest file isn't downloaded yet, and
  // with the source unreachable the diff says so instead of an error.
  breakSource();
  await click('preview-branch'); await until(s => s.controls.some(c => c.id === 'preview-branch-dev'), 'Branch menu'); await stable();
  assert(['preview-branch-main', 'preview-branch-notes'].every(id => control(id)), 'Every branch is listed');
  await capture('11-preview-branch-menu');
  await click('preview-branch-dev');
  await until(s => active().history_scope === 'refs/heads/dev' && active().history_ids?.length === 5 && !active().loading, 'Viewing dev');
  await click('commit-' + commits.dev);
  await until(s => active().selected_commit === commits.dev && !active().loading && preview().download_failure && control('diff-retry'), 'Not downloaded yet');
  await capture('12-preview-not-downloaded');
  fixSource(); await click('diff-retry');
  await until(s => !active().loading && !preview().download_failure && !preview().downloading, 'Read again');
  if (active().selected_commit !== commits.dev) await click('commit-' + commits.dev);
  await until(s => active().selected_commit === commits.dev && active().patch_added > 0, 'dev diff');
  await click('preview-branch'); await until(s => s.controls.some(c => c.id === 'preview-branch-main'), 'Branch menu again'); await stable();
  await click('preview-branch-main');
  await until(s => active().history_scope === 'refs/heads/main' && active().history_ids?.length === 4 && !active().loading, 'Viewing main');
  passed('Viewing branch lists the preview\'s branches and shows the one chosen; a failed download shows "Not downloaded yet" and Try again reads it again');
  return repo;
}

async function failures() {
  await click('preview-back'); await until(s => s.explore.open, 'Back to results');
  assert.equal(explore().result_count, 30, 'Results are where they were');
  // GitHub says 600 KB; its files are 3 MB, past the 1 MB limit.
  await click('explore-result-1'); const before = state.tabs.length; await click('explore-preview');
  await until(s => s.tabs.length === before + 1, 'Second preview');
  let p = await ready('Limit stop'); assert.equal(p.state, 'limit', JSON.stringify(p));
  assert.equal(slots().length, 2, 'A stopped preview keeps only its empty locked folder');
  await capture('13-preview-limit');
  await click('preview-unlimited'); p = await ready('Unlimited preview');
  assert.equal(p.state, 'ready', JSON.stringify(p)); assert(p.unlimited && p.limit === null && p.size > 2 * LIMIT);
  await capture('14-preview-banner');
  const folder = plain(p.folder); await click('preview-discard');
  await until(s => s.tabs.length === before, 'Discarded'); await until2(() => !fs.existsSync(folder), 'Discarded folder deleted');
  const note = state.activities.at(-1);
  assert(note.operation === 'discard-preview' && note.summary.startsWith('Discarded the preview of rocket-fixtures/huge-assets · ') && note.summary.endsWith(' freed'), JSON.stringify(note));
  passed('A preview past the limit stops with "Stopped at 1 MB"; Continue without a limit opens it; Discard closes the tab, deletes its folder and notes the space freed in Activity');

  await openExplore(); await click('explore-result-2'); await click('explore-preview');
  await until(s => s.explore.confirm_large === 2, 'Large repository card');
  await capture('15-explore-large');
  await click('explore-clone-instead'); await until(s => s.dialog?.kind === 'clone', 'Clone dialog');
  assert.equal(state.dialog.first, huge); await click('dialog-cancel');
  passed('A repository GitHub sizes past the limit asks first; Clone instead opens Clone with its address');

  await openExplore(); await click('explore-result-3'); const count = state.tabs.length; await click('explore-preview');
  await until(s => s.tabs.length === count + 1, 'Third preview');
  p = await ready('Failure card'); assert(['not_found', 'other'].includes(p.state), JSON.stringify(p));
  await capture('16-preview-failed');
  await click('preview-close'); await until(s => s.tabs.length === count, 'Failed preview closed');
  passed('A source that is not a repository shows a failure card; Close tab removes it');

  await openExplore(); await typeQuery(fuzzy); await click('explore-address-preview');
  await until(s => !s.explore.open && active()?.preview?.url === fuzzy, 'Existing preview');
  assert.equal(state.tabs.length, count, 'The tab already previewing it is shown');
  passed('Previewing a pasted address that is already open shows that tab instead of a second copy');
}

async function keeping(repo) {
  await click('preview-keep'); await until(s => s.dialog?.kind === 'keep-preview' && control('dialog-submit')?.enabled, 'Keep dialog');
  const destination = path.join(root, 'fuzzy-finder'); assert.equal(plain(state.dialog.second), destination);
  await capture('17-keep-dialog');
  const id = state.active_tab;
  // The notes branch's file was never needed, so Keep must download it: with the source
  // unreachable Keep fails, the preview is unchanged and Try again resumes it.
  breakSource();
  await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: id });
  await until(s => s.dialog?.kind === 'keep-preview' && s.dialog.error && !s.busy && control('dialog-submit')?.enabled, 'Keep failed');
  assert(!fs.existsSync(destination), 'Nothing was kept'); assert(active().preview?.state === 'ready');
  assert(!state.dialog.error.includes('did not complete'), 'Keep says why it failed: ' + state.dialog.error);
  await capture('18-keep-failed');
  fixSource();
  await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: id });
  await until(s => s.scope_error || (s.tabs && !s.tabs.some(t => t.preview?.url === fuzzy) && !s.busy), 'Kept');
  // The kept clone is a normal repository the runner now marks as its own.
  fs.writeFileSync(path.join(destination, '.git/rocket-ui-fixture'), branch);
  await until(s => !s.scope_error && !s.busy && !(s.tabs || []).some(t => t.loading), 'Kept clone in scope');
  const kept = state.tabs.find(t => plain(t.path) === destination);
  assert(kept, JSON.stringify(state.tabs.map(t => t.path))); assert.equal(kept.preview, null);
  assert.equal(kept.head?.branch, 'main'); assert.equal(kept.upstream, 'origin/main'); assert.equal(kept.change_count, 0);
  assert.equal(git(destination, ['remote', 'get-url', 'origin']), fuzzy);
  assert.equal(missing(destination), 0, 'Every file version is in the kept clone');
  assert.throws(() => git(destination, ['config', '--get', 'remote.origin.promisor']));
  assert.equal(git(destination, ['rev-parse', 'HEAD']), commits.docs);
  assert(fs.readFileSync(path.join(destination, 'docs/usage.md'), 'utf8').includes('Type part'));
  await until2(() => !fs.existsSync(repo), 'Kept preview folder deleted');
  const keptNote = state.activities.filter(a => a.operation === 'keep-preview').at(-1);
  assert(keptNote?.command_succeeded && keptNote.summary === `Kept rocket-fixtures/fuzzy-finder in ${destination}`, JSON.stringify(keptNote));
  assert(!state.activities.some(a => a.operation === 'discard-preview' && a.summary.includes('fuzzy-finder')), 'A kept preview is not noted as discarded');
  await capture('19-kept');
  passed('Keep fails cleanly while the source is unreachable and Try again resumes it; the kept clone has origin, upstream and every version, Activity names where it went, and the preview folder is deleted');
}

async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); port = server.address().port;
    fakeHelper(); sources(); const f = home();
    fs.writeFileSync(path.join(root, 'rocket-github-api'), `http://127.0.0.1:${port}\n`);
    fs.writeFileSync(path.join(root, 'rocket-preview-limit'), LIMIT + '\n');
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const mcpPort = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + mcpPort + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await searching(f);
    const repo = await previewing();
    await failures();
    await keeping(repo);
    assert.equal(hash(path.join(f.repo, '.git/config')), f.config, 'Home config changed');
    await closeWindow(); await client.raw('POST', '/shutdown');
    await until2(() => slots().length === 0, 'Every preview folder deleted on close');
    passed('Closing the window deletes every preview folder');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of [TOKEN, 'gho_', 'Bearer', 'Authorization', 'rocket-fake', 'rocket-fixtures', 'fuzzy', 'huge-assets', 'filler-', '127.0.0.1']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    for (const op of ['github-search', 'preview-open', 'keep-preview']) { const starts = records.filter(r => r.event === 'request_start' && r.operation === op); assert(starts.length, 'Missing ' + op); for (const start of starts) assert(records.some(r => r.event === 'request_end' && r.session === start.session && r.request === start.request), 'Unpaired ' + op); }
    assert(records.some(r => r.operation === 'preview-open' && r.failure_code === 'clone_size_limit'), 'Missing clone_size_limit');
    report.log_records = records.length; report.api_requests = requests.length;
    passed('Paired github-search, preview-open and keep-preview spans with the size-limit code, zero dropped events, no token, account, repository or host data');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync('work/explore-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish(); server.close();
  }
}
async function until2(predicate, label) { const start = performance.now(); while (!predicate()) { assert(performance.now() - start < WAIT_MS, label + ' timed out'); await delay(100); } }
run().catch(e => { console.error(e); process.exitCode = 1; });
