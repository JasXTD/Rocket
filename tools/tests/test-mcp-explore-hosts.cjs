// Native Windows acceptance for Explore's GitLab and Gitea search through authenticated
// localhost MCP, against loopback fake GitLab and Gitea APIs (the hosting fixture): the
// host menu; gitlab.com without an account (no Authorization header, whole namespaces,
// sizes unknown, Load more, Sort); GitHub's search words pointed out instead of sent and
// Search without them; a rate limit with its countdown keeping results and sending
// nothing; Open on GitLab recorded; a preview of a GitLab result; Codeberg on Gitea's API
// (language greyed out, a server that wants a sign-in, Add account into Preferences →
// Accounts and back); a remembered GitLab account (its token, sizes, private projects,
// the account replacing gitlab.com without one); a refused token not tried again; and
// diagnostics with paired explore-search spans and no tokens, names or addresses.
// Links are recorded by the MCP host, never opened.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), http = require('node:http'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
// Outside any repository: previews run Git beside their folder, and Git would otherwise
// read the enclosing checkout's settings, which the MCP transport scope refuses.
const root = path.join(require('node:os').tmpdir(), 'rocket-mcp-explore-hosts-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/explore-hosts';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template'));
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0', GIT_NO_LAZY_FETCH: '1' });
fs.writeFileSync(path.join(root, 'empty.config'), '');
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const TOKEN = 'glpat-rocketExploreFake' + crypto.randomBytes(8).toString('hex');
const exe = path.resolve(process.env.ROCKET_EXPLORE_EXE || 'target/release/rocket-mcp.exe');

// ---- Sources -----------------------------------------------------------------------
const git = (dir, args) => cp.execFileSync('git', ['-C', dir, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
function repository(dir, initial) {
  fs.mkdirSync(dir, { recursive: true });
  git(dir, ['init', '--initial-branch=' + initial]);
  for (const [key, value] of [['user.name', 'Rocket Explore Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['uploadpack.allowFilter', 'true']]) git(dir, ['config', key, value]);
  return (files, message) => {
    for (const [name, body] of Object.entries(files)) { fs.mkdirSync(path.dirname(path.join(dir, name)), { recursive: true }); fs.writeFileSync(path.join(dir, name), body); }
    git(dir, ['add', '-A']); git(dir, ['commit', '-m', message]); return git(dir, ['rev-parse', 'HEAD']);
  };
}
const finder = path.join(root, 'sources', 'fuzzyfind');
function sources() {
  const commit = repository(finder, 'main');
  commit({ 'README.md': '# fuzzyfind\n\nFind files by typing a few letters.\n', 'src/main.rs': 'fn main() {}\n' }, 'Start the finder');
  commit({ 'src/matcher.rs': 'pub fn score() -> u32 { 1 }\n' }, 'Add matching');
}
function home() {
  const dir = path.join(root, 'home'); const commit = repository(dir, branch);
  commit({ 'notes.txt': 'home\n' }, 'Home fixture');
  fs.writeFileSync(path.join(dir, '.git/rocket-ui-fixture'), branch);
  return { repo: dir, config: hash(path.join(dir, '.git/config')) };
}
const plain = p => p.replace(/^\\\\\?\\/, '');

// ---- Fake GitLab (/gitlab) and Gitea (/gitea) APIs ------------------------------------
let mode = 'ok', port = 0; const requests = [];
const project = (name, extra = {}) => ({ path_with_namespace: name, description: 'Fuzzy finder ' + name, star_count: 12, forks_count: 1,
  open_issues_count: 2, topics: ['cli'], default_branch: 'main', last_activity_at: '2026-11-15T03:12:44.123Z', visibility: 'public',
  archived: false, http_url_to_repo: finder, web_url: 'https://elsewhere.example/' + name, ...extra });
const server = http.createServer((req, res) => {
  const auth = req.headers.authorization;
  requests.push({ url: req.url, method: req.method, auth: auth === 'Bearer ' + TOKEN, anonymous: auth === undefined, agent: req.headers['user-agent'] });
  const send = (status, body, headers = {}) => { res.writeHead(status, { 'Content-Type': 'application/json', ...headers }); res.end(JSON.stringify(body)); };
  if (req.method !== 'GET') return send(405, { message: '405 Method Not Allowed' });
  if (auth !== undefined && auth !== 'Bearer ' + TOKEN) return send(401, { message: '401 Unauthorized' });
  const [route, query = ''] = req.url.split('?'), q = new URLSearchParams(query);
  if (route === '/gitlab/user') return auth ? send(200, { id: 7, username: 'sample-user' }) : send(401, { message: '401 Unauthorized' });
  if (route === '/gitlab/projects') {
    if (q.get('per_page') !== '30') return send(400, { message: 'per_page' });
    if (mode === 'rate') return send(429, { message: 'Retry later' }, { 'Retry-After': '3' });
    if (mode === 'refused' && auth) return send(401, { message: '401 Unauthorized' });
    if (q.get('page') === '2') return send(200, Array.from({ length: 5 }, (_, i) => project('fillers/more-' + i)), { 'X-Total': '35', 'X-Next-Page': '' });
    const sized = auth && q.get('statistics') === 'true' ? { statistics: { repository_size: 40960 } } : {};
    return send(200, [
      project('acme/tools/fuzzyfind', sized),
      project('northwind/fzf-lite', { archived: true }),
      ...(auth ? [project('contoso/platform/search-ui', { visibility: 'private' })] : []),
      ...Array.from({ length: 27 }, (_, i) => project('fillers/item-' + i)),
    ], { 'X-Total': '35', 'X-Next-Page': '2' });
  }
  if (route === '/gitea/repos/search') {
    if (mode === 'closed' && !auth) return send(403, { message: 'Only signed in user is allowed to call APIs.' });
    return send(200, { ok: true, data: [
      { full_name: 'sample-user/blog-builder', description: 'A small static site generator', stars_count: 42, forks_count: 6, open_issues_count: 3,
        language: 'Go', size: 12288, updated_at: '2026-11-13T10:20:30+01:00', default_branch: 'main', mirror: false, clone_url: finder, licenses: ['MIT'], topics: ['static-site'] },
      { full_name: 'mirrors/hugo', stars_count: 3, language: 'Go', size: 1468006, mirror: true, clone_url: finder },
    ] }, { 'X-Total-Count': '2' });
  }
  send(404, { message: '404 Not Found' });
});
const searches = route => requests.filter(r => r.url.startsWith(route));

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
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, error: state.dialog?.error, notice: state.notice, explore: state.explore, hosting: state.hosting, preferences: state.preferences?.page, scope: state.scope_error }));
    await delay(80);
  }
}
const active = () => state?.tabs?.find(t => t.id === state.active_tab);
const explore = () => state.explore, preview = () => active()?.preview;
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading) && !s.explore?.running && !s.hosting?.busy, 'Settled'); }
function control(id) { return state.controls?.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target, args = {}) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab, ...args }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
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
async function chooseHost(key) {
  await click('explore-host'); await until(s => s.controls.some(c => c.id.startsWith('explore-host-')), 'Host menu'); await stable();
  const index = explore().hosts.findIndex(h => h.key === key); assert(index >= 0, 'No host ' + key + ' in ' + JSON.stringify(explore().hosts));
  await click('explore-host-' + index);
  await until(s => s.explore.host.key === key && !s.explore.running, 'Host ' + key);
}
async function ready(label) { await until(s => active()?.preview && active().preview.state !== 'opening' && !active().loading && !s.busy, label); return preview(); }

