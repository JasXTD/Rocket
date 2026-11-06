// Repository manager status, Fetch all and Pull all, natively. Only marked local fixtures,
// local bare remotes inside the fixture root and an isolated profile.
// node tools/tests/test-mcp-repository-status.cjs   (ROCKET_STATUS_EXE: another rocket-mcp.exe)
'use strict';
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process'), crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/repo-status-' + crypto.randomUUID().replaceAll('-', '').slice(0, 12));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/status';
const report = { passed: false, root, cases: [] };
for (const d of ['empty-template', 'no-hooks', 'remotes', 'Projects', 'elsewhere', 'remotes/not-a-repo']) fs.mkdirSync(path.join(root, d), { recursive: true });
fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env };
for (const k of Object.keys(env)) if (/^(GIT_|SSH_|GCM_|ROCKET_)|_PROXY$/i.test(k)) delete env[k];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const gitAt = (dir, args) => cp.execFileSync('git', ['-C', dir, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
let clock = Date.UTC(2026, 9, 20, 9);
function configure(dir) {
  for (const [k, v] of [['user.name', 'Rocket Status Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', path.join(root, 'no-hooks')]]) gitAt(dir, ['config', k, v]);
}
function commit(dir, file, subject) {
  fs.writeFileSync(path.join(dir, file), `${subject}\n`);
  gitAt(dir, ['add', '--all']);
  const iso = new Date(clock += 3600000).toISOString();
  cp.execFileSync('git', ['-C', dir, 'commit', '--quiet', '-m', subject], { env: { ...env, GIT_AUTHOR_DATE: iso, GIT_COMMITTER_DATE: iso }, windowsHide: true });
}
/** A marked repository in Projects with `pushed` commits on its own bare origin. */
function repository(name, pushed, remote = true) {
  const dir = path.join(root, 'Projects', name);
  fs.mkdirSync(dir);
  gitAt(dir, ['init', '--quiet', '--initial-branch=' + branch]);
  configure(dir);
  fs.writeFileSync(path.join(dir, '.git/rocket-ui-fixture'), branch);
  for (let i = 0; i < pushed; i++) commit(dir, i ? `notes-${i}.md` : 'README.md', i ? `Update notes ${i}` : 'Initial commit');
  if (remote) {
    const bare = path.join(root, 'remotes', name + '.git');
    gitAt(root, ['init', '--quiet', '--bare', '--initial-branch=' + branch, bare]);
    gitAt(dir, ['remote', 'add', 'origin', bare]);
    gitAt(dir, ['push', '--quiet', '-u', 'origin', branch]);
  }
  return dir;
}
/** Someone else pushes `n` commits to this repository's origin; returns its new tip. */
function pushElsewhere(name, n) {
  const other = path.join(root, 'elsewhere', name);
  if (!fs.existsSync(other)) { gitAt(root, ['clone', '--quiet', '--branch', branch, path.join(root, 'remotes', name + '.git'), other]); configure(other); }
  else gitAt(other, ['pull', '--quiet', '--ff-only']);
  for (let i = 0; i < n; i++) commit(other, `incoming-${gitAt(other, ['rev-list', '--count', 'HEAD'])}.md`, 'Incoming change');
  gitAt(other, ['push', '--quiet', 'origin', branch]);
  return gitAt(other, ['rev-parse', 'HEAD']);
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
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, status: state.repository_manager?.status }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search?.running), 'Settled'); }
const control = id => state.controls.find(c => c.id === id);
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending, 'Resize ' + width); await settle(); }
async function closeWindow() {
  if (state?.repository_manager?.visible && !state.repository_manager.running) await click('manager-close');
  await client.tool('rocket_close');
  await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation');
  if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); }
}
const manager = () => state.repository_manager;
const status = () => manager().status;
const repo = name => manager().repositories.find(r => r.name === name);
const row = name => status().repositories.find(r => r.id === repo(name).id);
/** No read is queued or running, and every listed repository has an answer. */
const checked = s => !s.repository_manager.status.checking && s.repository_manager.status.repositories.every(r => !['unchecked', 'checking'].includes(r.state));
async function add(dir, name) {
  await click('manager-add'); await type('manager-path', dir); await type('manager-name', name); await click('manager-save');
  assert(!manager().form, JSON.stringify(manager().form));
}
function inside(id) {
  const c = control(id); assert(c, 'Missing ' + id);
  const [left, top, right, bottom] = c.rect;
  assert(left >= 0 && top >= 0 && right <= state.viewport[0] && bottom <= state.viewport[1], id + ' leaves the window: ' + JSON.stringify(c));
}

