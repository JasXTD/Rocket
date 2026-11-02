// Native Windows acceptance for 0.32.0 through authenticated localhost MCP: keeping
// one whole version of binary and delete/modify conflicts during merge,
// cherry-pick, revert and rebase, with refusals, cancellation and recovery copies.
// Synthetic, owned repositories only. Use --only=merge|cancel|cherry-pick|revert|rebase|unsupported.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const { gitRelay } = require('./git-relay.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root = path.resolve('work/mcp-fixtures/binary-conflicts-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/binary-conflicts';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const only = process.argv.find(a => a.startsWith('--only='))?.slice(7);
const report = { passed: false, version, root, only: only ?? null, cases: [] }, fixtures = [];
const BIN = 'assets/图像 🚀.bin', DOC = 'docs/说明 é.txt', GONE = 'gone 中文.txt';
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
// Explicit local consent: this runner creates only synthetic shell/editor data.
fs.writeFileSync(path.join(root, 'rocket-private-state'), 'synthetic-content-only-v1\n');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const relay = gitRelay(root, env, 'tools/helpers/file-history-preview/git-relay.rs'), { gate, marker } = relay;
const bin = tag => Buffer.concat([Buffer.from([0, 1, 2, 255]), Buffer.from('PRIVATE_' + tag)]);
const hex = name => Buffer.from(name).toString('hex');

