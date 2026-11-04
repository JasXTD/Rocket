// Focused localhost MCP acceptance. All crashes, reports and Git data are disposable.
'use strict';
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const project=path.resolve(__dirname,'../..');
const root=path.join(project,'work/mcp-fixtures/crash-'+crypto.randomUUID());
const evidence=path.resolve(process.argv[2]||path.join(root,'evidence'));
const profile=path.join(root,'profile'),repo=path.join(root,'repo 文'),reportDirectory=path.join(profile,'crashes');
const report={passed:false,root,evidence,cases:[]};
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(evidence,{recursive:true});fs.mkdirSync(repo);fs.mkdirSync(path.join(root,'template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_|ROCKET_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'template'),GIT_OPTIONAL_LOCKS:'0'});
const git=path.join(project,'target/release/git/2.56.0.windows.4/cmd/git.exe');
const g=(...args)=>cp.execFileSync(git,args,{cwd:repo,env,windowsHide:true,encoding:'utf8'}).trim();
const digest=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
let client,service,state;
const controls=()=>state?.controls??[];
const control=id=>controls().find(c=>c.id===id);
async function until(predicate,label){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return;assert(performance.now()-start<60000,label+' timed out');await delay(80);}}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!s.crash_reports.loading&&!s.crash_reports.exporting&&!(s.tabs||[]).some(t=>t.loading),'settled state');}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2;},'stable controls');}
async function reveal(id){await stable();if(!control(id)&&['crash-export-path','crash-export-save','crash-export-cancel'].includes(id)){await client.tool('rocket_scroll',{target:'crash-scroll',direction:'down',pixels:2000,expected_tab:state.active_tab});await settle();await stable();}}
async function click(id){await reveal(id);assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});await settle();}
async function type(id,text){await reveal(id);assert(control(id)?.enabled,'Missing field '+id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});const block=result.content.find(c=>c.type==='image');assert(block);fs.writeFileSync(path.join(evidence,name+'.png'),Buffer.from(block.data,'base64'));}
async function open(){await click('menu-help');await click('help-crash-reports');assert(state.crash_reports.open);}
async function refresh(){await client.tool('rocket_scroll',{target:'crash-scroll',direction:'up',pixels:2000,expected_tab:state.active_tab});await settle();await click('crash-refresh');}
function pass(name){report.cases.push(name);console.log('PASS '+name);}
function preserved(before){assert.deepEqual({head:g('rev-parse','HEAD'),index:digest(path.join(repo,'.git/index')),config:digest(path.join(repo,'.git/config')),data:fs.readFileSync(path.join(repo,'資料.txt'),'utf8')},before);}
async function run(){
try{
  g('init','--initial-branch=rocket-ui-test/crash');g('config','user.name','Rocket Crash Fixture');g('config','user.email','fixture@example.invalid');
  fs.writeFileSync(path.join(repo,'資料.txt'),'initial\n');g('add','--all');g('commit','-m','Synthetic report fixture');fs.writeFileSync(path.join(repo,'資料.txt'),'local edits must survive\n');
  fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),'rocket-ui-test/crash');
  const before={head:g('rev-parse','HEAD'),index:digest(path.join(repo,'.git/index')),config:digest(path.join(repo,'.git/config')),data:fs.readFileSync(path.join(repo,'資料.txt'),'utf8')};
  const exe=path.join(project,'target/release/rocket-mcp.exe');cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
  const probes=fs.readdirSync(path.join(project,'target/debug/deps')).filter(n=>/^crash_reports-[a-f0-9]+\.exe$/.test(n)).map(n=>path.join(project,'target/debug/deps',n)).sort((a,b)=>fs.statSync(b).mtimeMs-fs.statSync(a).mtimeMs);
  assert(probes.length,'Build the crash_reports integration test first');
  for(const mode of ['caught','native']){
    const result=cp.spawnSync(probes[0],['--exact','crash_probe','--ignored','--nocapture','--test-threads=1'],{env:{...env,ROCKET_CRASH_FIXTURE:reportDirectory,ROCKET_CRASH_MODE:mode},windowsHide:true,timeout:60000,encoding:'utf8'});
    assert(!result.error,'Isolated probe timeout');assert.equal(result.status===0,mode==='caught');assert(!result.stderr.includes('PRIVATE_'));
  }
  const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
  fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
  const log=fs.openSync(path.join(evidence,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);
  client=new RocketHttpClient(profile);for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>120)throw e;await delay(100);}}
  await client.initialize();await client.tool('rocket_open',{path:repo});await settle();
  await client.tool('rocket_resize',{width:1480,height:920});await settle();await open();
  assert.equal(state.crash_reports.reports.length,2);assert(state.crash_reports.reports.every(r=>r.version==='0.92.0'));
  for(let i=0;i<2;i++){
    await click('crash-report-'+i);const selected=state.crash_reports.selected;
    assert(['rust-panic','windows-exception'].includes(selected.kind));assert.equal(selected.os,'windows');assert.equal(selected.arch,'x86_64');assert(!JSON.stringify(selected).includes('PRIVATE_'));assert(!JSON.stringify(selected).includes(root));
  }
  await capture('reports-wide');await click('crash-copy');assert.match(state.crash_reports.notice,/copied/);
  pass('Real isolated panic/native reports load with private-safe metadata and copy');

  const destination=path.join(root,'报告 文.json');await click('crash-export');await type('crash-export-path',destination);await click('crash-export-save');
  assert.match(state.crash_reports.notice,/saved/);const exported=JSON.parse(fs.readFileSync(destination,'utf8'));assert.equal(exported.id,state.crash_reports.selected.id);assert(!fs.readFileSync(destination,'utf8').includes('PRIVATE_'));
  await click('crash-export');await type('crash-export-path',destination);await click('crash-export-save');assert.match(state.crash_reports.error,/already exists/);assert.deepEqual(JSON.parse(fs.readFileSync(destination,'utf8')),exported);
  await capture('existing-file-error');await click('crash-export-cancel');
  pass('Unicode JSON export succeeds and an existing file is preserved with a clear error');

  const cancelled=path.join(root,'cancelled.json');await click('crash-export');await type('crash-export-path',cancelled);await click('crash-export-cancel');assert(!fs.existsSync(cancelled));
  const outside=path.join(path.dirname(root),'outside-'+path.basename(root)+'.json');assert(!fs.existsSync(outside));
  await click('crash-export');await type('crash-export-path',outside);await click('crash-export-save');assert(state.crash_reports.error);assert(!fs.existsSync(outside));await click('crash-export-cancel');
  pass('Cancelled review creates no file and MCP export cannot leave its fixture scope');

  await client.tool('rocket_resize',{width:780,height:620});await settle();await capture('reports-compact');await click('crash-export');await capture('export-compact');
  for(const id of ['crash-close','crash-copy','crash-export']){const r=control(id);assert(r,'Compact missing '+id);assert(r.rect[0]>=0&&r.rect[1]>=0&&r.rect[2]<=780&&r.rect[3]<=620,'Offscreen '+id);}
  await client.tool('rocket_scroll',{target:'crash-scroll',direction:'down',pixels:700,expected_tab:state.active_tab});await settle();await stable();
  for(const id of ['crash-export-path','crash-export-save','crash-export-cancel']){const r=control(id);assert(r?.enabled,'Compact missing '+id);const body=control('crash-scroll');assert(r.rect[1]>=body.rect[1]&&r.rect[3]<=body.rect[3],'Export control outside scroll area: '+id);}
  await capture('export-compact-scrolled');
  await type('crash-export-path',destination);await click('crash-export-save');await stable();assert.match(state.crash_reports.error,/already exists/);assert(control('crash-feedback'),'Compact export error must be visible without scrolling');await capture('export-error-compact');assert.deepEqual(JSON.parse(fs.readFileSync(destination,'utf8')),exported);
  await client.tool('rocket_escape');await settle();assert(!state.crash_reports.export_open);assert(state.crash_reports.open);await click('crash-close');assert(!state.crash_reports.open);await open();
  pass('Compact report/export controls remain reachable and Escape cancels the review');

  const damaged=path.join(reportDirectory,'crash-0000-1-1.rcr');fs.writeFileSync(damaged,'PRIVATE_REPORT_DATA must never be shown\n');await refresh();assert.equal(state.crash_reports.damaged,1);assert.equal(state.crash_reports.reports.length,2);await capture('damaged-record');fs.unlinkSync(damaged);
  const extra=[];for(let i=0;i<257;i++){const file=path.join(reportDirectory,'entry-'+i);fs.writeFileSync(file,'');extra.push(file);}await refresh();assert(state.crash_reports.error);assert.equal(state.crash_reports.reports.length,2);await capture('read-error-retained');for(const file of extra)fs.unlinkSync(file);await refresh();assert(!state.crash_reports.error);
  pass('Damaged records are excluded and failed refresh retains prior reports');

  const retained=path.join(evidence,'seed-reports');fs.mkdirSync(retained);
  for(const name of fs.readdirSync(reportDirectory).filter(n=>n.endsWith('.rcr'))){const file=path.join(reportDirectory,name);if(fs.statSync(file).size){fs.copyFileSync(file,path.join(retained,name));fs.unlinkSync(file);}}
  await refresh();assert.equal(state.crash_reports.reports.length,0);assert(!control('crash-copy').enabled);assert(!control('crash-export').enabled);await capture('empty-compact');preserved(before);
  pass('Empty state disables export; repository HEAD/index/config and local edits are preserved');
  await click('crash-close');report.passed=true;
}catch(error){report.error=String(error);if(client)try{await capture('failure');}catch{}throw error;}
finally{
  fs.writeFileSync(path.join(evidence,'last-state.json'),JSON.stringify(state??null,null,2));
  if(service&&service.exitCode===null){
    try{
      if(state?.crash_reports?.open&&!state?.crash_reports?.exporting)await click('crash-close');
      await client.tool('rocket_close');await until(s=>s.window==='closed'||s.dialog?.kind==='quit','window closed');
      if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.window==='closed','quit complete');}
      await client.raw('POST','/shutdown');
    }catch(error){report.close_error=String(error);}
    const start=performance.now();while(service.exitCode===null&&performance.now()-start<20000)await delay(100);
    report.service_exit=service.exitCode;if(service.exitCode===null){report.passed=false;report.close_error='Owned MCP service did not stop';service.kill();}
  }
  if(client)await client.finish();
  if(report.close_error||report.service_exit!==0){report.passed=false;process.exitCode=1;}
  fs.writeFileSync(path.join(evidence,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
}
}
run().catch(error=>{console.error(error);process.exitCode=1});
