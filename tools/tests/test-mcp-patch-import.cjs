// Native Windows check of 0.52 patch import through authenticated localhost MCP: a patch file
// dropped on the window opens the reviewed import and applies; a dropped folder opens as a
// repository; MCP refuses a drop outside the fixture root; Repository → Apply patch from
// clipboard reviews and applies the clipboard's patch, and other clipboard text only gives a
// notice. The clipboard case puts synthetic text on the Windows clipboard and restores the
// previous text afterwards; if the clipboard holds anything but text, that case is skipped
// and reported rather than overwriting it. Synthetic owned repositories; no network.
// Usage: node tools/tests/test-mcp-patch-import.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root = path.resolve('work/mcp-fixtures/patch-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/patch';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), GIT_OPTIONAL_LOCKS: '0' });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const run = (file, args) => cp.execFileSync(file, args, { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();

function repository(name) {
  const repo = path.join(root, name), hooks = path.join(root, 'hooks-' + name);
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => run('git', ['-C', repo, ...args]);
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Patch Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  fs.writeFileSync(path.join(repo, 'notes 文.txt'), 'PRIVATE one\nPRIVATE two\nPRIVATE three\n');
  git(['add', '-A']); git(['commit', '-m', 'PRIVATE base']);
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo, git };
}
// A patch that changes line 2, made from a scratch edit and then undone.
function patchFor(r, line) {
  const file = path.join(r.repo, 'notes 文.txt');
  const original = fs.readFileSync(file, 'utf8');
  fs.writeFileSync(file, original.replace(/^.*\n(.*)\n/, m => m.split('\n')[0] + '\n' + line + '\n'));
  const patch = r.git(['-c', 'core.quotepath=false', 'diff', '--no-color']) + '\n';
  fs.writeFileSync(file, original);
  return patch;
}

const ps = script => cp.execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-STA', '-Command', 'Add-Type -AssemblyName System.Windows.Forms; ' + script], { encoding: 'utf8', windowsHide: true }).trim();
const clip = {
  formats: () => JSON.parse(ps('$d=[Windows.Forms.Clipboard]::GetDataObject(); $f=@(); if ($d) { $f=@($d.GetFormats()) }; ConvertTo-Json -Compress -InputObject @{ text=[Windows.Forms.Clipboard]::ContainsText(); formats=$f }')),
  save: file => ps(`[IO.File]::WriteAllText('${file}', [Windows.Forms.Clipboard]::GetText(), (New-Object Text.UTF8Encoding $false))`),
  set: file => ps(`[Windows.Forms.Clipboard]::SetText([IO.File]::ReadAllText('${file}', [Text.Encoding]::UTF8))`),
  clear: () => ps('[Windows.Forms.Clipboard]::Clear()'),
};

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, patch: state.dialog?.patch, notice: state.notice, error: state.dialog?.error }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
const control = id => state.controls.find(c => c.id === id);
const latest = () => state.activities.at(-1);
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function drop(paths) { await stable(); await client.tool('rocket_drop_files', { paths, expected_tab: state.active_tab }); await settle(); }
async function refused(args, pattern) { let error; try { await client.tool('rocket_drop_files', { ...args, expected_tab: state.active_tab }); } catch (e) { error = String(e); } assert(error && pattern.test(error), 'Expected refusal ' + pattern + ', got ' + error); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) await click('dialog-cancel'); assert(!state.dialog, 'Dialog still open'); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
const review = () => until(s => s.dialog?.kind === 'patch-import' && s.dialog.patch?.preview_loaded && !s.dialog.patch.loading && !s.busy, 'Patch review');
async function menuItem(id) {
  await click('menu-repository');
  if (SUBMENU[id]) { await click(SUBMENU[id]); await stable(); }
  const visible = () => { const c = control(id); return c && c.rect[3] - c.rect[1] >= 20; };
  for (let i = 0; i < 8 && !visible(); i++) { await client.tool('rocket_scroll', { target: 'repository-menu-scroll', direction: 'down', pixels: 250, expected_tab: state.active_tab }); await settle(); }
  await click(id);
}

