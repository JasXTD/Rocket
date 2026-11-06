// CI marks, the checks popover, reviewed re-runs and the PR Checks tab through the
// native UI and localhost MCP, against a synthetic GitHub API. No personal account,
// network or hosted write. Run from the checkout whose target/release/rocket-mcp.exe
// is under test (ROCKET_CI_EXE overrides). Needs the coordinated release/MCP slot.
'use strict';
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process'), crypto = require('node:crypto');
const http = require('node:http'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/ci-checks-' + crypto.randomUUID().replaceAll('-', '').slice(0, 12));
const profile = path.join(root, 'profile'), BRANCH = 'rocket-ui-test/main';
const token = 'fixture_CI_' + crypto.randomBytes(12).toString('hex');
const exe = path.resolve(process.env.ROCKET_CI_EXE || 'target/release/rocket-mcp.exe');
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
function repo(name) {
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
  git(['init', '--quiet', '--bare', '--initial-branch=' + BRANCH, bare], {}, root);
  git(['remote', 'add', 'origin', bare]);
  git(['push', '--quiet', '-u', 'origin', BRANCH, 'feature/sign-in']);
  ids.e = commit('Tidy the settings module');
  // Remote-tracking refs stay; the URL now names the GitHub repository Rocket matches.
  git(['remote', 'set-url', 'origin', `https://github.com/team/${name}.git`]);
  return { dir, ids, git };
}
const sample = repo('sample'), denied = repo('denied'), limited = repo('limited');
const S = sample.ids;
const before = sample.git(['rev-parse', 'HEAD']);

// Synthetic GitHub API.
let mode = 'ok', attempt = 1, rerunPosts = 0;
const requests = [];
const recent = () => new Date(Date.now() - 3600000).toISOString().replace(/\.\d+Z$/, 'Z');
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
        repository[k] = v === S.e ? null : { oid: v, statusCheckRollup: rollup[v] ? { state: rollup[v] } : null };
      }
      return send(200, { data: { repository } });
    }
    if (route === '/repos/team/denied/actions/runs') return send(403, { message: 'Resource not accessible by personal access token' }, { 'X-Accepted-GitHub-Permissions': 'actions=read' });
    if (req.method === 'POST') {
      assert.equal(route, '/repos/team/sample/actions/runs/807/rerun-failed-jobs');
      rerunPosts++;
      if (mode === 'refuse') return send(403, { message: 'Resource not accessible by personal access token' }, { 'X-Accepted-GitHub-Permissions': 'actions=write' });
      if (mode === 'lost') return send(500, '{}');
      attempt++;
      res.writeHead(201); return res.end();
    }
    const sha = (route.match(/\/commits\/([0-9a-f]{40})\//) || [])[1];
    if (route.endsWith('/check-runs')) { const rows = checkRuns(sha); return send(200, { total_count: rows.length, check_runs: rows }); }
    if (route.endsWith('/status')) return send(200, { sha, state: 'success', total_count: 1, statuses: [{ context: 'netlify/deploy-preview', state: 'success', description: 'Deploy preview ready', target_url: 'https://app.netlify.com/sites/sample/deploys/1' }] });
    if (route === '/repos/team/sample/actions/runs') {
      const head = url.searchParams.get('head_sha');
      const id = head === S.f ? 812 : 807;
      const retried = head === S.b && attempt > 1;
      return send(200, { total_count: 1, workflow_runs: [{ id, name: 'CI', head_sha: head, run_number: id, run_attempt: attempt, check_suite_id: id * 10, created_at: recent(), html_url: `https://github.com/team/sample/actions/runs/${id}`, ...(retried ? { status: 'in_progress', conclusion: null } : { status: 'completed', conclusion: 'failure' }) }] });
    }
    if (route === '/repos/team/sample') return send(200, { full_name: 'team/sample', archived: false, permissions: { push: mode !== 'readonly', pull: true } }, { 'X-OAuth-Scopes': 'repo, workflow' });
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
async function until(fn, label = 'state', ms = 45000) {
  const start = performance.now();
  for (;;) { state = await client.tool('rocket_state'); if (fn(state)) return state; assert(performance.now() - start < ms, label + ' timed out ' + JSON.stringify({ ci: state.ci, dialog: state.dialog?.kind, hosting: state.hosting?.error })); await delay(80); }
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
  fs.writeFileSync(path.join(shots, name + '.json'), JSON.stringify(state.ci, null, 1));
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
const graphqlPosts = () => requests.filter(r => r.route === '/graphql').length;
const marks = () => state.ci.tab?.marks || {};
const details = () => state.ci.popover?.details;
async function openPopover(commit) {
  await click('ci-mark-' + commit);
  await until(s => s.ci.popover?.commit === commit && s.ci.popover.details && !s.ci.popover.reading, 'popover ' + commit.slice(0, 7));
}
async function refreshPopover() {
  await click('ci-popover-refresh');
  await until(s => s.ci.popover?.details && !s.ci.popover.reading, 'popover refresh');
}
async function review() {
  await click('ci-popover-rerun');
  await until(s => s.ci.rerun && !s.ci.rerun.reviewing && (s.ci.rerun.ready || s.ci.rerun.error), 'rerun review');
}

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
    assert(!state.ci.tab.active, 'no marks before an account matches');

    // Connect in Hosting with Remember on; the live session matches origin.
    await openHosting();
    await click('hosting-tab-accounts'); await click('hosting-provider-github'); await type('hosting-token', token);
    if (!state.hosting.remember) await click('hosting-remember');
    await click('hosting-connect'); assert(state.hosting.connected, state.hosting.error);
    // Hosting's Close (Escape doesn't close a page).
    await click('dialog-cancel'); await until(s => !s.dialog, 'hosting closed');
    await until(s => s.ci.tab?.refreshes >= 1 && !s.ci.tab.reading, 'first marks');
    assert.deepEqual(marks(), { [S.a]: 'Passed', [S.b]: 'Failed', [S.c]: 'Passed', [S.d]: 'Running', [S.f]: 'Failed', [S.init]: 'Not checked' });
    assert(!(S.e in marks()), 'an unpushed commit has no mark');
    assert.equal(graphqlPosts(), 1, 'one GraphQL request answers every commit');
    assert(control('ci-mark-' + S.b) && !control('ci-mark-' + S.e));
    assert(control('ci-branch-mark-refs/heads/feature/sign-in'), 'branch mark in the sidebar');
    await capture('marks-wide');
    pass('Marks: one rollup request for pushed commits and branch tips; unpushed commits unmarked');

    await openPopover(S.b);
    assert.equal(details().summary, '1 failed · 3 passed · 1 skipped');
    assert.equal(details().checks[0].name, 'build (windows-latest)');
    assert.equal(details().checks[0].group, 'CI · run 807');
    assert.equal(details().checks.find(c => c.name === 'netlify/deploy-preview').external, 'app.netlify.com');
    assert.equal(details().rerun_block, null);
    await capture('popover');
    const opened = state.github.opened_count;
    await click('ci-popover-open-0');
    assert.equal(state.github.opened_count, opened + 1);
    assert.equal(state.github.opened_urls.at(-1), 'https://github.com/team/sample/actions/runs/807/job/101');
    await click('ci-popover-open');
    assert.equal(state.github.opened_urls.at(-1), 'https://github.com/team/sample/actions/runs/807');
    pass('Popover: failures first, a job and the run open in the browser, another site keeps its name');

    await review();
    assert(state.ci.rerun.ready && !state.ci.rerun.error);
    assert.deepEqual(state.ci.rerun.runs, ['CI · run 807']);
    assert.equal(rerunPosts, 0, 'the review sends nothing');
    await capture('rerun-review');
    await click('ci-rerun-submit');
    await until(s => !s.ci.rerun && s.ci.popover?.notice, 'rerun started');
    assert.equal(rerunPosts, 1);
    assert(state.ci.popover.notice.includes('CI · run 807, attempt 2'), state.ci.popover.notice);
    await until(s => s.ci.popover.details && !s.ci.popover.reading && s.ci.popover.details.checks[0].state === 'queued', 'refreshed after rerun');
    await capture('rerun-started');
    pass('Re-run: reviewed, one POST, the popover says it started and shows the queued job');

    // Refused: the strip names the permission; nothing is sent twice.
    attempt = 1; mode = 'refuse';
    await refreshPopover(); await review();
    await click('ci-rerun-submit');
    await until(s => s.ci.rerun && !s.ci.rerun.sending && s.ci.rerun.error, 'refused');
    assert(state.ci.rerun.error.includes('Actions: Read and write') && state.ci.rerun.error.includes('Nothing was re-run'), state.ci.rerun.error);
    assert(!control('ci-rerun-submit')?.enabled);
    await capture('rerun-refused');
    await escape();
    // Lost answer: uncertain, no retry until refreshed.
    mode = 'lost';
    await review();
    await click('ci-rerun-submit');
    await until(s => s.ci.rerun?.unknown && !s.ci.rerun.sending, 'unknown outcome');
    assert(!control('ci-rerun-submit')?.enabled);
    await capture('rerun-unknown');
    const posts = rerunPosts;
    await click('ci-rerun-refresh');
    await until(s => !s.ci.rerun && s.ci.popover?.details && !s.ci.popover.reading, 'refresh after unknown');
    assert.equal(rerunPosts, posts, 'no automatic retry');
    pass('Refused re-run names the permission; a lost answer waits for a refresh and never retries');

    mode = 'readonly';
    await refreshPopover();
    assert.equal(details().rerun_block, 'ci_rerun_no_write');
    assert(!control('ci-popover-rerun').enabled);
    await capture('no-write');
    mode = 'ok';
    await escape();
    assert(!state.ci.popover);
    pass('Without write access Re-run is not offered, before any review');

    await resize(780, 620);
    await openPopover(S.b);
    const pop = control('ci-popover').rect;
    assert(pop[0] >= 0 && pop[2] <= 780 && pop[1] >= 0 && pop[3] <= 620, 'popover inside the compact window ' + JSON.stringify(pop));
    await capture('compact-popover');
    await escape(); await resize(1480, 920);
    pass('Compact window: marks keep their place and the popover fits');

    await click('commit-' + S.b);
    await click('ci-details-checks');
    await until(s => s.ci.popover?.details && !s.ci.popover.reading, 'details line popover');
    await escape();
    pass('Show checks in the commit details opens the same checks');

    // The PR Checks tab reads the head the same way and offers the same review.
    await openHosting();
    await type('hosting-repository', 'team/sample'); await click('hosting-refresh');
    await click('hosting-details-0');
    await until(s => s.ci.pull.checks && !s.ci.pull.reading, 'PR checks');
    assert.equal(state.ci.pull.checks.commit, S.f);
    assert(control('hosting-checks-pill'), 'checks pill in the PR header');
    await click('hosting-pull-tab-checks');
    await capture('pr-checks');
    await click('hosting-checks-rerun');
    await until(s => s.ci.rerun && !s.ci.rerun.reviewing, 'PR rerun review');
    assert.deepEqual(state.ci.rerun.runs, ['CI · run 812']);
    await escape();
    assert(!state.ci.rerun);
    if (state.dialog?.kind === 'hosting') { await click('dialog-cancel'); await until(s => !s.dialog, 'hosting closed'); }
    pass('PR Checks tab: one list, header pill, same reviewed re-run');

    await client.tool('rocket_open', { path: denied.dir }); await settle();
    await until(s => s.ci.tab?.whole?.kind === 'hidden', 'hidden');
    assert.equal(state.ci.tab.whole.missing, 'Actions: Read');
    assert(control('ci-status'));
    await capture('hidden');
    await client.tool('rocket_open', { path: limited.dir }); await settle();
    await until(s => s.ci.tab?.whole?.kind === 'paused', 'paused');
    await capture('paused');
    pass('Whole-repository states: hidden names the permission; rate limit pauses');

    // A new window signs the remembered account in by itself, for reads.
    const users = requests.filter(r => r.route === '/user').length;
    await closeWindow();
    await client.tool('rocket_open', { path: sample.dir }); await settle(); await resize(1480, 920);
    await until(s => s.ci.tab?.refreshes >= 1 && Object.keys(s.ci.tab.marks || {}).length > 0, 'remembered account marks', 60000);
    assert(requests.filter(r => r.route === '/user').length > users, 'verified the remembered account');
    assert(!state.hosting.connected, 'Hosting itself stays disconnected');
    await openPopover(S.b);
    assert(!control('ci-popover-rerun').enabled, 'the quiet sign-in only reads');
    await capture('remembered');
    await escape();
    pass('A remembered account signs in by itself for marks; Hosting is untouched');

    assert.equal(sample.git(['rev-parse', 'HEAD']), before, 'local repository unchanged');
    assert(!report.server_error, report.server_error);
    assert(requests.every(r => r.method === 'GET' || r.route === '/graphql' || r.route.endsWith('/rerun-failed-jobs')), 'no other writes');
    await closeWindow();
    await client.raw('POST', '/shutdown');
    for (let i = 0; i < 150 && service.exitCode === null; i++) await delay(100);
    const logs = [];
    (function walk(p) { for (const f of fs.readdirSync(p, { withFileTypes: true })) { const n = path.join(p, f.name); if (f.isDirectory()) walk(n); else if (f.name.endsWith('.jsonl')) logs.push(n); } })(root);
    const text = logs.map(p => fs.readFileSync(p, 'utf8')).join('\n');
    for (const secret of [token, 'team/sample', 'Authorization']) assert(!text.includes(secret), 'private log payload ' + secret);
    pass('No token, repository path or credentials in diagnostics');
    report.requests = requests.length; report.rerun_posts = rerunPosts; report.graphql_posts = graphqlPosts();
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
