// Localhost acceptance for Security → Guard AI API keys. No network Git remotes,
// real credentials, or normal Rocket profiles. Synthetic key strings stay local.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const assert = require('node:assert/strict'), crypto = require('node:crypto'), net = require('node:net');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/ai-key-guard-' + crypto.randomUUID());
const out = path.resolve(process.argv[2] || 'work/results/ai-key-guard');
const exe = path.resolve(process.env.ROCKET_UI_EXE || 'target/release/rocket-mcp.exe');
const profile = path.join(root, 'profile'), branch = 'rocket-ui-test/ai-key-guard';
fs.mkdirSync(root, {recursive:true}); fs.mkdirSync(out, {recursive:true});
fs.mkdirSync(path.join(root, 'no-hooks')); fs.writeFileSync(path.join(root, 'empty.config'), '');
fs.writeFileSync(path.join(root, 'rocket-private-state'), 'synthetic-content-only-v1\n');
const env = {...process.env};
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, {GIT_CONFIG_GLOBAL:path.join(root,'empty.config'), GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),
  GIT_CONFIG_NOSYSTEM:'1', GIT_ALLOW_PROTOCOL:'file', GIT_TERMINAL_PROMPT:'0', GIT_TEMPLATE_DIR:path.join(root,'no-hooks')});
