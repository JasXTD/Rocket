// The Actions page through the native UI and localhost MCP, against a synthetic
// GitHub API: the sidebar entry and its dot, each empty state, the runs list and
// its filters, one run's jobs, the reviewed re-run of that run, and the links from
// the popover, the PR Checks tab and the status bar. No personal account, network
// or hosted write. Run from the checkout whose target/release/rocket-mcp.exe is
// under test (ROCKET_ACTIONS_EXE overrides). Needs the coordinated release/MCP slot.
'use strict';
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process'), crypto = require('node:crypto');
const http = require('node:http'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/actions-' + crypto.randomUUID().replaceAll('-', '').slice(0, 12));
const profile = path.join(root, 'profile'), BRANCH = 'rocket-ui-test/main';
const token = 'fixture_ACTIONS_' + crypto.randomBytes(12).toString('hex');
const exe = path.resolve(process.env.ROCKET_ACTIONS_EXE || 'target/release/rocket-mcp.exe');
const gitExe = path.resolve('target/release/git/2.56.0.windows.4/cmd/git.exe');
const shots = path.join(root, 'shots');
for (const d of ['template', 'no-hooks', 'remotes', 'shots']) fs.mkdirSync(path.join(root, d), { recursive: true });
fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env };
for (const k of Object.keys(env)) if (/^(GIT_|SSH_|GCM_|ROCKET_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k)) delete env[k];
Object.assign(env, {
  GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1',
  GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'template'), GIT_OPTIONAL_LOCKS: '0',
  http_proxy: 'http://127.0.0.1:1', all_proxy: 'http://127.0.0.1:1', NO_PROXY: '',
});
const report = { passed: false, root, cases: [] };
const pass = s => { report.cases.push(s); console.error('PASS ' + s); };
let clock = Date.UTC(2026, 10, 3, 1);
function repo(name, remote = true) {
  const dir = path.join(root, name), bare = path.join(root, 'remotes', name + '.git');
  const git = (args, extra = {}, cwd = dir) => cp.execFileSync(gitExe, args, { cwd, env: { ...env, ...extra }, windowsHide: true, encoding: 'utf8' }).trimEnd();
  fs.mkdirSync(dir, { recursive: true });
  git(['init', '--quiet', '--initial-branch=' + BRANCH]);
  for (const [k, v] of [['user.name', 'Mira Chen'], ['user.email', 'fixture@example.invalid'], ['core.autocrlf', 'false'], ['commit.gpgsign', 'false'], ['core.hooksPath', path.join(root, 'no-hooks')]]) git(['config', k, v]);
  fs.writeFileSync(path.join(dir, '.git/rocket-ui-fixture'), BRANCH);
  const commit = subject => {
    fs.appendFileSync(path.join(dir, 'notes.md'), subject + '\n');
    git(['add', '--all']);
    const iso = new Date(clock += 2.5 * 3600000).toISOString();
    git(['commit', '--quiet', '-m', subject], { GIT_AUTHOR_DATE: iso, GIT_COMMITTER_DATE: iso });
    return git(['rev-parse', 'HEAD']);
  };
  const ids = { init: commit('Initial project layout'), a: commit('Add settings loader'), b: commit('Retry sign-in after a timeout'), c: commit('Document the config file'), d: commit('Update the changelog') };
  git(['switch', '--quiet', '-c', 'feature/sign-in']);
  ids.f = commit('Handle an expired token');
  git(['switch', '--quiet', BRANCH]);
  if (remote) {
    git(['init', '--quiet', '--bare', '--initial-branch=' + BRANCH, bare], {}, root);
    git(['remote', 'add', 'origin', bare]);
    git(['push', '--quiet', '-u', 'origin', BRANCH, 'feature/sign-in']);
    // Remote-tracking refs stay; the URL now names the GitHub repository Rocket matches.
    git(['remote', 'set-url', 'origin', `https://github.com/team/${name}.git`]);
  }
  return { dir, ids, git };
}
const sample = repo('sample'), denied = repo('denied'), limited = repo('limited'), empty = repo('empty'), local = repo('local', false);
const S = sample.ids;
const before = sample.git(['rev-parse', 'HEAD']);