function fixture(name) {
  const repo = path.join(root, name), hooks = path.join(root, name + '-hooks'); fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  const rm = file => fs.unlinkSync(path.join(repo, file));
  const read = file => fs.existsSync(path.join(repo, file)) ? fs.readFileSync(path.join(repo, file)) : null;
  const commit = message => { git(['add', '-A']); git(['commit', '-m', message]); return git(['rev-parse', 'HEAD']); };
  const show = revision => cp.execFileSync('git', ['-C', repo, 'show', revision], { env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Conflict Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  write(BIN, bin('base')); write(DOC, 'PRIVATE_DOC base\n'); write(GONE, 'PRIVATE_GONE base\n'); commit('PRIVATE_BASELINE 中文');
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  const f = { repo, git, write, rm, read, commit, show, config: hash(path.join(repo, '.git/config')) }; fixtures.push(f); return f;
}
/// Both branches change the binary; `other` deletes DOC that ours edits and edits
/// GONE that ours deletes. Git leaves ours for the binary and the surviving sides.
function mergeFixture(name) {
  const f = fixture(name);
  f.git(['switch', '-c', 'other']); f.write(BIN, bin('theirs')); f.rm(DOC); f.write(GONE, 'PRIVATE_GONE theirs 界\n'); f.commit('PRIVATE_THEIRS');
  f.git(['switch', branch]); f.write(BIN, bin('ours')); f.write(DOC, 'PRIVATE_DOC ours 🚀\n'); f.rm(GONE); f.ours = f.commit('PRIVATE_OURS');
  let refused = false; try { f.git(['merge', 'other']); } catch { refused = true; } assert(refused, 'merge did not conflict');
  return f;
}
const snapshot = f => ({ head: f.git(['rev-parse', 'HEAD']), index: hash(path.join(f.repo, '.git/index')), unmerged: f.git(['ls-files', '-u']), files: [BIN, DOC, GONE].map(n => f.read(n)?.toString('hex') ?? null), config: hash(path.join(f.repo, '.git/config')) });
const recoveries = f => { const dir = path.join(f.repo, '.git/rocket-conflict-recovery'); return fs.existsSync(dir) ? fs.readdirSync(dir).map(d => path.join(dir, d)) : []; };

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
report.wait_ms = WAIT_MS;
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog, write: state.write, notice: state.notice, editor: state.tabs?.find(t => t.id === state.active_tab)?.conflict_editor, activity: state.activities?.at(-1) }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running || t.conflict_editor.running)); }
function active() { return state.tabs.find(t => t.id === state.active_tab); }
function editor() { return active().conflict_editor; }
function control(id) { return state.controls.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function scroll(target, direction = 'down', pixels = 600) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); await stable(); }
async function menuItem(id) { await click('menu-repository'); await scroll('repository-menu-scroll', 'up', 2000); if (SUBMENU[id]) { await click(SUBMENU[id]); await stable(); } for (let i = 0; i < 8; i++) { const c = control(id); if (c && c.rect[3] - c.rect[1] >= 28) break; await scroll('repository-menu-scroll', 'down', 250); } await click(id); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function open(f) { if (state?.dialog) await click('dialog-cancel'); await client.tool('rocket_open', { path: f.repo }); await settle(); relay.check(state); await resize(1480, 920); await closeConsole(); await click('view-changes'); }
async function submit() { await click('dialog-submit'); await settle(); }
function latest() { return state.activities.at(-1); }
async function closeWindow() { if (state?.dialog && !state.write) await click('dialog-cancel'); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
// Open the conflicted path from Changes and wait for both reviews to settle.
async function openConflict(name) {
  await closeConsole(); await click('view-changes'); await click('change-unstaged-' + hex(name)); await click('conflict-open');
  await until(s => { const e = s.tabs.find(t => t.id === s.active_tab).conflict_editor; return !e.running && e.path === name; }, 'Conflict review ' + name); await stable();
}
async function reload() { await closeConsole(); await click('conflict-reload'); await submit(); await until(s => !s.tabs.find(t => t.id === s.active_tab).conflict_editor.running, 'Reload'); await stable(); }
async function keep(side) { await closeConsole(); await click('conflict-keep-' + side); assert.equal(state.dialog.kind, 'conflict-keep'); }
// Pause one owned Git command at its output, cancel through the UI, and prove the
// relay and real Git processes are gone. Git itself may already have finished.
// Latency runs from the cancel click to the end of the write; the status refresh
// that follows is excluded, since a loaded machine can make it slow.
async function cancelled(mode, cancelTarget, settleMs = 0) {
  if (fs.existsSync(marker)) fs.unlinkSync(marker); fs.writeFileSync(gate, mode);
  await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab });
  await until(s => !s.input_pending && fs.existsSync(marker), 'gated ' + mode);
  const pids = fs.readFileSync(marker, 'utf8').split('\n').map(Number); if (settleMs) await delay(settleMs);
  const started = performance.now(); await client.tool('rocket_click', { target: cancelTarget, expected_tab: state.active_tab });
  await until(s => !s.write, 'cancelled write'); const ms = performance.now() - started; fs.unlinkSync(gate); await settle();
  assert(ms < 10000, 'Slow cancellation ' + ms); await delay(200);
  for (const pid of pids) { let alive = false; try { process.kill(pid, 0); alive = true; } catch {} assert(!alive, 'Owned process survived cancellation'); }
  return Math.round(ms);
}
const choices = () => editor().whole.choices.map(c => c.keep + ':' + (c.change ?? 'none'));