const git = (repo,args) => cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const fixtures = [
  ['openai', 'sk-'+'Ab9Cd7Ef5Gh3Ij1Kl0Mn'+'T3BlbkFJ'+'Op8Qr6St4Uv2Wx0Yz9Ab'],
  ['anthropic', 'sk-ant-api03-'+('Ab9Cd7Ef5Gh3Ij1Kl0MnOp8Qr6St4Uv2Wx0Yz').repeat(3).slice(0,93)+'AA'],
  ['deepseek', 'sk-'+'0123456789abcdef'.repeat(2)],
  ['glm', '0123456789abcdef'.repeat(2)+'.Ab9Cd7Ef5Gh3Ij1K'],
  ['normal', null],
];
function fixture(name,key) {
  const repo = path.join(root,name+' 文'); fs.mkdirSync(repo);
  git(repo,['init','--initial-branch='+branch]);
  for (const [k,v] of [['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']]) git(repo,['config',k,v]);
  fs.writeFileSync(path.join(repo,'readme'),'base\n');git(repo,['add','--all']);git(repo,['commit','-m','base']);
  fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
  const source = key ? `api_key = "${key}"\n` : ['const SECRET', ': &str = "QUEUE-TRACE-7f3a";\n', 'let token', ' = "AV_REQUESTED_V1:local-worker\\n";\n'].join('');
  fs.writeFileSync(path.join(repo,'資料.py'), source); git(repo,['add','--all']);
  return {name,key,repo,head:git(repo,['rev-parse','HEAD']),index:git(repo,['ls-files','--stage','-z']),source};
}
const repos = fixtures.map(([name,key]) => fixture(name,key));
const report = {passed:false,root,cases:[],version:null};
let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve,ms));
const pass = message => { report.cases.push(message); console.log('PASS '+message); };
const control = id => state?.controls?.find(item=>item.id===id);
const tab = () => state?.tabs?.find(item=>item.id===state.active_tab);
async function until(predicate,label='state',timeout=45000) {
  const start=performance.now();
  for(;;) { state=await client.tool('rocket_state'); if(predicate(state))return;
    assert(performance.now()-start<timeout,label+' timed out'); await delay(80); }
}
async function settled() { await until(s=>!s.busy&&!s.input_pending&&!s.write&&!(s.tabs||[]).some(t=>t.loading),'settled'); }
async function stable() { let last='',count=0; await until(s=>{const next=JSON.stringify(s.controls);count=last===next?count+1:0;last=next;return count>=2},'layout'); }
async function click(target) { await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settled(); }
async function type(target,text) { await stable();await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settled(); }
async function resize(width,height) { await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settled(); }
async function reveal(id) {
  for(let i=0;i<12;i++) { await stable();const c=control(id);if(c&&c.rect[3]-c.rect[1]>=19)return;
    await client.tool('rocket_scroll',{target:'prefs-scroll',direction:'down',pixels:150,expected_tab:state.active_tab});await settled(); }
  throw Error('Not visible: '+id);
}
function inView(id) { const c=control(id);assert(c,'Missing '+id);assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=state.viewport[0]+1&&c.rect[3]<=state.viewport[1]+1,'Outside window: '+id); }
async function capture(name) {
  await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);
  const img=r.content.find(item=>item.type==='image');assert(img);fs.writeFileSync(path.join(out,name+'.png'),Buffer.from(img.data,'base64'));
}
async function run() {
  try {
    cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
    const probe=net.createServer();await new Promise(resolve=>probe.listen(0,'127.0.0.1',resolve));const port=probe.address().port;await new Promise(resolve=>probe.close(resolve));
    fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
    const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);
    client=new RocketHttpClient(profile);
    for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>=150)throw e;await delay(100);}}
    report.version=(await client.initialize()).serverInfo.version;
    assert.equal(report.version,fs.readFileSync('Cargo.toml','utf8').match(/^version\s*=\s*"([^"]+)"/m)[1]);
    await client.tool('rocket_open',{path:repos[0].repo});await settled();await resize(1440,960);
    await click('menu-edit');await click('preferences');await click('pref-page-security');
    assert.equal(state.preferences.guard_ai_api_keys,true);assert.equal(state.preferences.secret_protection,true);
    for(const id of ['pref-secret-protection','pref-ai-key-guard']) {
      await reveal(id);inView(id);assert.equal(control(id).enabled,false);
      await assert.rejects(client.tool('rocket_click',{target:id,expected_tab:state.active_tab}),/Content guards are user preferences/);
    }
    await click('security-ai-key-coverage');await capture('security-desktop');pass('Separate guard defaults on; MCP cannot change either guard');
    await resize(780,620);await reveal('pref-ai-key-guard');inView('pref-ai-key-guard');await capture('security-compact');
    await reveal('security-ai-key-coverage');inView('security-ai-key-coverage');
    await client.tool('rocket_scroll',{target:'prefs-scroll',direction:'down',pixels:200,expected_tab:state.active_tab});await settled();
    await capture('security-compact-coverage');pass('Security controls remain accessible in a compact window');
    await click('dialog-submit');await resize(1440,960);
    for(const f of repos) {
      await client.tool('rocket_open',{path:f.repo});await settled();await click('view-changes');await type('commit-message','Fixture '+f.name);
      const previous=state.activities.length;
      await click('commit');await until(s=>s.activities.length>previous,'new commit activity');await settled();
      const activity=state.activities.at(-1);
      const outcome=JSON.stringify({error:tab()?.error,notice:state.notice,activity});
      if(f.key) {
        assert.equal(activity.operation,'commit');assert.equal(activity.command_succeeded,false);
        assert.match(outcome,/ai_api_key_detected/);assert(!outcome.includes(f.key));
        assert.equal(git(f.repo,['rev-parse','HEAD']),f.head);assert.equal(git(f.repo,['ls-files','--stage','-z']),f.index);
        assert.equal(fs.readFileSync(path.join(f.repo,'資料.py'),'utf8'),f.source);pass(f.name+' rejected before commit, with redacted guidance and preserved files/index');
        const cli=cp.spawnSync(path.join(path.dirname(exe),'rocket-cli.exe'),['--repo',f.repo,'--json','--log-dir',path.join(root,'cli-logs'),'commit','--message','CLI fixture'],{env,windowsHide:true,encoding:'utf8',timeout:30000});
        assert(!cli.error);assert.equal(cli.status,1);const json=JSON.parse(cli.stdout);
        assert.equal(json.error.code,'ai_api_key_detected');assert.equal(json.error.operation_started,false);assert.equal(json.error.retry_requires_changes,true);assert.equal(json.diagnostics.dropped_events,0);
        assert(!(cli.stdout+cli.stderr).includes(f.key));assert.equal(git(f.repo,['rev-parse','HEAD']),f.head);assert.equal(git(f.repo,['ls-files','--stage','-z']),f.index);
        pass(f.name+' CLI default protection returns redacted JSON and preserves HEAD/index');
        if(f.name==='deepseek') await capture('ai-key-refusal');
      } else { assert.notEqual(git(f.repo,['rev-parse','HEAD']),f.head);assert.equal(git(f.repo,['status','--porcelain']),'');pass('Ordinary constant/protocol fixtures commit successfully'); }
    }
    report.passed=true;
  } catch(error) {
    report.error=error.stack;process.exitCode=1;console.error(error.stack);
    if(client)try{await capture('failure');fs.writeFileSync(path.join(out,'failure-state.json'),JSON.stringify(state,null,2));}catch{}
  } finally {
    if(client)try{
      if(state?.dialog)await click(control('dialog-cancel')?'dialog-cancel':'dialog-close');await client.tool('rocket_close');
      await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');
      if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}
      await client.raw('POST','/shutdown');await client.finish();
    }catch(error){report.cleanup_error=error.message;process.exitCode=1;}
    if(service&&service.exitCode===null)await Promise.race([new Promise(resolve=>service.once('exit',resolve)),delay(5000)]);
    if(service&&service.exitCode===null)service.kill();
    const logFiles=[];
    function logs(folder){for(const e of fs.readdirSync(folder,{withFileTypes:true})){if(e.isSymbolicLink())throw Error('Unexpected link');const p=path.join(folder,e.name);if(e.isDirectory())logs(p);else if(e.name.endsWith('.jsonl')||e.name==='service.log')logFiles.push(p);}}
    logs(root);
    let refusal=false;
    for(const file of logFiles){const text=fs.readFileSync(file,'utf8');refusal=refusal||text.includes('ai_api_key_detected');for(const [,key]of fixtures)if(key&&text.includes(key)){report.log_privacy=false;report.passed=false;process.exitCode=1;}}
    if(!refusal){report.passed=false;report.log_error='No AI refusal diagnostic found';process.exitCode=1;}
    report.log_privacy=report.log_privacy!==false;report.logs_checked=logFiles.length;
    fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify({passed:report.passed,cases:report.cases.length,log_privacy:report.log_privacy}));
  }
}
run().catch(error=>{console.error(error);process.exitCode=1;});
