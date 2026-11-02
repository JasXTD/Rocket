// Native Windows acceptance for 0.36.0 GitHub integration through authenticated
// localhost MCP. Default: a loopback fake GitHub API and a fake credential helper
// (sign-in, repository links, create-PR and other github.com shortcuts, lists,
// paging, filters, failures, cancellation, the off-by-default MCP setting and
// diagnostics privacy). Links are recorded by the MCP host, never opened.
// --live: one read-only pass against api.github.com with the user's stored Git
// Credential Manager sign-in. It prints counts and failure codes only and writes
// no captures or state files.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), http = require('node:http'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const live = process.argv.includes('--live');
const root = path.resolve('work/mcp-fixtures/github-' + (live ? 'live-' : '') + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/github';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, live, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const TOKEN = 'gho_rocketMcpFake' + crypto.randomBytes(8).toString('hex');
const calls = path.join(root, 'helper-calls'), apiMarker = path.join(root, 'rocket-github-api');
const LIVE_ORIGIN = 'https://github.com/JasXTD/rocket-auth-test.git'; // the policy's approved disposable repository
const PULL = 'https://github.com/rocket-org/upstream/compare/main...rocket-fake:rocket-ui-test/github?expand=1';

// ---- Fake api.github.com -------------------------------------------------------------
let mode = 'ok', pullOpen = false, port = 0, slowClosed = null; const requests = [];
const repo = (owner, name, extra = {}) => ({ name, owner: { login: owner }, private: false, fork: false, archived: false, default_branch: 'main', html_url: `https://github.com/${owner}/${name}`, description: null, pushed_at: '2026-10-20T08:09:10Z', ...extra });
const note = (id, unread, type, title, subject, owner = 'rocket-org', name = 'upstream') => ({ id, unread, reason: 'review_requested', updated_at: '2026-10-21T09:00:00Z', subject: { title, url: subject, type }, repository: { name, owner: { login: owner } } });
const link = (route, page) => ({ Link: `<http://127.0.0.1:${port}${route}&page=${page}>; rel="next"` });
function api(url) {
  const [route, query = ''] = url.split('?'), q = new URLSearchParams(query), page = q.get('page');
  if (route === '/user') return [200, { login: 'rocket-fake', name: 'Rocket Fake 界', html_url: 'https://github.com/rocket-fake' }];
  if (route === '/repos/rocket-fake/forked_repo') return [200, repo('rocket-fake', 'forked_repo', { fork: true, private: true, default_branch: 'dev', parent: repo('rocket-org', 'upstream') })];
  if (route === '/repos/rocket-org/upstream/pulls' && q.get('state') === 'open' && q.get('head') === 'rocket-fake:' + branch)
    return [200, pullOpen ? [{ number: 7, title: 'PRIVATE_PULL 界', html_url: 'https://github.com/rocket-org/upstream/pull/7', draft: true, base: { ref: 'main' } }] : []];
  if (route === '/user/repos' && q.get('per_page') === '100' && q.get('sort') === 'pushed')
    return page === '2' ? [200, [repo('rocket-org', 'upstream', { archived: true, description: 'PRIVATE_UPSTREAM 界' })]]
      : [200, [repo('rocket-fake', 'forked_repo', { fork: true, private: true }), repo('rocket-fake', 'notes', { private: true, description: 'PRIVATE_NOTES 界面' })], link('/user/repos?per_page=100&sort=pushed', 2)];
  if (route === '/notifications' && q.get('per_page') === '50') {
    const unread = [note('1', true, 'PullRequest', 'PRIVATE_TITLE review 界', 'https://api.github.com/repos/rocket-org/upstream/pulls/42'), note('2', true, 'Issue', 'PRIVATE_TITLE issue', 'https://api.github.com/repos/rocket-fake/notes/issues/5', 'rocket-fake', 'notes')];
    if (q.get('all') === 'false') return [200, unread];
    return page === '2' ? [200, [note('4', false, 'Release', 'PRIVATE_TITLE release v2', 'https://api.github.com/repos/rocket-org/upstream/releases/9')]]
      : [200, [...unread, note('3', false, 'Discussion', 'PRIVATE_TITLE discussion', null)], link('/notifications?all=true&per_page=50', 2)];
  }
  return [404, { message: 'Not Found' }];
}
const server = http.createServer((req, res) => {
  requests.push({ url: req.url, method: req.method, auth: req.headers.authorization === 'Bearer ' + TOKEN, accept: req.headers.accept, version: req.headers['x-github-api-version'], agent: req.headers['user-agent'] });
  const send = (status, body, headers = {}) => { res.writeHead(status, { 'Content-Type': 'application/json', ...headers }); res.end(JSON.stringify(body)); };
  if (req.method !== 'GET' || req.headers.authorization !== 'Bearer ' + TOKEN || mode === 'unauthorized') return send(401, { message: 'Bad credentials' });
  if (mode === 'rate') return send(403, { message: 'API rate limit exceeded' }, { 'X-RateLimit-Remaining': '0' });
  if (mode === 'slow') { res.on('close', () => { slowClosed = performance.now(); }); return; }
  const [status, body, headers] = api(req.url); send(status, body, headers);
});
/// The credential helper Git runs for `git credential fill`, recording each mode.
function fakeHelper() {
  const script = `!f() { cat >/dev/null; [ \\"$1\\" = get ] || exit 0; echo \\"$GCM_INTERACTIVE\\" >> '${calls.replaceAll('\\', '/')}'; echo username=rocket-fake; echo password=${TOKEN}; }; f`;
  fs.writeFileSync(path.join(root, 'empty.config'), `[credential]\n\thelper = "${script}"\n`);
}
const helperCalls = () => fs.existsSync(calls) ? fs.readFileSync(calls, 'utf8').trim().split(/\r?\n/) : [];

// ---- Fixture and MCP helpers ----------------------------------------------------------
function fixture(name, origin) {
  const dir = path.join(root, name), hooks = path.join(root, name + '-hooks'); fs.mkdirSync(dir); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', dir, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
  const commit = (file, body) => { fs.writeFileSync(path.join(dir, file), body); git(['add', '-A']); git(['commit', '-m', 'PRIVATE_COMMIT ' + body]); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket GitHub Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  commit('file.txt', 'one 界\n');
  git(['remote', 'add', 'origin', origin]); git(['update-ref', 'refs/remotes/origin/' + branch, 'HEAD']); git(['branch', '--set-upstream-to=origin/' + branch]);
  commit('file.txt', 'two 界\n');
  fs.writeFileSync(path.join(dir, '.git/rocket-ui-fixture'), branch);
  return { repo: dir, git, config: hash(path.join(dir, '.git/config')) };
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
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + (live ? JSON.stringify({ kind: state.dialog?.kind, page: state.github?.page }) : JSON.stringify({ dialog: state.dialog?.kind, error: state.dialog?.error, github: state.github })));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading) && !s.github?.page?.loading, 'Settled'); }
function control(id) { return state.controls.find(c => c.id === id); }
const page = () => state.github.page, opened = () => state.github.opened_urls, error = () => state.dialog?.error ?? null;
const code = () => error()?.match(/\[(github_[a-z_]+)\]/)?.[1] ?? null;
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { if (live) return; await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function scroll(target, direction = 'down', pixels = 600) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); await stable(); }
async function menuItem(id) { await click('menu-repository'); await scroll('repository-menu-scroll', 'up', 2000); if (SUBMENU[id]) { await click(SUBMENU[id]); await stable(); } for (let i = 0; i < 10; i++) { const c = control(id); if (c && c.rect[3] - c.rect[1] >= 28) break; await scroll('repository-menu-scroll', 'down', 250); } await click(id); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function open(f) { if (state?.dialog) await click('dialog-cancel'); await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920); await closeConsole(); }
async function github() { await menuItem('repository-github'); await until(s => s.dialog?.kind === 'github' && !s.github?.page?.loading, 'GitHub page'); await stable(); }
async function closeWindow() { if (state?.dialog) await click('dialog-cancel'); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
/// Scroll a short window's region until a control is fully usable.
async function reveal(target, region) { for (let i = 0; i < 8 && !(control(target)?.rect[3] - control(target)?.rect[1] >= 20); i++) await scroll(region, 'down', 250); assert(control(target)?.enabled, target + ' unreachable'); }

// ---- Fake-API cases --------------------------------------------------------------------
async function repositoryLinks(f) {
  await open(f); await github();
  assert.equal(error(), null); assert.equal(page().section, 'Repository'); assert.deepEqual(helperCalls(), ['never']);
  assert(state.github.enabled && state.github.signed_in && !state.github.live); assert.equal(page().login, 'rocket-fake');
  assert.deepEqual(page().links, { remote: 'origin', repository: 'rocket-fake/forked_repo', branch, pushed: branch, ahead: 1, base: 'rocket-org/upstream', base_branch: 'main', pull: null });
  assert(control('github-create-pull')?.enabled && !control('github-open-pull'));
  await capture('repository-wide');
  await click('github-create-pull'); await click('github-open-repository'); await click('github-open-branch'); await click('github-open-pulls');
  assert.deepEqual(opened(), [PULL, 'https://github.com/rocket-fake/forked_repo', 'https://github.com/rocket-fake/forked_repo/tree/rocket-ui-test/github', 'https://github.com/rocket-org/upstream/pulls']);
  passed('Opening GitHub signs in through the credential helper without prompting, reads the fork, its parent and this branch, and Create pull request records the fork-to-parent comparison page');
  pullOpen = true; await click('github-refresh');
  assert.deepEqual(page().links.pull, { number: 7, url: 'https://github.com/rocket-org/upstream/pull/7', draft: true });
  assert(!control('github-create-pull') && control('github-open-pull')?.enabled); await click('github-open-pull');
  assert.equal(opened().at(-1), 'https://github.com/rocket-org/upstream/pull/7'); assert.deepEqual(helperCalls(), ['never']); pullOpen = false;
  await resize(780, 620); await closeConsole(); await reveal('github-open-pulls', 'github-scroll'); await capture('repository-narrow'); await click('github-open-pulls'); await resize(1480, 920);
  assert.equal(opened().at(-1), 'https://github.com/rocket-org/upstream/pulls');
  passed('An open pull request for the branch replaces Create with View pull request #7; refresh reuses the session; the actions stay reachable at 780x620');
}
async function notifications() {
  await click('github-tab-notifications'); await until(s => s.github.page.loaded[2], 'Notifications');
  assert.equal(page().notification_count, 2); assert.equal(page().unread_count, 2); assert(!page().notifications_more);
  await click('github-notifications-all'); assert(page().all_notifications); assert.equal(page().notification_count, 3); assert(page().notifications_more);
  await click('github-load-more'); assert.equal(page().notification_count, 4); assert(!page().notifications_more);
  assert.deepEqual(page().notifications.map(n => n.url), ['https://github.com/rocket-org/upstream/pull/42', 'https://github.com/rocket-fake/notes/issues/5', 'https://github.com/rocket-org/upstream/discussions', 'https://github.com/rocket-org/upstream/releases']);
  await capture('notifications-wide');
  await type('github-filter', 'release'); assert.deepEqual(state.controls.filter(c => c.id.startsWith('github-notification-row-')).map(c => c.id), ['github-notification-row-3']);
  await click('github-notification-row-3'); await click('github-notification-open');
  assert.equal(opened().at(-1), 'https://github.com/rocket-org/upstream/releases');
  assert(requests.filter(r => r.url.startsWith('/notifications')).every(r => !r.url.includes('participating')) && !requests.some(r => r.method !== 'GET'));
  passed('Notifications: unread first, Include read and Load more page through GitHub, the filter narrows rows, and Open records the subject page; only GET requests reach GitHub');
}
async function repositories() {
  await click('github-tab-repositories'); await until(s => s.github.page.loaded[1], 'Repositories');
  assert.equal(page().filter, '', 'Switching sections clears the filter');
  assert.deepEqual(page().repositories.map(r => r.name), ['rocket-fake/forked_repo', 'rocket-fake/notes']); assert(page().repositories_more);
  await click('github-load-more'); assert.equal(page().repository_count, 3); assert(!page().repositories_more);
  await click('github-repository-row-2'); await capture('repositories-wide');
  await click('github-repository-open'); assert.equal(opened().at(-1), 'https://github.com/rocket-org/upstream');
  await resize(780, 620); await closeConsole(); await reveal('github-repository-clone', 'page-details-scroll'); await capture('repositories-narrow');
  await click('github-repository-clone'); await until(s => s.dialog?.kind === 'clone', 'Clone dialog'); assert.equal(state.dialog.first, 'https://github.com/rocket-org/upstream.git');
  await capture('clone-prefilled'); await click('dialog-cancel'); await resize(1480, 920);
  assert.deepEqual(helperCalls(), ['never']);
  passed('Repositories: list with Load more, select, Open on GitHub, and Clone opens the Clone dialog with the HTTPS URL (nothing cloned); still one credential lookup');
}
async function failures() {
  await github(); assert(page().linked);
  mode = 'unauthorized'; await click('github-refresh');
  assert.equal(code(), 'github_unauthorized'); assert(page().sign_in && !state.github.signed_in);
  await click('github-refresh'); assert.equal(code(), 'github_unauthorized'); assert.deepEqual(helperCalls(), ['never', 'never']);
  await click('github-sign-in'); assert.equal(code(), 'github_unauthorized'); assert.deepEqual(helperCalls(), ['never', 'never', 'auto']);
  await capture('unauthorized');
  mode = 'ok'; await click('github-sign-in'); assert.equal(error(), null); assert(state.github.signed_in && !page().sign_in && page().linked);
  assert.deepEqual(helperCalls(), ['never', 'never', 'auto', 'auto']);
  passed('A refused token is dropped and Sign in is offered; refresh retries without prompting, Sign in lets the helper prompt, and a working sign-in clears the error');
  mode = 'rate'; await click('github-refresh'); assert.equal(code(), 'github_rate_limited'); assert(state.github.signed_in && !page().sign_in);
  mode = 'ok'; await click('github-refresh'); assert.equal(error(), null);
  passed('Rate limiting is reported with its own code and keeps the sign-in');
  mode = 'slow'; slowClosed = null; await stable();
  await client.tool('rocket_click', { target: 'github-refresh', expected_tab: state.active_tab });
  await until(s => s.github.page.loading && s.controls.some(c => c.id === 'github-cancel'), 'Slow read');
  await delay(300); const started = performance.now();
  await client.tool('rocket_click', { target: 'github-cancel', expected_tab: state.active_tab });
  await until(s => !s.github.page.loading, 'Cancelled read'); const ms = performance.now() - started;
  for (let i = 0; i < 100 && slowClosed === null; i++) await delay(50);
  assert(slowClosed !== null, 'curl kept the connection open after cancellation'); report.cancel_ms = Math.round(ms); report.cancel_disconnect_ms = Math.round(slowClosed - started);
  assert(ms < 10000 && slowClosed - started < 10000, 'Slow cancellation'); assert(page().notice.includes('cancelled')); assert(page().linked, 'Earlier results are kept');
  mode = 'ok'; await settle(); await capture('cancelled');
  passed('Cancel stops a hanging GitHub request (curl disconnects) and keeps earlier results');
}
async function disabledByDefault(f) {
  fs.unlinkSync(apiMarker); const before = requests.length; await closeWindow();
  await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920); await closeConsole();
  await github(); assert.equal(code(), 'github_disabled'); assert(!state.github.enabled && !state.github.signed_in);
  await click('github-tab-repositories'); assert.equal(code(), 'github_disabled'); assert.equal(requests.length, before);
  await capture('disabled');
  passed('Without the runner marker the MCP host keeps GitHub off: no request reaches GitHub');
}

