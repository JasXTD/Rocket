// Real MCP stdio fallback when the Codex-registered GUI session has been closed.
// One server/window handles HTTPS and SSH. These adapters do not bypass the UI.
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const assert = require('node:assert/strict');
const { Fixture } = require('./mcp-github-fixture.cjs');
const { RocketMcpClient } = require('../mcp/mcp-client.cjs');
const workspace = path.resolve(__dirname, '../..');
const first = new Fixture(process.argv[2]); const second = new Fixture(process.argv[3]);
assert.equal(first.mode, 'https'); assert.equal(second.mode, 'ssh');
for (const fixture of [first, second]) {
  assert(fixture.r.baseline && fixture.r.checks.includes('setup') && !fixture.r.remote_write_attempted && fixture.r.cleanup === 'not-needed',
    'Use fresh setup fixtures; a completed or uncertain transport run must not be replayed');
}
const env = { ...first.env, GIT_ALLOW_PROTOCOL: 'https:ssh' };
const executable = path.join(workspace, 'target', 'release', 'rocket-mcp.exe');
const executableHash = require('node:crypto').createHash('sha256').update(fs.readFileSync(executable)).digest('hex');
const mcp = new RocketMcpClient(executable, path.resolve('work/mcp-fixtures'), env);
const values = new Map(); let currentRoot; let captures = 0;
global.load = key => values.get(key); global.store = (key, value) => values.set(key, value);
global.notify = value => console.log(JSON.stringify(value)); global.text = value => console.log(JSON.stringify(value));
global.image = value => { assert.equal(value.mimeType, 'image/png'); fs.writeFileSync(path.join(currentRoot, 'mcp-capture-' + (++captures) + '.png'), Buffer.from(value.data, 'base64')); };
global.tools = {
  async exec_command(options) {
    // The authored driver supplies only one fixture helper command at a time.
    assert(options.cmd.startsWith('node .\\work\\mcp-github-fixture.cjs '));
    const r = spawnSync('powershell.exe', ['-NoProfile', '-Command', options.cmd], { cwd: workspace, env: process.env, windowsHide: true, encoding: 'utf8', timeout: 90000, maxBuffer: 1024 * 1024 });
    return { exit_code: r.status, output: r.stdout + r.stderr };
  },
  async apply_patch(patch) {
    const match = patch.match(/^\*\*\* Begin Patch\n\*\*\* Add File: ([^\n]+)\n([\s\S]*)\n\*\*\* End Patch$/);
    assert(match); assert.equal(path.dirname(match[1]), currentRoot);
    assert(['mcp-diagnostics.json', 'mcp-result.json'].includes(path.basename(match[1])));
    const lines = match[2].split('\n'); assert(lines.every(line => line.startsWith('+')));
    fs.writeFileSync(match[1], lines.map(line => line.slice(1)).join('\n') + '\n'); return {};
  }
};
for (const name of ['rocket_state', 'rocket_open', 'rocket_click', 'rocket_type', 'rocket_wait', 'rocket_capture', 'rocket_logs', 'rocket_escape', 'rocket_close']) {
  global.tools['mcp__rocket_ui__' + name] = async args => {
    try { const value = await mcp.tool(name, args); return value?.content ? value : { isError: false, structuredContent: value }; }
    catch (e) { return { isError: true, content: [{ type: 'text', text: e.message }] }; }
  };
}
const run = require('./mcp-github-host-driver.js');
(async () => {
  const summary = { passed: false, kind: 'Real stdio MCP acceptance; one persistent native window', executable_sha256: executableHash, results: [] };
  try {
    await mcp.initialize(); const cold = await mcp.tool('rocket_state'); assert.equal(cold.window_count, 0);
    for (const fixture of [first, second]) {
      currentRoot = fixture.root; captures = 0;
      const result = await run(fixture.root, fixture.mode);
      result.via = 'Real stdio MCP tools through RocketMcpClient; actual widgets, controller and Git backend';
      result.executable_sha256 = executableHash;
      result.capture_files = Array.from({ length: captures }, (_, i) => path.join(currentRoot, 'mcp-capture-' + (i + 1) + '.png'));
      fs.writeFileSync(path.join(currentRoot, 'mcp-result.json'), JSON.stringify(result, null, 2));
      summary.results.push({ mode: fixture.mode, root: fixture.root, passed: result.passed, pid: result.pid, phases: result.phases.length, cleanup: result.cleanup });
      assert.equal(result.pid, mcp.child.pid); assert.equal(result.window_count, 1);
      if (!result.passed) throw Error(fixture.mode + ' MCP acceptance stopped; inspect its result.json before another write');
    }
    summary.tool_calls = mcp.calls; summary.pid = mcp.child.pid; summary.passed = true;
    await mcp.tool('rocket_close'); await mcp.finish();
  } catch (error) { summary.error = error.message; console.error(error.message); }
  finally {
    try { await mcp.finish(); } catch (e) { summary.shutdown_error = e.message; mcp.terminateOwnedProcess(); summary.passed = false; }
    fs.writeFileSync(path.join(first.root, 'session-result.json'), JSON.stringify(summary, null, 2));
    console.log(JSON.stringify(summary)); if (!summary.passed) process.exitCode = 1;
  }
})();
