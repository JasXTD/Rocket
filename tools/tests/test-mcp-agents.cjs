// Local native acceptance. --real-codex opts in to one fixed read-only Astra prompt
// using the installed CLI and existing login; all repositories remain disposable.
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
// Resolve macOS's /var and /tmp aliases before applying the MCP fixture-root guard.
const root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'rocket-agent-'))),repo=path.join(root,'source 資料'),profile=path.join(root,'http-profile');
const reportDir=path.resolve(process.env.ROCKET_AGENTS_REPORT_ROOT||'tools/maintenance/agent-sessions-20261110','native-'+path.basename(root));fs.mkdirSync(reportDir,{recursive:true});
const windows=process.platform==='win32',macos=process.platform==='darwin';
// Hosted Mac displays clamp tall windows; use the proven native package-test
// size, while retaining the same exact 780x620 compact acceptance on all OSes.
const wideSize=macos?[1180,620]:[1480,920];
const exe=path.resolve(process.env.ROCKET_AGENTS_EXE||('target/release/rocket-mcp'+(windows?'.exe':'')));
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo);fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{ROCKET_AGENT_FIXTURE_ROOT:root,GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const git=(args,at=repo)=>cp.execFileSync('git',['-C',at,...args],{env,windowsHide:true,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trimEnd();
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
git(['init','--initial-branch=rocket-ui-test/agents']);for(const[k,v]of[['user.name','Synthetic Agent Test'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'資料.txt'),'original\n');fs.writeFileSync(path.join(repo,'.gitignore'),'ignored/\n');git(['add','.']);git(['commit','-m','Synthetic base']);
fs.writeFileSync(path.join(repo,'資料.txt'),'staged\n');git(['add','.']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');fs.writeFileSync(path.join(repo,'untracked.txt'),'preserve');
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),'rocket-ui-test/agents');fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const snapshot=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'untracked.txt'))});const before=snapshot();
const report={passed:false,platform:process.platform,architecture:process.arch,root,exe_sha256:hash(exe),cases:[],stop:[],viewports:[]};let client,service,state;
const wait=ms=>new Promise(r=>setTimeout(r,ms));const pass=s=>{report.cases.push(s);console.log('PASS '+s)};
async function until(fn,label='state',timeout=45000){const at=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-at<timeout,label+' timeout '+JSON.stringify({agents:state.agents,dialog:state.dialog?.kind,busy:state.busy,viewport:state.viewport}));await wait(65)}}
const control=id=>state.controls?.find(c=>c.id===id);const session=id=>state.agents.sessions.find(s=>s.id===id);
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.agents?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled')}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2},'stable')}
async function scroll(direction,pixels=350,target='agents-scroll'){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable()}
async function reveal(id){
 await stable();
 if(!state.agents.form&&['agents-remove','agents-forget'].includes(id)&&state.agents.pane!=='Details'){await click('agents-tab-details');}
 const visible=()=>control(id)?.rect[3]-control(id)?.rect[1]>18;
 const target=state.agents.form?'agents-scroll':id.startsWith('agents-session-')?'agents-list-scroll':id.startsWith('agents-change-')?'agents-changes-scroll':state.agents.pane==='Details'?'agents-details-scroll':null;
 if(!visible()&&target){await scroll('up',2000,target);for(let i=0;i<18&&!visible();i++)await scroll('down',180,target);}
 assert(visible(),'unreachable '+id);
}
async function click(id,settle=true){await reveal(id);assert(control(id)?.enabled,'disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(settle)await settled();else await until(s=>!s.input_pending)}
async function type(id,text){await reveal(id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settled()}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(reportDir,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(reportDir,name+'.json'),JSON.stringify(state,null,2))}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending,'resize '+width+'x'+height);report.viewports.push([...state.viewport])}
async function menuAction(menu,id){
 if(macos){await until(s=>s.native_menus?.some(row=>row.id===id&&row.enabled),'native menu '+id);const r=await client.tool('rocket_menu',{target:id,expected_tab:state.active_tab});assert(r.native_action_queued);await settled();}
 else{await click(menu);await click(id);}
}
function destination(n){const dst=path.join(root,'worktree 資料 '+n),branch='rocket-ui-test/agents-'+n;fs.writeFileSync(dst+'.rocket-worktree-fixture',branch+'\n'+repo);return{dst,branch}}
async function form(n,setup){const d=destination(n);await click('agents-new');await until(s=>s.agents.form,'New session form');await click('agents-provider-custom');await type('agents-name','PRIVATE task 資料 '+n);await type('agents-branch',d.branch);await type('agents-destination',d.dst);await type('agents-program',exe);await type('agents-arguments','--agent-session-fixture\nhold');if(setup){await click('agents-setup-enabled');await type('agents-setup-program',exe);await type('agents-setup-arguments','--agent-session-fixture\n'+setup);}return d}
async function review(){await click('agents-review');assert(state.agents.form?.reviewed,JSON.stringify(state.agents.form))}
async function start(){await click('agents-confirm');return state.agents.selected}
async function running(id){await until(s=>session(id)?.tail?.includes('SYNTHETIC_AGENT_READY')&&session(id)?.phase==='Running','running')}
function alive(pid){try{process.kill(pid,0);return true}catch{return false}}
async function stop(id){await click('agents-session-'+id);const child=Number(session(id).tail.match(/child=(\d+)/)[1]),pid=session(id).pid;const at=performance.now();await client.tool('rocket_click',{target:'agents-stop',expected_tab:state.active_tab});await until(s=>['Stopping','Stopped'].includes(session(id)?.phase),'stop acknowledgement');const ack=performance.now()-at;await until(s=>session(id)?.phase==='Stopped'&&!session(id)?.active,'process exit');for(let i=0;i<30&&(alive(pid)||alive(child));i++)await wait(50);assert(!alive(pid)&&!alive(child),'owned tree exited');report.stop.push({ack_ms:ack,closed_ms:performance.now()-at});}
async function close(){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window))}}
(async()=>{try{
cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));report.port=port;fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
const out=fs.openSync(path.join(reportDir,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);
for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>100)throw e;await wait(100)}}await client.initialize();await client.tool('rocket_open',{path:repo});await settled();await resize(...wideSize);if(control('console-close'))await click('console-close');
await click('toolbar-terminal');await until(s=>s.terminal?.running);await type('terminal-command',windows?"Write-Output ('ROCKET_' + 'TERMINAL_OK')":"printf 'ROCKET_%s\\n' TERMINAL_OK");await client.tool('rocket_terminal_enter',{expected_tab:state.active_tab});await until(s=>s.terminal?.tail?.includes('ROCKET_TERMINAL_OK'));await click('terminal-hide');pass('Ordinary repository terminal still accepts typed input and Enter');
await menuAction('menu-edit','preferences');await click('pref-page-appearance');await click('theme-clay-dark');await client.tool('rocket_escape');await settled();
await click('view-agents');await capture('empty');
await menuAction('menu-help','help-about');const hiddenInput=await client.request('tools/call',{name:'rocket_click',arguments:{target:'agents-new',expected_tab:state.active_tab}});assert(hiddenInput.isError,'modal dialog must block underlying Agents input');await client.tool('rocket_escape');await settled();assert(!state.agents.form);pass('Modal dialogs block underlying Agents controls');
let d=await form('cancel');await review();await capture('review-wide');await click('agents-cancel');assert(!fs.existsSync(d.dst));pass('Reviewed creation cancelled without a worktree or process');
d=await form('cancel-check');await click('agents-review',false);await client.tool('rocket_escape');await settled();assert(!state.agents.form);assert(!fs.existsSync(d.dst));pass('Cancelling validation creates no worktree');
d=await form('existing');fs.mkdirSync(d.dst);fs.writeFileSync(path.join(d.dst,'keep.txt'),'keep');await click('agents-review');assert(state.agents.form.error);assert.equal(fs.readFileSync(path.join(d.dst,'keep.txt'),'utf8'),'keep');await click('agents-cancel');pass('Existing destination refused and preserved');
d=await form('stale');await review();git(['branch',d.branch]);await start();assert.equal(session(state.agents.selected).phase,'Failed');assert(!fs.existsSync(d.dst));assert(git(['rev-parse',d.branch]));pass('Concurrent branch creation invalidates the preview');
const first=await form('one','setup');await review();const one=await start();await running(one);await capture('running-wide');
assert(!session(one).activity.includes('Restored'),'new session was not restored from disk');
assert(session(one).activity.includes('Saving session')&&session(one).activity.includes('Running'),'observed startup events');
if(process.argv.includes('--presentation-only')){
 await click('agents-tab-details');await capture('details-final');await click('agents-tab-terminal');
 await resize(780,620);await capture('terminal-final-compact');
 await stop(one);await close();await client.raw('POST','/shutdown');assert.deepEqual(snapshot(),before);
 pass('Final native presentation, observed activity, compact input and process cleanup');report.passed=true;return;
}
const second=await form('two');await review();const two=await start();await running(two);assert(session(one).active);assert.notEqual(session(one).pid,session(two).pid);assert.equal(git(['branch','--show-current'],first.dst),first.branch);assert.equal(git(['branch','--show-current'],second.dst),second.branch);pass('Successful setup followed by two simultaneous agent processes in separate Unicode worktrees');
await type('terminal-command','status');await client.tool('rocket_terminal_enter',{expected_tab:state.active_tab});await until(s=>session(two)?.tail?.includes('SYNTHETIC_AGENT_ALIVE'));pass('Interactive terminal input reaches the selected agent');
await resize(780,620);await reveal('terminal-command');const send=control('terminal-send');assert(send.rect[2]-send.rect[0]>=60,'compact Send button fully visible');assert(send.rect[2]<=state.viewport[0],'compact Send button stays inside window');await capture('terminal-compact');await type('terminal-command','status');await client.tool('rocket_terminal_enter',{expected_tab:state.active_tab});await until(s=>(session(two)?.tail?.match(/SYNTHETIC_AGENT_ALIVE/g)||[]).length>=2);await type('terminal-command','status');await click('terminal-send');await until(s=>(session(two)?.tail?.match(/SYNTHETIC_AGENT_ALIVE/g)||[]).length>=3);await resize(...wideSize);pass('Compact window keeps Send and the selected terminal visible and accepts keyboard and pointer input');

