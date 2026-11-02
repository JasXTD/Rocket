'use strict';
// Small offline acceptance of the relocated clients/helpers. No feature-wide suite.
const fs=require('node:fs'), path=require('node:path'), cp=require('node:child_process');
const crypto=require('node:crypto'), net=require('node:net'), assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const workspace=path.resolve(__dirname,'../..');
process.chdir(workspace);
const exe=path.resolve(process.argv[2]||'tools/releases/0.81.1/rocket-mcp.exe');
assert(fs.existsSync(exe),'Pass the path to a built rocket-mcp.exe');
const root=path.resolve('work/tooling-smoke-'+crypto.randomUUID());
const repo=path.join(root,'repo-中文'),profile=path.join(root,'profile'),branch='rocket-ui-test/tooling';
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));
fs.writeFileSync(path.join(root,'empty.config'),'');
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),
  GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const report={passed:false,root,cases:[]};
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim();
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
let service,client,state;
async function until(predicate,label){const started=Date.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(Date.now()-started<45000,label+' timed out');await delay(75);}}
async function close(){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending),'close');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window),'confirm close');}}
async function main(){try{
  git(['init','--initial-branch='+branch]);
  for(const [k,v]of [['user.name','Tooling Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.hooksPath',path.join(root,'empty-template')]])git(['config',k,v]);
  fs.writeFileSync(path.join(repo,'资料.txt'),'synthetic tooling check\n');git(['add','.']);git(['commit','-m','Tooling fixture']);
  fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
  const before=git(['rev-parse','HEAD']);
  const realGit=cp.execFileSync('where.exe',['git.exe'],{windowsHide:true,encoding:'utf8'}).trim().split(/\r?\n/)[0];
  const relayEnv={...env,ROCKET_FILE_TEST_GIT:realGit,ROCKET_FILE_TEST_GATE:path.join(root,'unused-gate'),ROCKET_FILE_TEST_MARKER:path.join(root,'unused-marker')};
  const version=git(['--version']);
  for(const name of fs.readdirSync('tools/helpers/bin')){
    if(name==='auto-bisect')continue; // Separate test-runner.exe, not a Git relay.
    const relay=path.resolve('tools/helpers/bin',name,'git.exe');
    assert.equal(cp.execFileSync(relay,['--version'],{env:relayEnv,windowsHide:true,encoding:'utf8',timeout:10000}).trim(),version);
  }
  report.cases.push('All six relocated Git relays invoke the installed Git');
  cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
  const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
  fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
  const log=fs.openSync(path.join(root,'service.log'),'w');
  service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);
  service.on('error',error=>{report.spawn_error=error.message;});
  client=new RocketHttpClient(profile);
  for(let attempt=0;;attempt++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(error){if(attempt===150||service.exitCode!==null)throw error;await delay(100);}}
  report.version=(await client.initialize()).serverInfo.version;
  report.cases.push('Relocated HTTP client authenticates to an isolated localhost service');
  await client.tool('rocket_open',{path:repo});
  await until(s=>s.tabs?.length===1&&!s.busy&&!s.input_pending&&!s.tabs.some(t=>t.loading),'open repository');
  assert(state.controls.length>0);assert(state.active_tab);
  assert.equal(git(['rev-parse','HEAD']),before);assert.equal(git(['status','--porcelain']), '');
  report.cases.push('Native UI opens a Unicode temporary repository; HEAD and files stay unchanged');
  await close();await client.raw('POST','/shutdown');
  for(let attempt=0;service.exitCode===null&&attempt<200;attempt++)await delay(50);
  assert.equal(service.exitCode,0,'Owned MCP service should exit cleanly');
  report.cases.push('Native window and owned localhost service close cleanly');
  report.passed=true;
}catch(error){report.error=error.stack;process.exitCode=1;}finally{
  if(service&&service.exitCode===null){
    if(client){try{await close();await client.raw('POST','/shutdown');}catch(error){report.cleanup_error=error.message;}}
    for(let attempt=0;service.exitCode===null&&attempt<100;attempt++)await delay(50);
    if(service.exitCode===null){cp.spawnSync('taskkill.exe',['/PID',String(service.pid),'/T','/F'],{windowsHide:true,stdio:'pipe'});report.forced_cleanup=true;report.passed=false;process.exitCode=1;}
  }
  if(client)await client.finish();
  fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));
  fs.mkdirSync('work/results',{recursive:true});fs.writeFileSync('work/results/tooling-latest.json',JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
}}
main().catch(error=>{console.error(error);process.exitCode=1;});
