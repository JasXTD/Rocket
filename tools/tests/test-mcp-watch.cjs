'use strict';
// Real native watcher + HTTP MCP. Only synthetic repos/profiles and installed Git.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const {windows,exe:program}=require('./platform.cjs');
let root=path.resolve(process.env.ROCKET_MCP_RESULTS||'work/mcp-fixtures','watch-'+crypto.randomUUID().replaceAll('-',''));
fs.mkdirSync(root,{recursive:true,mode:0o700});root=fs.realpathSync(root);
const repo=path.join(root,'資料 repository'),profile=path.join(root,'http-profile');
const exe=path.resolve(process.env.ROCKET_MCP_EXE||program('target/release/rocket-mcp'));
const branch='rocket-ui-test/watch',file='資料 日本語 한국어.rs',report={passed:false,root,cases:[]};
const delay=ms=>new Promise(r=>setTimeout(r,ms)),hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
fs.mkdirSync(repo);fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
if(!windows){const home=path.join(root,'os-home');fs.mkdirSync(home,{mode:0o700});Object.assign(env,{HOME:home,XDG_CONFIG_HOME:path.join(home,'.config'),XDG_STATE_HOME:path.join(home,'.local/state')});}
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['ignore','pipe','pipe'],timeout:30000}).trimEnd();
git(['init','--initial-branch='+branch]);for(const [key,value]of [['user.name','Watcher Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false'],['core.hooksPath',path.join(root,'empty-template')]])git(['config',key,value]);
fs.writeFileSync(path.join(repo,file),'// PRIVATE_WATCH_BASE\n');git(['add','-A']);git(['commit','-m','Synthetic watch base']);
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const relay=gitRelay(root,env,'tools/helpers/productivity/git-relay.rs');
const baseline={head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config'))};
let state,client,service;
const active=()=>state?.tabs?.find(t=>t.id===state.active_tab),control=id=>state?.controls?.find(c=>c.id===id);
const settled=s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.background_refresh||t.search?.running);
const pass=name=>{report.cases.push(name);console.log('PASS '+name)};
function records(strict=false){return fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').split(/\r?\n/).filter(Boolean).flatMap(line=>{try{return [JSON.parse(line)]}catch(e){if(strict)throw e;return []}})))}
async function until(fn,label='UI state',timeout=30000){const start=performance.now();for(;;){state=await client.tool('rocket_state');assert(!state.startup_error&&!state.state_error,'UI initialization failed');if(fn(state))return;assert(performance.now()-start<timeout,label+' timed out');await delay(45)}}
async function settle(){await until(settled);assert(!active()?.error,active()?.error);assert(!active()?.patch_error,active()?.patch_error)}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return settled(s)&&count>=2},'stable controls')}
async function click(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle()}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'))}
async function close(){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit','close window');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.window==='closed')}}
async function open(){await client.tool('rocket_open',{path:repo});await settle();if(control('console-close'))await click('console-close');await click('view-changes')}
async function preferences(){await click('menu-edit');await click('preferences');await click('pref-page-general')}
async function toggleRefresh(){await preferences();await click('pref-auto-refresh');await click('dialog-submit');await until(s=>!s.dialog&&settled(s))}
async function idle(){await delay(500);await settle();const before=records().filter(r=>r.event==='git_start').length,count=active().refresh_count;await delay(1200);await settle();assert.equal(active().refresh_count,count,'idle refresh loop');assert.equal(records().filter(r=>r.event==='git_start').length,before,'idle Git subprocesses')}
async function run(){try{
 report.binary_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env:relay.env,windowsHide:true,stdio:['ignore','pipe','pipe']});
 const socket=net.createServer();await new Promise(r=>socket.listen(0,'127.0.0.1',r));const port=socket.address().port;await new Promise(r=>socket.close(r));fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>=100)throw e;await delay(100)}}
 await client.initialize();await open();relay.check(state);assert(state.preferences.auto_refresh&&active().watching,'Native watch must be active');assert(!active().warnings?.some(w=>/watch|notif|refresh/i.test(w)),JSON.stringify(active().warnings));await idle();
 pass('Native watcher starts without warnings and creates no idle Git loop');
 const row=part=>'change-'+part+'-'+Buffer.from(file).toString('hex');
 for(let i=0;i<20;i++)fs.writeFileSync(path.join(repo,file),'// 🦀 中文 PRIVATE_WATCH_EDIT '+i+'\n');
 await until(s=>settled(s)&&control(row('unstaged')),'external Unicode edit');await click(row('unstaged'));assert.equal(active().patch_added,1);
 fs.appendFileSync(path.join(repo,file),'// 🚀 PRIVATE_WATCH_SECOND\n');await until(s=>settled(s)&&active().patch_added===2,'selected diff refreshed twice');
 assert.equal(git(['rev-parse','HEAD']),baseline.head);assert.equal(hash(path.join(repo,'.git/index')),baseline.index);assert.equal(hash(path.join(repo,'.git/config')),baseline.config);
 await client.tool('rocket_resize',{width:780,height:620});await until(s=>s.viewport[0]===780&&s.viewport[1]===620&&settled(s),'compact viewport');await capture('01-unicode-diff');
 pass('Burst and repeated external edits refresh the selected Unicode diff in a compact window; HEAD/index/config stay intact');
 git(['add','--',file]);await until(s=>settled(s)&&control(row('staged')),'external stage');await click(row('staged'));assert.equal(active().patch_added,2);
 git(['commit','-m','Synthetic external watch commit']);const head=git(['rev-parse','HEAD']);await until(s=>settled(s)&&active().head?.oid===head&&active().change_count===0&&active().history_ids.includes(head),'external commit and history');
 pass('External index and HEAD changes update staging, history and repository summary');
 // Use a larger viewport for Preferences, then return to the compact diff.
 await client.tool('rocket_resize',{width:1200,height:620});await until(s=>s.viewport[0]===1200&&settled(s));
 await toggleRefresh();assert.equal(state.preferences.auto_refresh,false);assert.equal(active().watching,false);const count=active().refresh_count;
 fs.appendFileSync(path.join(repo,file),'// PRIVATE_WATCH_DISABLED\n');await delay(900);await settle();assert.equal(active().refresh_count,count);assert.equal(active().change_count,0);
 await toggleRefresh();await until(s=>settled(s)&&active().watching&&active().change_count===1,'re-enabled watch catches edit');await idle();
 pass('Disabling refresh releases the watcher; re-enabling catches accumulated edits without polling');
 // Hold a real owned Git invocation so cancellation proves process cleanup.
 const protectedPaths=['.git/HEAD','.git/index','.git/config',file];const preserved=protectedPaths.map(p=>hash(path.join(repo,p)));
 fs.writeFileSync(relay.gate,'before:status');const prior=new Set(records().map(r=>JSON.stringify(r)));fs.appendFileSync(path.join(repo,file),'// PRIVATE_WATCH_CANCEL\n');preserved[3]=hash(path.join(repo,file));
 await until(()=>fs.existsSync(relay.marker)&&active().background_refresh,'automatic refresh reaches Git');
 const at=performance.now();await close();await until(()=>records().some(r=>!prior.has(JSON.stringify(r))&&r.event==='git_end'&&r.outcome==='cancelled'),'cancelled process log');
 report.close_ms=Math.round(performance.now()-at);const stopped=records().filter(r=>!prior.has(JSON.stringify(r))&&r.event==='git_end'&&r.outcome==='cancelled');report.actual_stop_ms=stopped.filter(r=>r.cancel_to_stop_us!=null).map(r=>r.cancel_to_stop_us/1000);assert(report.actual_stop_ms.length,'No measured process stop');assert(report.close_ms<5000,'Close cancellation exceeded bound');
 fs.unlinkSync(relay.gate);assert.deepEqual(protectedPaths.map(p=>hash(path.join(repo,p))),preserved);
 await open();await until(s=>settled(s)&&active().watching&&active().change_count===1,'reopened watch');await click(row('unstaged'));assert.equal(active().patch_added,2);fs.appendFileSync(path.join(repo,file),'// PRIVATE_WATCH_REOPEN\n');await until(s=>settled(s)&&active().patch_added===3,'reopened selected diff');await close();
 pass('Closing an in-flight automatic refresh cancels Git and preserves files; reopening restores working notifications');
 const events=records(true);assert(events.length);assert(events.some(r=>r.operation==='auto_refresh'));assert(events.every(r=>!r.dropped_before));assert(!JSON.stringify(events).includes('PRIVATE_WATCH_'));report.events=events.length;
 pass('Refresh/cancellation diagnostics are present, with zero dropped events and no file payloads');report.passed=true;
}catch(error){report.error=error.stack;try{await capture('failure')}catch{}throw error}finally{
 try{if(service&&service.exitCode===null&&service.signalCode===null){if(state?.window==='running')await close();await client.raw('POST','/shutdown');const at=performance.now();while(service.exitCode===null&&service.signalCode===null){if(performance.now()-at>15000){service.kill();throw Error('Owned watcher test server did not stop')}await delay(50)}}}catch(error){report.passed=false;report.cleanup_error=error.stack;throw error}finally{if(client)await client.finish();fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log('RESULT '+path.join(root,'result.json'))}
}}
run().catch(error=>{console.error(error);process.exitCode=1});
