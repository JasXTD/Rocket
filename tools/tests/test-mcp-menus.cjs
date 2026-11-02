// Native acceptance for the menus (canvas section 16) through authenticated localhost MCP:
// the Repository menu's submenus, View → Go to and Edit's text commands. One disposable
// local repository and isolated settings; the Edit cases use the Windows clipboard.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/menus-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/menus';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true });
fs.mkdirSync(path.join(root, 'empty-template'));
fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, {
  GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'),
  GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0',
  GIT_TEMPLATE_DIR: path.join(root, 'empty-template')
});
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = message => { report.cases.push(message); console.error('PASS ' + message); };
// The rows of the Repository menu that carry ids (Fetch, Pull and Push have none), each
// with the ids its submenu shows.
const SUBMENUS = [
  ['repository-commit-menu', ['commit-amend', 'commit-cherry-pick', 'commit-revert']],
  ['repository-branch-menu', ['repository-merge', 'repository-rebase', 'repository-interactive-rebase', 'repository-delete-branches', 'repository-workflows']],
  ['repository-gitflow', ['gitflow-init', 'gitflow-start-feature', 'gitflow-start-release', 'gitflow-start-hotfix', 'gitflow-start-support', 'gitflow-finish']],
  ['repository-stash-menu', []],
  ['repository-patches-menu', ['repository-patch-import', 'repository-series-import', 'repository-patch-clipboard', 'repository-patch-export']],
  ['repository-history-menu', ['repository-recovery', 'repository-file-backups', 'repository-tree', 'repository-bisect']],
  ['repository-custom-commands', ['command-edit']],
  ['repository-remotes-menu', ['repository-manage-remotes', 'repository-hosting', 'repository-github']],
  ['repository-settings-menu', ['repository-settings', 'repository-signing', 'repository-ignore', 'repository-dependencies']],
];
const GO_TO = ['summary', 'changes', 'commits', 'compare', 'review', 'files', 'branches', 'remotes', 'stashes',
  'tags', 'worktrees', 'submodules', 'lfs', 'history', 'integrations', 'statistics'].map(page => 'goto-' + page);
const TEXT = 'Fix sign-in redirect loop';
function fixture(name) {
  const repo = path.join(root, name), hooks = path.join(root, name + '-hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8' }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Fixture'], ['user.email', 'rocket@example.invalid'],
    ['core.hooksPath', hooks], ['core.autocrlf', 'false'], ['commit.gpgsign', 'false']]) git(['config', key, value]);
  write('src/sign_in.rs', 'first\n'); git(['add', '.']); git(['commit', '-m', 'PRIVATE_BASE fixture']);
  write('src/sign_in.rs', 'second\n');
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo, git };
}
let client, service, state;
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < 45000, label + ' timed out: ' + JSON.stringify({ dialog: state.dialog, notice: state.notice }));
    await delay(80);
  }
}
const active = () => state.tabs.find(t => t.id === state.active_tab);
const control = id => state.controls.find(c => c.id === id);
const visible = id => { const c = control(id); return Boolean(c && c.rect[3] - c.rect[1] >= 28 && c.rect[1] >= 0 && c.rect[3] <= state.viewport[1]); };
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading)); }
async function stable() {
  let last = '', count = 0;
  await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable control geometry');
}
async function click(target) {
  await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle();
}
async function type(target, text) {
  await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle();
}
async function resize(width, height) {
  await client.tool('rocket_resize', { width, height });
  await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle();
}
async function capture(name) {
  await stable();
  const r = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!r.isError); const image = r.content.find(c => c.type === 'image'); assert(image);
  fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64'));
}
const clipboard = () => cp.execFileSync('powershell.exe', ['-NoProfile', '-Command', 'Get-Clipboard -Raw'], { encoding: 'utf8', windowsHide: true }).replace(/\r?\n$/, '');
// The copied text reaches the clipboard a frame after the click.
async function clipboardHolds(text) {
  for (let i = 0; i < 20 && clipboard() !== text; i++) await delay(150);
  assert.equal(clipboard(), text);
}
async function closeWindow() {
  if (state?.dialog) await click('dialog-cancel');
  await client.tool('rocket_close');
  await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit' && !s.input_pending);
  if (state.dialog?.kind === 'quit') {
    await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab });
    await until(s => ['closed', 'not_started'].includes(s.window));
  }
}