// ---- Live: read-only, counts only ------------------------------------------------------
async function liveCheck(f) {
  await open(f); await github();
  report.live_repository = { signed_in: state.github.signed_in, failure: code(), linked: page().linked, pull_open: page().has_pull, sign_in_offered: page().sign_in };
  if (control('github-create-pull')?.enabled) { await click('github-create-pull'); report.live_repository.create_pull_recorded = state.github.opened_count === 1; }
  await click('github-tab-repositories'); await until(s => s.github.page.loaded[1] || s.dialog?.error, 'Live repositories');
  report.live_repositories = { failure: code(), count: page().repository_count, more: page().repositories_more };
  await click('github-tab-notifications'); await until(s => s.github.page.loaded[2] || s.dialog?.error, 'Live notifications');
  report.live_notifications = { failure: code(), count: page().notification_count, unread: page().unread_count, more: page().notifications_more };
  assert(state.github.live && state.github.opened_urls === null && page().repositories === undefined, 'Live state must be redacted');
  assert(report.live_repository.signed_in && [null, 'github_not_found'].includes(report.live_repository.failure), 'Live sign-in failed: ' + report.live_repository.failure);
  assert.equal(report.live_repositories.failure, null); assert.equal(report.live_notifications.failure, null);
  passed('Live read-only pass completed (counts only): ' + JSON.stringify({ repository: report.live_repository, repositories: report.live_repositories, notifications: report.live_notifications }));
}