// Synthetic GitHub API.
let attempt = 1, rerunPosts = 0;
const requests = [];
const ago = minutes => new Date(Date.now() - minutes * 60000).toISOString().replace(/\.\d+Z$/, 'Z');
const rollup = { [S.d]: 'PENDING', [S.c]: 'SUCCESS', [S.b]: 'FAILURE', [S.a]: 'SUCCESS', [S.f]: 'FAILURE' };
function checkRuns(sha) {
  const run = sha === S.f ? 812 : 807;
  const retried = sha === S.b && attempt > 1;
  const rows = [
    { name: 'build (windows-latest)', ...(retried ? { status: 'queued', conclusion: null } : { status: 'completed', conclusion: 'failure', started_at: '2026-11-04T09:12:03Z', completed_at: '2026-11-04T09:16:44Z' }) },
    { name: 'build (ubuntu-latest)', status: 'completed', conclusion: 'success', started_at: '2026-11-04T09:12:03Z', completed_at: '2026-11-04T09:14:31Z' },
    { name: 'lint', status: 'completed', conclusion: 'success', started_at: '2026-11-04T09:12:04Z', completed_at: '2026-11-04T09:12:59Z' },
    { name: 'docs', status: 'completed', conclusion: 'skipped' },
  ];
  return rows.map((r, i) => ({ ...r, head_sha: sha, html_url: `https://github.com/team/sample/actions/runs/${run}/job/${101 + i}`, app: { name: 'GitHub Actions' }, check_suite: { id: run * 10 } }));
}
/** The repository's 31 runs, newest first; 807 runs again once re-run. */
function allRuns() {
  const retried = attempt > 1;
  const rows = [
    { id: 815, name: 'CI', run_number: 815, run_attempt: 1, event: 'push', status: 'in_progress', conclusion: null, head_branch: BRANCH, head_sha: S.d, created_at: ago(3), run_started_at: ago(1) },
    { id: 812, name: 'CI', run_number: 812, run_attempt: 1, event: 'pull_request', pull_requests: [{ number: 42 }], status: 'completed', conclusion: 'failure', head_branch: 'feature/sign-in', head_sha: S.f, created_at: ago(120), run_started_at: '2026-11-04T09:12:03Z', updated_at: '2026-11-04T09:16:44Z' },
    { id: 77, name: 'Docs', run_number: 77, run_attempt: 1, event: 'push', status: 'completed', conclusion: 'success', head_branch: BRANCH, head_sha: S.c, created_at: ago(300), run_started_at: '2026-11-04T08:00:00Z', updated_at: '2026-11-04T08:00:55Z' },
    { id: 807, name: 'CI', run_number: 807, run_attempt: attempt, event: 'push', head_branch: BRANCH, head_sha: S.b, check_suite_id: 8070, created_at: ago(1440), ...(retried ? { status: 'in_progress', conclusion: null, run_started_at: ago(0) } : { status: 'completed', conclusion: 'failure', run_started_at: '2026-11-04T09:12:03Z', updated_at: '2026-11-04T09:16:44Z' }) },
    { id: 806, name: 'CI', run_number: 806, run_attempt: 1, event: 'push', status: 'completed', conclusion: 'success', head_branch: BRANCH, head_sha: S.a, created_at: ago(1500) },
    { id: 31, name: 'Nightly', run_number: 31, run_attempt: 1, event: 'schedule', status: 'completed', conclusion: 'cancelled', head_branch: BRANCH, head_sha: S.a, created_at: ago(2900) },
  ];
  for (let i = 0; rows.length < 31; i++) rows.push({ id: 700 - i, name: 'CI', run_number: 700 - i, run_attempt: 1, event: 'push', status: 'completed', conclusion: 'success', head_branch: BRANCH, head_sha: S.init, created_at: ago(4000 + i * 60) });
  return rows.map(r => ({ ...r, html_url: `https://github.com/team/sample/actions/runs/${r.id}` }));
}
const server = http.createServer(async (req, res) => {
  try {
    let data = ''; for await (const chunk of req) data += chunk;
    const url = new URL(req.url, 'http://127.0.0.1');
    assert(url.pathname.startsWith('/github/'), 'unexpected API base ' + url.pathname);
    const route = url.pathname.slice('/github'.length);
    requests.push({ method: req.method, route, query: url.search, body: data });
    const send = (code, body, headers = {}) => { res.writeHead(code, { 'Content-Type': 'application/json', ...headers }); res.end(typeof body === 'string' ? body : JSON.stringify(body)); };
    if (req.headers.authorization !== 'Bearer ' + token) return send(401, { message: 'bad token' });
    if (route === '/user') return send(200, { login: 'mchen' });
    if (route === '/graphql') {
      const { variables } = JSON.parse(data);
      if (variables.name === 'denied') return send(200, { data: null, errors: [{ type: 'FORBIDDEN', message: 'Resource not accessible' }] });
      if (variables.name === 'limited') return send(200, { data: null, errors: [{ type: 'RATE_LIMITED', message: 'API rate limit exceeded' }] });
      const repository = { nameWithOwner: 'team/' + variables.name };
      for (const [k, v] of Object.entries(variables)) {
        if (!/^c\d+$/.test(k)) continue;
        repository[k] = { oid: v, statusCheckRollup: variables.name === 'sample' && rollup[v] ? { state: rollup[v] } : null };
      }
      return send(200, { data: { repository } });
    }
    if (route === '/repos/team/denied/actions/runs') return send(403, { message: 'Resource not accessible by personal access token' }, { 'X-Accepted-GitHub-Permissions': 'actions=read' });
    if (route === '/repos/team/limited/actions/runs') return send(403, { message: 'API rate limit exceeded' }, { 'X-RateLimit-Remaining': '0' });
    if (route === '/repos/team/empty/actions/runs') return send(200, { total_count: 0, workflow_runs: [] });
    if (req.method === 'POST') {
      assert.equal(route, '/repos/team/sample/actions/runs/807/rerun-failed-jobs');
      rerunPosts++;
      attempt++;
      res.writeHead(201); return res.end();
    }
    const sha = (route.match(/\/commits\/([0-9a-f]{40})\//) || [])[1];
    if (route.endsWith('/check-runs')) { const rows = checkRuns(sha); return send(200, { total_count: rows.length, check_runs: rows }); }
    if (route.endsWith('/status')) return send(200, { sha, state: 'success', total_count: 0, statuses: [] });
    if (route === '/repos/team/sample/actions/runs') {
      const head = url.searchParams.get('head_sha');
      if (head) return send(200, { total_count: 1, workflow_runs: allRuns().filter(r => r.head_sha === head && r.id !== 31 && r.id !== 806).slice(0, 1) });
      assert.equal(url.searchParams.get('per_page'), '30');
      const branch = url.searchParams.get('branch');
      const rows = allRuns().filter(r => !branch || r.head_branch === branch);
      const page = Number(url.searchParams.get('page'));
      return send(200, { total_count: rows.length, workflow_runs: rows.slice((page - 1) * 30, page * 30) });
    }
    const one = route.match(/^\/repos\/team\/sample\/actions\/runs\/(\d+)(\/jobs)?$/);
    if (one) {
      const run = allRuns().find(r => r.id === Number(one[1]));
      if (!run) return send(404, { message: 'Not Found' });
      if (!one[2]) return send(200, run);
      assert(url.searchParams.get('filter') === 'latest', 'jobs of the latest attempt');
      const rows = [807, 812].includes(run.id) ? checkRuns(run.head_sha).map(({ app, check_suite, head_sha, ...j }) => j) : [{ name: 'build', status: run.status, conclusion: run.conclusion }];
      return send(200, { total_count: rows.length, jobs: rows });
    }
    if (route === '/repos/team/sample') return send(200, { full_name: 'team/sample', archived: false, permissions: { push: true, pull: true } }, { 'X-OAuth-Scopes': 'repo, workflow' });
    const pull = () => ({ number: 42, title: 'Handle an expired sign-in token', body: 'Signs in again.', user: { login: 'sokafor' }, state: 'open', draft: false, merged: false, mergeable: true, mergeable_state: 'unstable', head: { sha: S.f, ref: 'feature/sign-in' }, base: { sha: S.d, ref: BRANCH }, commits: 1, changed_files: 1, additions: 2, deletions: 0 });
    if (route === '/repos/team/sample/pulls') return send(200, [pull()]);
    if (route === '/repos/team/sample/pulls/42') return send(200, pull());
    if (route.endsWith('/pulls/42/commits')) return send(200, [{ sha: S.f, commit: { message: 'Handle an expired token', author: { name: 'Sam Okafor' } } }]);
    if (route.endsWith('/pulls/42/files')) return send(200, [{ filename: 'notes.md', status: 'modified', additions: 2, deletions: 0, patch: '@@ -1 +1,2 @@\n a\n+b\n' }]);
    if (route.endsWith('/comments') || route.endsWith('/reviews')) return send(200, []);
    throw Error('Unexpected ' + req.method + ' ' + route);
  } catch (e) { report.server_error = String(e.stack || e); res.writeHead(500); res.end('{}'); }
});

const delay = ms => new Promise(r => setTimeout(r, ms));
let client, service, state;
const control = id => state.controls?.find(c => c.id === id);
const tab = () => state.tabs?.find(t => t.id === state.active_tab);
async function until(fn, label = 'state', ms = 45000) {
  const start = performance.now();
  for (;;) { state = await client.tool('rocket_state'); if (fn(state)) return state; assert(performance.now() - start < ms, label + ' timed out ' + JSON.stringify({ actions: { ...state.actions, runs: undefined }, dialog: state.dialog?.kind, view: tab()?.view })); await delay(80); }
}
const settle = () => until(s => !s.busy && !s.input_pending && !s.hosting?.busy && !(s.tabs || []).some(t => t.loading), 'settled');
async function stable() { let last = '', n = 0; await until(s => { if (s.input_pending) return false; const now = JSON.stringify([s.controls, s.viewport]); n = now === last ? n + 1 : 0; last = now; return n >= 2; }, 'stable controls'); }
async function scroll(region, direction, pixels = 300) { await client.tool('rocket_scroll', { target: region, direction, pixels, expected_tab: state.active_tab }); await until(s => !s.input_pending); await stable(); }
async function reveal(id) {
  await stable();
  const visible = () => control(id) && control(id).rect[3] - control(id).rect[1] > 12;
  if (visible()) return;
  if (control('hosting-scroll')) { for (let i = 0; i < 6 && !visible(); i++) await scroll('hosting-scroll', 'up', 2000); for (let i = 0; i < 32 && !visible(); i++) await scroll('hosting-scroll', 'down', 240); }
  assert(visible(), 'unreachable ' + id);
}
async function click(id) { await reveal(id); assert(control(id)?.enabled, 'missing/disabled ' + id); await client.tool('rocket_click', { target: id, expected_tab: state.active_tab }); await settle(); await stable(); }
async function type(id, text) { await reveal(id); await client.tool('rocket_type', { target: id, text, expected_tab: state.active_tab }); await settle(); await stable(); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending, 'resize ' + width); await settle(); await stable(); }
async function escape() { await client.tool('rocket_escape'); await settle(); await stable(); }
async function capture(name) {
  await stable();
  const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!result.isError, 'capture refused');
  const im = result.content.find(c => c.type === 'image'); assert(im);
  fs.writeFileSync(path.join(shots, name + '.png'), Buffer.from(im.data, 'base64'));
  fs.writeFileSync(path.join(shots, name + '.json'), JSON.stringify(state.actions, null, 1));
}
async function openHosting() {
  await click('menu-repository'); await scroll('repository-menu-scroll', 'up', 2000); await click('repository-remotes-menu'); await stable();
  for (let i = 0; i < 14 && !(control('repository-hosting') && control('repository-hosting').rect[3] - control('repository-hosting').rect[1] > 25); i++) await scroll('repository-menu-scroll', 'down', 230);
  await click('repository-hosting'); await until(s => s.dialog?.kind === 'hosting' && !s.hosting.busy, 'hosting dialog'); await settle();
}
async function closeWindow() {
  for (let i = 0; i < 3 && (state?.dialog || state?.ci?.popover || state?.ci?.rerun); i++) await escape();
  await client.tool('rocket_close');
  await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit' && !s.input_pending, 'close');
  if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'closed'); }
}
const lists = () => requests.filter(r => r.route === '/repos/team/sample/actions/runs' && !r.query.includes('head_sha'));
const runOf = id => state.actions.runs?.find(r => r.id === id);
/** Waits for a list read after `reads` to finish. */
const listed = (reads, label) => until(s => s.actions.reads > reads && !s.actions.reading, label);
const shown = (label, ms) => until(s => s.actions?.shown && !s.dialog, label, ms);