async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe');
    report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer();
    await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
    const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const fd = fs.openSync(path.join(root, 'service.log'), 'w');
    service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', fd] }); fs.closeSync(fd);
    client = new RocketHttpClient(profile);
    for (let i = 0;; i++) {
      try { assert.equal((await client.raw('GET', '/health')).status, 200); break; }
      catch (e) { if (i === 100) throw e; await delay(100); }
    }
    assert.equal((await client.initialize()).serverInfo.version, version);
    const f = fixture('PRIVATE_MENUS');
    await settle();
    await client.tool('rocket_open', { path: f.repo });
    await settle(); await resize(1480, 960);
    if (control('console-close')) await click('console-close');

    await click('menu-repository'); await stable();
    const menu = control('repository-menu-scroll').rect;
    for (const [row] of SUBMENUS.concat([['repository-atomic-push']])) assert(visible(row), 'Row not fully shown: ' + row);
    assert(menu[3] - menu[1] < 600, 'Repository menu taller than its 13 rows: ' + JSON.stringify(menu));
    assert(!control('repository-merge') && !control('repository-settings'), 'Items show only in their submenu');
    await capture('repository-menu');
    passed('Repository shows its 13 rows at once, without scrolling; items wait in their submenus');

    for (const [row, items] of SUBMENUS) {
      await click(row);
      for (const id of items) assert(visible(id), row + ' does not show ' + id);
      assert(control(row) && visible(row), row + ' left the menu');
      if (row === 'repository-branch-menu') await capture('repository-branch-submenu');
    }
    passed('Each submenu opens beside the menu with every item');

    await click('repository-branch-menu'); await click('repository-merge');
    assert.equal(state.dialog?.kind, 'merge'); await click('dialog-cancel');
    await click('menu-repository'); await click('repository-settings-menu'); await click('repository-settings');
    assert.equal(state.dialog?.kind, 'identity'); await click('dialog-cancel');
    passed('Items open their dialogs from the submenus');

    await click('menu-view'); await click('view-goto'); await stable();
    for (const id of GO_TO) assert(visible(id), 'Go to does not show ' + id);
    await capture('view-goto');
    await click('goto-tags'); await until(s => s.dialog?.kind === 'tags', 'Tags page');
    await click('menu-view'); await click('view-goto'); await click('goto-commits');
    await until(s => !s.dialog && active()?.view === 'Commits', 'Commits view');
    passed('View → Go to lists the sidebar pages and opens them');

    await click('view-changes'); await type('commit-message', TEXT);
    assert(active().draft_present);
    await click('menu-edit'); await stable();
    for (const id of ['edit-undo', 'edit-redo', 'edit-paste', 'edit-select-all']) assert(control(id)?.enabled, id + ' while typing');
    for (const id of ['edit-cut', 'edit-copy']) assert(!control(id)?.enabled, id + ' needs selected text');
    await capture('edit-menu-typing');
    await click('edit-select-all');
    await click('menu-edit'); await stable(); assert(control('edit-copy')?.enabled, 'Copy after Select All');
    await click('edit-copy');
    await clipboardHolds(TEXT); assert(active().draft_present);
    await click('menu-edit'); await click('edit-cut');
    await until(s => !s.tabs.find(t => t.id === s.active_tab).draft_present, 'Cut empties the field');
    await clipboardHolds(TEXT);
    await click('menu-edit'); await click('edit-paste');
    await until(s => s.tabs.find(t => t.id === s.active_tab).draft_present && !s.dialog, 'Paste fills the field, not a patch review');
    passed('Edit Select All, Copy, Cut and Paste act on the focused field, through the Windows clipboard');

    await click('view-commits'); await click('menu-edit'); await stable();
    for (const id of ['edit-undo', 'edit-redo', 'edit-cut', 'edit-copy', 'edit-paste', 'edit-select-all']) assert(!control(id)?.enabled, id + ' without a focused field');
    await capture('edit-menu-idle'); await click('menu-edit');
    passed('Without a focused field the text commands are grey');
    report.passed = true;
  } catch (error) {
    if (client) try { await capture('failure'); } catch {}
    throw error;
  } finally {
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2));
    console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) {
      try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {}
      await delay(150); if (service.exitCode === null) service.kill();
    }
    if (client) await client.finish();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