// ---- Merge: binary keep theirs, modify/delete keep deletion, delete/modify keep theirs --------
async function merge() {
  const f = mergeFixture('PRIVATE_MERGE');
  await open(f); const original = snapshot(f);
  await openConflict(BIN);
  assert.equal(editor().bound, false); assert(editor().error.includes('unsupported_conflict_file'), editor().error);
  assert(editor().whole.loaded && editor().whole.binary && !editor().whole.deleted, JSON.stringify(editor().whole));
  assert.deepEqual(choices(), ['ours:none', 'theirs:replaced']); assert(editor().whole.ours.working_holds && !editor().whole.theirs.working_holds);
  assert(control('conflict-keep-ours')?.enabled && control('conflict-keep-theirs')?.enabled && !control('conflict-keep-deletion'));
  assert(!control('conflict-save').enabled && !control('conflict-stage').enabled && !control('conflict-part-0'));
  await capture('binary-wide');
  await resize(780, 620); await closeConsole();
  for (let i = 0; i < 6 && !control('conflict-keep-theirs'); i++) await scroll('conflict-whole-scroll', 'down', 300);
  assert(control('conflict-keep-theirs')?.enabled, 'Keep theirs unreachable at 780x620'); await capture('binary-narrow'); await resize(1480, 920);
  await keep('theirs'); await capture('keep-theirs-review'); await click('dialog-cancel');
  assert.deepEqual(snapshot(f), original); assert(editor().notice.includes('cancelled'), editor().notice);
  passed('Binary merge conflict: text editing refused, whole-version review lists ours (already in the tree) and theirs, wide/narrow layouts, cancelled confirmation writes nothing');

  f.write(BIN, bin('external')); await keep('theirs'); await submit();
  assert(!latest().command_succeeded); assert(latest().details.some(d => d.includes('conflict_review_stale')), JSON.stringify(latest()));
  assert(editor().error.includes('conflict_review_stale')); assert.deepEqual(f.read(BIN), bin('external'));
  assert.equal(snapshot(f).index, original.index); assert.equal(recoveries(f).length, 0); await capture('stale-refusal');
  await reload(); assert.deepEqual(choices(), ['ours:replaced', 'theirs:replaced']);
  const before = snapshot(f); await keep('theirs'); report.cancel_before_write_ms = await cancelled('ls-files', 'conflict-cancel-save');
  assert.deepEqual(snapshot(f), before); assert.equal(recoveries(f).length, 0); assert(!latest().command_succeeded);
  passed('An external edit after review is refused without writing; in-flight cancellation before publication stops owned Git processes and leaves file, index and recovery untouched');

  await reload(); await keep('theirs'); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert.deepEqual(latest().whole_conflict, { keep: 'theirs', file_change: 'replaced', staging_confirmed: true });
  assert.deepEqual(f.read(BIN), bin('theirs')); assert.equal(f.git(['ls-files', '-u', '--', BIN]), '');
  assert.equal(editor().kept, 'theirs'); assert(editor().notice.includes('Kept theirs'));
  const dirs = recoveries(f); assert.equal(dirs.length, 1);
  assert.deepEqual(fs.readFileSync(path.join(dirs[0], 'replaced-original')), bin('external')); assert.deepEqual(fs.readFileSync(path.join(dirs[0], 'result')), bin('theirs'));
  assert(latest().details.some(d => d.includes('rocket-conflict-recovery')));
  await capture('kept-theirs');
  passed('Keep theirs replaces the binary, keeps observed/replaced/result recovery copies and stages only this path');

  await openConflict(DOC);
  assert.equal(editor().bound, true); assert(editor().whole.deleted); assert.deepEqual(choices(), ['ours:none', 'deletion:removed']);
  assert(control('conflict-part-0') && control('conflict-keep-deletion')?.enabled && !control('conflict-keep-theirs')); await capture('delete-modify-text');
  await keep('deletion'); await submit();
  assert.deepEqual(latest().whole_conflict, { keep: 'deletion', file_change: 'removed', staging_confirmed: true }); assert.equal(f.read(DOC), null);
  assert.equal(f.git(['ls-files', '--stage', '--', DOC]), '');
  assert(recoveries(f).some(d => fs.existsSync(path.join(d, 'replaced-original')) && fs.readFileSync(path.join(d, 'replaced-original'), 'utf8') === 'PRIVATE_DOC ours 🚀\n'));
  await openConflict(GONE); assert.deepEqual(choices(), ['theirs:none', 'deletion:removed']);
  await keep('theirs'); await submit();
  assert.deepEqual(latest().whole_conflict, { keep: 'theirs', file_change: null, staging_confirmed: true }); assert.equal(f.read(GONE).toString(), 'PRIVATE_GONE theirs 界\n');
  assert.equal(f.git(['ls-files', '-u']), ''); assert.equal(f.git(['rev-parse', 'HEAD']), original.head); assert(fs.existsSync(path.join(f.repo, '.git/MERGE_HEAD')));
  await click('conflict-back'); await closeConsole(); await type('commit-message', 'PRIVATE_MERGE kept versions'); await click('commit');
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().operation_state.idle);
  assert.deepEqual(f.git(['rev-list', '--parents', '-1', 'HEAD']).split(' ').slice(1), [f.ours, f.git(['rev-parse', 'other'])]);
  assert.equal(f.git(['show', 'HEAD:' + GONE]), 'PRIVATE_GONE theirs 界'); assert.throws(() => f.git(['cat-file', '-e', 'HEAD:' + DOC]));
  assert.equal(f.git(['status', '--porcelain']), '');
  passed('Modify/delete keeps the deletion (moved to recovery), delete/modify keeps theirs with only an index change, and the merge commits through Rocket');
}