fs.writeFileSync(path.join(second.dst,'資料.txt'),'staged in agent\n');git(['add','--','資料.txt'],second.dst);
fs.appendFileSync(path.join(second.dst,'資料.txt'),'unstaged in agent\n');
fs.writeFileSync(path.join(second.dst,'新規 🧪.txt'),'external version one\nsecond line\nthird line\n');
await click('agents-tab-changes');await until(s=>s.agents.changes.owner===two&&s.agents.changes.loaded&&!s.agents.changes.busy,'worktree changes');
assert.equal(state.agents.changes.files.length,3);
const area=which=>state.agents.changes.files.find(f=>f.area===which).index;
await click('agents-change-'+area('Staged'));assert(state.agents.changes.patch.includes('+staged in agent'));assert(!state.agents.changes.patch.includes('+unstaged in agent'));
await click('agents-change-'+area('Unstaged'));assert(state.agents.changes.patch.includes('+unstaged in agent'));
await click('agents-change-'+area('Untracked'));assert(state.agents.changes.patch.includes('external version one'));
await capture('changes-wide');await resize(780,620);await capture('changes-compact');const selected=control('agents-change-'+state.agents.changes.selected);assert(selected&&selected.rect[3]-selected.rect[1]>=40,'selected file remains visible after resize');
for(const id of ['agents-stop','agents-tab-terminal','agents-tab-changes','agents-tab-details','agents-refresh','agents-open']){await reveal(id);const r=control(id).rect;assert(r[0]>=0&&r[2]<=state.viewport[0]&&r[3]<=state.viewport[1],id+' compact bounds');}
await resize(...wideSize);
fs.writeFileSync(path.join(second.dst,'新規 🧪.txt'),'external version two\nsecond line\nthird line\n');
await until(s=>!s.agents.changes.busy&&s.agents.changes.patch?.includes('external version two'),'external change notification');
await click('agents-refresh',false);await click('agents-read-stop',false);
await until(s=>!s.agents.changes.busy&&s.agents.changes.notice?.includes('cancelled'),'read cancellation');
assert(session(two).active,'read cancellation leaves agent running');
await click('agents-refresh');await until(s=>s.agents.changes.patch?.includes('external version two')&&!s.agents.changes.busy);
await click('agents-refresh',false);await click('agents-session-'+one,false);await until(s=>s.agents.changes.owner===one&&s.agents.changes.loaded&&!s.agents.changes.busy);
assert.equal(state.agents.changes.files.length,0);assert(!state.agents.changes.patch,'no stale patch from previous session');
await click('agents-session-'+two);await until(s=>s.agents.changes.owner===two&&s.agents.changes.loaded&&!s.agents.changes.busy);
assert.equal(state.agents.changes.files.length,3);
await type('agents-search','does-not-match-any-session');assert(!(state.controls||[]).some(c=>c.id.startsWith('agents-session-')));
await type('agents-search','');
await click('agents-filter-active');assert(state.controls.filter(c=>c.id.startsWith('agents-session-')).every(c=>session(Number(c.id.slice(15))).active));
await click('agents-filter-finished');assert(state.controls.filter(c=>c.id.startsWith('agents-session-')).every(c=>!session(Number(c.id.slice(15))).active));await click('agents-filter-all');
await click('agents-tab-details');await capture('details-wide');
await click('agents-tab-changes');
git(['restore','--staged','--worktree','--','資料.txt'],second.dst);fs.unlinkSync(path.join(second.dst,'新規 🧪.txt'));
pass('Selected worktree staged/unstaged/Unicode diffs, external edits, cancelled reads, session isolation, search and compact tabs');

