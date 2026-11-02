// Native acceptance through authenticated loopback MCP, using only owned local fixtures.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {checkLogs}=require('./auto-bisect-evidence.cjs');
const root=path.resolve('work/mcp-fixtures/auto-bisect-'+crypto.randomUUID().replaceAll('-','')),profile=path.join(root,'http-profile'),branch='rocket-ui-test/auto-bisect';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report={passed:false,version,root,cases:[],ui_actions:[]},fixtures=[],FILE='版本 文.txt';
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'),delay=ms=>new Promise(r=>setTimeout(r,ms));
function fixture(name,count=7){
 const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
 const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trimEnd();
 const write=(name,body)=>{const p=path.join(repo,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,body)};
 const commit=message=>{git(['add','.']);git(['commit','-m',message]);return git(['rev-parse','HEAD'])};
 git(['init','--initial-branch='+branch]);for(const [k,v]of [['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',k,v]);
 write('.gitignore','ignored/\nignored.txt\n');const ids=[];for(let i=0;i<count;i++){write(FILE,i+'\n');ids.push(commit('PRIVATE_BISECT version '+i));}
 fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);const f={repo,hooks,git,write,commit,ids,config:hash(path.join(repo,'.git/config'))};fixtures.push(f);return f;
}
let client,service,state;
const passed=text=>{report.cases.push(text);console.error('PASS '+text)};
async function until(predicate,label='UI state'){
 const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<90000,label+' timed out; '+JSON.stringify({dialog:state.dialog,write:state.write,notice:state.notice}));await delay(100);}
}
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!s.write&&!s.dialog?.bisect?.loading&&!s.dialog?.bisect?.applying&&!(s.tabs||[]).some(t=>t.loading||t.search.running));}
const active=()=>state.tabs.find(t=>t.id===state.active_tab),control=id=>state.controls.find(c=>c.id===id),latest=()=>state.activities.at(-1),session=()=>state.dialog?.bisect?.session;
async function stable(){let last='',n=0;await until(s=>{let next=JSON.stringify(s.controls);n=next===last?n+1:0;last=next;return n>=3},'Stable controls');}
async function scroll(target,direction='down',pixels=500){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settle();await stable();}
async function visible(id){
 await stable();if(control(id)?.enabled)return;
 if(state.dialog?.kind==='bisect'){await scroll('dialog-bisect-scroll','up',2000);for(let i=0;i<6&&!control(id)?.enabled;i++)await scroll('dialog-bisect-scroll','down',220);}
 assert(control(id)?.enabled,'Missing/disabled '+id);
}
async function click(target){await visible(target);report.last_action={target,dialog:state.dialog?.kind,rect:control(target)?.rect};const operation=target==='dialog-submit'?state.dialog?.bisect?.preview?.action:null,start=performance.now();await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();if(operation)report.ui_actions.push({operation,elapsed_ms:Math.round(performance.now()-start)});}
async function type(target,text){await visible(target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const img=r.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();await stable();}
async function menu(){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);await click('repository-history-menu');await stable();for(let n=0;n<8;n++){const c=control('repository-bisect');if(c?.enabled&&c.rect[3]-c.rect[1]>=28)break;await stable();}await click('repository-bisect');assert.equal(state.dialog.kind,'bisect');}
async function open(f){if(state?.dialog)await click('dialog-cancel');console.error('OPEN '+path.basename(f.repo));await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await menu();}
async function reviewRange(f,good=f.ids[0],bad=f.ids.at(-1)){await type('dialog-bisect-good-input',good);await type('dialog-bisect-bad-input',bad);await click('dialog-preview-refresh');assert(state.dialog.bisect.preview,JSON.stringify(state.dialog));}
async function apply(){await click('dialog-submit');}
async function step(action){await click('dialog-bisect-'+action);assert.equal(state.dialog.bisect.preview?.action,'bisect-'+action);await apply();}
async function restore(){await step('restore');assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(session(),null);}
async function closeWindow(){
 if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');
 await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');
 if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}
}
function snapshot(f){return {head:f.git(['rev-parse','HEAD']),index:f.git(['ls-files','--stage']),file:fs.readFileSync(path.join(f.repo,FILE),'utf8')};}
const auto=()=>state.dialog?.bisect?.automatic;
const helper=path.join(root,'test-runner.exe');
fs.copyFileSync(path.resolve('tools/helpers/bin/auto-bisect/test-runner.exe'),helper);
async function configure(f,args='version',seconds='20'){
 if(!auto().enabled)await click('dialog-bisect-auto');
 if(!session()){await type('dialog-bisect-good-input',f.ids[0]);await type('dialog-bisect-bad-input',f.ids.at(-1));}
 await type('dialog-bisect-program',helper);await type('dialog-bisect-timeout',seconds);await type('dialog-bisect-arguments',args);
 await click('dialog-preview-refresh');assert(auto().reviewed,JSON.stringify(state.dialog));
}
async function startHeld(marker){await visible('dialog-submit');await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>fs.existsSync(marker)&&s.write&&!s.input_pending,'test command started');}
function alive(pid){try{process.kill(pid,0);return true}catch(e){assert.equal(e.code,'ESRCH');return false}}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
 const port=await new Promise(resolve=>{const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})});fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
 const log=fs.openSync(path.join(root,'service.stderr.log'),'a');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 const normal=fixture('PRIVATE_NORMAL 中文');normal.write('untracked 文','PRIVATE_UNTRACKED');normal.write('ignored/output','PRIVATE_IGNORED');const original=snapshot(normal);
 await open(normal);await configure(normal);assert.deepEqual(snapshot(normal),original);await capture('auto-review-wide');
 await type('dialog-bisect-timeout','21');assert(!auto().reviewed);assert(!control('dialog-submit')?.enabled);await click('dialog-preview-refresh');assert(auto().reviewed);
 await resize(780,620);for(const id of ['dialog-submit','dialog-preview-refresh','dialog-cancel']){const r=control(id).rect;assert(r[0]>=0&&r[1]>=0&&r[2]<=780&&r[3]<=620,id+' clipped')}
 await capture('auto-review-compact');await resize(1480,920);await click('dialog-cancel');assert.deepEqual(snapshot(normal),original);assert(!fs.existsSync(path.join(normal.repo,'.git/rocket-bisect')));
 passed('Reviewed command and exact arguments, invalidation on edits, compact footer and Close without writes');
 await menu();await configure(normal);await apply();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(session().phase,'Found');assert.equal(session().target,normal.ids[4]);assert(auto().results.every(r=>r.recorded&&r.stdout_bytes>0));await capture('auto-found');
 await restore();assert.deepEqual(snapshot(normal),original);assert.equal(fs.readFileSync(path.join(normal.repo,'untracked 文'),'utf8'),'PRIVATE_UNTRACKED');assert.equal(fs.readFileSync(path.join(normal.repo,'ignored/output'),'utf8'),'PRIVATE_IGNORED');
 passed('Automatic Good/Bad finds the exact first bad commit and restores branch/index/Unicode files while keeping untracked and ignored files');
 const skip=fixture('PRIVATE_SKIP');await open(skip);await configure(skip,'skip');await apply();assert(latest().command_succeeded);assert.equal(session().phase,'Inconclusive');assert(session().candidate_count>1);assert(auto().results.every(r=>r.outcome==='Skip'&&r.recorded));await capture('auto-inconclusive');await restore();
 passed('Exit 125 skips revisions and reports multiple possible culprits without a false answer');
 const failed=fixture('PRIVATE_FAILED');await open(failed);await configure(failed,'exit\n127');await apply();assert(!latest().command_succeeded);assert.equal(auto().results.length,1);assert.equal(auto().results[0].exit,127);assert(!auto().results[0].recorded);assert.match(state.dialog.bisect.notice,/127/);await capture('auto-command-error');await restore();
 passed('Exit 127 is a command error, not a bad revision; clear failure and safe restoration');
 const timeout=fixture('PRIVATE_TIMEOUT'),timedPid=path.join(root,'timeout-child.pid');await open(timeout);await configure(timeout,'hold\n'+timedPid,'1');await apply();assert(!latest().command_succeeded);assert.equal(auto().results[0].outcome,'Timed out');assert(!auto().results[0].recorded);assert(!alive(Number(fs.readFileSync(timedPid,'utf8'))));assert(!alive(Number(fs.readFileSync(timedPid+'.parent','utf8'))));await capture('auto-timeout');await restore();
 passed('Per-revision timeout stops the test and its child, keeps the revision unclassified and allows restore');
 const external=fixture('PRIVATE_EXTERNAL'),ready=path.join(root,'external-ready'),release=path.join(root,'external-release');await open(external);await configure(external,'gate\n'+ready+'\n'+release);await startHeld(ready);external.write(FILE,'PRIVATE_EXTERNAL_EDIT');fs.writeFileSync(release,'done');await settle();assert(!latest().command_succeeded);assert.equal(auto().results[0].outcome,'Checkout changed');assert(!auto().results[0].recorded);assert.equal(fs.readFileSync(path.join(external.repo,FILE),'utf8'),'PRIVATE_EXTERNAL_EDIT');await click('dialog-bisect-restore');assert.match(state.dialog.error,/bisect_dirty_worktree/);await capture('auto-external-edit');
 external.write('preserved-external-edit.txt',fs.readFileSync(path.join(external.repo,FILE)));external.git(['restore','--',FILE]);await restore();assert.equal(fs.readFileSync(path.join(external.repo,'preserved-external-edit.txt'),'utf8'),'PRIVATE_EXTERNAL_EDIT');
 passed('An external tracked edit stops classification and blocks restoration until edits are preserved');
 const cancelled=fixture('PRIVATE_CANCEL'),child=path.join(root,'cancel-child.pid');await open(cancelled);await configure(cancelled,'hold\n'+child,'60');await startHeld(child+'.parent');const pids=[child,child+'.parent'].map(p=>Number(fs.readFileSync(p,'utf8')));
 const start=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await until(s=>!s.input_pending&&(s.write?.cancelling||!s.write),'cancel feedback');report.cancel_feedback_ms=Math.round(performance.now()-start);
 await until(()=>pids.every(p=>!alive(p)),'test processes stopped');report.cancel_process_stop_ms=Math.round(performance.now()-start);await settle();report.cancel_settled_ms=Math.round(performance.now()-start);assert(!latest().command_succeeded);assert.equal(auto().results[0].outcome,'Cancelled');assert(!auto().results[0].recorded);const target=session().target;await capture('auto-cancelled');
 await click('dialog-cancel');await closeWindow();await client.tool('rocket_open',{path:cancelled.repo});await settle();await menu();assert.equal(session().target,target);assert.equal(auto().results[0].outcome,'Cancelled');assert.equal(auto().program,'');assert.equal(auto().results[0].stdout_bytes,0);assert(!auto().reviewed);
 await configure(cancelled);await apply();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(session().target,cancelled.ids[4]);await restore();
 passed('Cancel gives immediate feedback, stops both owned processes, survives window reopening and resumes only after a fresh review');
 const moved=fixture('PRIVATE_MOVED'),ready2=path.join(root,'moved-ready'),release2=path.join(root,'moved-release');await open(moved);await configure(moved,'gate\n'+ready2+'\n'+release2);await startHeld(ready2);moved.git(['update-ref','refs/heads/'+branch,moved.ids[0]]);fs.writeFileSync(release2,'done');await settle();assert(!auto().results[0].recorded);assert(!latest().command_succeeded);await click('dialog-bisect-restore');assert.match(state.dialog.error,/bisect_original_branch_moved/);assert.equal(moved.git(['rev-parse',branch]),moved.ids[0]);moved.git(['update-ref','refs/heads/'+branch,moved.ids.at(-1)]);await restore();
 passed('An externally moved original branch is preserved; test classification and restoration both refuse the changed state');
 for(const f of fixtures)assert.equal(hash(path.join(f.repo,'.git/config')),f.config);
 await closeWindow();await client.raw('POST','/shutdown');const end=performance.now()+15000;while(service.exitCode===null&&performance.now()<end)await delay(30);assert.equal(service.exitCode,0,'clean MCP shutdown');
 // Profiles intentionally keep credentials across service restarts. This runner
 // owns a disposable profile and removes its token after confirmed shutdown.
 fs.rmSync(path.join(profile,'access-token'),{force:true});assert(!fs.existsSync(path.join(profile,'access-token')));report.cleanup={exit:service.exitCode,token_removed:true};
 report.log_records=checkLogs(root);passed('Private structured test outcomes, durations, clear refusals and paired requests; zero dropped logs and clean service shutdown');report.passed=true;
}catch(e){report.error=e.stack;if(client)try{await capture('failure')}catch{}throw e}finally{
 fs.writeFileSync('work/auto-bisect-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(200);if(service.exitCode===null){report.forced_cleanup=true;service.kill()}}if(client)await client.finish();fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));
}}
run().catch(e=>{console.error(e);process.exitCode=1});