// ---- Cases ---------------------------------------------------------------------------
async function gitlabWithoutAccount(f) {
  await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920);
  if (control('console-close')) await click('console-close');
  await openExplore();
  assert.deepEqual(explore().hosts.map(h => h.key), ['github', 'public:gitlab|https://gitlab.com', 'public:gitea|https://codeberg.org']);
  assert.equal(explore().host_kind, 'github');
  await click('explore-host'); await until(s => s.controls.some(c => c.id === 'explore-host-2'), 'Host menu'); await stable();
  for (const id of ['explore-host-0', 'explore-host-1', 'explore-host-2', 'explore-host-accounts']) assert(control(id)?.enabled, id);
  await capture('01-host-menu');
  await click('explore-host-1');
  await until(s => s.explore.host.key === 'public:gitlab|https://gitlab.com', 'GitLab chosen');
  assert.equal(requests.length, 0, 'An empty box searches nothing');
  assert(control('explore-add-account')?.enabled && control('explore-language')?.enabled, 'GitLab: Add account and the Language list');
  assert(!control('explore-sort-best'), 'Best match is GitHub\'s');
  await typeQuery('fuzzy finder'); assert.equal(requests.length, 0, 'Typing alone sends nothing');
  await click('explore-search'); await until(s => !s.explore.running && s.explore.result_count > 0, 'GitLab results');
  const first = searches('/gitlab/projects')[0];
  assert.equal(first.url, '/gitlab/projects?search=fuzzy%20finder&order_by=star_count&sort=desc&per_page=30&page=1');
  assert(first.anonymous, 'No Authorization header without an account'); assert(/^Rocket\/\d+\.\d+\.\d+$/.test(first.agent));
  assert.equal(explore().result_count, 29); assert.equal(explore().total, 35); assert(explore().more); assert.equal(explore().selected, 0);
  const top = explore().results[0];
  assert.equal(top.name, 'acme/tools/fuzzyfind'); assert.equal(top.size_kb, null); assert.equal(top.language, null);
  assert.equal(top.web_url, 'https://gitlab.com/acme/tools/fuzzyfind', 'The page is the configured server\'s, not the answer\'s');
  assert.equal(top.clone_url, finder, 'The fixture names the local source');
  assert(explore().results[1].archived);
  await capture('02-gitlab-results');
  await reveal('explore-more', 'explore-results-scroll'); await click('explore-more');
  await until(s => !s.explore.running && s.explore.result_count === 34, 'Second page');
  assert(!explore().more); assert(searches('/gitlab/projects').at(-1).url.includes('&page=2'));
  passed('The host menu lists GitHub, gitlab.com and Codeberg; gitlab.com without an account sends no Authorization header, shows whole namespaces, unknown sizes and 29 of 35 with Load more to 34');

  await click('explore-sort-updated'); await until(s => !s.explore.running, 'Sorted');
  assert(searches('/gitlab/projects').at(-1).url.includes('&order_by=last_activity_at&sort=desc'));
  const before = requests.length;
  await typeQuery('fuzzy language:go'); await click('explore-search');
  await until(s => s.explore.problem === 'github_words', 'GitHub\'s words pointed out');
  assert.deepEqual(explore().words, ['language:go']); assert.equal(requests.length, before, 'Nothing was sent');
  assert(control('explore-without-words')?.enabled);
  await capture('03-github-words');
  await click('explore-without-words');
  await until(s => !s.explore.running && s.explore.query === 'fuzzy' && !s.explore.problem && s.explore.result_count > 0, 'Searched without them');
  assert(searches('/gitlab/projects').at(-1).url.startsWith('/gitlab/projects?search=fuzzy&'));
  passed('Sort by Updated asks GitLab for last_activity_at; GitHub\'s search words are pointed out instead of sent, and Search without them searches the rest');

  mode = 'rate'; await click('explore-search'); await until(s => !s.explore.running && s.explore.wait_seconds > 0, 'Rate limited');
  assert.equal(explore().result_count, 29, 'Results stay while limited'); assert.equal(explore().remaining, null, 'GitLab sends no remaining count');
  await capture('04-gitlab-limit');
  const limited = requests.length;
  await until(s => s.explore.wait_seconds === null, 'Countdown over'); mode = 'ok';
  assert.equal(requests.length, limited, 'Nothing is sent while waiting');
  await click('explore-search'); await until(s => !s.explore.running && s.explore.wait_seconds === null && s.explore.result_count === 29, 'Searching again');
  await click('explore-result-0'); await click('explore-open');
  await until(s => (s.github.opened_urls || []).includes('https://gitlab.com/acme/tools/fuzzyfind'), 'Open on GitLab recorded');
  passed('A rate limit counts down from Retry-After keeping results and sending nothing; Open on GitLab opens the configured server\'s page');
}

