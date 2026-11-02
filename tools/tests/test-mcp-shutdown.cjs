// Focused localhost MCP shutdown/draft/review checks. No accounts or remote Git.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/shutdown-'+crypto.randomUUID()),repo=path.join(root,'repo 資料'),profile=path.join(root,'http-profile'),exe=path.resolve('target/release/rocket-mcp.exe');
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'template'));fs.writeFileSync(path.join(root,'global.config'),'');fs.writeFileSync(path.join(root,'system.config'),'');fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'global.config'),GIT_CONFIG_SYSTEM:path.join(root,'system.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'template')});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:'pipe'}).trimEnd();
const branch='rocket-ui-test/shutdown';git(['init','-b',branch]);git(['config','user.name','Shutdown Fixture']);git(['config','user.email','fixture@example.invalid']);git(['config','commit.gpgsign','false']);
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);fs.writeFileSync(path.join(repo,'表示.rs'),'fn main() {}\n');git(['add','.']);git(['commit','-m','fixture']);
const head=git(['rev-parse','HEAD']),hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'),fileHash=hash(path.join(repo,'表示.rs')),configHash=hash(path.join(repo,'.git/config'));
const report={passed:false,root,cases:[],exe_hash:hash(exe),close_ms:[]};let service,client,state;
const delay=ms=>new Promise(r=>setTimeout(r,ms)),pass=s=>{report.cases.push(s);console.log('PASS '+s);};
async function until(predicate,label='state',timeout=30000){const end=performance.now()+timeout;for(;;){state=await client.tool('rocket_state');if(predicate(state))return;assert(performance.now()<end,label+' timeout');await delay(70);}}
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading),'settled');}
const tab=()=>state.tabs.find(t=>t.id===state.active_tab);
async function stable(){let prev='',count=0;await until(s=>{const next=JSON.stringify(s.controls);count=next===prev?count+1:0;prev=next;return count>=2;},'stable');}
async function click(target,settle=true){await stable();assert(state.controls.find(c=>c.id===target)?.enabled,'Missing '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});if(settle)await settled();}
async function type(target,text){await stable();await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settled();}
async function close(){const start=performance.now();await client.tool('rocket_close');await until(s=>s.window==='closed','closed');report.close_ms.push(Math.round(performance.now()-start));}
async function reopen(){await client.tool('rocket_open',{path:repo});await settled();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const img=r.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));}
(async()=>{try{
  cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
  const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
  const fd=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
  for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(n>120)throw e;await delay(100);}}
  report.version=(await client.initialize()).serverInfo.version;
  await reopen();await close();await reopen();pass('Idle close and reopen complete without a second request');
  await click('view-commits');await click('commit-'+head);await click('commit-open-review');await until(s=>s.review_windows.length===1);
  await click('view-changes');await type('commit-message','PRIVATE_SHUTDOWN_DRAFT');
  await client.tool('rocket_close');await until(s=>s.dialog?.kind==='quit');assert(!state.quitting&&tab().draft_present&&state.review_windows.length===1);
  await capture('quit-protection');await click('dialog-cancel');assert(!state.quitting&&tab().draft_present&&state.review_windows.length===1);pass('Cancel Quit preserves the draft and independent review');
  await client.tool('rocket_close');await until(s=>s.dialog?.kind==='quit');await click('dialog-submit',false);await until(s=>s.window==='closed');await reopen();
  assert(tab().draft_present);assert.equal(state.review_windows.length,0);pass('Confirmed Quit saves the draft and closes its child review');
  await click('view-changes');await type('commit-message','');await close();await reopen();assert(!tab().draft_present);pass('Clearing the draft remains saved across shutdown');
  assert.equal(git(['rev-parse','HEAD']),head);assert.equal(git(['status','--porcelain']), '');assert.equal(hash(path.join(repo,'表示.rs')),fileHash);assert.equal(hash(path.join(repo,'.git/config')),configHash);
  const dirs=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-'));const rows=dirs.flatMap(dir=>fs.readdirSync(path.join(root,dir)).filter(n=>n.endsWith('.jsonl')).flatMap(n=>fs.readFileSync(path.join(root,dir,n),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
  assert(rows.length>0);assert(!JSON.stringify(rows).includes('PRIVATE_SHUTDOWN_DRAFT'));assert(rows.every(r=>!r.dropped_before));report.log_records=rows.length;
  pass('Repository files/config remain unchanged; diagnostics omit draft text with zero dropped events');report.passed=true;
}catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client)try{await capture('failure');}catch{}}
finally{
  if(client)try{await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await click('dialog-submit',false);await until(s=>s.window==='closed');}await client.raw('POST','/shutdown');await client.finish();}catch(e){report.cleanup_error=String(e);}
  if(service){for(let i=0;i<100&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null){report.forced_cleanup=true;cp.execFileSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});}}
  const token=path.join(profile,'access-token');if(fs.existsSync(token))fs.unlinkSync(token);report.cleanup={service_exit:service?.exitCode,token_removed:!fs.existsSync(token)};
  if(report.cleanup_error||report.forced_cleanup||service?.exitCode!==0){report.passed=false;process.exitCode=1;}
  fs.mkdirSync('work/results/shutdown-cli-0.84',{recursive:true});fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync('work/results/shutdown-cli-0.84/native-mcp.json',JSON.stringify(report,null,2));console.log('Evidence: '+root);
}})();
