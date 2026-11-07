const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { create } = require('./mcp-github-fixture.cjs');
const { RocketMcpClient } = require('../mcp/mcp-client.cjs');
const { exe } = require('./platform.cjs');
const fixture = create('local'); fixture.run('setup');
const root = fixture.root;
const report = { passed: false, root, kind: 'Native stdio MCP operation Console regression', cases: [], shell_terminal_implemented: true };
const executable = path.join(__dirname, '..', '..', 'target', 'release', exe('rocket-mcp'));
report.executable_sha256 = require('node:crypto').createHash('sha256').update(fs.readFileSync(executable)).digest('hex');
const mcp = new RocketMcpClient(executable, root, fixture.env);
let state;
async function settle() {
  const until = Date.now() + 60000;
  do { state = await mcp.tool('rocket_wait', { timeout_ms: 10000 }); if (!state.busy && !state.input_pending) return; } while (Date.now() < until);
  throw Error('Workers did not settle');
}
// A popup's first frame is egui's invisible sizing pass, where every row is disabled:
// read the state a frame later, without waiting for an operation that blocks on purpose.
async function click(target) { await mcp.tool('rocket_click', { target, expected_tab: state.active_tab }); state = await mcp.tool('rocket_wait', { timeout_ms: 300 }); }
async function type(text) { await mcp.tool('rocket_type', { target: 'commit-message', text, expected_tab: state.active_tab }); state = await mcp.tool('rocket_state'); }
function enabled(id) { return state.controls.find(c => c.id === id)?.enabled; }
function pass(name) { report.cases.push(name); console.log('PASS ' + name); }
async function capture(name) {
  const value = await mcp.tool('rocket_capture'); const image = value.content.find(c => c.type === 'image');
  assert.equal(image.mimeType, 'image/png'); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64'));
}
function semanticState() { const value = fixture.snapshot(); delete value.index; return value; }
async function logs() { const all = []; let seq = 0; for (;;) { const b = await mcp.tool('rocket_logs', { after_seq: seq, limit: 1000 }); assert(!b.retention_gap); all.push(...b.records); seq = b.next_seq; if (!b.more) return { all, seq }; } }
(async () => {
  try {
    await mcp.initialize(); await mcp.tool('rocket_open', { path: fixture.client }); await settle();
    await click('menu-terminal');
    // The menu acts on the shell only: Clear Output (Activity) and Stop Process (the Git
    // operation) moved to the Activity drawer, and the never-enabled items are gone.
    for (const id of ['terminal-external', 'terminal-shell', 'terminal-stop', 'terminal-clear']) assert.equal(enabled(id), undefined, id);
    for (const id of ['terminal-interrupt-menu', 'terminal-clear-scrollback-menu', 'terminal-end-menu']) assert.equal(enabled(id), false, id + ' needs a session');
    assert.equal(enabled('terminal-new'), true);
    assert.equal(enabled('terminal-console'), true);
    await assert.rejects(mcp.tool('rocket_click', { target: 'terminal-end-menu', expected_tab: state.active_tab }), /disabled/);
    await capture('terminal-idle'); pass('New Terminal is enabled; the session actions wait for a session; the old items are gone');
    await click('terminal-console'); assert(state.console_visible); await capture('console-empty');
    await click('console-close'); assert.equal(state.console_visible, false); assert(state.drawer_collapsed);
    await click('toolbar-console'); assert(state.console_visible); pass('Terminal Show Activity, the drawer arrow and the toolbar toggle work');
    fs.writeFileSync(path.join(fixture.client, 'ROCKET_PRIVATE_file'), 'Staged terminal cancellation fixture\n');
    fixture.git(['add', '--', 'ROCKET_PRIVATE_file']);
    await click('refresh'); await settle(); await click('view-changes'); await settle(); await type('Terminal Stop Process cancellation draft');
    const entered = path.join(root, 'hook-entered'); const heartbeat = path.join(root, 'hook-heartbeat');
    const q = s => "'" + s.replaceAll('\\', '/').replaceAll("'", "'\"'\"'") + "'";
    fs.writeFileSync(path.join(fixture.hooks, 'pre-commit'), `#!/bin/sh\nprintf x > ${q(entered)}\nwhile :; do printf x >> ${q(heartbeat)}; sleep 0.1; done\n`, { mode: 0o755 });
    const before = semanticState(); await click('commit');
    const deadline = Date.now() + 10000; while (!fs.existsSync(entered) && Date.now() < deadline) await new Promise(r => setTimeout(r, 100)); assert(fs.existsSync(entered));
    state = await mcp.tool('rocket_state'); assert(state.busy);
    // A running operation is not the terminal's: the menu offers no way to stop it.
    await click('menu-terminal'); assert.equal(enabled('terminal-stop'), undefined); assert.equal(enabled('terminal-end-menu'), false);
    await capture('terminal-running');
    await click('terminal-console'); await click('console-close'); assert.equal(state.console_visible, false);
    await click('menu-terminal'); await click('terminal-console'); assert(state.console_visible);
    assert.equal(enabled('console-clear'), undefined, 'Clear waits for the operation');
    await click('cancel-operation'); await settle();
    assert.equal(state.activities.length, 1); assert.equal(state.activities[0].operation, 'commit'); assert.equal(state.activities[0].command_succeeded, false);
    assert.equal(state.tabs[0].draft_present, true); assert.deepEqual(semanticState(), before); assert(!fs.existsSync(path.join(fixture.client, '.git', 'index.lock')));
    const size = fs.statSync(heartbeat).size; await new Promise(r => setTimeout(r, 1000)); assert.equal(fs.statSync(heartbeat).size, size);
    fs.unlinkSync(path.join(fixture.hooks, 'pre-commit')); await capture('terminal-cancelled');
    pass('The Activity drawer Cancel stops the blocked commit; preserves draft, staged data and files; child stops');
    const beforeClear = await logs(); await click('console-clear'); assert.equal(state.activities.length, 0);
    const afterClear = await logs(); assert(afterClear.seq >= beforeClear.seq); assert(afterClear.all.length >= beforeClear.all.length);
    assert.equal(enabled('unstage-all'), true);
    await click('unstage-all'); await settle(); assert.equal(state.activities.length, 1); assert(state.activities[0].command_succeeded);
    const beforeConsoleClear = semanticState();
    assert.equal(beforeConsoleClear.head, before.head); assert.deepEqual(beforeConsoleClear.files, before.files);
    await click('console-clear'); assert.equal(state.activities.length, 0); assert.deepEqual(semanticState(), beforeConsoleClear);
    const diagnostic = await logs(); assert(diagnostic.all.every(r => r.dropped_before === 0));
    for (const start of diagnostic.all.filter(r => ['request_start', 'step_start', 'git_start'].includes(r.event)))
      assert.equal(diagnostic.all.filter(r => r.event === start.event.replace('_start', '_end') && r.id === start.id && r.session === start.session).length, 1);
    assert(!/ROCKET_PRIVATE_|rocket-ui-test|github_pat_|PRIVATE KEY/.test(JSON.stringify(diagnostic.all)));
    assert(diagnostic.all.some(r => r.event === 'git_end' && r.outcome === 'cancelled'));
    fs.writeFileSync(path.join(root, 'mcp-diagnostics.json'), JSON.stringify(diagnostic.all, null, 2));
    report.diagnostic_records = diagnostic.all.length; report.dropped_events = 0;
    pass('Clear empties the visible history twice without deleting diagnostics; spans pair and no private payloads appear');
    await type(''); assert.equal(state.tabs[0].draft_present, false);
    report.pid = state.pid; report.window_count = state.window_count; report.tool_calls = mcp.calls;
    await mcp.tool('rocket_close'); await mcp.finish(); report.passed = true;
  } catch (error) { report.error = error.message; console.error(error.stack); }
  finally {
    try { await mcp.finish(); } catch (error) { report.shutdown_error = error.message; mcp.terminateOwnedProcess(); report.passed = false; }
    try { fixture.run('cleanup'); report.cleanup = fixture.r.cleanup; } catch (error) { report.cleanup_error = error.message; report.passed = false; }
    fs.writeFileSync(path.join(root, 'terminal-result.json'), JSON.stringify(report, null, 2));
    console.log('Terminal MCP results: ' + path.join(root, 'terminal-result.json')); if (!report.passed) process.exitCode = 1;
  }
})();
