// Native Windows acceptance of Codex as the second commit-message drafting CLI through
// authenticated localhost MCP. Fake claude.exe and codex.exe are built from
// crates/rocket-git/examples/claude_cli_fixture.rs inside the fixture root (never a real CLI
// or service). Covers: choosing Codex in Preferences with its own program setting, Claude
// Code's earlier consent not covering Codex, a Codex draft that fills only the editable
// fields from a read-only no-tools `codex exec`, real cancellation of the process tree,
// typed Codex failures, and log privacy.
// Usage: node tools/tests/test-mcp-codex-draft.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/codex-draft-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/codex-draft';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
const DRAFT_BODY = 'PRIVATE_DRAFT_BODY explains the staged change.';
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const stubs = { claude: path.join(root, 'claude stub 文', 'claude.exe'), codex: path.join(root, 'codex stub 文', 'codex.exe') };
const dir = cli => path.dirname(stubs[cli]);
const mode = (cli, m) => fs.writeFileSync(path.join(dir(cli), 'mode'), m);
const requests = cli => fs.readdirSync(dir(cli)).filter(n => /^request-\d+\.txt$/.test(n)).length;
const recorded = (cli, n, kind) => fs.readFileSync(path.join(dir(cli), `${kind}-${n}.txt`), 'utf8');

