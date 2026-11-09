// Language Preferences acceptance through authenticated localhost MCP. Only an
// owned repository/profile is used; no hosting, credentials or Git writes via UI.
// node tools/tests/test-mcp-language.cjs [rocket-mcp.exe] [result-directory]
'use strict';
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/language-' + crypto.randomUUID());
const repo = path.join(root, 'repository 文'), profile = path.join(root, 'http-profile');
const results = path.resolve(process.argv[3] || path.join(root, 'results'));
const exe = path.resolve(process.argv[2] || 'target/release/rocket-mcp.exe');
const branch = 'rocket-ui-test/language';
const tags = ['en', 'de', 'de-CH', 'fr', 'sv', 'ru', 'uk', 'ar', 'zh-Hans', 'ja', 'ko'];
fs.mkdirSync(repo, { recursive: true }); fs.mkdirSync(results, { recursive: true });
fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$|^ROCKET_LANG$/i.test(key)) delete env[key];
Object.assign(env, {
  GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'),
  GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0',
  GIT_OPTIONAL_LOCKS: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'),
});
const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trimEnd();
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
git(['init', '--initial-branch=' + branch]);
for (const [key, value] of [['user.name', 'Language Fixture'], ['user.email', 'fixture@example.invalid'], ['core.autocrlf', 'false'], ['commit.gpgsign', 'false']]) git(['config', key, value]);
fs.writeFileSync(path.join(repo, '資料.rs'), 'fn main() {}\n'); git(['add', '--', '資料.rs']); git(['commit', '-m', 'Language fixture']);
fs.appendFileSync(path.join(repo, '資料.rs'), '// staged fixture\n'); git(['add', '--', '資料.rs']);
fs.appendFileSync(path.join(repo, '資料.rs'), '// unstaged fixture\n'); fs.writeFileSync(path.join(repo, '未追跡.txt'), 'keep\n');
fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
const fingerprint = () => ({ head: git(['rev-parse', 'HEAD']), index: git(['ls-files', '--stage', '-z']), config: hash(path.join(repo, '.git/config')), tracked: hash(path.join(repo, '資料.rs')), untracked: hash(path.join(repo, '未追跡.txt')) });
const before = fingerprint(), report = { passed: false, root, executable_sha256: hash(exe), cases: [], captures: [] };
let service, client, state, starts = 0;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const control = id => state?.controls?.find(c => c.id === id);
const pass = text => { report.cases.push(text); console.log('PASS ' + text); };
async function until(predicate, label) {
  const started = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return;
    assert(performance.now() - started < 45000, 'Timed out: ' + label);
    await delay(60);
  }
}
const settled = () => until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading), 'settled');
async function stable() {
  let previous = '', same = 0;
  await until(s => { const next = JSON.stringify(s.controls); same = next === previous ? same + 1 : 0; previous = next; return same >= 2 && !s.input_pending; }, 'stable controls');
}
async function click(id) {
  await stable(); assert(control(id)?.enabled, 'Missing/disabled ' + id);
  const [left, top, right, bottom] = control(id).rect;
  assert(right > left && bottom - top >= 15 && left >= 0 && top >= 0 && right <= state.viewport[0] + 1 && bottom <= state.viewport[1] + 1, 'Clipped ' + id);
  await client.tool('rocket_click', { target: id, expected_tab: state.active_tab }); await settled();
}
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending, 'resize'); await stable(); }
async function preferences() { await click('menu-edit'); await click('preferences'); await click('pref-page-general'); await stable(); }
async function capture(name) {
  await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image);
  const file = path.join(results, name + '.png'); fs.writeFileSync(file, Buffer.from(image.data, 'base64')); report.captures.push(file);
}
async function select(tag) {
  await click('pref-language'); await click('pref-language-' + tag.toLowerCase());
  await until(s => s.language.choice === tag && s.language.shown === tag, 'language ' + tag);
  await until(s => !s.state_pending, 'saved language'); assert.equal(state.state_error, null);
}
async function theme(choice, mode) {
  await click('pref-page-appearance'); await click('theme-' + choice);
  if (mode) await click('theme-clay-' + mode);
  await until(s => s.appearance.theme === choice && (!mode || s.appearance.clay_mode === mode) && !s.state_pending, 'saved theme');
  await click('pref-page-general');
}
async function start() {
  const log = fs.openSync(path.join(root, 'service-' + (++starts) + '.log'), 'w');
  service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log);
  client = new RocketHttpClient(profile);
  for (let i = 0;; i++) {
    try { assert.equal((await client.raw('GET', '/health')).status, 200); break; }
    catch (error) { if (i > 150 || service.exitCode !== null) throw error; await delay(100); }
  }
  await client.initialize(); await client.tool('rocket_open', { path: repo }); await settled(); await resize(1440, 960);
}
async function stop() {
  if (!client) return;
  if (state?.dialog && control('dialog-cancel')) await click('dialog-cancel');
  await client.tool('rocket_close');
  await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'closed or save prompt');
  if (state.dialog?.kind === 'quit') {
    await click('dialog-submit'); await until(s => ['closed', 'not_started'].includes(s.window), 'saved and closed');
  }
  assert([200, 202].includes((await client.raw('POST', '/shutdown')).status));
  await client.finish(); client = undefined;
  const started = performance.now();
  while (service.exitCode === null && performance.now() - started < 8000) await delay(100);
  assert.notEqual(service.exitCode, null, 'Owned MCP service exited'); service = undefined;
}
async function restart(tag, palette) {
  await stop(); await start();
  assert.equal(state.language.choice, tag); assert.equal(state.language.shown, tag || state.language.system);
  assert.equal(state.first_presentation.language, tag || state.language.system, 'saved language on the first native frame');
  if (palette) {
    assert.equal(state.first_presentation.appearance.palette, palette, 'saved theme on the first native frame');
    const background = { 'clay-dark': [31, 30, 29, 255], light: [248, 250, 252, 255], ocean: [10, 22, 33, 255] };
    assert.deepEqual(state.first_presentation.panel_fill, background[palette], 'actual first-frame UI background');
  }
  report.first_frames ??= []; report.first_frames.push(state.first_presentation);
  await preferences(); assert(control('pref-language')?.enabled);
}
async function main() {
  try {
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
    const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    await start(); await preferences();
    assert(control('pref-language')?.enabled, 'Language selector must be visible without ROCKET_LANG');
    assert.deepEqual(state.language.offered, tags); assert.equal(state.language.choice, null); assert.equal(state.language.shown, 'en');
    await capture('01-general-english'); await click('pref-language');
    for (const tag of tags) assert(control('pref-language-' + tag.toLowerCase())?.enabled, 'Offered ' + tag);
    assert(!control('pref-language-pseudo')); assert(!control('pref-language-system'));
    await capture('02-preview-options'); await click('pref-language-en');
    assert.equal(state.language.choice, 'en', 'choosing the detected value still saves an explicit choice');
    pass('Languages dropdown shows all eleven choices without a Match Windows entry; detected value can be overridden');
    for (const tag of tags.slice(1)) await select(tag);
    pass('Every Preview language switches immediately and saves without a settings error');
    await select('zh-Hans'); await theme('clay', 'dark'); await capture('03-chinese'); await restart('zh-Hans', 'clay-dark');
    pass('Saved Clay Dark and Simplified Chinese are present on the first native frame and survive restart');
    await resize(780, 620); await click('pref-language'); await capture('04-compact-options'); await click('pref-language-ja');
    await until(s => s.language.shown === 'ja' && !s.state_pending, 'Japanese'); await capture('05-compact-japanese');
    await select('ar'); await capture('06-compact-arabic');
    await select('de-CH'); await capture('07-compact-swiss');
    await select('ru'); await capture('08-compact-russian');
    await select('en'); await capture('09-compact-english');
    pass('Language list and selected values remain usable at 780x620, including CJK, Arabic and longer German/Russian labels');
    await theme('light'); await restart('en', 'light'); await select('fr'); await theme('ocean'); await restart('fr', 'ocean');
    await click('pref-page-git'); await capture('10-french-with-untranslated-git-settings'); await click('pref-page-general');
    await select('en'); await theme('clay', 'dark'); await restart('en', 'clay-dark');
    pass('English/French and Light/Ocean/Clay Dark use saved values from the first native frame; untranslated settings stay available');
    await select('de-CH'); await select('ko'); await select('en');
    assert.deepEqual(fingerprint(), before);
    pass('Switching preserves local HEAD, index, staged/unstaged Unicode files, untracked file and repository config');
    await stop(); report.passed = true;
  } catch (error) {
    report.error = error.stack; process.exitCode = 1;
    if (client && state?.window === 'open') try { await capture('failure'); } catch (captureError) { report.capture_error = captureError.message; }
  } finally {
    if (client) try { await stop(); } catch (error) { report.cleanup_error = error.message; process.exitCode = 1; }
    if (service?.exitCode === null) { service.kill(); await delay(300); }
    if (client) await client.finish();
    report.final_language = state?.language; report.starts = starts;
    report.repository_unchanged = JSON.stringify(fingerprint()) === JSON.stringify(before);
    assert(report.repository_unchanged, 'Fixture was modified');
    fs.writeFileSync(path.join(results, 'result.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
