// Real native widgets through authenticated localhost MCP; synthetic local data only.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const root = path.resolve('work/mcp-fixtures/unicode-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), repo = path.join(root, 'Rocket 中文 日本語 한국어 🚀');
// Keep the MCP's reserved ASCII write branch; test Unicode on a secondary ref.
const branch = 'rocket-ui-test/unicode', unicodeBranch = '中文-日本語-한국어';
const author = '张伟 山田太郎 𠮷田 김민수 Élodie 🚀', message = '修复字符 日本語 한국어 🚀\n\n繁體漢字・カタカナ・한글・café・cafe\u0301';
const names = ['中文/文档.txt', '日本語/カタカナ.txt', '한국어/한글.txt', 'café.txt', 'cafe\u0301.txt', 'emoji-🚀-𠮷.txt'];
const content = '中文・繁體漢字・ひらがな・カタカナ・𠮷田・한국어・Élodie・Анна・🚀';
const report = { passed: false, version, root, cases: [], timings_ms: {} };
let client, service, state;
fs.mkdirSync(repo, { recursive: true }); fs.mkdirSync(path.join(root, 'hooks'));
fs.writeFileSync(path.join(root, 'global.config'), '[user]\n name = Synthetic Baseline\n email = baseline@example.invalid\n');
fs.writeFileSync(path.join(root, 'system.config'), '');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'global.config'), GIT_CONFIG_SYSTEM: path.join(root, 'system.config'),
  GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0',
  GIT_CONFIG_COUNT: '1', GIT_CONFIG_KEY_0: 'commit.gpgsign', GIT_CONFIG_VALUE_0: 'false' });
