// Native Windows acceptance for 0.40.0 through authenticated localhost MCP:
// partial stash (selected files), saving a stash as a patch, deleting several
// branches, deleting several local tags and publishing several tags to a local
// bare remote. Synthetic, owned repositories only; no network.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root = path.resolve('work/mcp-fixtures/stash-batch-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/stash-batch';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');

function fixture() {
  const repo = path.join(root, 'PRIVATE_BATCH'), remote = path.join(root, 'PRIVATE_REMOTE 上游.git'), hooks = path.join(root, 'hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const run = (cwd, args) => cp.execFileSync('git', ['-C', cwd, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
  const git = args => run(repo, args), server = args => run(remote, args);
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Batch Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['tag.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  for (const name of ['a.txt', 'b.txt', 'c.txt', 'd.txt', 'old 文.txt']) write(name, 'PRIVATE_BASE ' + name + '\n');
  git(['add', '-A']); git(['commit', '-m', 'PRIVATE_BASE']);
  for (const name of ['merged-a', 'merged-b', 'unmerged']) git(['branch', name]);
  git(['switch', '--quiet', 'unmerged']); write('u-branch.txt', 'PRIVATE_UNMERGED\n'); git(['add', '-A']); git(['commit', '-m', 'PRIVATE_UNMERGED']); git(['switch', '--quiet', branch]);
  git(['tag', 'v1']); git(['tag', '-a', 'v2', '-m', 'PRIVATE_NOTE 説明']); git(['tag', 'keep']); git(['tag', 'moved']); git(['tag', 'gone']);
  git(['init', '--bare', remote]); git(['remote', 'add', 'origin', remote]);
  git(['tag', 'pub/one']); git(['tag', 'pub/two-文']); git(['tag', 'pub/old']); git(['push', '--quiet', 'origin', 'refs/tags/pub/old:refs/tags/pub/old']);
  // Changes for the partial stash: two unstaged files, one staged file, a staged
  // deletion, a staged rename and an untracked file.
  write('a.txt', 'PRIVATE_A changed\n'); write('b.txt', 'PRIVATE_B changed\n'); write('c.txt', 'PRIVATE_C staged\n'); git(['add', 'c.txt']);
  git(['rm', '--quiet', 'd.txt']); git(['mv', 'old 文.txt', 'new 文.txt']); write('u.txt', 'PRIVATE_UNTRACKED\n');
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo, remote, git, server, write };
}
const lines = text => text.split('\n').filter(Boolean).sort();
const remoteRef = (f, name) => { const r = cp.spawnSync('git', ['-C', f.remote, 'rev-parse', '--verify', '--quiet', 'refs/tags/' + name], { env, windowsHide: true, encoding: 'utf8' }); return r.status === 0 ? r.stdout.trim() : null; };

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
// A hang limit, not a speed check: publishing two tags with the bundled Git takes about 45 s.
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 120000);
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, batch: state.dialog?.tag_batch, branches: state.dialog?.delete_branches, write: state.write, notice: state.notice }));
    await delay(80);
  }
}
const tab = () => state?.tabs?.find(t => t.id === state.active_tab);
const latest = () => state.activities.at(-1);
const batch = () => state.dialog?.tag_batch;
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
function control(id) { return state.controls.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function scroll(target, direction, pixels) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) await click('dialog-cancel'); assert(!state.dialog, 'Dialog still open'); }
async function open(f) { await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920); await closeConsole(); }
async function menuItem(id) { await click('menu-repository'); await scroll('repository-menu-scroll', 'up', 2000); if (SUBMENU[id]) { await click(SUBMENU[id]); await stable(); } for (let i = 0; i < 8; i++) { const c = control(id); if (c && c.rect[3] - c.rect[1] >= 28) break; await scroll('repository-menu-scroll', 'down', 250); } await click(id); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
function insideViewport(prefixes) { for (const c of state.controls.filter(c => prefixes.some(p => c.id.startsWith(p)))) assert(c.rect[0] >= 0 && c.rect[2] <= state.viewport[0] + 1 && c.rect[3] <= state.viewport[1] + 1, 'Control outside the window: ' + c.id); }
async function narrow(name, prefixes) { await resize(780, 620); await stable(); insideViewport(prefixes); await capture(name); await resize(1480, 920); }

async function stashes(f) {
  await open(f); await click('view-stashes'); await click('stash-save');
  assert.equal(state.dialog.kind, 'stash-save'); assert.equal(state.dialog.stash_selected, null);
  await click('dialog-stash-selected');
  // A staged deletion or rename can only be stashed together with every change.
  assert.equal(state.dialog.stash_blocked, true); assert.equal(state.dialog.stash_selected, 0);
  assert(!control('dialog-submit').enabled, 'A blocked partial stash cannot be saved');
  await capture('partial-stash-blocked'); await closeDialog();
  f.git(['reset', '--quiet', '--', 'd.txt', 'old 文.txt', 'new 文.txt']);
  await until(s => tab().change_count === 7, 'Unstaged deletion and rename refreshed');
  await click('stash-save'); await click('dialog-stash-selected');
  assert.equal(state.dialog.stash_blocked, false);
  const files = () => state.dialog.stash_files, at = name => { const i = files().indexOf(name); assert(i >= 0, 'Missing row ' + name); return 'dialog-stash-file-' + i; };
  assert.deepEqual([...files()].sort(), ['a.txt', 'b.txt', 'c.txt', 'd.txt', 'old 文.txt']);
  // The staged c.txt is always saved: Git records the whole index in a stash.
  assert.equal(state.dialog.stash_selected, 1); assert(!control(at('c.txt')).enabled, 'Staged file locked on');
  await click(at('a.txt')); assert.equal(state.dialog.stash_selected, 2);
  await capture('partial-stash-wide'); await narrow('partial-stash-narrow', ['dialog-stash-', 'dialog-submit']);
  await click('dialog-submit');
  await until(s => !s.write && tab().stashes?.length === 1, 'Partial stash saved');
  assert(latest().command_succeeded, JSON.stringify(latest()));
  assert.deepEqual(lines(f.git(['-c', 'core.quotepath=false', 'stash', 'show', '--name-only', 'stash@{0}'])), ['a.txt', 'c.txt']);
  assert.deepEqual(lines(f.git(['-c', 'core.quotepath=false', 'diff', '--name-only'])), ['b.txt', 'd.txt', 'old 文.txt']);
  assert.equal(f.git(['diff', '--cached', '--name-only']), '');
  assert.equal(fs.readFileSync(path.join(f.repo, 'a.txt'), 'utf8'), 'PRIVATE_BASE a.txt\n');
  passed('Partial stash: a staged deletion/rename blocks it until unstaged; staged files are always included; the stash holds exactly the chosen and staged files and the rest stays');
  await click('stash-save'); await click('dialog-stash-untracked'); await click('dialog-stash-selected');
  assert.deepEqual([...files()].sort(), ['b.txt', 'd.txt', 'new 文.txt', 'old 文.txt', 'u.txt']);
  await click(at('u.txt')); await click('dialog-submit');
  await until(s => !s.write && tab().stashes?.length === 2, 'Untracked partial stash saved');
  assert.deepEqual(lines(f.git(['show', '--name-only', '--format=', 'stash@{0}^3'])), ['u.txt']);
  assert(!fs.existsSync(path.join(f.repo, 'u.txt')) && fs.existsSync(path.join(f.repo, 'new 文.txt')));
  assert.deepEqual(lines(f.git(['-c', 'core.quotepath=false', 'diff', '--name-only'])), ['b.txt', 'd.txt', 'old 文.txt']);
  passed('With untracked files included, one chosen untracked file is saved in the stash and removed; the rest stays');
  // Save the first stash (a.txt, c.txt) as a patch file through the reviewed export.
  if (control('stash-picker')) await click('stash-picker'); await click('stash-select-1');
  const stashId = f.git(['rev-parse', 'stash@{1}']); assert.equal(tab().stash_preview.selected, stashId);
  const before = f.git(['status', '--porcelain=v2']);
  await click('stash-save-patch');
  assert.equal(state.dialog.kind, 'patch-export'); assert.equal(state.dialog.patch.revision, stashId);
  const output = path.join(root, 'PRIVATE stash 文.patch');
  await type('dialog-patch-path', output); await click('dialog-patch-review');
  assert.deepEqual(state.dialog.patch.paths, ['a.txt', 'c.txt']); await capture('stash-patch-review');
  await click('dialog-submit'); await until(s => !s.write, 'Patch written');
  const patch = fs.readFileSync(output, 'utf8');
  assert(patch.includes('+PRIVATE_A changed') && patch.includes('+PRIVATE_C staged') && !patch.includes('u.txt'), patch);
  assert.equal(f.git(['status', '--porcelain=v2']), before); assert.equal(f.git(['rev-parse', 'stash@{1}']), stashId);
  await closeDialog();
  passed('Save as patch exports the stash\'s tracked changes against the commit it was saved on; the stash list and working tree are unchanged');
}

async function branches(f) {
  await closeConsole(); await menuItem('repository-delete-branches');
  assert.equal(state.dialog.kind, 'delete-branches'); assert(!control('dialog-submit').enabled);
  assert.deepEqual(state.dialog.delete_branches.rows, ['merged-a', 'merged-b', 'unmerged']);
  await until(() => control('dialog-branches-merged')?.enabled, 'Merge check');
  // Select merged takes only the merged rows the filter leaves.
  await type('dialog-branches-filter', 'MERGED-A');
  assert.deepEqual(state.dialog.delete_branches.rows, ['merged-a']);
  await click('dialog-branches-merged'); await stable();
  assert.deepEqual(state.dialog.delete_branches.selected, ['merged-a']);
  await type('dialog-branches-filter', ''); await click('dialog-branches-merged'); await stable();
  assert.deepEqual(state.dialog.delete_branches.selected, ['merged-a', 'merged-b']);
  await click('dialog-branches-2');
  assert.deepEqual(state.dialog.delete_branches.selected, ['merged-a', 'merged-b', 'unmerged']);
  await capture('delete-branches-wide'); await narrow('delete-branches-narrow', ['dialog-branches-', 'dialog-submit']);
  await click('dialog-submit'); await until(s => !s.write && !s.dialog, 'Branches deleted');
  assert.deepEqual(lines(f.git(['for-each-ref', '--format=%(refname:short)', 'refs/heads'])), [branch, 'unmerged']);
  const a = latest(); assert(!a.command_succeeded); assert(a.summary.startsWith('Deleted 2 of 3 local branches'), a.summary);
  assert(a.details.some(d => d.startsWith('Deleted branch merged-a (was ')), JSON.stringify(a.details));
  assert(a.details.some(d => d.startsWith('Kept unmerged:') && d.includes('not fully merged')), JSON.stringify(a.details));
  await capture('delete-branches-result'); await closeConsole();
  passed('Delete branches: a filtered checklist without the current branch; Git deletes merged branches and keeps the unmerged one, reported by name');
}

async function localTags(f) {
  await closeConsole(); await click('view-tags'); assert.equal(state.dialog.kind, 'tags');
  // The page opens newest first, and these tags can share a second, so ask for name order.
  const names = ['gone', 'keep', 'moved', 'pub/old', 'pub/one', 'pub/two-文', 'v1', 'v2'];
  await click('dialog-tags-sort-name');
  await until(s => s.dialog.tags.entries?.map(e => e.name).join() === names.join(), 'Tags in name order');
  await click('dialog-tags-batch');
  assert.deepEqual(batch().rows, names);
  await type('dialog-batch-filter', 'V');
  assert.deepEqual(batch().rows, ['moved', 'v1', 'v2']);
  await click('dialog-batch-1'); await click('dialog-batch-2'); await type('dialog-batch-filter', '');
  assert.deepEqual(batch().selected, ['v1', 'v2']); assert(!control('dialog-submit').enabled, 'No review yet');
  await click('dialog-batch-review'); await until(s => batch()?.review, 'Tag review');
  assert.deepEqual(batch().review, { ready: ['v1', 'v2'], refused: [] }); assert(control('dialog-submit').enabled);
  await capture('delete-tags-review-wide'); await narrow('delete-tags-review-narrow', ['dialog-batch-', 'dialog-submit']);
  await click('dialog-submit'); await until(s => !s.write && !batch(), 'Tags deleted');
  assert(latest().command_succeeded, JSON.stringify(latest())); assert.equal(latest().summary, 'Deleted 2 local tags and verified each; remote tags unchanged.');
  assert(latest().details.every(d => d.includes('object kept at refs/')), JSON.stringify(latest().details));
  assert(!f.git(['tag', '--list']).split('\n').includes('v1'));
  passed('Delete several tags: filtered checklist, a review of exactly the selection, then each tag deleted and verified with its object kept for recovery');
  // A tag deleted elsewhere after the list was read is refused by name while reviewing.
  await click('dialog-tags-batch'); await type('dialog-batch-filter', 'gone'); await click('dialog-batch-0');
  f.git(['tag', '-d', 'gone']); await click('dialog-batch-review'); await until(s => batch()?.review, 'Refused review');
  assert.deepEqual(batch().review.ready, []); assert.equal(batch().review.refused[0].name, 'gone'); assert(!control('dialog-submit').enabled);
  await capture('delete-tags-refused'); await click('dialog-batch-back');
  // A tag moved after its review stops the batch before it changes.
  await click('dialog-preview-refresh'); await click('dialog-tags-batch'); await type('dialog-batch-filter', 'moved'); await click('dialog-batch-0'); await click('dialog-batch-review');
  await until(s => batch()?.review?.ready?.length === 1, 'Moved tag review');
  f.write('moved.txt', 'PRIVATE_MOVED\n'); f.git(['add', 'moved.txt']); f.git(['commit', '--quiet', '-m', 'PRIVATE_MOVED']); f.git(['tag', '-f', 'moved', 'HEAD']);
  const movedTo = f.git(['rev-parse', 'refs/tags/moved']);
  await click('dialog-submit'); await until(s => !s.write, 'Stopped batch');
  assert(!latest().command_succeeded); assert(latest().summary.startsWith('Deleted 0 of 1 local tags, then stopped'), latest().summary);
  assert.equal(f.git(['rev-parse', 'refs/tags/moved']), movedTo); assert(state.dialog.error, 'The batch dialog shows the result');
  await capture('delete-tags-stopped'); await click('dialog-batch-back');
  passed('Tag batches refuse a tag deleted elsewhere while reviewing, and stop without changing a tag that moved after its review');
}

async function publishTags(f) {
  await click('dialog-tags-remote'); await click('dialog-remote-tag-refresh');
  await until(s => s.dialog?.remote_tags && !s.dialog.remote_tags.loading && !s.dialog.remote_tags.stale, 'Remote snapshot');
  await click('dialog-remote-tags-batch');
  // Only local tags the remote does not have are offered (pub/old is published).
  assert(!batch().rows.includes('pub/old')); assert(batch().rows.includes('pub/one') && batch().rows.includes('pub/two-文'));
  await type('dialog-batch-filter', 'pub/'); assert.deepEqual(batch().rows, ['pub/one', 'pub/two-文']);
  await click('dialog-batch-all'); await click('dialog-batch-review'); await until(s => batch()?.review, 'Publish review');
  assert.deepEqual(batch().review, { ready: ['pub/one', 'pub/two-文'], refused: [] });
  await capture('publish-tags-review-wide'); await narrow('publish-tags-review-narrow', ['dialog-batch-', 'dialog-submit']);
  await click('dialog-submit'); await until(s => !s.write && !batch(), 'Tags published');
  assert(latest().command_succeeded, JSON.stringify(latest())); assert.equal(latest().summary, 'Published 2 tags and verified each on the remote.');
  assert.equal(remoteRef(f, 'pub/one'), f.git(['rev-parse', 'refs/tags/pub/one'])); assert.equal(remoteRef(f, 'pub/two-文'), f.git(['rev-parse', 'refs/tags/pub/two-文']));
  assert(!state.dialog.remote_tags.stale && state.dialog.remote_tags.done); await capture('publish-tags-result');
  passed('Publish several tags: only unpublished local tags are offered; the reviewed tags are pushed one at a time and each is verified on the local bare remote');
  await closeDialog();
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
    await stashes(f); await branches(f); await localTags(f); await publishTags(f);
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', '上游', '説明', 'merged-a', 'pub/one', 'a.txt', 'stash 文']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    for (const op of ['stash-push', 'patch-export', 'branch-delete-batch', 'tag-preview', 'tag-delete-batch', 'remote-tag-preview', 'remote-tag-publish-batch']) { const starts = records.filter(r => r.event === 'request_start' && r.operation === op); assert(starts.length, 'Missing ' + op); for (const start of starts) assert(records.some(r => r.event === 'request_end' && r.session === start.session && r.request === start.request), 'Unpaired ' + op); }
    report.log_records = records.length; passed('Paired stash/patch/batch diagnostics with fixed labels, zero dropped events, no names, paths or messages'); report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    fs.writeFileSync('work/stash-batch-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(150); if (service.exitCode === null) service.kill(); }
    if (client) await client.finish();
  }
}
run().catch(e => { console.error(e); process.exitCode = 1; });