// ---- Cancellation after publication: the file write stands, staging is unconfirmed --------
async function cancelAfterPublication() {
  const f = mergeFixture('PRIVATE_CANCEL_AFTER');
  await open(f); await openConflict(BIN); const head = f.git(['rev-parse', 'HEAD']);
  await keep('theirs'); report.cancel_after_write_ms = await cancelled('update-index', 'conflict-cancel-save', 400);
  assert(!latest().command_succeeded); assert.deepEqual(latest().whole_conflict, { keep: 'theirs', file_change: 'replaced', staging_confirmed: false });
  assert(latest().summary.includes('staging not confirmed'), latest().summary); assert.deepEqual(f.read(BIN), bin('theirs'));
  assert.equal(recoveries(f).length, 1); assert.equal(f.git(['rev-parse', 'HEAD']), head); assert(fs.existsSync(path.join(f.repo, '.git/MERGE_HEAD')));
  assert(!fs.existsSync(path.join(f.repo, '.git/index.lock'))); report.cancel_after_index_unmerged = f.git(['ls-files', '-u', '--', BIN]);
  await capture('cancel-after-publication');
  passed('Cancellation after publication reports the completed file write, does not claim staging, and never rolls back');
}

// ---- Cherry-pick: binary keep ours (index only), then reviewed Continue --------
async function cherryPick() {
  const f = fixture('PRIVATE_CHERRY');
  f.git(['switch', '-c', 'source']); f.write(BIN, bin('picked')); f.write('added 新.txt', 'PRIVATE_ADDED\n'); const source = f.commit('PRIVATE_SOURCE');
  f.git(['switch', branch]); f.write(BIN, bin('main')); const main = f.commit('PRIVATE_MAIN');
  await open(f); await click('view-commits'); await click('history-scope'); await click('history-all'); await click('commit-' + source);
  await menuItem('commit-cherry-pick'); await until(s => s.dialog && !s.dialog.commit_preview_loading && !s.busy, 'Cherry-pick review'); await submit();
  assert(latest().details.some(d => d.includes('conflicts_remain')), JSON.stringify(latest()));
  await openConflict(BIN); assert.deepEqual(choices(), ['ours:none', 'theirs:replaced']);
  await keep('ours'); await submit();
  assert.deepEqual(latest().whole_conflict, { keep: 'ours', file_change: null, staging_confirmed: true }); assert.deepEqual(f.read(BIN), bin('main')); assert.equal(recoveries(f).length, 0);
  await click('conflict-back'); await closeConsole(); await click('operation-continue');
  await until(s => s.dialog && !s.dialog.commit_preview_loading && !s.busy, 'Continue review'); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().operation_state.idle);
  assert.equal(f.git(['rev-parse', 'HEAD^']), main); assert.equal(f.git(['show', 'HEAD:added 新.txt']), 'PRIVATE_ADDED');
  assert.equal(f.git(['rev-parse', 'HEAD:' + BIN]), f.git(['rev-parse', main + ':' + BIN]));
  passed('Cherry-pick binary conflict: keeping ours changes only the index, then reviewed Continue commits the rest of the picked change');
}

