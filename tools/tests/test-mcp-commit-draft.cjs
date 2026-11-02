// Native Windows acceptance of AI commit message drafts through authenticated localhost MCP, with
// a fake Claude Code built from crates/rocket-git/examples/claude_cli_fixture.rs inside the
// fixture root (never the real CLI or service). Covers: off by default, Preferences, the first-use
// consent (Cancel sends nothing), a draft that fills only the editable fields, Restore, real
// cancellation of the process tree, a typed failure, a shortened large diff, and log privacy.
// Usage: node tools/tests/test-mcp-commit-draft.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/commit-draft-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/draft';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
const DRAFT_BODY = 'PRIVATE_DRAFT_BODY explains the staged change.';
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const stubDir = path.join(root, 'claude stub 文'), stub = path.join(stubDir, 'claude.exe');
const mode = m => fs.writeFileSync(path.join(stubDir, 'mode'), m);
const requests = () => fs.readdirSync(stubDir).filter(n => /^request-\d+\.txt$/.test(n)).length;
const recorded = (n, kind) => fs.readFileSync(path.join(stubDir, `${kind}-${n}.txt`), 'utf8');

function buildStub() {
  fs.mkdirSync(stubDir);
  cp.execFileSync('rustc', ['--edition=2024', '--crate-name=claude_cli_fixture', '-C', 'opt-level=1', path.resolve('crates/rocket-git/examples/claude_cli_fixture.rs'), '-o', stub], { windowsHide: true, stdio: 'pipe' });
  report.stub_sha256 = hash(stub);
}
function fixture() {
  const repo = path.join(root, 'PRIVATE_DRAFT_REPO'), hooks = path.join(root, 'hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8' }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Draft Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  write('src/app.rs', 'fn main() {}\n'); write('notes.txt', 'base\n');
  git(['add', '-A']); git(['commit', '-m', 'PRIVATE_BASE']);
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  write('src/app.rs', 'fn main() { println!("PRIVATE_STAGED_MARK"); }\n'); git(['add', 'src/app.rs']);
  write('notes.txt', 'PRIVATE_UNSTAGED_MARK\n'); write('untracked.txt', 'PRIVATE_UNTRACKED_MARK\n');
  return { repo, git, write };
}

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
async function until(predicate, label = 'UI state', limit = WAIT_MS) {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < limit, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, draft: tab()?.composer?.draft, drafting: state.drafting, notice: state.notice }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !s.write && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
const control = id => state.controls.find(c => c.id === id);
const tab = () => state?.tabs?.find(t => t.id === state.active_tab);
const draft = () => tab().composer.draft;
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); assert(control(target)?.enabled, 'Missing/disabled input ' + target); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function refusedType(target, text, pattern) { await stable(); await assert.rejects(client.tool('rocket_type', { target, text, expected_tab: state.active_tab }), pattern); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) await click('dialog-cancel'); assert(!state.dialog, 'Dialog still open'); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
async function scroll(target, direction, pixels) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); }
/// Scroll Preferences until a control is fully visible (published controls are clipped).
async function reveal(id) { await stable(); for (let i = 0; i < 12 && !(control(id) && control(id).rect[3] - control(id).rect[1] >= 16); i++) { await scroll('dialog-scroll', 'down', 160); await stable(); } assert(control(id), 'Not visible: ' + id); }
async function preferences() { await click('menu-edit'); await click('preferences'); await until(s => s.dialog && control('pref-sidebar'), 'Preferences'); await reveal('pref-ai-enabled'); }
async function closePreferences() { await click('dialog-submit'); await until(s => !s.dialog, 'Preferences closed'); }
/// Draft and wait for the result (no first-use dialog expected).
async function drafted() { const count = requests(); await click('composer-draft'); await until(s => !draft().running && !draft().requested && !s.busy, 'Draft finished'); return requests() - count; }
/// Stage a change outside Rocket, then wait until the composer sees it (Draft is enabled again).
async function staged(f, file, body) { f.write(file, body); f.git(['add', file]); await until(s => !tab().loading && control('composer-draft')?.enabled, 'Staged change visible'); await settle(); }

