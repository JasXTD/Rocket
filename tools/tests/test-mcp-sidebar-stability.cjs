// Focused native regression: static Repository links stay enabled and in place
// during a real write the Git relay holds and during a real foreground refresh.
// All Git/config/profile data is disposable.
const fs = require('node:fs'), os = require('node:os'), path = require('node:path');
const cp = require('node:child_process'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const { gitRelay } = require('./git-relay.cjs');
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rocket-sidebar-mcp-'));
const profile = path.join(root, 'profile'), repo = path.join(root, 'repository 文');
const report = { root, passed: false, cases: [], samples: [] };
const keys = ['view-tags', 'view-worktrees', 'view-submodules', 'view-lfs', 'view-filehistory', 'view-statistics'];
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
fs.mkdirSync(repo); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, encoding: 'utf8', windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }).trim();
git(['init', '--initial-branch=rocket-ui-test/sidebar']);
for (const [key, value] of [['user.name', 'Rocket Fixture'], ['user.email', 'fixture@example.invalid'], ['commit.gpgsign', 'false'], ['core.hooksPath', path.join(root, 'empty-template')]]) git(['config', key, value]);
for (let i = 0; i < 4; i++) {
  fs.writeFileSync(path.join(repo, 'source 文.rs'), `version ${i}\n`);
  git(['add', '.']); git(['commit', '-m', `Synthetic ${i}`]);
}
fs.writeFileSync(path.join(repo, '.git', 'rocket-ui-fixture'), 'rocket-ui-test/sidebar');
const head = git(['rev-parse', 'HEAD']);
// MCP runs Git with hooks off, so this run's Git is the relay, which can hold `git add`.
// Its saved Git choice sits in rocket-ui-state beside the repository, in `root`.
const relay = gitRelay(root, env, 'tools/helpers/productivity/git-relay.rs');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let service, client, state;
const control = id => state.controls?.find(c => c.id === id);
const active = () => state.tabs?.find(t => t.id === state.active_tab);
async function read() { return state = await client.tool('rocket_state'); }
async function until(predicate, label, ms = 60000) {
  const deadline = performance.now() + ms;
  do { await read(); if (predicate()) return; await delay(50); } while (performance.now() < deadline);
  throw Error(`Timed out: ${label}`);
}
async function settle() { await until(() => !state.busy && !state.input_pending && !state.write, 'idle UI'); }
async function click(target) {
  assert(control(target)?.enabled, `Missing/disabled ${target}`);
  await client.tool('rocket_click', { target, expected_tab: state.active_tab });
}
function rows() {
  return keys.map(key => {
    const c = control(key); assert(c && c.enabled, `${key} disappeared or dimmed`);
    return { key, rect: c.rect };
  });
}
async function capture(name) {
  const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!result.isError);
  const img = result.content.find(c => c.type === 'image'); assert(img);
  fs.writeFileSync(path.join(root, `${name}.png`), Buffer.from(img.data, 'base64'));
}
// Stage all, held by the relay before `git add`: the frames sampled here are drawn while a
// real write runs, as for any slow write. MCP takes no clicks until it ends, so the rows
// are only read, several times, to catch a dim that lasts a few frames.
async function heldStage(width, before) {
  // Edits from outside Rocket: its watcher lists them (Refresh only with that off), and the
  // next click waits for that read, which MCP would otherwise refuse while it runs.
  const auto = state.preferences?.auto_refresh;
  const edit = async (change, listed, label) => {
    change();
    if (!auto) await click('refresh');
    await until(() => listed() && !state.busy && !state.input_pending && !state.write, label);
  };
  await click('view-changes'); await settle();
  await edit(() => fs.writeFileSync(path.join(repo, 'source 文.rs'), `held at ${width}\n`),
    () => control('stage-all')?.enabled, 'Changes lists the edit');
  for (const file of [relay.marker, relay.marker + '.child']) fs.rmSync(file, { force: true });
  fs.writeFileSync(relay.gate, 'before:add');
  await click('stage-all');
  for (const deadline = performance.now() + 10000; !fs.existsSync(relay.marker) && performance.now() < deadline;) await delay(50);
  assert(fs.existsSync(relay.marker), 'The relay did not hold git add');
  const samples = [];
  for (let i = 0; i < 10; i++) {
    await delay(50); await read();
    assert(state.write, 'Stage all ended while the relay held it');
    samples.push(rows());
  }
  for (const sample of samples) assert.deepEqual(sample, before);
  report.samples.push({ width, held_write: true, rows: samples.at(-1) });
  await capture(`writing-${width}`);
  fs.rmSync(relay.gate, { force: true });
  await settle(); assert.deepEqual(rows(), before);
  assert.notEqual(git(['diff', '--cached', '--name-only']), '', 'Stage all did not land');
  await edit(() => git(['reset', '-q', '--hard']),
    () => !control('stage-all')?.enabled && !control('unstage-all')?.enabled, 'Changes is clean again');
  await click('view-commits'); await settle(); assert.deepEqual(rows(), before);
}
async function close() {
  await client.tool('rocket_close');
  await until(() => ['closed', 'not_started'].includes(state.window) || state.dialog?.kind === 'quit', 'close');
  if (state.dialog?.kind === 'quit') { await click('dialog-submit'); await until(() => state.window === 'closed', 'closed window'); }
}
async function run() {
  try {
    const exe = path.resolve(process.env.ROCKET_MCP_EXE || 'target/debug/rocket-mcp.exe');
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true, stdio: 'pipe' });
    const port = await new Promise(resolve => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => resolve(p)); }); });
    fs.writeFileSync(path.join(profile, 'settings'), `ROCKET-MCP-1\n${port}\n${root}\n`);
    const log = fs.openSync(path.join(root, 'service.log'), 'a');
    service = cp.spawn(exe, ['--http-profile', profile], { env: relay.env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log);
    client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i >= 100) throw e; await delay(100); } }
    await client.initialize(); await client.tool('rocket_open', { path: repo }); await settle(); relay.check(state);
    for (const width of [1480, 780]) {
      await client.tool('rocket_resize', { width, height: 1000 });
      await until(() => state.viewport[0] === width && state.viewport[1] === 1000 && !state.input_pending, 'resized viewport');
      await settle();
      // Show the same labeled sidebar in the compact window.
      if (width === 780 && control('view-tags').rect[2] - control('view-tags').rect[0] < 100) { await click('sidebar-labels'); await settle(); }
      for (const [i, target] of ['collapse-branches', 'collapse-remotes', 'collapse-stashes'].entries()) {
        if (active().worktree_view.sections[i] && control(target)) { await click(target); await settle(); }
      }
      if (!active().worktree_view.sections[3]) { await click('collapse-repository'); await settle(); }
      await delay(100); await read(); const before = rows(); await capture(`before-${width}`);
      await heldStage(width, before);
      report.cases.push(`${width}px: all six links stayed enabled/in place in 10 samples while the relay held Stage all's git add`);
      await click('refresh');
      await until(() => active()?.loading && !state.input_pending, 'foreground refresh observed', 10000);
      const during = rows(); assert.deepEqual(during, before);
      report.samples.push({ width, foreground_loading: active().loading, rows: during });
      await capture(`loading-${width}`); await settle(); assert.deepEqual(rows(), before);
      // The navigation must actually work, not just look enabled.
      await click('view-tags'); await settle(); assert.equal(state.dialog?.kind, 'tags');
      await click('view-commits'); await settle(); assert.deepEqual(rows(), before);
      report.cases.push(`${width}px: all six links stayed enabled/in place during foreground refresh, and Tags navigation worked`);
    }
    assert.equal(git(['rev-parse', 'HEAD']), head); assert.equal(git(['status', '--porcelain']), '');
    report.passed = true;
  } catch (error) {
    report.error = error.stack;
    if (client && state?.window === 'running') try { await capture('failure'); } catch {}
    throw error;
  } finally {
    // A failure mid-hold must not leave the write waiting forever, and Exit waits for it.
    fs.rmSync(relay.gate, { force: true });
    if (client && service?.exitCode === null) {
      try { await until(() => !state?.write, 'the released write', 15000); } catch {}
      try { await close(); await client.raw('POST', '/shutdown'); } catch (error) { report.cleanup_error = error.message; }
      for (let i = 0; service.exitCode === null && i < 100; i++) await delay(50);
      if (service.exitCode === null) { service.kill(); report.forced_service_stop = true; }
    }
    if (client) await client.finish();
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  }
}
run().catch(error => { console.error(error.message); process.exitCode = 1; });