// ---- Revert: binary keep theirs and modify/delete keep deletion, then Continue --------
async function revert() {
  const f = fixture('PRIVATE_REVERT');
  f.write(BIN, bin('v1')); f.write('notes 文.txt', 'PRIVATE_NOTES\n'); const target = f.commit('PRIVATE_TARGET');
  f.write(BIN, bin('v2')); f.write('notes 文.txt', 'PRIVATE_NOTES later\n'); const later = f.commit('PRIVATE_LATER');
  await open(f); await click('view-commits'); await click('commit-' + target);
  await menuItem('commit-revert'); await until(s => s.dialog && !s.dialog.commit_preview_loading && !s.busy, 'Revert review'); await submit();
  assert(latest().details.some(d => d.includes('conflicts_remain')), JSON.stringify(latest()));
  await openConflict(BIN); assert.deepEqual(choices(), ['ours:none', 'theirs:replaced']); await keep('theirs'); await submit();
  assert.deepEqual(latest().whole_conflict, { keep: 'theirs', file_change: 'replaced', staging_confirmed: true }); assert.deepEqual(f.read(BIN), bin('base'));
  await openConflict('notes 文.txt'); assert.deepEqual(choices(), ['ours:none', 'deletion:removed']); await keep('deletion'); await submit();
  assert.equal(latest().whole_conflict.file_change, 'removed'); assert.equal(f.read('notes 文.txt'), null);
  await click('conflict-back'); await closeConsole(); await click('operation-continue');
  await until(s => s.dialog && !s.dialog.commit_preview_loading && !s.busy, 'Continue review'); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(active().operation_state.idle); assert.equal(f.git(['rev-parse', 'HEAD^']), later);
  assert.equal(f.git(['rev-parse', 'HEAD:' + BIN]), f.git(['rev-parse', f.git(['rev-list', '--max-parents=0', 'HEAD']) + ':' + BIN]));
  assert.throws(() => f.git(['cat-file', '-e', 'HEAD:notes 文.txt']));
  passed('Revert conflicts: keeping theirs restores the pre-commit binary, keeping the deletion removes a later-edited file, and reviewed Continue completes the revert');
}

// ---- Rebase: replayed binary (theirs) and deletion kept on a detached HEAD --------
async function rebase() {
  const f = fixture('PRIVATE_REBASE');
  f.git(['branch', 'main']); f.write(BIN, bin('feature')); f.rm(DOC); f.write('feature 新.txt', 'PRIVATE_FEATURE\n'); f.commit('PRIVATE_FEATURE_COMMIT');
  f.git(['switch', 'main']); f.write(BIN, bin('main')); f.write(DOC, 'PRIVATE_DOC main\n'); const main = f.commit('PRIVATE_MAIN'); f.git(['switch', branch]);
  await open(f); await menuItem('repository-rebase'); await type('dialog-rebase-target', 'main'); await click('dialog-preview-refresh');
  await until(s => s.dialog?.rebase_preview_loaded || s.dialog?.error, 'Rebase review'); await submit();
  assert(active().operation_state.rebase, JSON.stringify(latest())); assert(!latest().command_succeeded);
  await openConflict(BIN); assert(editor().whole.ours.working_holds); await keep('theirs'); await capture('rebase-keep-theirs-review'); await submit();
  assert.deepEqual(latest().whole_conflict, { keep: 'theirs', file_change: 'replaced', staging_confirmed: true }); assert.deepEqual(f.read(BIN), bin('feature'));
  await openConflict(DOC); assert.deepEqual(choices(), ['ours:none', 'deletion:removed']); await keep('deletion'); await submit();
  assert.equal(latest().whole_conflict.file_change, 'removed');
  await click('conflict-back'); await closeConsole(); await click('operation-continue');
  await until(s => s.dialog?.rebase_preview_loaded || s.dialog?.error, 'Continue review'); await submit();
  assert(latest().command_succeeded, JSON.stringify(latest())); assert(!active().operation_state.rebase);
  assert.equal(f.git(['branch', '--show-current']), branch); assert.equal(f.git(['rev-parse', 'HEAD^']), main);
  assert.equal(f.git(['show', 'HEAD:feature 新.txt']), 'PRIVATE_FEATURE'); assert.throws(() => f.git(['cat-file', '-e', 'HEAD:' + DOC]));
  assert.deepEqual(f.show('HEAD:' + BIN), bin('feature'));
  passed('Rebase conflicts on a detached HEAD: keeping the replayed binary and the deletion, then reviewed Continue finishes the owned branch');
}