async function main() {
  try {
    buildStub();
    const f = fixture();
    const before = { head: f.git(['rev-parse', 'HEAD']), index: hash(path.join(f.repo, '.git/index')) };
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920);
    if (control('console-close')) await click('console-close');

    // 1. Off by default; Preferences turn it on, with the program limited to the fixture stub.
    await click('view-changes');
    assert.equal(state.drafting.enabled, false); assert(!control('composer-draft'), 'No draft button while off');
    await preferences();
    await click('pref-ai-enabled'); assert.equal(state.drafting.enabled, true);
    await reveal('pref-ai-executable');
    await refusedType('pref-ai-executable', 'C:\\Windows\\System32\\notepad.exe', /fixture root/);
    await type('pref-ai-executable', stub);
    await reveal('pref-ai-language');
    await click('pref-ai-style-conventional'); await type('pref-ai-language', 'Deutsch');
    assert.deepEqual([state.drafting.executable, state.drafting.style, state.drafting.language], [stub, 'Conventional', 'Deutsch']);
    await capture('draft-preferences');
    await closePreferences();
    passed('Off by default with no button; Preferences turn drafting on, refuse a program outside the fixture root and keep the stub path, Conventional Commits style and the language');

    // 2. The first draft explains what is sent and asks; Cancel sends nothing.
    await click('view-changes');
    await click('composer-draft');
    await until(s => s.dialog?.kind === 'draft-notice', 'First-use explanation');
    assert.equal(requests(), 0, 'Nothing is sent before the explicit click');
    await capture('draft-notice');
    await click('dialog-cancel');
    assert.equal(state.dialog, null); assert.equal(state.drafting.notice_accepted, false); assert.equal(requests(), 0);
    passed('The first draft opens the explanation of what is sent; Cancel closes it and nothing reaches Claude Code');

    // 3. Send: the draft fills the editable fields only; the commit is the user's own click.
    await click('composer-draft');
    await until(s => s.dialog?.kind === 'draft-notice', 'First-use explanation');
    await click('dialog-submit');
    await until(s => !draft().running && !s.busy && draft().notice, 'Draft finished');
    assert.equal(state.drafting.notice_accepted, true); assert.equal(draft().failure, null); assert.equal(draft().error, false);
    assert.equal(requests(), 1); assert.deepEqual([draft().scope.files, draft().scope.binary_files, draft().scope.omitted_files], [1, 0, 0]);
    assert.equal(f.git(['rev-parse', 'HEAD']), before.head); assert.equal(hash(path.join(f.repo, '.git/index')), before.index);
    const prompt = recorded(1, 'request');
    for (const sent of ['PRIVATE_STAGED_MARK', 'src/app.rs (+1 -1)', 'Conventional Commits', 'Write the message in Deutsch.']) assert(prompt.includes(sent), 'Prompt lacks ' + sent);
    for (const kept of ['PRIVATE_UNSTAGED_MARK', 'PRIVATE_UNTRACKED_MARK', 'PRIVATE_BASE']) assert(!prompt.includes(kept), 'Prompt includes ' + kept);
    const args = recorded(1, 'arguments').split('\n');
    assert(args.some((a, i) => a === '--tools' && args[i + 1] === ''), 'No tools'); assert(args.includes('--safe-mode') && args.includes('--no-session-persistence') && args.includes('--strict-mcp-config'));
    const cwd = recorded(1, 'environment').split(/\r?\n/).find(l => l.startsWith('cwd=')).slice(4);
    assert(!path.resolve(cwd).startsWith(f.repo) && /rocket-claude-/.test(cwd), cwd);
    await capture('draft-filled');
    await type('commit-message', 'PRIVATE_EDITED summary');
    await click('commit');
    await until(s => f.git(['rev-parse', 'HEAD']) !== before.head, 'Commit');
    assert.equal(f.git(['log', '-1', '--format=%B']).trimEnd(), 'PRIVATE_EDITED summary\n\n' + DRAFT_BODY);
    passed('After the explicit Send click the draft fills the summary and description; HEAD and index stay unchanged until the user edits the summary and clicks Commit');

    // 4. Restore brings back a hand-typed message that a draft replaced.
    await staged(f, 'src/app.rs', 'fn main() { println!("PRIVATE_SECOND_MARK"); }\n');
    await type('commit-message', 'PRIVATE_HAND message');
    assert.equal(await drafted(), 1, 'Later drafts do not ask again');
    assert.equal(draft().restorable, true);
    await click('composer-draft-restore');
    assert.equal(draft().restorable, false);
    let head = f.git(['rev-parse', 'HEAD']);
    await click('commit');
    await until(s => f.git(['rev-parse', 'HEAD']) !== head, 'Commit');
    assert.equal(f.git(['log', '-1', '--format=%B']).trimEnd(), 'PRIVATE_HAND message');
    passed('A draft that replaced a hand-typed message offers Restore, which brings that message back unchanged');

    // 5. Cancel stops the Claude Code process tree; Commit is disabled while it runs.
    await staged(f, 'src/app.rs', 'fn main() { println!("PRIVATE_THIRD_MARK"); }\n');
    await type('commit-message', 'PRIVATE_KEEP message');
    mode('slow');
    // A running draft keeps Rocket busy, so this click cannot wait for it to settle.
    await stable(); assert(control('composer-draft')?.enabled);
    await client.tool('rocket_click', { target: 'composer-draft', expected_tab: state.active_tab });
    await until(s => draft().running, 'Draft running');
    for (let n = 0; !fs.existsSync(path.join(stubDir, 'heartbeat')); n++) { assert(n < 400, 'The stub did not start'); await delay(50); }
    assert.equal(control('commit').enabled, false, 'Commit is disabled while drafting');
    await capture('draft-running');
    const stop = performance.now();
    await client.tool('rocket_click', { target: 'composer-draft-cancel', expected_tab: state.active_tab });
    await until(s => !draft().running && !s.busy, 'Draft cancelled');
    report.cancel_ms = Math.round(performance.now() - stop);
    assert.equal(draft().notice, 'Draft cancelled; the message was not changed.');
    const beats = fs.statSync(path.join(stubDir, 'heartbeat')).size; await delay(700);
    assert.equal(fs.statSync(path.join(stubDir, 'heartbeat')).size, beats, 'The stub\'s child process still runs');
    passed(`Cancel stops Claude Code and its child process in ${report.cancel_ms} ms; Commit was disabled while drafting and the typed message is unchanged`);

    // 6. A failure is typed and keeps the message.
    mode('signin');
    assert.equal(await drafted(), 1);
    assert.equal(draft().failure, 'draft_sign_in_required'); assert.equal(draft().error, true);
    await capture('draft-sign-in');
    passed('A signed-out Claude Code is reported as draft_sign_in_required and the message is kept');

    // 7. A large diff is shortened at file boundaries and says so.
    mode('ok');
    f.write('big.txt', ('PRIVATE_BULK_LINE '.repeat(4) + '\n').repeat(3000)); f.git(['add', 'big.txt']);
    await staged(f, 'src/app.rs', 'fn main() { println!("PRIVATE_FOURTH_MARK"); }\n');
    const n = requests() + 1;
    assert.equal(await drafted(), 1);
    const scope = draft().scope;
    assert(scope.sent_bytes < scope.diff_bytes && scope.omitted_files === 1, JSON.stringify(scope));
    assert(/shortened/.test(draft().notice), draft().notice);
    const large = recorded(n, 'request');
    assert(large.includes('big.txt (+3000 -0)') && !large.includes('PRIVATE_BULK_LINE') && large.includes('PRIVATE_FOURTH_MARK'));
    await capture('draft-shortened');
    passed(`A ${scope.diff_bytes}-byte staged diff is shortened to the ${scope.sent_bytes} bytes of the whole files that fit; the omitted file is named only and the notice says so`);

    // 8. Turning it off removes the button again.
    await preferences(); await click('pref-ai-enabled'); await closePreferences(); await click('view-changes');
    assert.equal(state.drafting.enabled, false); assert(!control('composer-draft'));
    passed('Turning drafting off in Preferences removes the button');

    // 9. Logs: fixed labels, sizes and codes only.
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'Deutsch', 'Conventional', 'imperative', 'claude stub', 'diff --git']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    const starts = records.filter(r => r.event === 'request_start' && r.operation === 'commit-draft');
    assert.equal(starts.length, 5, 'One request per draft');
    for (const s of starts) assert(records.some(r => r.event === 'request_end' && r.session === s.session && r.request === s.request), 'Unpaired commit-draft');
    const ends = records.filter(r => r.event === 'request_end' && r.operation === 'commit-draft');
    assert(ends.some(r => r.outcome === 'cancelled'), 'Cancelled draft logged');
    assert(ends.some(r => r.outcome === 'commit_draft_refused' && r.failure_code === 'draft_sign_in_required'), 'Typed failure logged');
    assert.equal(records.filter(r => r.event === 'git_start' && r.operation === 'claude').length, 5, 'One contained Claude Code process per draft');
    report.log_records = records.length;
    passed('Diagnostics: five paired commit-draft requests and five contained claude processes, cancellation and the typed failure by code, zero dropped events, no diff, prompt, message, language or paths');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); }
    fs.writeFileSync('work/commit-draft-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
  }
}
main().catch(() => { process.exitCode = 1; });
