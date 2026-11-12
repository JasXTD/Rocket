// Requires an operator-audited native CLI via ROCKET_COPILOT_EXE. Never installs,
// signs in, calls a hosted model, or changes the user's Copilot/Git configuration.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), http = require('node:http'), net = require('node:net'), os = require('node:os');
const assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const windows = process.platform === 'win32';
assert(process.env.ROCKET_COPILOT_EXE, 'Set ROCKET_COPILOT_EXE to the audited native executable');
const supplied = fs.realpathSync(process.env.ROCKET_COPILOT_EXE);
const exe = path.resolve(process.env.ROCKET_AGENTS_EXE || `target/release/rocket-mcp${windows ? '.exe' : ''}`);
const base = path.resolve('work/mcp-fixtures'); fs.mkdirSync(base, { recursive: true });
// Worktrees must be outside every existing repository, including Rocket itself.
const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'rocket-copilot-')));
const reportDir = path.join(base, path.basename(root)); fs.mkdirSync(reportDir);
const repo = path.join(root, 'source 資料'), profile = path.join(root, 'http-profile');
const bin = path.join(root, 'copilot-bin'); fs.mkdirSync(bin);
const copilot = path.join(bin, windows ? 'copilot.exe' : 'copilot'); fs.copyFileSync(supplied, copilot);
if (!windows) fs.chmodSync(copilot, 0o700);
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const canonical = p => path.toNamespacedPath(fs.realpathSync(p));
assert.equal(hash(copilot), hash(supplied));
const env = { ...process.env };
for (const key of Object.keys(env)) {
  if (/^(GIT_|GCM_|GH_|GITHUB_|COPILOT_|CODEX_|CLAUDE_|ANTHROPIC_|OPENAI_|SSH_|AZURE_|AWS_|NODE_|OTEL_)|TOKEN|SECRET|API_KEY|_PROXY$/i.test(key) || ['BASH_ENV', 'ENV'].includes(key)) delete env[key];
}
for (const key of ['HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'TEMP', 'TMP', 'COPILOT_HOME']) {
  env[key] = path.join(root, 'isolated', key); fs.mkdirSync(env[key], { recursive: true });
}
const pathKey = Object.keys(env).find(k => k.toUpperCase() === 'PATH') || 'PATH';
env[pathKey] = bin + path.delimiter + (env[pathKey] || '');
fs.mkdirSync(repo); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
Object.assign(env, {
  ROCKET_AGENT_FIXTURE_ROOT: root, GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'),
  GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1',
  GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_OPTIONAL_LOCKS: '0',
  GIT_TEMPLATE_DIR: path.join(root, 'empty-template'), COPILOT_OFFLINE: 'true',
  COPILOT_PROVIDER_TYPE: 'openai', COPILOT_MODEL: 'rocket-lifecycle-fixture',
});
const args = '--no-auto-update\n--no-custom-instructions\n--disable-builtin-mcps\n--disable-client-telemetry\n--deny-tool\nshell\nwrite\nread\nurl\nmemory';
const git = (args, at = repo) => cp.execFileSync('git', ['-C', at, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trimEnd();
git(['init', '--initial-branch=rocket-ui-test/copilot']);
for (const [k, v] of [['user.name', 'Synthetic Test'], ['user.email', 'fixture@example.invalid'], ['core.autocrlf', 'false'], ['commit.gpgsign', 'false']]) git(['config', k, v]);
fs.writeFileSync(path.join(repo, '資料.txt'), 'original\n'); git(['add', '.']); git(['commit', '-m', 'Synthetic base']);
fs.writeFileSync(path.join(repo, '資料.txt'), 'staged\n'); git(['add', '.']); fs.appendFileSync(path.join(repo, '資料.txt'), 'unstaged\n');
fs.writeFileSync(path.join(repo, 'untracked.txt'), 'keep');
fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), 'rocket-ui-test/copilot');
fs.writeFileSync(path.join(root, 'rocket-private-state'), 'synthetic-content-only-v1\n');
fs.writeFileSync(path.join(root, 'rocket-agent-copilot'), 'offline-copilot-lifecycle-v1\n');
const snapshot = () => ({ head: git(['rev-parse', 'HEAD']), index: hash(path.join(repo, '.git/index')), config: hash(path.join(repo, '.git/config')), file: hash(path.join(repo, '資料.txt')), untracked: hash(path.join(repo, 'untracked.txt')) });
const before = snapshot();
const report = { passed: false, root, copilot_sha256: hash(copilot), mcp_sha256: hash(exe), cases: [], requests: [], viewports: [] };
let state, client, service, provider;
const wait = ms => new Promise(r => setTimeout(r, ms));
const pass = text => { report.cases.push(text); console.log('PASS ' + text); };
const control = id => state.controls?.find(c => c.id === id);
const selected = () => state.agents?.sessions.find(s => s.id === state.agents.selected);
async function until(check, label = 'state', timeout = 60000) {
  const start = performance.now();
  do { state = await client.tool('rocket_state'); if (check(state)) return; await wait(65); }
  while (performance.now() - start < timeout);
  throw Error(label + ' timed out: ' + JSON.stringify({ agents: state.agents, viewport: state.viewport, busy: state.busy }));
}
async function settled() { await until(s => !s.busy && !s.input_pending && !s.agents?.busy && !(s.tabs || []).some(t => t.loading), 'settled'); }
async function stable() { let last = '', count = 0; await until(s => { const next = JSON.stringify(s.controls); count = next === last ? count + 1 : 0; last = next; return count >= 2; }, 'layout'); }
async function reveal(id) {
  await stable();
  const visible = () => control(id)?.rect[3] - control(id)?.rect[1] > 18;
  if (!visible() && state.agents?.form) {
    await client.tool('rocket_scroll', { target: 'agents-scroll', direction: 'up', pixels: 2000, expected_tab: state.active_tab }); await settled();
    for (let i = 0; i < 12 && !visible(); i++) { await client.tool('rocket_scroll', { target: 'agents-scroll', direction: 'down', pixels: 160, expected_tab: state.active_tab }); await settled(); await stable(); }
  }
  assert(visible(), 'Not visible: ' + id);
}
async function click(id) { await reveal(id); assert(control(id).enabled, 'Disabled: ' + id); await client.tool('rocket_click', { target: id, expected_tab: state.active_tab }); await settled(); }
async function type(id, text) { await reveal(id); await client.tool('rocket_type', { target: id, text, expected_tab: state.active_tab }); await settled(); }
async function capture(name) { await stable(); const r = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!r.isError); fs.writeFileSync(path.join(reportDir, name + '.png'), Buffer.from(r.content.find(c => c.type === 'image').data, 'base64')); fs.writeFileSync(path.join(reportDir, name + '.json'), JSON.stringify(state, null, 2)); }
async function resize(width, height) { await until(s => s.first_presentation && !s.state_pending, 'window initialized'); await stable(); await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending, 'resize'); report.viewports.push([width, height]); }
async function advanced(){await stable();if(control('agents-program')||control('agents-arguments'))return;try{await reveal('agents-advanced')}catch{return}await click('agents-advanced')}
function destination(name) { const folder = path.join(root, 'worktree 資料 ' + name), branch = 'rocket-ui-test/copilot-' + name; fs.writeFileSync(folder + '.rocket-worktree-fixture', branch + '\n' + repo); return { folder, branch }; }
async function form(name) {
  const d = destination(name); await click('agents-new'); await click('agents-provider-copilot');
  assert.equal(state.agents.form.provider, 'copilot'); assert.equal(canonical(state.agents.form.program), canonical(copilot)); assert.equal(state.agents.form.arguments, '--no-auto-update');
  await type('agents-name', 'PRIVATE Copilot task 資料'); await type('agents-branch', d.branch); await type('agents-destination', d.folder);
  await advanced(); await type('agents-arguments', args); return d;
}
async function review() { await click('agents-review'); assert(state.agents.form.reviewed, state.agents.form.error); }
async function close() { await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window)); } }
const alive = pid => { try { process.kill(pid, 0); return true; } catch { return false; } };
(async () => {
  try {
    // A loopback-only provider is sufficient to start the real interactive CLI.
    // One fixed prompt gets a fixed local response. No hosted model is contacted.
    provider = http.createServer((req, res) => {
      const observed = { method: req.method, path: req.url }; report.requests.push(observed);
      res.setHeader('Content-Type', 'application/json');
      if (req.method === 'GET' && req.url === '/v1/models') { req.resume(); res.end(JSON.stringify({ object: 'list', data: [{ id: 'rocket-lifecycle-fixture', object: 'model', owned_by: 'fixture' }] })); }
      else if (req.method === 'POST' && req.url === '/v1/chat/completions') {
        let body=''; req.on('data',chunk=>{body+=chunk.toString();if(body.length>2*1024*1024)req.destroy();});
        req.on('end',()=>{
          try {
            const payload=JSON.parse(body), last=(payload.messages||[]).findLast(m=>m.role==='user');
            const text=typeof last?.content==='string'?last.content:(last?.content||[]).map(c=>c.text||'').join('\n');
            observed.expected_prompt=text.includes('Reply exactly ROCKET_LOCAL_ACTIVITY_OK. Do not use tools.');
            if(!observed.expected_prompt) {res.statusCode=400;res.end(JSON.stringify({error:{message:'Unexpected fixture input'}}));return;}
            const base={id:'local-activity',created:1,model:'rocket-lifecycle-fixture'};
            if(payload.stream){
              res.setHeader('Content-Type','text/event-stream');
              for(const [delta,finish_reason] of [[{role:'assistant',content:'ROCKET_LOCAL_ACTIVITY_OK'},null],[{},'stop']])
                res.write('data: '+JSON.stringify({...base,object:'chat.completion.chunk',choices:[{index:0,delta,finish_reason}]})+'\n\n');
              res.end('data: [DONE]\n\n');
            }else res.end(JSON.stringify({...base,object:'chat.completion',choices:[{index:0,message:{role:'assistant',content:'ROCKET_LOCAL_ACTIVITY_OK'},finish_reason:'stop'}]}));
          }catch {observed.invalid_request=true;res.statusCode=400;res.end('{}');}
        });
      } else { req.resume();res.statusCode = 503; res.end(JSON.stringify({ error: { message: 'Only the local lifecycle response is available' } })); }
    });
    await new Promise(r => provider.listen(0, '127.0.0.1', r)); env.COPILOT_PROVIDER_BASE_URL = `http://127.0.0.1:${provider.address().port}/v1`;
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const portProbe = net.createServer(); await new Promise(r => portProbe.listen(0, '127.0.0.1', r)); const port = portProbe.address().port; await new Promise(r => portProbe.close(r));
    fs.writeFileSync(path.join(profile, 'settings'), `ROCKET-MCP-1\n${port}\n${root}\n`);
    const out = fs.openSync(path.join(reportDir, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', out] }); fs.closeSync(out);
    client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i > 100) throw e; await wait(100); } }
    await client.initialize(); await client.tool('rocket_open', { path: repo }); await settled(); await stable();
    await resize(1480, 920); if (control('console-close')) await click('console-close');
    await click('view-agents');
    const cancelled = await form('cancel'); await review(); await click('agents-cancel'); assert(!fs.existsSync(cancelled.folder));
    assert.equal(state.agents.sessions.length, 0); pass('Copilot discovery, safe defaults and cancelled review without a worktree or process');
    const d = await form('live');
    await resize(780, 620); await reveal('agents-provider-copilot'); await capture('copilot-form-compact');
    for (const id of ['agents-provider-claude', 'agents-provider-codex', 'agents-provider-copilot', 'agents-provider-custom']) { const c = control(id); assert(c && c.rect[0] >= 0 && c.rect[2] <= state.viewport[0] && c.rect[3] > c.rect[1]); }
    await type('agents-arguments', '--no-auto-update');
    const denied = await client.request('tools/call', { name: 'rocket_click', arguments: { target: 'agents-review', expected_tab: state.active_tab } }); assert(denied.isError && denied.content.some(c => c.text?.includes('MCP runs only')), 'Unrestricted real CLI argv must be denied by MCP policy');
    await type('agents-arguments', args); await review(); await capture('copilot-review-compact');
    await resize(1480, 920);
    // The temporary profile trusts this one synthetic worktree only. Existing
    // trust settings, login credentials and the user's config are not consulted.
    fs.writeFileSync(path.join(env.COPILOT_HOME, 'config.json'), JSON.stringify({ trustedFolders: [d.folder] }));
    fs.writeFileSync(path.join(env.COPILOT_HOME, 'settings.json'), JSON.stringify({ ide: { autoConnect: false }, autoUpdate: false }));
    await click('agents-confirm'); await until(() => {
      assert(!['Failed', 'Exited'].includes(selected()?.phase), selected()?.error + '\n' + selected()?.tail);
      return selected()?.phase === 'Running' && /Copilot/i.test(selected()?.tail || '');
    }, 'Copilot interactive startup');
    // Copilot creates a session lazily on its first prompt, not on TUI startup.
    await until(() => selected()?.bracketed_paste === true, 'Copilot requested bracketed paste');
    const deniedInput=await client.request('tools/call',{name:'rocket_type',arguments:{target:'terminal-command',text:'Unapproved arbitrary prompt',expected_tab:state.active_tab}});
    assert(deniedInput.isError&&deniedInput.content.some(c=>c.text?.includes('isolated Copilot probe')),'MCP must refuse arbitrary Copilot prompts');
    await type('terminal-command', 'Reply exactly ROCKET_LOCAL_ACTIVITY_OK. Do not use tools.');
    await client.tool('rocket_terminal_enter', { expected_tab: state.active_tab });
    await until(() => selected()?.activity_state === 'Turn finished' && /^[0-9a-f-]{36}$/i.test(selected()?.conversation || '') && selected()?.tail?.includes('ROCKET_LOCAL_ACTIVITY_OK'), 'Copilot conversation and completion hooks');
    const conversation = selected().conversation;
    report.activity_hook = { completed_turn: true, exact_conversation: true };
    pass('Real Copilot reports an exact conversation and finished turn using only a fixed localhost response');
    await capture('copilot-running');
    const beforeHelp = selected().tail; await type('terminal-command', '/help'); await client.tool('rocket_terminal_enter', { expected_tab: state.active_tab });
    await until(() => selected()?.tail !== beforeHelp && /\/help\s+show full help/i.test(selected()?.tail || '') && /ctrl\+r\s+reverse search history/i.test(selected()?.tail || ''), 'Copilot help response');
    await capture('copilot-help-wide');
    pass('Real Copilot CLI launches in the Unicode worktree and handles typed /help through the PTY');
    await resize(780, 620); await capture('copilot-terminal-compact'); await reveal('agents-stop');
    const pid = selected().pid, started = performance.now(); assert(pid && alive(pid));
    await client.tool('rocket_click', { target: 'agents-stop', expected_tab: state.active_tab });
    await until(() => ['Stopping', 'Stopped'].includes(selected()?.phase), 'Stop feedback'); const feedback = performance.now() - started;
    await until(() => selected()?.phase === 'Stopped' && !selected()?.active && !alive(pid), 'process termination');
    report.stop = { feedback_ms: feedback, exited_ms: performance.now() - started }; assert(report.stop.exited_ms < 5000, 'Stop should not stall');
    const pluginRoot=path.join(env.LOCALAPPDATA,'Rocket','agent-events');
    assert(!fs.existsSync(pluginRoot)||fs.readdirSync(pluginRoot).length===0,'Temporary activity plugin must be removed on Stop');
    report.activity_plugin_removed=true;
    assert(fs.existsSync(d.folder)); assert.equal(git(['status', '--porcelain'], d.folder), ''); await capture('copilot-stopped-compact');
    pass('One Stop click terminates the real CLI and retains the clean worktree');
    await resize(1480, 920); await close(); await client.tool('rocket_open', { path: repo }); await settled(); await until(s => s.first_presentation && !s.state_pending, 'restored window'); await stable(); await click('view-agents');
    assert.equal(state.agents.sessions.length, 1); assert.equal(selected().provider, 'copilot'); assert.equal(selected().phase, 'Restored'); assert(!selected().active && !selected().pid);
    await click('agents-restart'); assert.equal(state.agents.form.provider, 'copilot'); assert.equal(state.agents.form.arguments, args); await click('agents-cancel');
    pass('Saved Copilot provider and arguments persist without automatic relaunch');
    await close(); await client.raw('POST', '/shutdown'); assert.deepEqual(snapshot(), before);
    assert(report.requests.some(r => r.method === 'POST' && r.path === '/v1/chat/completions'));
    assert(report.requests.every(r => r.method === 'GET' && r.path === '/v1/models' || r.method === 'POST' && r.path === '/v1/chat/completions'), 'Only the fixed loopback provider routes may be used');
    assert(report.requests.filter(r=>r.method==='POST').every(r=>r.expected_prompt===true&&!r.invalid_request),'Only the fixed synthetic prompt may be submitted');
    pass('Source HEAD/index/config/local changes preserved; no login or hosted model used');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(f => /rocket.*\.jsonl$/.test(f)).flatMap(f => fs.readFileSync(path.join(root, n, f), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length && records.every(r => r.dropped_before === 0));
    for (const op of ['agent-session-preview', 'agent-session-start', 'agent_started', 'agent_stop_requested', 'agent_stopped', 'agent_conversation_ready', 'agent_turn_working', 'agent_turn_finished']) assert(records.some(r => r.operation === op), 'Missing log step ' + op);
    const logs = JSON.stringify(records); for (const value of ['PRIVATE Copilot task', args, copilot, d.folder, '/help', conversation, 'ROCKET_LOCAL_ACTIVITY_OK']) assert(!logs.includes(value), 'Diagnostic payload leak');
    report.agent_timings = records.filter(r => r.operation?.startsWith('agent')).map(({ event, operation, request, elapsed_ms, outcome }) => ({ event, operation, request, elapsed_ms, outcome }));
    pass('Lifecycle diagnostics are present with no terminal/argv payloads or dropped events'); report.passed = true;
  } catch (e) { report.failure = e.stack; try { await capture('failure'); } catch {} console.error(e.stack); process.exitCode = 1; }
  finally {
    if (client) { try { await client.raw('POST', '/shutdown'); } catch {} await client.finish(); }
    if (service && service.exitCode === null) await new Promise(r => { const timer = setTimeout(() => { service.kill(); r(); }, 5000); service.once('exit', () => { clearTimeout(timer); r(); }); });
    if (provider) { provider.closeAllConnections(); await new Promise(r => provider.close(r)); }
    const token = path.join(profile, 'access-token'); if (fs.existsSync(token)) fs.unlinkSync(token);
    report.token_removed = !fs.existsSync(token); fs.writeFileSync(path.join(reportDir, 'result.json'), JSON.stringify(report, null, 2)); console.log(reportDir);
  }
})();
