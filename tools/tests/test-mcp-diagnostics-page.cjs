// Native acceptance of the Activity & diagnostics page through authenticated localhost MCP.
// Everything runs in a disposable fixture; seeded records stay in its own log folder.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/diagnostics-'+crypto.randomUUID().slice(0,12));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/diagnostics';
const report={passed:false,root,cases:[]};
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
let client,service,state;
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const passed=text=>{report.cases.push(text);console.error('PASS '+text);};
const control=id=>state.controls.find(c=>c.id===id),history=()=>state.activity_history;
async function until(predicate,label='UI state'){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<45000,label+' timed out');await delay(80);}}
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!s.activity_history?.loading&&!(s.tabs||[]).some(t=>t.loading));}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=3;},'Stable widget geometry');}
async function click(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();await stable();}
async function type(target,text){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();await stable();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(result.content.find(c=>c.type==='image').data,'base64'));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending,'Resize');await settle();}
async function openPage(){await click('menu-view');await click('activity-open-menu');await settle();}
const session='ac71-123-1';
const row=(seq,request,event,extra={})=>({schema:1,session,seq,request,time_unix_ms:Date.now()-3600000+seq*1000,event,operation:'stage',dropped_before:0,...extra});
function seed(){
 const records=[];let seq=0;
 const add=(request,event,extra)=>records.push(row(++seq,request,event,extra));
 add(900,'request_start',{id:1,operation:'fetch'});
 add(900,'step_start',{id:2,operation:'discover',step:'verify_git'});
 add(900,'git_start',{id:3,operation:'--version',kind:'read'});
 add(900,'git_end',{id:3,operation:'--version',elapsed_ms:70,outcome:'exited',exit_code:0});
 add(900,'step_end',{id:2,operation:'discover',step:'verify_git',elapsed_ms:72,outcome:'ok'});
 add(900,'git_start',{id:5,operation:'config',kind:'read'});
 add(900,'git_end',{id:5,operation:'config',elapsed_ms:60,outcome:'exited',exit_code:1});
 add(900,'git_start',{id:4,operation:'fetch',kind:'read'});
 add(900,'git_end',{id:4,operation:'fetch',elapsed_ms:2140,outcome:'exited',exit_code:128});
 add(900,'request_end',{id:1,operation:'fetch',elapsed_ms:2310,outcome:'git_failed',failure_code:'authentication_rejected',exit_code:128,level:'warn',stdout:'PRIVATE_VIEWER_SECRET'});
 add(901,'request_start',{operation:'fetch'});add(901,'request_end',{operation:'fetch',outcome:'cancelled',elapsed_ms:5});
 add(902,'request_start',{operation:'pull'});
 add(903,'request_start',{operation:'status'});add(903,'request_end',{operation:'status',outcome:'ok',elapsed_ms:9});
 add(904,'request_start',{operation:'commit'});add(904,'request_end',{operation:'commit',outcome:'ok',elapsed_ms:300});
 add(905,'request_start',{operation:'tag-preview'});
 add(905,'request_end',{operation:'tag-preview',outcome:'tag_refused',failure_code:'tag_invalid_message',level:'warn',elapsed_ms:30});
 return records.map(r=>JSON.stringify(r)+'\n').join('');
}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 await client.initialize();
 const repo=path.join(root,'repo');fs.mkdirSync(repo);const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8'});
 git(['init','--initial-branch='+branch]);git(['config','user.name','Rocket Fixture']);git(['config','user.email','rocket@example.invalid']);fs.writeFileSync(path.join(repo,'a.txt'),'a\n');git(['add','.']);git(['commit','-m','First']);fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
 await client.tool('rocket_open',{path:repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');
 const file=path.join(state.log_directory.replace(/^\\\\\?\\/,''),'rocket.4.jsonl');assert(!fs.existsSync(file));

 fs.writeFileSync(file,'not a record\n');await openPage();await until(s=>s.activity_history.error,'First read fails');
 assert.match(history().error,/damaged/);assert(!history().error.includes('invalid input'));assert(control('activity-retry'));assert(!control('activity-list'));assert(!control('activity-guide'),'no log status without a log');await capture('d-first-read-failed');
 fs.writeFileSync(file,seed());await click('activity-retry');assert(!history().error);assert(control('activity-list'));
 passed('A failed first read is one state with Try again, not a list, a green status and a red line at once');

 const entry=(operation,status)=>history().rows.find(r=>r.operation===operation&&(!status||r.status===status));
 assert(entry('status'),'MCP state lists reads');assert(!control('activity-row-'+entry('status').index),'a read that went fine is left out');
 assert(control('activity-row-'+entry('commit').index));assert(control('activity-row-'+entry('fetch','Needs attention').index));
 await capture('d-reads-hidden');await click('activity-reads');assert(control('activity-row-'+entry('status').index));await capture('d-reads-shown');await click('activity-reads');
 await type('activity-search','status');assert(control('activity-row-'+entry('status').index),'a search looks at reads');await type('activity-search','');
 passed('Reads that went fine stay out of the list, one click brings them back and a search always finds them');

 await click('activity-row-'+entry('fetch','Needs attention').index);assert.equal(history().selected.operation,'fetch');assert(!JSON.stringify(history().selected).includes('PRIVATE'));
 await capture('d-failed');await click('activity-setup');await capture('d-setup-open');await click('activity-setup');
 await click('activity-view-records');assert(history().records_view);await capture('d-records');await click('activity-view-steps');assert(!history().records_view);
 await click('activity-row-'+entry('tag-preview').index);await capture('d-refused');
 passed('A failure leads with its own words and where it stopped; Steps fold setup and Records keep the full timeline');

 await type('activity-search','zzz');assert(control('activity-clear'));assert(!control('activity-list'));await capture('d-no-match');await click('activity-clear');assert.equal(history().query,'');assert(control('activity-list'));
 const fold=state.controls.findLast(c=>c.id.startsWith('activity-group-'));assert(fold,'Earlier session group');const row0=entry('commit').index;await click(fold.id);assert(!control('activity-row-'+row0));await capture('d-folded');await click(fold.id);assert(control('activity-row-'+row0));
 passed('No match clears in one click, and a session folds and opens');

 fs.appendFileSync(file,'not a record\n');await click('activity-refresh');await until(s=>s.activity_history.error,'Later read fails');assert(control('activity-retry'));assert(control('activity-list'),'the last good list stays');await capture('d-read-failed');
 fs.writeFileSync(file,seed());await click('activity-retry');assert(!history().error);
 passed('A later failed read keeps the list, says so and retries');

 const seen=new Set();const start=performance.now();while(performance.now()-start<12000){const s=await client.tool('rocket_state');const c=s.controls.find(c=>c.id==='activity-auto');if(c)seen.add(Math.round(c.rect[0]));await delay(40);}
 assert.equal(seen.size,1,'Live moved the header: '+[...seen]);
 passed('Live reads leave the header where it is');

 await resize(780,620);assert(control('activity-tab-list'));assert(control('activity-list'));await capture('d-narrow-list');await click('activity-row-'+entry('fetch','Needs attention').index);assert(control('activity-details'));assert(!control('activity-list'));await capture('d-narrow-details');await click('activity-tab-list');await resize(1100,760);await capture('d-mid');await resize(920,720);await capture('d-920');await resize(1480,920);
 passed('A narrow window shows Operations and Details as tabs, and a row opens Details');

 await click('activity-close');assert(!history().open);if(fs.existsSync(file))fs.unlinkSync(file);
 await require('./activity-native-checks.cjs')({client,root,getState:()=>state,settle,click,capture,pass:passed,report});
 report.passed=true;
}catch(error){report.error=String(error);if(client)try{await capture('failure');}catch{}throw error;}
finally{
 fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit','Close window');if(state.dialog?.kind==='quit')await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}
 if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});