function git(args, options = {}) {
  return cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8', ...options });
}
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const globalHash = hash(path.join(root, 'global.config')), systemHash = hash(path.join(root, 'system.config'));
git(['init', '--template=', '--initial-branch=' + branch]);
git(['config', 'commit.gpgsign', 'false']); git(['config', 'core.autocrlf', 'false']);
git(['config', 'core.hooksPath', path.join(root, 'hooks')]);
for (const [index, name] of names.entries()) {
  const file = path.join(repo, name); fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `baseline ${index}\n${content}\n`);
}
git(['add', '--all']); git(['commit', '-m', 'Synthetic Unicode baseline']);
git(['branch', unicodeBranch]);
fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
for (const [index, name] of names.entries()) fs.appendFileSync(path.join(repo, name), `changed ${index} ${content}\n`);
const headBefore = git(['rev-parse', 'HEAD']).trim(), indexBefore = hash(path.join(repo, '.git/index'));
const hashes = names.map(name => hash(path.join(repo, name)));
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const current = () => state.tabs.find(tab => tab.id === state.active_tab);
async function settle() {
  for (let n = 0; n < 800; n++) {
    state = await client.tool('rocket_state');
    if (!state.busy && !state.input_pending && !(state.tabs || []).some(tab => tab.loading || tab.search.running)) return state;
    await delay(40);
  }
  throw Error('Native UI did not settle');
}
async function click(target) { await client.tool('rocket_click', { target, expected_tab: state.active_tab }); return settle(); }
async function type(target, text) { await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); return settle(); }
async function resize(width, height) {
  await client.tool('rocket_resize', { width, height });
  for (let n = 0; n < 150; n++) {
    await settle(); if (state.viewport[0] === width && state.viewport[1] === height) return;
    await delay(30);
  }
  throw Error('Resize did not settle');
}
async function capture(name) {
  const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!result.isError); const pixels = result.content.find(item => item.type === 'image');
  assert(pixels); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(pixels.data, 'base64'));
}
async function closeWindow() {
  await client.tool('rocket_close');
  let confirmed = false;
  for (let n = 0; n < 200; n++) {
    state = await client.tool('rocket_state');
    if (['closed', 'not_started'].includes(state.window)) return;
    if (!confirmed && !state.input_pending && state.dialog?.kind === 'quit') {
      // Exercise the real save-before-exit dialog for the synthetic draft. Never
      // choose "exit anyway" after a persistence error.
      assert.equal(state.state_error, null);
      await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab });
      confirmed = true;
    }
    await delay(40);
  }
  throw Error('Owned test window did not close');
}
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
async function run() {
  try {
    const exe = path.resolve('target/release/rocket-mcp.exe');
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
    const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), `ROCKET-MCP-1\n${port}\n${root}\n`);
    report.endpoint = `http://127.0.0.1:${port}/mcp`;
    const log = fs.openSync(path.join(root, 'service.log'), 'w');
    service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log);
    client = new RocketHttpClient(profile);
    for (let n = 0; ; n++) {
      try { assert.equal((await client.raw('GET', '/health')).status, 200); break; }
      catch (error) { if (n === 100) throw error; await delay(100); }
    }
    assert.equal((await client.initialize()).serverInfo.version, version);
    const start = performance.now(); await client.tool('rocket_open', { path: repo }); await settle();
    report.timings_ms.open_and_repository_load = Math.round(performance.now() - start);
    assert.equal(fs.realpathSync.native(current().path), fs.realpathSync.native(repo));
    assert.equal(current().head.branch, branch); assert.equal(current().change_count, names.length);
    assert(git(['for-each-ref', '--format=%(refname)', 'refs/heads']).includes('refs/heads/' + unicodeBranch));
    await click('view-changes');
    for (const name of names) {
      const begin = performance.now(); await click('change-unstaged-' + Buffer.from(name).toString('hex'));
      report.timings_ms['diff_' + names.indexOf(name)] = Math.round(performance.now() - begin);
      assert.equal(current().patch_error, null); assert.equal(current().patch_added, 1);
    }
    await click('change-unstaged-' + Buffer.from(names[0]).toString('hex'));
    await capture('changes-wide'); await click('diff-split'); await capture('diff-split'); await click('diff-unified');
    await resize(780, 620); await capture('changes-narrow'); await resize(1480, 920);
    assert.equal(git(['rev-parse', 'HEAD']).trim(), headBefore); assert.equal(hash(path.join(repo, '.git/index')), indexBefore);
    assert.deepEqual(names.map(name => hash(path.join(repo, name))), hashes);
    passed('Unicode repository, secondary branch label and six distinct UTF-8 file paths load through real widgets with unchanged HEAD/index/files');

    const hunk = state.controls.find(control => control.enabled && control.id.startsWith('diff-hunk-'));
    assert(hunk); await click(hunk.id); await click('diff-apply-selection'); assert(state.activities.at(-1).command_succeeded);
    assert.equal(git(['diff', '--cached', '--name-only', '-z']), names[0] + '\0');
    await click('unstage-all'); assert(state.activities.at(-1).command_succeeded);
    assert.equal(git(['diff', '--cached', '--name-only', '-z']), '');
    assert.deepEqual(names.map(name => hash(path.join(repo, name))), hashes);
    passed('partial staging and unstaging use the original Unicode path/patch bytes, preserving all working files');

    await click('menu-repository'); await click('repository-settings-menu'); for (let i = 0; i < 8; i++) { const c = state.controls.find(c => c.id === 'repository-settings'); if (c && c.rect[3] - c.rect[1] >= 28) break; await settle(); await delay(100); await settle(); } await click('repository-settings');
    await type('dialog-identity-name', author); await type('dialog-identity-email', 'unicode@example.invalid');
    await capture('identity-wide'); await resize(780, 620); await capture('identity-narrow'); await resize(1480, 920);
    await click('dialog-submit'); assert(state.activities.at(-1).command_succeeded);
    assert.equal(git(['config', '--local', 'user.name']).trimEnd(), author);
    passed('native identity input round-trips Chinese, Japanese, Korean, accented Latin and emoji exactly');

    await click('view-changes'); await type('commit-message', message); await closeWindow();
    const reopened = performance.now(); await client.tool('rocket_open', { path: repo }); await settle();
    report.timings_ms.reopen_and_repository_load = Math.round(performance.now() - reopened);
    assert(current().draft_present); await click('view-changes'); await capture('draft-reopened');
    await click('stage-all'); await click('commit'); assert(state.activities.at(-1).command_succeeded);
    const commit = git(['rev-parse', 'HEAD']).trim();
    assert.equal(git(['log', '-1', '--format=%an']).trimEnd(), author);
    assert.equal(git(['log', '-1', '--format=%B']).trimEnd(), message);
    assert.deepEqual(git(['ls-files', '-z']).split('\0').filter(Boolean).sort(), [...names].sort());
    for (const name of names) assert.deepEqual(git(['show', `${commit}:${name}`], { encoding: null }), fs.readFileSync(path.join(repo, name)));
    assert.deepEqual(names.map(name => hash(path.join(repo, name))), hashes);
    passed('saved Unicode draft survives window reopening and creates the exact author/message/tree; composed and decomposed names stay distinct');

    if (state.console_visible) await click('console-close');
    await click('view-commits'); await click('commit-' + commit); await click('detail-details'); await capture('history-wide');
    await click('detail-files'); await capture('history-files');
    await type('commit-filter', '修复字符'); assert.deepEqual(current().history_ids, [commit]); assert(current().search.complete);
    await capture('unicode-search'); await type('commit-filter', '');
    await resize(780, 620); await click('detail-details'); await capture('history-narrow'); await resize(1480, 920);
    passed('Unicode history search finds the committed message; native history, author, branch, file and diff captures recorded at wide/narrow sizes');

    const records = []; let after_seq = 0;
    for (let n = 0; ; n++) {
      assert(n < 20); const page = await client.tool('rocket_logs', { after_seq, limit: 1000 });
      assert(!page.retention_gap); records.push(...page.records); if (!page.more) break; after_seq = page.next_seq;
    }
    for (const payload of [author, message, ...names]) assert(!JSON.stringify(records).includes(payload));
    assert.equal(state.dropped_events, 0); assert.equal(hash(path.join(root, 'global.config')), globalHash); assert.equal(hash(path.join(root, 'system.config')), systemHash);
    report.current_session_log_records = records.length;
    await closeWindow(); await client.raw('POST', '/shutdown');
    // Closing flushes each window's writer. Inspect both sessions, not just the
    // current-window records exposed by rocket_logs after reopening.
    const allRecords = fs.readdirSync(root).filter(name => name.startsWith('mcp-logs-')).flatMap(name =>
      fs.readFileSync(path.join(root, name, 'rocket.jsonl'), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line)));
    for (const payload of [author, message, ...names]) assert(!JSON.stringify(allRecords).includes(payload));
    assert(allRecords.every(record => record.dropped_before === 0));
    assert(allRecords.length > records.length); report.log_records = allRecords.length;
    passed('both diagnostic sessions contain valid JSON without private Unicode payloads; zero dropped events and unchanged global/system config');
    report.passed = true;
  } finally {
    fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2));
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
    if (service && service.exitCode === null) {
      try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {}
      await delay(150); if (service.exitCode === null) service.kill();
    }
    if (client) await client.finish();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