const produced=path.join(second.dst,'agent-output 資料.txt');fs.writeFileSync(produced,'Synthetic agent change\n');
// Wait for this external edit to settle before clicking a header whose read
// progress controls can move while the watcher refreshes it. Never retry clicks.
await until(s=>!s.agents.changes.busy&&s.agents.changes.files?.some(f=>f.path==='agent-output 資料.txt'),'produced file observed');
await click('agents-open');await until(s=>{const t=s.tabs.find(t=>t.id===s.active_tab);return t?.view==='Changes'&&path.resolve(t.path).replace(/^\\\\\?\\/,'')===path.resolve(second.dst)&&!t.loading&&!s.input_pending},'worktree review opened');
const reviewed=state.tabs.find(t=>t.id===state.active_tab);assert.equal(reviewed.view,'Changes');assert.equal(path.resolve(reviewed.path).replace(/^\\\\\?\\/,''),path.resolve(second.dst));assert(session(two).active);await capture('review-agent-changes');await client.tool('rocket_open',{path:repo});await settled();await click('view-agents');fs.unlinkSync(produced);pass('Review changes opens the agent worktree without stopping its process');
await click('agents-tab-terminal');await stop(two);assert(session(one).active);await stop(one);pass('First-click Stop ends each owned process tree without stopping its neighbor');
const setupStop=await form('setup-stop','hold');await review();const stoppedSetup=await start();await until(s=>session(stoppedSetup)?.phase==='Setting up'&&session(stoppedSetup)?.tail?.includes('SYNTHETIC_AGENT_READY'));await stop(stoppedSetup);assert(fs.existsSync(setupStop.dst));await wait(500);await settled();assert.equal(session(stoppedSetup).phase,'Stopped');assert(!session(stoppedSetup).active);pass('Stopping setup ends its child and does not launch the agent; worktree retained');await click('agents-session-'+one);
fs.mkdirSync(path.join(first.dst,'ignored'));fs.writeFileSync(path.join(first.dst,'ignored/keep.txt'),'keep');await click('agents-remove');await click('agents-review');assert(state.agents.form.error);assert(fs.existsSync(path.join(first.dst,'ignored/keep.txt')));await click('agents-cancel');fs.unlinkSync(path.join(first.dst,'ignored/keep.txt'));fs.rmdirSync(path.join(first.dst,'ignored'));
await click('agents-remove');await review();fs.writeFileSync(path.join(first.dst,'external.txt'),'keep');await start();assert.equal(session(one).phase,'Failed');assert.equal(fs.readFileSync(path.join(first.dst,'external.txt'),'utf8'),'keep');fs.unlinkSync(path.join(first.dst,'external.txt'));await click('agents-remove');await review();await start();assert(!fs.existsSync(first.dst));assert(git(['rev-parse',first.branch]));pass('Ignored files and edits after removal preview prevent deletion; clean removal keeps branch');
d=await form('failure','fail');await review();const failed=await start();await until(s=>session(failed)?.phase==='Failed'&&!session(failed)?.active);assert(!session(failed).tail?.includes('SYNTHETIC_AGENT_READY'));assert(session(failed).tail?.includes('Synthetic setup failure'));assert(fs.existsSync(d.dst));await capture('setup-failed');await click('terminal-settings');await until(s=>s.preferences?.page==='terminal'&&s.controls?.some(c=>c.id==='pref-page-terminal'&&c.enabled),'terminal preferences rendered');assert.equal(state.preferences?.page,'terminal');assert(control('pref-page-terminal')?.enabled);await client.tool('rocket_escape');await settled();pass('Failed setup retains its worktree and error output; terminal settings action opens correctly');
await resize(780,620);await click('agents-restart');await reveal('agents-review');await capture('restart-compact');await review();await capture('review-compact');await click('agents-cancel');await resize(...wideSize);
await click('agents-session-'+two);await click('agents-tab-terminal');await click('agents-restart');await review();await start();await running(two);const pid=session(two).pid,child=Number(session(two).tail.match(/child=(\d+)/)[1]);await close();for(let i=0;i<40&&(alive(pid)||alive(child));i++)await wait(50);assert(!alive(pid)&&!alive(child));
await client.tool('rocket_open',{path:repo});await settled();await click('view-agents');assert(state.agents.sessions.every(s=>s.phase==='Restored'&&!s.active&&!s.pid));await capture('restored');pass('Compact review controls, shutdown child cleanup and persisted sessions without automatic relaunch');
if(process.argv.includes('--real-codex')){
 fs.writeFileSync(path.join(root,'rocket-agent-codex'),'read-only-codex-smoke-v1\n');
 const real=await form('real-codex');await click('agents-provider-codex');
 await type('agents-arguments','exec\n--ignore-user-config\n--ephemeral\n--sandbox\nread-only\n--model\ngpt-6-astra\nRespond with exactly ROCKET_AGENT_OK. Do not use tools, inspect files, modify anything, contact websites or delegate tasks.');
 await review();const id=await start();await until(s=>['Exited','Failed'].includes(session(id)?.phase)&&!session(id)?.active,'real Codex exit',120000);
 report.real_codex={phase:session(id).phase,exit_code:session(id).exit_code,reply_ok:session(id).tail?.split(/\r?\n/).some(line=>line.trim()==='ROCKET_AGENT_OK')===true};
 assert.equal(session(id).phase,'Exited',session(id).error||'Codex failed');assert.equal(session(id).exit_code,0);assert(report.real_codex.reply_ok,'real Codex response');assert.equal(git(['status','--porcelain'],real.dst),'');
 fs.unlinkSync(path.join(root,'rocket-agent-codex'));pass('Real installed Codex with Astra answers through the managed PTY, exits successfully and leaves the disposable worktree clean');
}
await close();await client.raw('POST','/shutdown');assert.deepEqual(snapshot(),before);pass('Original HEAD, index, config, staged/unstaged and untracked bytes unchanged');
const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/rocket.*\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
assert(records.length);assert(records.every(r=>r.dropped_before===0));const logs=JSON.stringify(records);for(const text of ['PRIVATE task','SYNTHETIC_AGENT_READY','--agent-session-fixture',first.dst])assert(!logs.includes(text),'log payload leak');
for(const op of ['agent-session-changes','agent-session-diff','agent-session-preview','agent-session-start','agent-session-remove','agent_started','agent_stop_requested','agent_stopped'])assert(records.some(r=>r.operation===op),'missing log step '+op);pass('Payload-free lifecycle diagnostics and no dropped records');report.passed=true;
}catch(e){report.failure=e.stack;try{await capture('failure')}catch{}console.error(e.stack);process.exitCode=1;}finally{
if(client){try{await client.raw('POST','/shutdown')}catch{}await client.finish();}if(service&&service.exitCode===null){await new Promise(r=>{const timer=setTimeout(()=>{service.kill();r()},5000);service.once('exit',()=>{clearTimeout(timer);r()})})}
// Retain only diagnostic timing fields, including on a failed run. Temporary
// filesystems can disappear after WSL exits; never copy terminal payloads here.
try{report.agent_timings=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/rocket.*\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse))).filter(r=>r.operation?.startsWith('agent')).map(({event,operation,request,elapsed_ms,outcome})=>({event,operation,request,elapsed_ms,outcome}));}catch(e){report.timing_capture_error=e.message;}
const token=path.join(profile,'access-token');if(fs.existsSync(token))fs.unlinkSync(token);report.token_removed=!fs.existsSync(token);fs.writeFileSync(path.join(reportDir,'result.json'),JSON.stringify(report,null,2));console.log(reportDir);
}})();