async function main() {
  const first = repository('PRIVATE_PATCH'), second = repository('PRIVATE_OTHER');
  const notes = path.join(first.repo, 'notes 文.txt');
  const dropped = path.join(root, 'change 文.patch');
  fs.writeFileSync(dropped, patchFor(first, 'PRIVATE dropped'));
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: first.repo }); await settle(); await resize(1480, 920); await closeConsole();

    // A dropped patch file: its review, then Apply.
    const index = hash(path.join(first.repo, '.git/index'));
    await drop([dropped]);
    await review();
    assert.equal(state.dialog.patch.path, dropped);
    assert.equal(state.dialog.patch_clipboard_bytes, null);
    await capture('patch-dropped-review');
    await click('dialog-submit');
    await until(s => !s.dialog?.patch?.applying && !s.busy, 'Applied');
    assert(fs.readFileSync(notes, 'utf8').includes('PRIVATE dropped'));
    assert.equal(hash(path.join(first.repo, '.git/index')), index);
    await capture('patch-dropped-applied');
    await closeDialog();
    first.git(['checkout', '--', '.']);
    passed('A patch file dropped on the window opens its reviewed import; Apply changes only the working file');

    // Folders still open as repositories; MCP refuses paths outside the fixture.
    const tabs = state.tabs.length;
    await drop([second.repo]);
    await until(s => s.tabs.length === tabs + 1 && !s.busy, 'Second repository');
    await click('tab-' + state.tabs.find(t => t.path.includes('PRIVATE_PATCH')).id);
    await refused({ paths: [path.resolve('Cargo.toml')] }, /fixture root/);
    await refused({ paths: [path.join(root, 'missing.patch')] }, /exist|fixture/);
    passed('A dropped folder opens as a repository; drops outside the fixture root or of missing files are refused');

    // The clipboard: only if it holds text or nothing, and restored afterwards.
    // Only plain text can be put back exactly; any other format (HTML, RTF, images, files)
    // would be lost, so then the case is skipped. (The first 0.52.0 run checked only for
    // text: it restored the text of a clipboard that listed 8 formats, leaving 3.)
    const before = clip.formats();
    const plain = ['System.String', 'UnicodeText', 'Text', 'Locale', 'OEMText'];
    report.clipboard_formats_before = before.formats;
    if (before.formats.some(f => !plain.includes(f))) {
      report.clipboard_skipped = 'The clipboard held more than plain text; it was left untouched';
      console.error('SKIP clipboard case: ' + report.clipboard_skipped);
    } else {
      const saved = path.join(root, 'clipboard-saved.txt'), text = path.join(root, 'clipboard-patch.txt');
      if (before.text) clip.save(saved);
      try {
        fs.writeFileSync(text, patchFor(first, 'PRIVATE pasted'));
        clip.set(text);
        await menuItem('repository-patch-clipboard');
        await review();
        assert(state.dialog.patch_clipboard_bytes > 0);
        assert.equal(state.dialog.patch.path, '');
        await capture('patch-clipboard-review');
        await click('dialog-submit');
        await until(s => !s.dialog?.patch?.applying && !s.busy, 'Applied');
        assert(fs.readFileSync(notes, 'utf8').includes('PRIVATE pasted'));
        await closeDialog();
        first.git(['checkout', '--', '.']);
        fs.writeFileSync(text, 'PRIVATE ordinary copied text\n');
        clip.set(text);
        await menuItem('repository-patch-clipboard');
        await until(s => !s.busy && /not a patch/.test(s.notice || ''), 'Not-a-patch notice');
        assert(!state.dialog);
        await capture('patch-clipboard-not-a-patch');
        passed('Repository → Apply patch from clipboard reviews and applies the clipboard patch (CRLF normalized by Windows), and ordinary text only gives a notice');
      } finally {
        if (before.text) { clip.set(saved); fs.rmSync(saved); } else clip.clear();
        report.clipboard_restored = before.text ? 'text' : 'cleared (it was empty)';
      }
    }
    await resize(780, 620); await drop([dropped]); await review(); await capture('patch-dropped-narrow'); await closeDialog(); await resize(1480, 920);
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'notes 文', 'change 文']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    for (const op of ['patch-preview', 'patch-import']) {
      const starts = records.filter(r => r.event === 'request_start' && r.operation === op);
      assert(starts.length, 'Missing ' + op);
      for (const s of starts) assert(records.some(r => r.event === 'request_end' && r.session === s.session && r.request === s.request), 'Unpaired ' + op);
    }
    report.log_records = records.length; passed('Diagnostics: paired patch requests, zero dropped events, no file names or patch text');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); }
    fs.writeFileSync('work/patch-import-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
  }
}
main().catch(() => { process.exitCode = 1; });
