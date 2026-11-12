// Native Windows check of Repository Statistics (commit activity and the activity treemap)
// through authenticated localhost MCP, with a synthetic owned repository whose commit dates
// are relative to today (no network, no writes by Rocket). Stop is checked on a read held
// by the existing Git relay gate.
// Usage: node tools/tests/test-mcp-statistics.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const { gitRelay } = require('./git-relay.cjs');
const root = path.resolve('work/mcp-fixtures/statistics-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/statistics';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const relay = gitRelay(root, env, 'tools/helpers/file-history-preview/git-relay.rs'), { gate, marker } = relay;
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const run = (file, args) => cp.execFileSync(file, args, { env, windowsHide: true, encoding: 'utf8' }).trimEnd();
const people = { alice: ['PRIVATE_Alice', 'alice@example.invalid'], bob: ['PRIVATE_Bob 文', 'bob@example.invalid'], carol: ['PRIVATE_Carol', 'carol@example.invalid'], dave: ['PRIVATE_Dave', 'dave@example.invalid'] };
const pad = n => String(n).padStart(2, '0');
// A local time `daysAgo` days before today at `hour`, in a zone `offset` minutes east of UTC.
function stamp(daysAgo, hour, offset) {
  const local = new Date(Date.now() - daysAgo * 86400000);
  const d = `${local.getUTCFullYear()}-${pad(local.getUTCMonth() + 1)}-${pad(local.getUTCDate())}`;
  const sign = offset < 0 ? '-' : '+', abs = Math.abs(offset);
  const weekday = (new Date(d + 'T00:00:00Z').getUTCDay() + 6) % 7; // Monday first
  return { text: `${d}T${pad(hour)}:00:00${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`, weekday, hour };
}

// main: c1 Alice (400 days), feature f1 Bob (65), c2 Bob (60), merge M Alice (50), c3 Alice (20),
// c4 Carol (5); side (unmerged): s1 Dave (10).
function fixture() {
  const repo = path.join(root, 'PRIVATE_STATS'), hooks = path.join(root, 'hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = (args, extra = {}) => cp.execFileSync('git', ['-C', repo, ...args], { env: { ...env, ...extra }, windowsHide: true, encoding: 'utf8' }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  const who = (person, when) => ({ GIT_AUTHOR_NAME: person[0], GIT_AUTHOR_EMAIL: person[1], GIT_COMMITTER_NAME: person[0], GIT_COMMITTER_EMAIL: person[1], GIT_AUTHOR_DATE: when.text, GIT_COMMITTER_DATE: when.text });
  const commit = (person, when, message) => { git(['add', '-A']); git(['commit', '-m', message], who(person, when)); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Stats Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  const dates = { c1: stamp(400, 9, 120), f1: stamp(65, 14, -300), c2: stamp(60, 23, -300), M: stamp(50, 10, 120), c3: stamp(20, 8, 0), c4: stamp(5, 17, 540), s1: stamp(10, 11, 60) };
  write('src/core/engine.rs', 'a'.repeat(4000)); write('src/ui/view.rs', 'a'.repeat(2000)); write('docs/guide.md', 'a'.repeat(1000)); write('README.md', 'a'.repeat(200));
  commit(people.alice, dates.c1, 'PRIVATE_c1');
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  git(['switch', '-c', 'feature']); write('src/ui/view.rs', 'b'.repeat(2000)); commit(people.bob, dates.f1, 'PRIVATE_f1');
  git(['switch', branch]); write('src/core/engine.rs', 'b'.repeat(4000)); commit(people.bob, dates.c2, 'PRIVATE_c2');
  git(['merge', '--no-ff', '-m', 'PRIVATE_merge', 'feature'], who(people.alice, dates.M));
  write('src/core/engine.rs', 'c'.repeat(4000)); write('検索/データ 🐦.txt', 'a'.repeat(500)); commit(people.alice, dates.c3, 'PRIVATE_c3');
  write('docs/guide.md', 'b'.repeat(1000)); commit(people.carol, dates.c4, 'PRIVATE_c4');
  git(['switch', '-c', 'side', 'HEAD~1']); write('side.txt', 'side'); commit(people.dave, dates.s1, 'PRIVATE_s1');
  git(['switch', branch]); git(['branch', '-D', 'feature']);
  return { repo, git, dates };
}

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
async function until(predicate, label = 'UI state', limit = 45000) {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < limit, label + ' timed out; ' + JSON.stringify(state.dialog?.statistics ?? state.notice));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
const control = id => state.controls.find(c => c.id === id);
const stats = () => state.dialog?.statistics;
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { if (target === 'view-statistics') { for (const id of ['menu-view', 'view-goto', 'goto-statistics']) await click(id); return; } await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeWindow() { await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
async function type(target, text) { await stable(); assert(control(target), 'Missing ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
function csvRecords(text) { const out = []; let record = [], field = '', quoted = false; for (let i = 0; i < text.length; i++) { const c = text[i]; if (c === '"') { if (quoted && text[i + 1] === '"') { field += '"'; i++; } else quoted = !quoted; } else if (c === ',' && !quoted) { record.push(field); field = ''; } else if (c === '\n' && !quoted) { record.push(field.replace(/\r$/, '')); out.push(record); record = []; field = ''; } else field += c; } assert(!quoted); return out; }
const activity = () => until(s => s.dialog?.statistics && !s.dialog.statistics.activity_running && (s.dialog.statistics.activity || s.dialog.statistics.activity_error), 'Activity');
const treemap = () => until(s => s.dialog?.statistics && !s.dialog.statistics.treemap_running && (s.dialog.statistics.treemap || s.dialog.statistics.treemap_error), 'Treemap');
const children = () => stats().treemap.folder.children.map(c => [c.name, c.size, c.changes]);

async function main() {
  try {
    const f = fixture(), d = f.dates;
    const head = f.git(['rev-parse', 'HEAD']), index = hash(path.join(f.repo, '.git/index'));
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    // The existing Git relay pauses a command whose arguments contain the gate's text.
    async function startService(name) {
      const log = fs.openSync(path.join(root, name), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env: relay.env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
      for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
      assert.equal((await client.initialize()).serverInfo.version, version);
    }
    await startService('service.log');
    await client.tool('rocket_open', { path: f.repo }); await settle(); relay.check(state); await resize(1480, 920);
    if (control('console-close')) await click('console-close');

    // Commit activity of the current branch, all time.
    await click('view-statistics');
    await activity();
    let a = stats().activity;
    assert.deepEqual([a.commits, a.merges, a.authors], [6, 1, 3]);
    assert.deepEqual(a.top_authors.map(x => [x.name, x.commits]), [['PRIVATE_Alice', 3], ['PRIVATE_Bob 文', 2], ['PRIVATE_Carol', 1]]);
    const weekdays = [0, 0, 0, 0, 0, 0, 0], hours = Array(24).fill(0);
    for (const k of ['c1', 'f1', 'c2', 'M', 'c3', 'c4']) { weekdays[d[k].weekday]++; hours[d[k].hour]++; }
    assert.deepEqual(a.weekdays, weekdays); assert.deepEqual(a.hours, hours);
    assert.equal(a.months.reduce((n, m) => n + m[2], 0), 6);
    await capture('activity-all');
    passed('Commit activity of the current branch: 6 commits, 1 merge, 3 authors by count; weekdays and hours in each author\'s own time zone');

    // Ranges and all branches.
    for (const [range, commits, merges] of [['30', 2, 0], ['90', 5, 1], ['365', 5, 1], ['all', 6, 1]]) {
      await click('stats-range-' + range); await activity();
      assert.deepEqual([stats().range, stats().activity.commits, stats().activity.merges], [range, commits, merges], 'range ' + range);
    }
    await click('stats-scope-all'); await activity();
    a = stats().activity;
    assert.deepEqual([a.commits, a.authors], [7, 4]);
    await click('stats-range-30'); await activity();
    assert.equal(stats().activity.commits, 3);
    await capture('activity-all-branches-30-days');
    await resize(900, 700); await capture('activity-narrow'); await resize(1480, 920);
    passed('Ranges count 2 / 5 / 5 / 6 commits for 30 days / 90 days / 1 year / all; all branches add the unmerged side commit and its author');

    // Stop: the relay holds the history read until the gate goes away.
    fs.writeFileSync(gate, '--date=raw');
    await click('stats-refresh');
    await until(s => s.dialog?.statistics?.activity_running && fs.existsSync(marker), 'Held activity read');
    const stopped = performance.now();
    await click('stats-cancel');
    await activity();
    report.stop_ms = Math.round(performance.now() - stopped);
    assert(/Stopped/.test(stats().activity_error), stats().activity_error);
    assert(report.stop_ms < 10000, 'Stop took ' + report.stop_ms);
    fs.unlinkSync(gate); fs.unlinkSync(marker);
    await capture('activity-stopped');
    await click('stats-refresh'); await activity();
    assert.equal(stats().activity_error, null); assert.equal(stats().activity.commits, 3);
    passed('Stop ends a held history read (' + report.stop_ms + ' ms) and keeps the last result; Refresh reads again');

    // The row says when the numbers were read; leaving and returning shows them again
    // without a history read: the relay would hold one, and none arrives.
    assert(control('stats-refreshed'), 'Refreshed line on the Activity row'); const readAt = stats().activity_at; assert(readAt > 0);
    fs.writeFileSync(gate, '--date=raw');
    await click('dialog-cancel'); assert.equal(state.dialog, null);
    await click('view-statistics'); await delay(1500); state = await client.tool('rocket_state');
    assert(!fs.existsSync(marker), 'Opening Insights again read the history again');
    assert.deepEqual([stats().activity_running, stats().range, stats().all_branches, stats().activity.commits, stats().activity_at], [false, '30', true, 3, readAt]);
    fs.unlinkSync(gate);
    await capture('activity-reopened');
    passed('Insights again: the numbers, choices and read time at once, without a history read');

    // Export what Activity shows: one table a CSV, every table in JSON, addresses only when ticked.
    const exports = path.join(root, 'exports 検索'); fs.mkdirSync(exports);
    await click('stats-export');
    assert.deepEqual([stats().export.kind, stats().export.rows, stats().export.name, stats().export.emails], ['activity', 3, 'rocket-activity-contributors.csv', false]);
    await type('insights-export-folder', exports); await capture('activity-export-contributors');
    const save = async name => { await click('insights-export-save'); await until(s => s.dialog?.statistics?.export?.saved === name && !s.dialog.statistics.export.running, 'Saved ' + name); return fs.readFileSync(path.join(exports, name), 'utf8'); };
    const contributors = await save('rocket-activity-contributors.csv'); assert(contributors.startsWith('\ufeff')); assert(!contributors.includes('example.invalid'), 'addresses only when ticked');
    let table = csvRecords(contributors.slice(1)); const fields = table[0], col = n => fields.indexOf(n);
    assert.deepEqual(fields.slice(-2), ['name', 'commits']);
    assert.deepEqual(table.slice(1).map(r => [r[col('name')], r[col('commits')], r[col('branches')], r[col('period')]]).sort(), [['PRIVATE_Alice', '1', 'all_branches', '30'], ['PRIVATE_Carol', '1', 'all_branches', '30'], ['PRIVATE_Dave', '1', 'all_branches', '30']]);
    const monday = w => Date.parse(w.text.slice(0, 10) + 'T00:00:00Z') / 86400000 - w.weekday;
    const mondays = ['c3', 's1', 'c4'].map(k => monday(d[k])), weeks = [];
    for (let m = Math.min(...mondays); m <= Math.max(...mondays); m += 7) weeks.push([new Date(m * 86400000).toISOString().slice(0, 10), String(mondays.filter(x => x === m).length)]);
    await click('insights-export-table-weeks'); assert.deepEqual([stats().export.name, stats().export.rows], ['rocket-activity-weeks.csv', weeks.length]);
    table = csvRecords((await save('rocket-activity-weeks.csv')).slice(1));
    assert.deepEqual(table[0].slice(-2), ['week_start', 'commits']); assert.deepEqual(table.slice(1).map(r => r.slice(-2)), weeks);
    await click('insights-export-json'); await click('insights-export-emails'); assert.deepEqual([stats().export.name, stats().export.emails], ['rocket-activity.json', true]);
    await capture('activity-export-json');
    const json = JSON.parse(await save('rocket-activity.json'));
    assert.deepEqual([json.kind, json.scope.branches, json.scope.period, json.scope.email_addresses, json.totals.commits, json.contributors.length], ['commit_activity', 'all_branches', '30', true, 3, 3]);
    assert(json.contributors.every(c => c.email.endsWith('@example.invalid')));
    assert.deepEqual(json.weeks.map(w => [w.start, String(w.commits)]), weeks);
    await click('insights-export-close'); assert.equal(stats().export, null);
    assert.deepEqual(fs.readdirSync(exports).sort(), ['rocket-activity-contributors.csv', 'rocket-activity-weeks.csv', 'rocket-activity.json']);
    passed('Activity export: contributors and weeks as CSV, every table as JSON, email addresses only when ticked, new files only');

    // The treemap: sizes at HEAD, changes in the last year (c1 is older).
    await click('stats-section-treemap');
    await treemap();
    assert.equal(stats().treemap.commit, head);
    assert.equal(stats().treemap.commits, 4);
    assert.deepEqual(children(), [['src', 6000, 3], ['docs', 1000, 1], ['検索', 500, 1], ['README.md', 200, 0]]);
    assert.deepEqual([stats().treemap.folder.files, stats().treemap.folder.size, stats().treemap.folder.changes], [5, 7700, 4]);
    assert(control('stats-refreshed'), 'Refreshed line on the Files row');
    await capture('treemap-root');
    await click('treemap-cell-0');
    assert.deepEqual(stats().treemap.zoom, ['src']);
    assert.deepEqual(children(), [['core', 4000, 2], ['ui', 2000, 1]]);
    await capture('treemap-src');
    await click('treemap-crumb-0');
    assert.deepEqual(stats().treemap.zoom, []);
    await click('treemap-period-30'); await treemap();
    assert.deepEqual(children(), [['src', 6000, 1], ['docs', 1000, 1], ['検索', 500, 1], ['README.md', 200, 0]]);
    await click('treemap-period-all'); await treemap();
    assert.deepEqual(children(), [['src', 6000, 4], ['docs', 1000, 2], ['検索', 500, 1], ['README.md', 200, 1]]);
    await resize(900, 700); await capture('treemap-narrow'); await resize(1480, 920);
    passed('Treemap of HEAD: folders sized by bytes and coloured by commits in the period (last year, 30 days, all); a folder opens and the breadcrumb returns');

    // After a restart the repository's tab and choices come back; the numbers are read again.
    await closeWindow(); await client.raw('POST', '/shutdown');
    for (let i = 0; service.exitCode === null; i++) { assert(i < 150, 'Service did not exit'); await delay(100); }
    await startService('service-restarted.log');
    await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920);
    if (control('console-close')) await click('console-close');
    await click('view-statistics');
    assert.deepEqual([stats().section, stats().period, stats().range, stats().all_branches], ['Treemap', 'all', '30', true]);
    await treemap(); assert.deepEqual(children(), [['src', 6000, 4], ['docs', 1000, 2], ['検索', 500, 1], ['README.md', 200, 1]]);
    await capture('treemap-after-restart');
    passed('After a restart the repository\'s Insights tab and choices come back from the state file, and its numbers are read again');

    await click('dialog-cancel');
    assert.equal(state.dialog, null);
    assert.equal(f.git(['rev-parse', 'HEAD']), head); assert.equal(hash(path.join(f.repo, '.git/index')), index);
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'example.invalid', '検索', 'engine.rs']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    for (const op of ['commit-activity', 'activity-treemap']) {
      const starts = records.filter(r => r.event === 'request_start' && r.operation === op);
      assert(starts.length, 'Missing ' + op);
      for (const s of starts) assert(records.some(r => r.event === 'request_end' && r.session === s.session && r.request === s.request), 'Unpaired ' + op);
    }
    assert(records.some(r => r.operation === 'commit-activity' && r.outcome === 'cancelled'), 'No cancelled activity read');
    report.log_records = records.length; passed('HEAD and index unchanged; paired commit-activity and activity-treemap requests incl. the cancelled one, zero dropped events, no names, emails or paths');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    if (fs.existsSync(gate)) fs.unlinkSync(gate);
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); }
    fs.writeFileSync('work/statistics-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
  }
}
main().catch(() => { process.exitCode = 1; });
