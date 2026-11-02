// Local-only native widget acceptance for diff selection latency and diagnostics.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const { RocketMcpClient } = require('../mcp/mcp-client.cjs');

async function main() {
  const workspace = path.resolve(__dirname, '../..');
  const name = `diff-${crypto.randomUUID().replaceAll('-', '')}`;
  const root = path.join(workspace, 'work', 'mcp-fixtures', name);
  const repo = path.join(root, 'client');
  const branch = `rocket-ui-test/${name}`;
  fs.mkdirSync(repo, { recursive: true });
  fs.mkdirSync(path.join(root, 'hooks'));
  fs.writeFileSync(path.join(root, 'empty.config'), '');
  const env = { ...process.env };
  for (const key of Object.keys(env)) if (/^(GIT_|SSH_)/i.test(key)) delete env[key];
  Object.assign(env, {
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'),
    GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_ALLOW_PROTOCOL: 'file',
    GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'hooks'),
  });
  const git = (...args) => {
    const r = spawnSync('C:\\Program Files\\Git\\cmd\\git.exe', args, { cwd: repo, env, windowsHide: true, encoding: 'utf8' });
    assert.equal(r.status, 0, `fixture Git ${args[0]} failed: ${r.stderr}`);
    return r.stdout.trim();
  };
  git('init', `--initial-branch=${branch}`, '.');
  for (const [key,value] of [['user.name','Rocket Diff Test'],['user.email','rocket@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',path.join(root,'hooks')]]) git('config',key,value);
  fs.writeFileSync(path.join(repo,'.git','rocket-ui-fixture'), branch);
  const bulk = version => Array.from({length:5000}, (_,i) => `row ${i} ${version} ${'synthetic local data '.repeat(6)}\n`).join('');
  const commit = message => {git('add','--all','--');git('commit','-m',message);return git('rev-parse','HEAD');};
  fs.writeFileSync(path.join(repo,'large.txt'),bulk('base'));
  fs.writeFileSync(path.join(repo,'small.txt'),'base\n');
  commit('Synthetic baseline');
  fs.writeFileSync(path.join(repo,'large.txt'),bulk('changed'));
  const large = commit('Change 5000 synthetic lines');
  fs.writeFileSync(path.join(repo,'small.txt'),'small change\n');
  const small = commit('Change one synthetic line');
  const fingerprint = () => JSON.stringify({
    head:git('rev-parse','HEAD'),status:git('status','--porcelain=v2'),
    index:crypto.createHash('sha256').update(fs.readFileSync(path.join(repo,'.git','index'))).digest('hex'),
    files:['large.txt','small.txt'].map(f=>crypto.createHash('sha256').update(fs.readFileSync(path.join(repo,f))).digest('hex')),
  });
  const before = fingerprint();
  const executable = path.join(workspace,'target','release','rocket-mcp.exe');
  const client = new RocketMcpClient(executable, path.join(workspace,'work','mcp-fixtures'), env);
  const report = {passed:false,root,cases:[],executable_sha256:crypto.createHash('sha256').update(fs.readFileSync(executable)).digest('hex')};
  try {
    await client.initialize();
    await client.tool('rocket_open',{path:repo});
    const settle = async () => {
      for(let n=0;n<4;n++) {
        const state = await client.tool('rocket_wait',{timeout_ms:15000});
        if(!state.busy) {assert.equal(state.tabs[0].error,null);return state;}
      }
      throw Error('UI did not settle');
    };
    let state = await settle();
    const tab = state.active_tab;
    await client.tool('rocket_click',{target:'detail-files',expected_tab:tab});
    state = await settle();
    for(const [label,id] of [['large',large],['small',small],['large',large],['small',small]]) {
      const previous = await client.tool('rocket_logs',{limit:1000});
      assert.equal(previous.more,false);
      const start = performance.now();
      await client.tool('rocket_click',{target:`commit-${id}`,expected_tab:tab});
      state = await settle();
      const elapsed = performance.now()-start;
      const batch = await client.tool('rocket_logs',{after_seq:previous.next_seq,limit:1000});
      assert.equal(batch.more,false);assert.equal(batch.retention_gap,false);
      const commands = batch.records.filter(r=>r.event==='git_start');
      assert.deepEqual(commands.map(r=>r.operation),['show']);
      assert(commands.every(r=>r.kind==='read'));
      // Word highlights (0.18.0) add a separate diff-words request; Git still runs once.
      const requests = batch.records.filter(r=>r.event==='request_end'&&r.operation==='diff');
      assert.equal(requests.length,1);assert.equal(requests[0].operation,'diff');assert.equal(requests[0].outcome,'ok');
      assert.equal(state.dropped_events,0);assert.equal(state.window_count,1);
      report.cases.push({label,widget_ms:elapsed,diff_ms:requests[0].elapsed_ms,git_commands:commands.length});
      if(!fs.existsSync(path.join(root,`${label}.png`))) {
        const capture = await client.tool('rocket_capture');
        const png = capture.content.find(c=>c.type==='image');
        assert(png);fs.writeFileSync(path.join(root,`${label}.png`),Buffer.from(png.data,'base64'));
      }
    }
    const logs = await client.tool('rocket_logs',{limit:1000});
    assert.equal(logs.more,false);assert.equal(logs.retention_gap,false);
    for(const kind of ['request','step','git']) {
      const starts = logs.records.filter(r=>r.event===`${kind}_start`);
      const ends = logs.records.filter(r=>r.event===`${kind}_end`);
      assert.equal(starts.length,ends.length);
      for(const start of starts) assert.equal(ends.filter(e=>e.id===start.id&&e.request===start.request).length,1);
    }
    assert(logs.records.every(r=>!r.dropped_before));
    assert(!/rocket-ui-test|large\.txt|small\.txt|synthetic local|github\.com|gh[pousr]_|github_pat_/i.test(JSON.stringify(logs.records)));
    assert.equal(fingerprint(),before);
    report.pid=state.pid;report.window_count=state.window_count;report.diagnostic_records=logs.records.length;
    report.dropped_events=0;report.repository_unchanged=true;report.passed=true;
    fs.writeFileSync(path.join(root,'diagnostics.json'),JSON.stringify(logs,null,2));
    await client.tool('rocket_close');
    await client.finish();
  } catch (error) {
    report.error=error.message;
    try { await client.finish(); } catch {client.terminateOwnedProcess();}
    throw error;
  } finally {
    report.tool_calls=client.calls;
    fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));
    console.log(JSON.stringify(report,null,2));
  }
}
main().catch(e=>{console.error(e.message);process.exitCode=1;});