async function run() {
  try {
    const exe = path.resolve(process.env.ROCKET_STATUS_EXE || 'target/release/rocket-mcp.exe');
    report.executable_sha256 = hash(exe);
    // One repository per state: clean, 2 changed + 1 new, ahead 2, behind 4 (fetched),
    // diverged +1/-3, no upstream, an origin that is not a repository, and one whose
    // folder goes away after it is saved.
    const dirs = {
      'acme-web': repository('acme-web', 3),
      'acme-api': repository('acme-api', 2),
      'design-system': repository('design-system', 1),
      'docs-site': repository('docs-site', 2),
      'storefront': repository('storefront', 2),
      'release-scripts': repository('release-scripts', 1, false),
      'mobile-app': repository('mobile-app', 2),
      'legacy-portal': repository('legacy-portal', 1),
    };
    fs.appendFileSync(path.join(dirs['acme-api'], 'README.md'), 'A local edit\n');
    fs.appendFileSync(path.join(dirs['acme-api'], 'notes-1.md'), 'Another local edit\n');
    fs.writeFileSync(path.join(dirs['acme-api'], 'draft.md'), 'not added yet\n');
    commit(dirs['design-system'], 'tokens.md', 'Add spacing tokens'); commit(dirs['design-system'], 'tokens-2.md', 'Add colour tokens');
    pushElsewhere('docs-site', 4); gitAt(dirs['docs-site'], ['fetch', '--quiet', 'origin']);
    pushElsewhere('storefront', 3); commit(dirs['storefront'], 'cart.md', 'Local cart change'); gitAt(dirs['storefront'], ['fetch', '--quiet', 'origin']);
    gitAt(dirs['mobile-app'], ['remote', 'set-url', 'origin', path.join(root, 'remotes', 'not-a-repo')]);
    const heads = Object.fromEntries(Object.entries(dirs).map(([n, d]) => [n, gitAt(d, ['rev-parse', 'HEAD'])]));
    const apiFiles = ['README.md', 'notes-1.md', 'draft.md'].map(f => hash(path.join(dirs['acme-api'], f)));

    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(r => listener.listen(0, '127.0.0.1', r)); const port = listener.address().port; await new Promise(r => listener.close(r));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const fd = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', fd] }); fs.closeSync(fd);
    client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    report.server = (await client.initialize()).serverInfo;
    await client.tool('rocket_open', { path: dirs['acme-web'] }); await settle(); await resize(1480, 920);
    await stable(); if (control('console-close')) await click('console-close');
    await click('menu-file'); await click('repository-manager'); assert(manager().visible);
    for (const name of ['acme-web', 'design-system', 'docs-site', 'release-scripts', 'mobile-app', 'legacy-portal']) await add(dirs[name], name);
    await click('manager-folder-new'); await type('manager-name', 'Client work'); await click('manager-save');
    for (const name of ['acme-api', 'storefront']) await add(dirs[name], name);
    await click('manager-workspace-new'); await type('manager-name', 'Acme');
    for (const name of ['acme-web', 'acme-api', 'design-system']) await click('manager-member-' + repo(name).id);
    await click('manager-save');
    fs.rmSync(dirs['legacy-portal'], { recursive: true, force: true });
    // Check again waits for the reads the page started on its own.
    await click('manager-all'); await until(s => checked(s), 'First status reads'); await click('manager-check');
    await until(s => checked(s) && s.repository_manager.status.repositories.find(r => r.id === s.repository_manager.repositories.find(x => x.name === 'legacy-portal').id).state === 'missing', 'Every status read');
    const expect = {
      'acme-web': { row: 'Up to date', ahead: 0, behind: 0, changes: 0, attention: false },
      'acme-api': { row: 'Up to date', changes: 3, tracked: 2, attention: true },
      'design-system': { row: '↑2', ahead: 2, behind: 0, attention: true },
      'docs-site': { row: '↓4', ahead: 0, behind: 4, attention: true },
      'storefront': { row: '↑1 ↓3', ahead: 1, behind: 3, attention: true },
      'release-scripts': { row: 'No upstream', upstream: null, attention: false },
      'mobile-app': { row: 'Up to date', attention: false },
      'legacy-portal': { row: 'Folder missing', state: 'missing', attention: true },
    };
    for (const [name, want] of Object.entries(expect)) {
      const r = row(name);
      for (const [key, value] of Object.entries(want)) assert.deepEqual(r[key], value, `${name}.${key}: ${JSON.stringify(r)}`);
      if (name !== 'legacy-portal') assert.equal(r.branch, branch, name);
    }
    assert.equal(status().checked, true);
    await click('manager-repo-' + repo('storefront').id); await capture('status-wide');
    passed('Each row reads its state from a local read: up to date, 3 changes, ↑2, ↓4, ↑1 ↓3, No upstream, Folder missing');

    await click('manager-chip-attention'); assert.equal(status().attention_only, true);
    const listed = () => manager().repositories.filter(r => control('manager-repo-' + r.id)).map(r => r.name).sort();
    assert.deepEqual(listed(), ['acme-api', 'design-system', 'docs-site', 'legacy-portal', 'storefront']);
    await capture('status-attention');
    await click('manager-chip-all'); assert.equal(listed().length, 8);
    passed('Needs attention lists only changed, ahead, behind, diverged and missing repositories');

    // Fetch all: one new commit waits on docs-site's origin; mobile-app's origin is no repository.
    const docsTip = pushElsewhere('docs-site', 1);
    await click('manager-fetch-all');
    await until(s => s.repository_manager.status.result?.kind === 'fetch' && !s.repository_manager.running && checked(s), 'Fetch all result');
    const outcome = (name, r = status().result) => r.outcomes.find(o => o.id === repo(name).id);
    assert.deepEqual([outcome('mobile-app').outcome, outcome('legacy-portal').outcome, outcome('release-scripts').outcome], ['failed', 'skipped', 'skipped']);
    // Outside MCP Git says the folder is no repository; under MCP its transport scope refuses
    // that destination first. Either way the repository fails and says why.
    const refused = /No repository at that address|outside the approved MCP fixture scope/;
    assert.match(outcome('mobile-app').detail, /^origin: /);
    assert.match(outcome('mobile-app').detail, refused);
    report.fetch_failure = outcome('mobile-app').detail;
    assert.equal(outcome('legacy-portal').detail, 'Folder missing');
    assert.equal(outcome('release-scripts').detail, 'No remotes');
    assert.deepEqual([outcome('docs-site').outcome, outcome('docs-site').detail], ['fetched', 'Updated']);
    for (const name of ['acme-web', 'acme-api', 'design-system', 'storefront']) assert.equal(outcome(name).outcome, 'fetched', name);
    assert.equal(row('docs-site').behind, 5); assert.equal(row('mobile-app').row, 'Fetch failed'); assert(row('mobile-app').fetch_error); assert.equal(row('mobile-app').attention, true);
    for (const [name, dir] of Object.entries(dirs)) if (fs.existsSync(dir)) assert.equal(gitAt(dir, ['rev-parse', 'HEAD']), heads[name], name + ' HEAD moved during Fetch all');
    assert.equal(gitAt(dirs['docs-site'], ['rev-parse', 'refs/remotes/origin/' + branch]), docsTip);
    await capture('fetch-result');
    passed('Fetch all fetches every listed repository, lists the failed origin and skips the missing folder and the one without remotes; no branch moves');

    // Pull all: only docs-site is clean and behind. Nothing else is touched.
    await click('manager-pull-all');
    await until(s => s.repository_manager.status.result?.kind === 'pull' && !s.repository_manager.running && checked(s), 'Pull all result');
    const pulled = status().result;
    const summary = Object.fromEntries(Object.keys(dirs).map(n => [n, [outcome(n, pulled).outcome, outcome(n, pulled).detail]]));
    assert.equal(summary['mobile-app'][0], 'failed');
    assert.match(summary['mobile-app'][1], refused);
    report.pull_failure = summary['mobile-app'][1];
    delete summary['mobile-app'];
    assert.deepEqual(summary, {
      'acme-web': ['up_to_date', 'Up to date'],
      'acme-api': ['skipped', '2 changes uncommitted'],
      'design-system': ['up_to_date', 'Up to date'],
      'docs-site': ['pulled', `5 commits · now at ${docsTip.slice(0, 7)}`],
      'storefront': ['skipped', 'Diverged'],
      'release-scripts': ['skipped', 'No upstream'],
      'legacy-portal': ['skipped', 'Folder missing'],
    });
    assert.equal(gitAt(dirs['docs-site'], ['rev-parse', 'HEAD']), docsTip);
    assert.equal(gitAt(dirs['docs-site'], ['status', '--porcelain']), '');
    for (const name of ['acme-web', 'acme-api', 'design-system', 'storefront', 'release-scripts', 'mobile-app']) assert.equal(gitAt(dirs[name], ['rev-parse', 'HEAD']), heads[name], name + ' HEAD moved during Pull all');
    assert.deepEqual(['README.md', 'notes-1.md', 'draft.md'].map(f => hash(path.join(dirs['acme-api'], f))), apiFiles);
    assert.equal(row('docs-site').row, 'Up to date');
    await capture('pull-result');
    passed('Pull all fast-forwards only the clean branch behind its upstream, skips changes, divergence, no upstream and the missing folder, fails the bad origin, and leaves every other repository as it was');

    // Open 4 in tabs: the failed and skipped ones whose folders exist.
    await click('manager-result-open');
    await until(s => !s.repository_manager.running && !s.repository_manager.opening, 'Opened');
    const open = state.tabs.map(t => path.basename(t.path)).sort();
    assert.deepEqual(open, ['acme-api', 'acme-web', 'mobile-app', 'release-scripts', 'storefront']);
    passed('Open 4 in tabs opens the failed and skipped repositories whose folders exist');

    await click('manager-workspace-' + manager().workspaces.find(w => w.name === 'Acme').id);
    await capture('workspace-status');
    await click('manager-all');
    await resize(780, 620);
    for (const id of ['manager-fetch-all', 'manager-pull-all', 'manager-check', 'manager-chip-attention', 'manager-chip-all']) inside(id);
    await capture('status-narrow');
    await resize(1480, 920);
    passed('At 780 × 620 the strip keeps its chips and actions in the window');

    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(f => /^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f => fs.readFileSync(path.join(root, n, f), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0));
    const pending = new Map();
    for (const r of records) {
      if (!/^(request|step|git)_(start|end)$/.test(r.event)) continue;
      const key = r.session + ':' + r.event.split('_')[0] + ':' + r.id;
      if (r.event.endsWith('_start')) { assert(!pending.has(key), 'Duplicate span ' + key); pending.set(key, r.operation); } else { assert.equal(pending.get(key), r.operation, 'Unpaired span ' + key); pending.delete(key); }
    }
    assert.equal(pending.size, 0, 'Unclosed diagnostic spans');
    for (const operation of ['repository-summary', 'fetch-all', 'pull-all']) assert(records.some(r => r.operation === operation && r.event === 'request_end'), 'No ' + operation + ' request');
    const payload = JSON.stringify(records);
    for (const secret of ['acme-api', 'design-system', 'docs-site', 'storefront', 'mobile-app', 'Client work', 'not-a-repo']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    report.log_records = records.length;
    passed('Paired repository-summary, fetch-all and pull-all diagnostics omit names and paths');
    report.passed = true;
  } catch (error) {
    report.failure = error.stack; if (client) try { await capture('failure'); } catch {}
    throw error;
  } finally {
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2));
    console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
run().catch(e => { console.error(e); process.exitCode = 1; });