function buildStubs() {
  for (const [cli, exe] of Object.entries(stubs)) {
    fs.mkdirSync(path.dirname(exe));
    cp.execFileSync('rustc', ['--edition=2024', '--crate-name=claude_cli_fixture', '-C', 'opt-level=1', path.resolve('crates/rocket-git/examples/claude_cli_fixture.rs'), '-o', exe], { windowsHide: true, stdio: 'pipe' });
    report[cli + '_stub_sha256'] = hash(exe);
  }
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
async function closeDialog() { for (let n = 0; state?.dialog && n < 3; n++) await click(control('dialog-close')?.enabled ? 'dialog-close' : 'dialog-cancel'); assert(!state.dialog, 'Dialog still open'); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
async function scroll(target, direction, pixels) { await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await settle(); }
/// Scroll Preferences until a control is fully visible (published controls are clipped).
async function reveal(id) { await stable(); for (let i = 0; i < 12 && !(control(id) && control(id).rect[3] - control(id).rect[1] >= 16); i++) { await scroll('prefs-scroll', 'down', 160); await stable(); } assert(control(id), 'Not visible: ' + id); }
async function preferences() { await click('menu-edit'); await click('preferences'); await until(s => s.dialog && control('pref-page-commit'), 'Preferences'); await click('pref-page-commit'); await reveal('pref-ai-enabled'); }
async function closePreferences() { await click('dialog-close'); await until(s => !s.dialog, 'Preferences closed'); }
/// Choose the drafting CLI and its program in Preferences.
async function choose(cli, program) { await preferences(); await reveal('pref-ai-provider-' + cli); await click('pref-ai-provider-' + cli); await reveal('pref-ai-executable'); if (program) await type('pref-ai-executable', program); assert.equal(state.drafting.provider, cli); }
/// Draft and wait for the result (no first-use dialog expected).
async function drafted(cli) { const count = requests(cli); await click('composer-draft'); await until(s => !draft().running && !draft().requested && !s.busy, 'Draft finished'); return requests(cli) - count; }

async function main() {
  try {
    buildStubs();
    const f = fixture();
    const before = { head: f.git(['rev-parse', 'HEAD']), index: hash(path.join(f.repo, '.git/index')) };
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    report.driver_sha256 = hash(__filename);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920);
    if (control('console-close')) await click('console-close');

    // 1. Codex is a choice in Preferences with its own program, limited to the fixture stub.
    await click('view-changes');
    assert.equal(state.drafting.enabled, false); assert(!control('composer-draft'), 'No draft button while off');
    await preferences();
    await click('pref-ai-enabled'); assert.equal(state.drafting.enabled, true); assert.equal(state.drafting.provider, 'claude');
    await reveal('pref-ai-provider-codex'); await click('pref-ai-provider-codex'); assert.equal(state.drafting.provider, 'codex');
    await reveal('pref-ai-executable');
    await refusedType('pref-ai-executable', 'C:\\Windows\\System32\\notepad.exe', /fixture root/);
    await type('pref-ai-executable', stubs.codex);
    assert.deepEqual([state.drafting.codex_executable, state.drafting.executable], [stubs.codex, '']);
    await capture('codex-preferences');
    // Claude Code keeps its own program setting.
    await click('pref-ai-provider-claude'); await reveal('pref-ai-executable'); await type('pref-ai-executable', stubs.claude);
    assert.deepEqual([state.drafting.provider, state.drafting.executable, state.drafting.codex_executable], ['claude', stubs.claude, stubs.codex]);
    await closePreferences();
    passed('Preferences offer Claude Code and Codex; each keeps its own program, limited under MCP to the fixture stubs');

    // 2. Claude Code's consent does not cover Codex.
    await click('view-changes');
    assert.equal(draft().provider, 'claude');
    await click('composer-draft'); await until(s => s.dialog?.kind === 'draft-notice', 'Claude Code explanation');
    await click('dialog-submit'); await until(s => !draft().running && !s.busy && draft().notice, 'Claude Code draft');
    assert.equal(requests('claude'), 1); assert.equal(state.drafting.notice_accepted, true); assert.equal(state.drafting.codex_notice_accepted, false);
    await choose('codex'); await closePreferences(); await click('view-changes');
    assert.equal(draft().provider, 'codex');
    await click('composer-draft');
    await until(s => s.dialog?.kind === 'draft-notice', 'Codex explanation');
    assert.equal(requests('codex'), 0, 'Nothing is sent before the explicit click');
    await capture('codex-notice');
    await click('dialog-cancel');
    assert.equal(state.dialog, null); assert.equal(state.drafting.codex_notice_accepted, false); assert.equal(requests('codex'), 0);
    passed('After a Claude Code draft was accepted, the first Codex draft still opens its own explanation; Cancel sends nothing to Codex');

    // 3. Send: a read-only no-tools codex exec fills the editable fields only.
    await click('composer-draft');
    await until(s => s.dialog?.kind === 'draft-notice', 'Codex explanation');
    await click('dialog-submit');
    await until(s => !draft().running && !s.busy && draft().notice, 'Codex draft finished');
    assert.equal(state.drafting.codex_notice_accepted, true); assert.equal(draft().failure, null); assert.equal(draft().error, false);
    assert.equal(requests('codex'), 1); assert(/^Drafted by Codex from 1 staged file/.test(draft().notice), draft().notice);
    assert.equal(f.git(['rev-parse', 'HEAD']), before.head); assert.equal(hash(path.join(f.repo, '.git/index')), before.index);
    const prompt = recorded('codex', 1, 'request');
    for (const sent of ['PRIVATE_STAGED_MARK', 'src/app.rs (+1 -1)']) assert(prompt.includes(sent), 'Prompt lacks ' + sent);
    for (const kept of ['PRIVATE_UNSTAGED_MARK', 'PRIVATE_UNTRACKED_MARK', 'PRIVATE_BASE']) assert(!prompt.includes(kept), 'Prompt includes ' + kept);
    const args = recorded('codex', 1, 'arguments').split('\n');
    assert.equal(args[0], 'exec'); assert.equal(args.at(-1), '-');
    for (const flag of ['--json', '--ephemeral', '--ignore-user-config', '--ignore-rules', '--strict-config', '--skip-git-repo-check']) assert(args.includes(flag), flag);
    assert(args.some((a, i) => a === '--sandbox' && args[i + 1] === 'read-only'), 'read-only sandbox');
    for (const setting of ['approval_policy="never"', 'web_search="disabled"']) assert(args.some((a, i) => a === '-c' && args[i + 1] === setting), setting);
    for (const feature of ['shell_tool', 'code_mode_host', 'apps', 'hooks']) assert(args.some((a, i) => a === '--disable' && args[i + 1] === feature), feature);
    report.codex_arguments = args.length;
    const cwd = recorded('codex', 1, 'environment').split(/\r?\n/).find(l => l.startsWith('cwd=')).slice(4);
    assert(!path.resolve(cwd).startsWith(f.repo) && /rocket-codex-/.test(cwd) && !fs.existsSync(cwd), cwd);
    await capture('codex-filled');
    await type('commit-message', 'PRIVATE_EDITED summary');
    await click('commit');
    await until(s => f.git(['rev-parse', 'HEAD']) !== before.head, 'Commit');
    assert.equal(f.git(['log', '-1', '--format=%B']).trimEnd(), 'PRIVATE_EDITED summary\n\n' + DRAFT_BODY);
    passed('After the explicit Send click, a read-only codex exec without tools, user config, rules or session files fills the summary and description from the staged change only; HEAD and index stay unchanged until the user commits');

    // 4. Cancel stops the Codex process tree; Commit is disabled while it runs.
    f.write('src/app.rs', 'fn main() { println!("PRIVATE_SECOND_MARK"); }\n'); f.git(['add', 'src/app.rs']);
    await until(s => !tab().loading && control('composer-draft')?.enabled, 'Staged change visible'); await settle();
    await type('commit-message', 'PRIVATE_KEEP message');
    mode('codex', 'slow');
    await stable(); assert(control('composer-draft')?.enabled);
    await client.tool('rocket_click', { target: 'composer-draft', expected_tab: state.active_tab });
    await until(s => draft().running, 'Draft running');
    for (let n = 0; !fs.existsSync(path.join(dir('codex'), 'heartbeat')); n++) { assert(n < 400, 'The stub did not start'); await delay(50); }
    // Starting the task and painting its disabled controls occur on adjacent
    // frames; refresh the state after the filesystem-only heartbeat wait.
    await until(s => draft().running && !s.input_pending && control('composer-draft-cancel')?.enabled && control('commit')?.enabled === false, 'Draft controls disabled');
    assert.equal(control('commit').enabled, false, 'Commit is disabled while drafting');
    await capture('codex-running');
    const stop = performance.now();
    await client.tool('rocket_click', { target: 'composer-draft-cancel', expected_tab: state.active_tab });
    await until(s => !draft().running && !s.busy, 'Draft cancelled');
    report.cancel_ms = Math.round(performance.now() - stop);
    assert.equal(draft().notice, 'Draft cancelled; the message was not changed.');
    const beats = fs.statSync(path.join(dir('codex'), 'heartbeat')).size; await delay(700);
    assert.equal(fs.statSync(path.join(dir('codex'), 'heartbeat')).size, beats, 'The stub\'s child process still runs');
    passed(`Cancel stops Codex and its child process in ${report.cancel_ms} ms; Commit was disabled while drafting and the typed message is unchanged`);

    // 5. Codex failures are typed and keep the message.
    mode('codex', 'signin');
    assert.equal(await drafted('codex'), 1);
    assert.equal(draft().failure, 'draft_sign_in_required'); assert.equal(draft().error, true);
    await capture('codex-sign-in');
    mode('codex', 'outdated');
    assert.equal(await drafted('codex'), 1);
    assert.equal(draft().failure, 'draft_codex_outdated');
    for (const warning of ['config-warning', 'feature-warning']) {
      mode('codex', warning);
      assert.equal(await drafted('codex'), 1);
      assert.equal(draft().failure, 'draft_codex_outdated');
    }
    // The hand-typed message survives configuration warnings even after exit zero.
    const head = f.git(['rev-parse', 'HEAD']);
    await click('commit');
    await until(s => f.git(['rev-parse', 'HEAD']) !== head, 'Commit');
    assert.equal(f.git(['log', '-1', '--format=%B']).trimEnd(), 'PRIVATE_KEEP message');
    passed('Sign-in, old CLI and successful-but-unsupported configuration runs are rejected with typed failures; the hand-typed message commits unchanged');

    // 6. Logs: fixed labels, sizes and codes only.
    await closeWindow(); await client.raw('POST', '/shutdown');
    for (let n = 0; service.exitCode === null && n < 200; n++) await delay(25);
    assert.equal(service.exitCode, 0, 'Owned service must exit cleanly');
    await client.finish();
    // Profile tokens persist for normal service restarts; this profile is ours.
    fs.rmSync(path.join(profile, 'access-token'), { force: true });
    assert(!fs.existsSync(path.join(profile, 'access-token')), 'Owned profile token must be removed');
    report.cleanup = 'service exited cleanly; profile token removed';
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'stub 文', 'diff --git', 'imperative', 'You write Git commit messages']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    const starts = records.filter(r => r.event === 'request_start' && r.operation === 'commit-draft');
    assert.equal(starts.length, 7, 'One request per draft');
    for (const s of starts) assert(records.some(r => r.event === 'request_end' && r.session === s.session && r.request === s.request), 'Unpaired commit-draft');
    const ends = records.filter(r => r.event === 'request_end' && r.operation === 'commit-draft');
    assert(ends.some(r => r.outcome === 'cancelled'), 'Cancelled draft logged');
    for (const code of ['draft_sign_in_required', 'draft_codex_outdated']) assert(ends.some(r => r.outcome === 'commit_draft_refused' && r.failure_code === code), 'Typed failure logged: ' + code);
    assert.equal(records.filter(r => r.event === 'git_start' && r.operation === 'claude').length, 1, 'One contained Claude Code process');
    assert.equal(records.filter(r => r.event === 'git_start' && r.operation === 'codex').length, 6, 'One contained Codex process per Codex draft');
    report.log_records = records.length;
    passed('Diagnostics: seven paired commit-draft requests, one contained claude and six contained codex processes, cancellation and typed failures by code, zero dropped events, no diff, prompt, instructions, message or paths');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); }
    fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
  }
}
main().catch(() => { process.exitCode = 1; });