async function run() {
  try {
    await new Promise(r => server.listen(0, '127.0.0.1', r));
    fs.writeFileSync(path.join(root, 'rocket-hosting-api'), 'http://127.0.0.1:' + server.address().port);
    report.executable_sha256 = crypto.createHash('sha256').update(fs.readFileSync(exe)).digest('hex');
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true, stdio: 'pipe' });
    const listener = net.createServer(); await new Promise(r => listener.listen(0, '127.0.0.1', r)); const port = listener.address().port; await new Promise(r => listener.close(r));
    fs.writeFileSync(path.join(profile, 'settings'), `ROCKET-MCP-1\n${port}\n${root}\n`);
    const fd = fs.openSync(path.join(root, 'service.log'), 'w');
    service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', fd] }); fs.closeSync(fd);
    client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i > 120) throw e; await delay(100); } }
    report.version = (await client.initialize()).serverInfo.version;
    await client.tool('rocket_open', { path: sample.dir }); await settle(); await resize(1480, 920);
    if (control('console-close')) await click('console-close');

    // No account: Actions is in the sidebar anyway, and says how to see runs.
    assert(control('view-actions'), 'Actions in the sidebar');
    await click('view-actions');
    await shown('actions shown');
    await until(s => s.actions.reach === 'no_account', 'no account');
    assert(control('actions-connect') && control('actions-guide'));
    assert.equal(requests.length, 0, 'nothing is read without an account');
    await capture('no-account');
    await click('actions-connect');
    await until(s => s.preferences.page === 'accounts', 'accounts');
    assert.equal(state.hosting.provider, 'github');
    pass('No account: Actions shows anyway, names the host and opens Accounts on GitHub; no request');

    // Connect there with Remember on; back on Actions the runs read once.
    await type('hosting-token', token);
    if (!state.hosting.remember) await click('hosting-remember');
    await click('hosting-connect'); assert(state.hosting.connected, state.hosting.error);
    await click('dialog-cancel'); await until(s => !s.dialog, 'accounts closed');
    await shown('actions after connect');
    await until(s => s.actions.reach === 'ready' && s.actions.reads >= 1 && !s.actions.reading, 'first runs');
    assert.equal(state.actions.count, 30); assert(state.actions.more);
    assert.equal(lists().length, 1, 'one request for the newest page');
    assert(lists()[0].query.includes('page=1') && !lists()[0].query.includes('branch='));
    // The newest failed run is selected and its jobs read.
    await until(s => s.actions.details?.run === 812 && !s.actions.reading_details, 'details 812');
    assert.equal(state.actions.selected, 812);
    assert.deepEqual(state.actions.details.failed, ['build (windows-latest)']);
    const r812 = runOf(812);
    assert.deepEqual([r812.event, r812.branch, r812.took, r812.state], ['pull request #42', 'feature/sign-in', 281, 'failed']);
    assert.equal(runOf(815).state, 'running');
    assert.equal(runOf(31).state, 'cancelled');
    assert(control('actions-rerun')?.enabled, 'Hosting\'s account can re-run');
    assert.equal(state.actions.dot, 'Running', 'the current branch\'s newest run is running');
    await capture('actions-wide');
    pass('Runs: one page of 30 newest first, the newest failed run selected with its jobs, the sidebar dot');

    await click('actions-show-failed');
    assert(control('actions-run-812') && control('actions-run-807') && !control('actions-run-815'));
    await click('actions-show-running');
    assert(control('actions-run-815') && !control('actions-run-812'));
    await click('actions-show-all');
    assert.equal(lists().length, 1, 'state filters read nothing');
    let reads = state.actions.reads;
    await click('actions-branch-current');
    await listed(reads, 'branch runs');
    assert(lists().at(-1).query.includes('branch=rocket-ui-test%2Fmain'), lists().at(-1).query);
    assert(state.actions.runs.every(r => r.branch === BRANCH));
    reads = state.actions.reads;
    await click('actions-branch-all');
    await listed(reads, 'all branches again');
    assert(!state.actions.on_branch && state.actions.count === 30);
    reads = state.actions.reads;
    await click('actions-load-more');
    await listed(reads, 'page 2');
    assert.deepEqual([state.actions.count, state.actions.more, state.actions.pages], [31, false, 2]);
    assert(lists().at(-1).query.includes('page=2'));
    pass('Filters: failed and running filter what was read; a branch and Load more read once each');

    await click('actions-run-807');
    await until(s => s.actions.details?.run === 807 && !s.actions.reading_details, 'details 807');
    const opened = state.github.opened_count;
    await click('actions-open-run');
    assert.equal(state.github.opened_urls.at(-1), 'https://github.com/team/sample/actions/runs/807');
    await click('actions-open-host');
    assert.equal(state.github.opened_urls.at(-1), 'https://github.com/team/sample/actions');
    await click('actions-jobs-open-0');
    assert.equal(state.github.opened_urls.at(-1), 'https://github.com/team/sample/actions/runs/807/job/101');
    assert.equal(state.github.opened_count, opened + 3);
    await click('actions-rerun');
    await until(s => s.ci.rerun && !s.ci.rerun.reviewing && (s.ci.rerun.ready || s.ci.rerun.error), 'run re-run review');
    assert(state.ci.rerun.ready && state.ci.rerun.from_run, state.ci.rerun.error);
    assert.deepEqual(state.ci.rerun.runs, ['CI · run 807']);
    assert.equal(rerunPosts, 0, 'the review sends nothing');
    await capture('rerun-review');
    await click('ci-rerun-submit');
    await until(s => !s.ci.rerun, 're-run sent');
    assert.equal(rerunPosts, 1);
    reads = state.actions.reads;
    await click('actions-refresh');
    await listed(reads, 'refresh after re-run');
    await until(s => s.actions.details?.run === 807 && !s.actions.reading_details && runOf(807)?.attempt === 2, 'run 807 attempt 2');
    assert.equal(runOf(807).state, 'running');
    assert(!control('actions-rerun')?.enabled, 'nothing to re-run while it runs again');
    await capture('rerun-started');
    pass('One run: its jobs and links open; Re-run reviews that run only, one POST, then it runs again');

    await click('actions-show-commit');
    await until(s => tab()?.view === 'Commits' && tab()?.selected_commit === S.b, 'show in commits');
    pass('Show in Commits selects the run\'s commit in history');

    await click('ci-mark-' + S.b);
    await until(s => s.ci.popover?.commit === S.b && s.ci.popover.details && !s.ci.popover.reading, 'popover');
    reads = state.actions.reads;
    await click('ci-popover-all-runs');
    await shown('actions from the popover');
    await listed(reads, 'runs from the popover');
    assert(state.actions.on_branch && state.actions.branch === BRANCH && state.actions.selected === 807, JSON.stringify({ ...state.actions, runs: undefined }));
    assert(!state.ci.popover);
    pass('The popover\'s All runs opens Actions on the run\'s branch with that run selected');

    await openHosting();
    await type('hosting-repository', 'team/sample'); await click('hosting-refresh');
    await click('hosting-details-0');
    await until(s => s.ci.pull.checks && !s.ci.pull.reading, 'PR checks');
    await click('hosting-pull-tab-checks');
    reads = state.actions.reads;
    await click('hosting-checks-all-runs');
    await shown('actions from the PR');
    await listed(reads, 'PR branch runs');
    assert(state.actions.on_branch && state.actions.branch === 'feature/sign-in');
    assert(lists().at(-1).query.includes('branch=feature%2Fsign-in'));
    await capture('from-pull');
    pass('The PR Checks tab\'s All runs opens Actions on the pull request\'s branch');

    reads = state.actions.reads;
    await click('actions-branch-all');
    await listed(reads, 'all branches before compact');
    await resize(780, 620);
    assert(!control('actions-open-host') && !control('actions-branch-all'), 'the compact page keeps the state filter and Refresh');
    await click('actions-run-812');
    await until(s => s.actions.focused && s.actions.details?.run === 812 && !s.actions.reading_details, 'compact details');
    assert(control('actions-back'));
    await capture('compact-details');
    await click('actions-back');
    assert(control('actions-run-812'));
    await capture('compact');
    await resize(1480, 920);
    pass('Compact window: the list, then one run\'s details with a way back');

    await client.tool('rocket_open', { path: denied.dir }); await settle();
    await until(s => s.ci.tab?.whole?.kind === 'hidden', 'hidden marks');
    await click('ci-status');
    await shown('actions hidden');
    await until(s => s.actions.hidden && !s.actions.reading, 'hidden runs');
    assert.equal(state.actions.missing, 'Actions: Read');
    assert(control('actions-accounts'));
    await capture('hidden');
    await client.tool('rocket_open', { path: limited.dir }); await settle();
    await until(s => s.ci.tab?.whole?.kind === 'paused', 'paused marks');
    await click('ci-status');
    await shown('actions paused');
    await until(s => s.actions.failure === 'hosting_rate_limited' && s.actions.paused && !s.actions.reading, 'paused runs');
    assert(control('actions-retry'));
    await capture('paused');
    pass('The status bar\'s Checks hidden and Checks paused open Actions, which says what to do');

    await client.tool('rocket_open', { path: empty.dir }); await settle();
    await until(s => s.ci.tab?.refreshes >= 1 && !s.ci.tab.reading, 'marks without CI');
    assert(state.ci.tab.quiet, 'no loaded commit has a check');
    assert(!state.controls.some(c => c.id.startsWith('ci-mark-') || c.id.startsWith('ci-branch-mark-')), 'no Not checked rings in history or beside branches');
    await click('view-actions');
    await shown('actions without CI');
    await until(s => s.actions.reads >= 1 && !s.actions.reading, 'empty runs');
    assert.equal(state.actions.count, 0);
    assert(control('actions-open-host'));
    await capture('no-workflow');
    await client.tool('rocket_open', { path: local.dir }); await settle();
    await click('view-actions');
    await until(s => s.actions?.reach === 'no_remote', 'no remote');
    await capture('no-remote');
    await click('actions-remotes');
    await until(s => tab()?.view === 'Remotes', 'remotes');
    pass('No CI: no rings in history and Actions says how to add a workflow; no remote: Remotes');

    // A new window signs the remembered account in by itself. On a tab that never
    // showed Actions, the popover's All runs keeps its run; Re-run waits for Hosting.
    await closeWindow();
    await client.tool('rocket_open', { path: sample.dir }); await settle(); await resize(1480, 920);
    await until(s => s.ci.tab?.refreshes >= 1 && Object.keys(s.ci.tab.marks || {}).length > 0, 'remembered marks', 60000);
    if (tab()?.view !== 'Commits') await click('view-commits');
    await click('ci-mark-' + S.b);
    await until(s => s.ci.popover?.commit === S.b && s.ci.popover.details && !s.ci.popover.reading, 'remembered popover');
    await click('ci-popover-all-runs');
    await shown('actions on a fresh tab');
    await until(s => s.actions.reach === 'ready' && s.actions.reads >= 1 && !s.actions.reading && s.actions.details?.run === 807 && !s.actions.reading_details, 'fresh tab link', 60000);
    assert(state.actions.selected === 807 && state.actions.on_branch && state.actions.branch === BRANCH, JSON.stringify({ ...state.actions, runs: undefined }));
    assert(!state.actions.live && !state.hosting.connected);
    reads = state.actions.reads;
    await click('actions-branch-all');
    await listed(reads, 'all branches, remembered');
    await click('actions-run-812');
    await until(s => s.actions.details?.run === 812 && !s.actions.reading_details, 'remembered 812');
    assert(!control('actions-rerun')?.enabled, 'the quiet sign-in only reads');
    assert(control('actions-connect-hosting'));
    await capture('remembered');
    pass('A fresh tab keeps the popover link\'s run; a remembered account reads Actions and Re-run waits for Hosting');

    assert.equal(sample.git(['rev-parse', 'HEAD']), before, 'local repository unchanged');
    assert(!report.server_error, report.server_error);
    assert(requests.every(r => r.method === 'GET' || r.route === '/graphql' || r.route.endsWith('/rerun-failed-jobs')), 'no other writes');
    await closeWindow();
    await client.raw('POST', '/shutdown');
    for (let i = 0; i < 150 && service.exitCode === null; i++) await delay(100);
    const logs = [];
    (function walk(p) { for (const f of fs.readdirSync(p, { withFileTypes: true })) { const n = path.join(p, f.name); if (f.isDirectory()) walk(n); else if (f.name.endsWith('.jsonl')) logs.push(n); } })(root);
    const text = logs.map(p => fs.readFileSync(p, 'utf8')).join('\n');
    for (const secret of [token, 'team/sample', 'Authorization', 'feature/sign-in']) assert(!text.includes(secret), 'private log payload ' + secret);
    for (const name of ['hosting-ci-runs', 'hosting-ci-run']) assert(text.includes(`"operation":"${name}"`), 'diagnostics name ' + name);
    pass('Diagnostics name the reads and hold no token, repository path or branch');
    report.requests = requests.length; report.rerun_posts = rerunPosts; report.list_reads = lists().length;
    report.passed = true;
  } catch (e) {
    report.error = String(e.stack || e); process.exitCode = 1;
    try { if (client) await capture('failure'); } catch {}
  } finally {
    try { if (service?.exitCode === null) { try { await closeWindow(); } catch {} await client.raw('POST', '/shutdown'); for (let i = 0; i < 100 && service.exitCode === null; i++) await delay(100); if (service.exitCode === null) service.kill(); } } catch {}
    try { if (client) await client.finish(); } catch {}
    const tokenPath = path.join(profile, 'access-token'); if (fs.existsSync(tokenPath)) fs.unlinkSync(tokenPath);
    server.closeAllConnections(); server.close();
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify({ passed: report.passed, cases: report.cases.length, error: report.error, root }, null, 2));
  }
}
run();