async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    let serviceEnv = env, f;
    if (live) {
      // The real Git configuration, so Git Credential Manager answers; fixtures stay isolated.
      serviceEnv = { ...process.env, GIT_TERMINAL_PROMPT: '0' };
      fs.writeFileSync(apiMarker, 'https://api.github.com\n'); f = fixture('PRIVATE_LIVE', LIVE_ORIGIN);
    } else {
      await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); port = server.address().port;
      fakeHelper(); fs.writeFileSync(apiMarker, `http://127.0.0.1:${port}\n`); f = fixture('PRIVATE_GITHUB', 'https://github.com/rocket-fake/forked_repo.git');
    }
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const mcpPort = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + mcpPort + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env: serviceEnv, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    if (live) await liveCheck(f);
    else { await repositoryLinks(f); await notifications(); await repositories(); await failures(); await disabledByDefault(f); }
    assert.equal(hash(path.join(f.repo, '.git/config')), f.config, 'Repository config changed');
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['gho_', 'ghp_', 'github_pat_', 'Bearer', 'Authorization', 'PRIVATE_', '界', 'rocket-fake', 'rocket-org', 'forked_repo', 'JasXTD', 'rocket-auth-test', 'api.github.com', '127.0.0.1']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    const ops = live ? ['github-sign-in', 'github-repository', 'github-repositories', 'github-notifications'] : ['github-sign-in', 'github-repository', 'github-pulls', 'github-repositories', 'github-notifications', 'github-open'];
    for (const op of ops) { const starts = records.filter(r => r.event === 'request_start' && r.operation === op); assert(starts.length, 'Missing ' + op); for (const start of starts) assert(records.some(r => r.event === 'request_end' && r.session === start.session && r.request === start.request), 'Unpaired ' + op); }
    for (const step of ['read_credential', 'read_github_user', 'read_github_repository', 'read_github_repositories', 'read_github_notifications']) assert(payload.includes('"' + step + '"'), 'Missing step ' + step);
    if (!live) {
      for (const failure of ['github_unauthorized', 'github_rate_limited']) assert(records.some(r => r.failure_code === failure), 'Missing failure ' + failure);
      assert(records.some(r => r.event === 'request_end' && r.outcome === 'cancelled' && String(r.operation).startsWith('github-')), 'Missing cancelled GitHub request');
      assert(records.some(r => r.event === 'git_start' && r.operation === 'credential') && records.some(r => r.event === 'git_start' && r.operation === 'curl'), 'credential/curl commands lack fixed labels');
      assert(requests.length && requests.every(r => r.auth && r.method === 'GET' && r.accept === 'application/vnd.github+json' && r.version === '2022-11-28' && /^Rocket\/\d+\.\d+\.\d+$/.test(r.agent)), 'Request headers');
      report.api_requests = requests.length;
    }
    report.log_records = records.length; passed('Paired github-* diagnostic spans with fixed steps, failure codes and command labels, zero dropped events, no token, account, repository or host data'); report.passed = true;
  } catch (error) { report.failure = live ? String(error.message).slice(0, 300) : error.stack; if (client && !live) try { await capture('failure'); } catch {} throw error; }
  finally {
    if (!live) { fs.writeFileSync('work/github-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); }
    console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish(); server.close();
  }
}
run().catch(e => { console.error(live ? 'Live check failed' : e); process.exitCode = 1; });