async function previewing() {
  await click('explore-result-0'); const before = state.tabs.length; await click('explore-preview');
  await until(s => s.tabs.length === before + 1 && !s.explore.open, 'Preview tab');
  const p = await ready('Preview ready');
  assert.equal(p.state, 'ready', JSON.stringify(p)); assert.equal(p.name, 'acme/tools/fuzzyfind'); assert.equal(p.host, 'gitlab.com'); assert.equal(p.url, finder);
  assert.equal(active().commit_count, 2);
  await capture('05-gitlab-preview');
  const folder = plain(p.folder); await click('preview-discard');
  await until(s => s.tabs.length === before, 'Discarded'); assert(!fs.existsSync(folder), 'Preview folder deleted');
  passed('A GitLab result previews in a tab named by its whole path, on gitlab.com, and Discard deletes it');
}

async function codeberg() {
  await openExplore(); await chooseHost('public:gitea|https://codeberg.org');
  await until(s => !s.explore.running && s.explore.result_count === 2, 'Codeberg results');
  const r = searches('/gitea/repos/search').at(-1);
  assert.equal(r.url, '/gitea/repos/search?q=fuzzy&includeDesc=true&sort=updated&order=desc&limit=30&page=1'); assert(r.anonymous);
  assert.equal(control('explore-language')?.enabled, false, 'Gitea can\'t filter by language');
  const [blog, hugo] = explore().results;
  assert.equal(blog.name, 'sample-user/blog-builder'); assert.equal(blog.language, 'Go'); assert.equal(blog.size_kb, 12288);
  assert.equal(blog.web_url, 'https://codeberg.org/sample-user/blog-builder'); assert(hugo.mirror);
  await capture('06-codeberg');
  passed('Choosing Codeberg searches Gitea\'s API at once with the words in the box, includes descriptions, greys out Language and lists sizes, languages and mirrors');

  mode = 'closed'; await click('explore-search'); await until(s => !s.explore.running && s.explore.problem === 'sign_in_required', 'Sign-in required');
  await capture('07-sign-in-required');
  await click('explore-problem-add-account');
  await until(s => s.preferences.page === 'accounts' && s.hosting.provider === 'gitea' && !s.explore.open, 'Accounts for Codeberg');
  await click('dialog-close'); await until(s => s.explore.open && !s.dialog?.kind?.startsWith('pref'), 'Back to Explore');
  mode = 'ok';
  passed('A server that lets only signed-in users search says so; Add account opens Preferences → Accounts with Gitea chosen, and closing it comes back to Explore');
}