// ---- Unsupported: content filters are refused before any content is read --------
async function unsupported() {
  const f = mergeFixture('PRIVATE_UNSUPPORTED'); fs.mkdirSync(path.join(f.repo, '.git/info'), { recursive: true });
  fs.writeFileSync(path.join(f.repo, '.git/info/attributes'), '*.bin filter=lfs\n');
  await open(f); const before = snapshot(f); await openConflict(BIN);
  assert.equal(editor().whole.loaded, false); assert(editor().error?.includes('unsupported_conflict_version'), JSON.stringify(editor()));
  assert(!control('conflict-keep-ours') && !control('conflict-keep-theirs')); assert.deepEqual(snapshot(f), before); await capture('unsupported-filter');
  passed('A filtered (LFS-style) binary conflict explains that Git is needed and offers no whole-version write');
}

async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe');
    report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n'); report.endpoint = 'http://127.0.0.1:' + port + '/mcp';
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env: relay.env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    const cases = { merge, cancel: cancelAfterPublication, 'cherry-pick': cherryPick, revert, rebase, unsupported };
    for (const [name, test] of Object.entries(cases)) if (!only || only === name) await test();
    for (const f of fixtures) assert.equal(hash(path.join(f.repo, '.git/config')), f.config, 'Repository config changed');
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(f => /^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f => fs.readFileSync(path.join(root, n, f), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE_', '图像', '说明', 'gone 中文', '新.txt', '文.txt']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    const all = !only, ops = ['conflict-edit', 'conflict-keep-review', 'conflict-keep'], steps = ['read_conflict_versions'], codes = [];
    if (all || only === 'merge') { ops.push('conflict-keep-cancel', 'merge-continue'); steps.push('revalidate_versions', 'publish_version', 'stage_version'); codes.push('conflict_review_stale', 'unsupported_conflict_file'); }
    if (all || only === 'cherry-pick') ops.push('cherry-pick', 'cherry-pick-continue');
    if (all || only === 'revert') ops.push('revert', 'revert-continue');
    if (all || only === 'rebase') ops.push('rebase', 'rebase-continue');
    if (all || only === 'unsupported') codes.push('unsupported_conflict_version');
    for (const op of ops) { const starts = records.filter(r => r.event === 'request_start' && r.operation === op); assert(starts.length, 'Missing ' + op); for (const start of starts) assert(records.some(r => r.event === 'request_end' && r.session === start.session && r.request === start.request), 'Unpaired ' + op); }
    for (const step of steps) assert(payload.includes('"' + step + '"'), 'Missing step ' + step);
    for (const code of codes) assert(records.some(r => r.failure_code === code), 'Missing failure ' + code);
    if (all || only === 'merge') assert(records.some(r => r.event === 'git_start' && r.operation === 'update-index'), 'update-index logged without its fixed command label');
    if (all || only === 'merge' || only === 'cancel') assert(records.some(r => r.outcome === 'cancelled'), 'Missing cancelled outcome');
    report.log_records = records.length; passed('Paired diagnostic spans, whole-version steps and fixed failure codes, cancelled outcomes, zero dropped events, no paths or contents'); report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync('work/binary-conflicts-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { if (fs.existsSync(gate)) fs.unlinkSync(gate); await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
run().catch(e => { console.error(e); process.exitCode = 1; });