async function account() {
  await chooseHost('public:gitlab|https://gitlab.com');
  await click('explore-add-account');
  await until(s => s.preferences.page === 'accounts' && s.hosting.provider === 'gitlab', 'Accounts for gitlab.com');
  await type('hosting-label', 'Work'); await type('hosting-token', TOKEN);
  if (!state.hosting.remember) await click('hosting-remember');
  await click('hosting-connect'); await until(s => s.hosting.connected && !s.hosting.busy, 'Connected');
  await click('dialog-close'); await until(s => s.explore.open, 'Back to Explore');
  await until(s => s.explore.hosts.some(h => h.key === 'account:gitlab|https://gitlab.com|Work'), 'The account is a host');
  assert(!explore().hosts.some(h => h.key === 'public:gitlab|https://gitlab.com'), 'gitlab.com without an account gives way to the account');
  assert.equal(explore().host.key, 'account:gitlab|https://gitlab.com|Work', 'The choice follows the server');
  await typeQuery('fuzzy finder'); await click('explore-search'); await until(s => !s.explore.running && s.explore.result_count === 30, 'Signed-in results');
  const r = searches('/gitlab/projects').at(-1);
  assert(r.auth, 'The account\'s token'); assert(r.url.endsWith('&statistics=true'), r.url);
  assert.equal(explore().results[0].size_kb, 40); assert(explore().results.some(x => x.private && x.name === 'contoso/platform/search-ui'));
  await capture('08-gitlab-account');
  passed('A remembered GitLab account replaces gitlab.com without one, keeps the choice, sends its token and gets sizes and private projects');

  mode = 'refused'; await click('explore-search'); await until(s => !s.explore.running && s.explore.problem === 'token_refused', 'Token refused');
  assert(control('explore-problem-accounts')?.enabled);
  await capture('09-token-refused');
  const count = requests.length; mode = 'ok';
  await click('explore-search'); await until(s => !s.explore.running && s.explore.problem === 'token_refused', 'Still refused');
  assert.equal(requests.length, count, 'The refused token is not tried again until the account changes');
  passed('A refused token says so with Open Accounts and is not sent again');
}

async function run() {
  try {
    report.executable_sha256 = hash(exe);
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); port = server.address().port;
    sources(); const f = home();
    fs.writeFileSync(path.join(root, 'rocket-hosting-api'), `http://127.0.0.1:${port}`);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const mcpPort = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + mcpPort + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await gitlabWithoutAccount(f);
    await previewing();
    await codeberg();
    await account();
    assert.equal(hash(path.join(f.repo, '.git/config')), f.config, 'Home config changed');
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of [TOKEN, 'glpat-', 'Bearer', 'Authorization', 'sample-user', 'acme', 'fuzzyfind', 'northwind', 'blog-builder', 'codeberg', '127.0.0.1']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    const starts = records.filter(r => r.event === 'request_start' && r.operation === 'explore-search'); assert(starts.length >= 8, 'explore-search spans: ' + starts.length);
    for (const start of starts) assert(records.some(r => r.event === 'request_end' && r.session === start.session && r.request === start.request), 'Unpaired explore-search');
    assert(!records.some(r => r.operation === 'other'), 'Every request is named');
    report.log_records = records.length; report.api_requests = requests.length;
    passed('Paired explore-search spans, every request named, zero dropped events, no token, account, repository or host names');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.mkdirSync('work', { recursive: true });
    fs.writeFileSync('work/explore-hosts-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish(); server.close();
  }
}
run().catch(e => { console.error(e); process.exitCode = 1; });
