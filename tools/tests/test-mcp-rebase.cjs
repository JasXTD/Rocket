// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('work/mcp-fixtures/rebase-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/rebase';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const focused=process.argv.includes('--late-only');
const report={passed:false,version,root,focused,cases:[]},fixtures=[];
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const relay=gitRelay(root,env,'tools/helpers/productivity/git-relay.rs'),{gate,marker}=relay;
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function fixture(name){
  const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
  const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8'}).trimEnd();
  const write=(name,body)=>{const p=path.join(repo,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,body);};
  const commit=message=>{git(['add','.']);git(['commit','-m',message]);return git(['rev-parse','HEAD']);};
  git(['init','--initial-branch='+branch]);
  for(const [key,value]of [['user.name','Rocket Commit Fixture'],['user.email','rocket@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',key,value]);
  write('base.txt','base\n');commit('PRIVATE_SOURCE baseline 中文');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
  const f={repo,hooks,git,write,commit,config:hash(path.join(repo,'.git/config'))};fixtures.push(f);return f;
}
let client,service,state;
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const passed=text=>{report.cases.push(text);console.error('PASS '+text);};
async function until(predicate,label='UI state'){
  const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<45000,label+' timed out; '+JSON.stringify({dialog:state.dialog,write:state.write,notice:state.notice}));await delay(80);}
}
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running));}
function active(){return state.tabs.find(t=>t.id===state.active_tab);}
function control(id){return state.controls.find(c=>c.id===id);}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=3;},'Stable widget geometry');}
async function click(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();}
async function type(target,text){await stable();assert(control(target)?.enabled,'Missing/disabled input '+target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const image=result.content.find(c=>c.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function scroll(target,direction='down',pixels=600){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settle();await stable();}
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();relay.check(state);await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function submit(){await click('dialog-submit');await settle();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}


const FILE='src/検索/結果 🐦 é.rs';
function setup(name,conflict=false){
 const f=fixture(name);f.write(FILE,'base\n');f.write('.gitignore','ignored/\n');f.commit('PRIVATE_BASE 中文');
 f.git(['branch','main']);f.write(FILE,'feature\n');f.commit('PRIVATE_SOURCE feature');f.write('second','second\n');f.commit('PRIVATE_SOURCE second');f.git(['branch','other']);
 f.git(['switch','main']);if(conflict)f.write(FILE,'main\n');else f.write('upstream','upstream\n');f.commit('PRIVATE_DESTINATION');f.git(['switch',branch]);return f;
}
function snapshot(f){return {head:f.git(['rev-parse','HEAD']),index:f.git(['ls-files','--stage']),file:fs.readFileSync(path.join(f.repo,FILE),'utf8'),config:hash(path.join(f.repo,'.git/config'))};}
async function plan(f){await click('menu-repository');await click('repository-branch-menu');await stable();await click('repository-rebase');await type('dialog-rebase-target','main');await click('dialog-preview-refresh');await stable();assert(state.dialog.rebase_preview_loaded,JSON.stringify(state.dialog));}
async function recover(action){await click('operation-'+action);await stable();}
async function finishDialog(){if(state.dialog)await click('dialog-cancel');}
// MCP runs Git with hooks off (SECURITY.md), so the relay holds the real command for Stop. The gate stays set until
// Stop has settled: a held command it failed to end would never finish, and settle would time out.
function hold(point){fs.rmSync(marker,{force:true});fs.writeFileSync(gate,point);}
function release(){fs.rmSync(gate,{force:true});}
async function moveOthers(){for(let i=0;i<8&&!control('dialog-rebase-update-refs')?.enabled;i++)await scroll('dialog-rebase-scroll','down',240);await click('dialog-rebase-update-refs');assert.equal(state.dialog.rebase_update_refs.choice,true);}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 if(!focused){
 const linear=setup('PRIVATE_LINEAR');linear.write('private','PRIVATE_UNTRACKED\n');linear.write('ignored/cache','PRIVATE_IGNORED\n');
 for(const [key,value] of [['rebase.updateRefs','true'],['rebase.autoStash','true'],['rebase.autosquash','true']])linear.git(['config',key,value]);linear.config=hash(path.join(linear.repo,'.git/config'));
 const original=snapshot(linear),other=linear.git(['rev-parse','other']),destination=linear.git(['rev-parse','main']);await open(linear);
 await click('menu-repository');await click('repository-branch-menu');await stable();await click('repository-rebase');await type('dialog-rebase-target','main');await client.tool('rocket_click',{target:'dialog-preview-refresh',expected_tab:state.active_tab});
 await until(s=>s.dialog?.rebase_preview_loading);const readCancel=performance.now();await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();report.read_cancel_ms=performance.now()-readCancel;assert(report.read_cancel_ms<10000);assert.equal(state.dialog,null);assert.deepEqual(snapshot(linear),original);
 await plan(linear);assert.equal(state.dialog.rebase_preview.commits.length,2);await capture('rebase-preview-wide');
 await resize(780,620);await capture('rebase-preview-narrow');await click('dialog-cancel');assert.deepEqual(snapshot(linear),original);
 await click('menu-terminal');await click('terminal-new');await plan(linear);await capture('rebase-preview-narrow-terminal');await click('dialog-cancel');
 if(process.argv.includes('--preview-only')){passed('Native rebase preview at wide/narrow sizes with terminal and cancelled confirmation');report.passed=true;return;}
 await resize(1480,920);await plan(linear);linear.git(['update-ref','refs/heads/main','main~']);const moved=snapshot(linear);await submit();assert(state.dialog.error.includes('stale_preview'));assert.deepEqual(snapshot(linear),moved);await capture('stale-preview');
 await click('dialog-cancel');linear.git(['update-ref','refs/heads/main',destination]);await plan(linear);const old=linear.git(['rev-parse','HEAD']);await submit();assert(latest().command_succeeded,JSON.stringify(latest()));
 assert.equal(linear.git(['rev-parse','other']),other);assert.equal(linear.git(['rev-parse','refs/rocket/recovery/rebase/'+old]),old);assert.equal(linear.git(['rev-list','--count','main..HEAD']),'2');
 assert.equal(fs.readFileSync(path.join(linear.repo,'private'),'utf8'),'PRIVATE_UNTRACKED\n');assert.equal(fs.readFileSync(path.join(linear.repo,'ignored/cache'),'utf8'),'PRIVATE_IGNORED\n');
 passed('Real native preview, target pinning, cancellation and stale refusal; linear rebase preserves unrelated files, other branches, config and recovery tip');

 const continued=setup('PRIVATE_CONTINUE',true);await open(continued);await plan(continued);await submit();assert(active().operation_state.rebase);assert(!latest().command_succeeded);await capture('rebase-conflict');
 await recover('continue');assert(state.dialog.error.includes('conflicts_remain'));assert(!control('dialog-submit').enabled);await click('dialog-cancel');
 // Detached recovery scope must still reject an unrelated fixture branch marker.
 const headName=path.join(continued.repo,'.git/rebase-merge/head-name'),savedName=fs.readFileSync(headName);fs.writeFileSync(headName,'refs/heads/main\n');
 await assert.rejects(client.tool('rocket_click',{target:'operation-abort',expected_tab:state.active_tab}),/fixture ownership|Active branch/);fs.writeFileSync(headName,savedName);
 const todo=path.join(continued.repo,'.git/rebase-merge/git-rebase-todo'),savedTodo=fs.readFileSync(todo);fs.writeFileSync(todo,'pick '+continued.git(['rev-parse','main'])+' PRIVATE_REPLACEMENT\n');
 await recover('skip');assert(state.dialog.error.includes('unsupported_rebase'));assert(!control('dialog-submit').enabled);await capture('modified-plan-refusal');await click('dialog-cancel');fs.writeFileSync(todo,savedTodo);
 await click('view-changes');await click('change-unstaged-'+Buffer.from(FILE).toString('hex'));await click('conflict-open');await type('conflict-result','PRIVATE_RESOLUTION 中文 é 🚀\n');await click('conflict-stage');await submit();assert(latest().command_succeeded);await click('conflict-back');
 await recover('continue');assert(state.dialog.rebase_preview_loaded);const beforeContinue=snapshot(continued);await capture('continue-preview');await click('dialog-cancel');assert.deepEqual(snapshot(continued),beforeContinue);
 await recover('continue');await submit();assert(!active().operation_state.rebase);assert.equal(fs.readFileSync(path.join(continued.repo,FILE),'utf8'),'PRIVATE_RESOLUTION 中文 é 🚀\n');assert(fs.existsSync(path.join(continued.repo,'second')));
 passed('Actual detached rebase conflict, owned-branch guard, Unicode conflict editor resolution, staged Continue preview and completion');

 for(const action of ['skip','abort']){
   const f=setup('PRIVATE_'+action.toUpperCase(),true),original=f.git(['rev-parse','HEAD']);await open(f);await plan(f);await submit();assert(active().operation_state.rebase);
   f.write(FILE,'PRIVATE_TRACKED_RESOLUTION\n');f.write('local-keep','PRIVATE_KEEP\n');await refresh();await recover(action);assert(state.dialog.rebase_preview_loaded);const before=snapshot(f);await capture(action+'-preview');await click('dialog-cancel');assert.deepEqual(snapshot(f),before);
   await recover(action);f.write(FILE,'PRIVATE_EXTERNAL_EDIT\n');await submit();assert(state.dialog.error.includes('stale_preview'));assert.equal(fs.readFileSync(path.join(f.repo,FILE),'utf8'),'PRIVATE_EXTERNAL_EDIT\n');await click('dialog-preview-refresh');await submit();assert(!active().operation_state.rebase);assert(latest().command_succeeded);
   if(action==='abort')assert.equal(f.git(['rev-parse','HEAD']),original);else{assert.equal(fs.readFileSync(path.join(f.repo,FILE),'utf8'),'main\n');assert(fs.existsSync(path.join(f.repo,'second')));}
   assert.equal(fs.readFileSync(path.join(f.repo,'local-keep'),'utf8'),'PRIVATE_KEEP\n');assert.equal(f.git(['rev-parse','refs/rocket/recovery/rebase/'+original]),original);
   passed(action+' is explicitly reviewed; cancel preserves state, changed tracked files invalidate preview, untracked data and original tip survive');
 }
 const empty=setup('PRIVATE_EMPTY',true);empty.git(['switch','main']);empty.write(FILE,'feature\n');empty.commit('same patch');empty.git(['switch',branch]);await open(empty);await plan(empty);await submit();assert(active().operation_state.rebase);assert(latest().details.some(d=>d.includes('empty_change')));await recover('continue');assert(state.dialog.error.includes('empty_change'));await click('dialog-cancel');await recover('skip');await submit();assert(latest().command_succeeded);assert.equal(empty.git(['rev-list','--count','main..HEAD']),'1');
 passed('Empty replay pauses with an explicit error and is omitted only by reviewed Skip');

 const guards=setup('PRIVATE_GUARDS');await open(guards);guards.git(['update-index','--assume-unchanged','--',FILE]);guards.write(FILE,'PRIVATE_HIDDEN_EDIT\n');await click('menu-repository');await click('repository-branch-menu');await stable();await click('repository-rebase');await type('dialog-rebase-target','main');await click('dialog-preview-refresh');assert(state.dialog.error.includes('unsupported_index'));assert.equal(fs.readFileSync(path.join(guards.repo,FILE),'utf8'),'PRIVATE_HIDDEN_EDIT\n');await capture('hidden-index-refusal');await click('dialog-cancel');guards.git(['update-index','--no-assume-unchanged','--',FILE]);guards.write(FILE,'dirty\n');await click('menu-repository');await click('repository-branch-menu');await stable();await click('repository-rebase');await type('dialog-rebase-target','main');await click('dialog-preview-refresh');assert(state.dialog.error.includes('dirty_worktree'));await click('dialog-cancel');guards.write(FILE,'feature\n');await plan(guards);
 const lock=path.join(guards.repo,'.git/index.lock');fs.writeFileSync(lock,'PRIVATE_EXTERNAL_LOCK');await submit();assert(!latest().command_succeeded);assert(state.dialog.error.includes('index_locked'));assert.equal(fs.readFileSync(lock,'utf8'),'PRIVATE_EXTERNAL_LOCK');await capture('index-lock-error');fs.unlinkSync(lock);await click('dialog-cancel');
 // A value only git rebase reads makes real Git refuse with that value in its error, which must stay out of the dialog.
 guards.git(['config','rebase.stat','PRIVATE_GIT_ERROR']);await plan(guards);const head=guards.git(['rev-parse','HEAD']);await submit();assert(!latest().command_succeeded);assert(state.dialog.error.includes('command_failed'));assert(!state.dialog.error.includes('PRIVATE_GIT_ERROR'));assert.equal(guards.git(['rev-parse','HEAD']),head);await click('dialog-cancel');guards.git(['config','--unset','rebase.stat']);
 await plan(guards);hold('before:rebase');await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});
 await until(s=>s.write&&fs.existsSync(marker));const cancelled=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.cancel_ms=performance.now()-cancelled;assert(report.cancel_ms<10000);assert.equal(guards.git(['rev-parse','HEAD']),head);assert(!latest().command_succeeded);await capture('cancelled-operation');await finishDialog();release();
 passed('Dirty worktree, existing index lock, failing Git and in-flight cancellation have clear outcomes without exposing the Git error text or changing the original branch');
 }

 const late=setup('PRIVATE_LATE_CANCEL');const lateOriginal=late.git(['rev-parse','HEAD']);
 // Hooks can't stop Git between moving the branch and clearing the rebase. A locked other branch fails --update-refs in
 // that gap, which leaves the same state, and the relay holds that result so Stop lands there.
 fs.writeFileSync(path.join(late.repo,'.git/refs/heads/other.lock'),'');
 await open(late);await plan(late);await moveOthers();hold('after:rebase');await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});
 await until(s=>s.write&&fs.existsSync(marker));
 const alreadyApplied=late.git(['rev-parse','HEAD']);assert.notEqual(alreadyApplied,lateOriginal);assert(fs.existsSync(path.join(late.repo,'.git/rebase-merge')));
 const lateStart=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.late_cancel_ms=performance.now()-lateStart;
 assert(report.late_cancel_ms<10000);assert.equal(late.git(['rev-parse','HEAD']),alreadyApplied);assert.equal(late.git(['rev-parse','refs/rocket/recovery/rebase/'+lateOriginal]),lateOriginal);assert(!latest().command_succeeded);
 report.late_cancel_rebase_pending=active().operation_state.rebase;await capture('cancel-after-commits');await finishDialog();release();await stable();assert.equal(control('commit').enabled,false,'Continue rebase offered on a rebase only Git can finish');
 assert(report.late_cancel_rebase_pending);assert(latest().details.some(d=>d.includes('[recovery_branch_moved]')));assert(latest().summary.includes('finish recovery with Git'));
 const lateSnapshot=snapshot(late);
 for(const action of ['continue','skip','abort']){
   await recover(action);assert(state.dialog.error.includes('[recovery_branch_moved]'));assert(!control('dialog-submit').enabled);assert.deepEqual(snapshot(late),lateSnapshot);
   if(action==='abort')await capture('late-recovery-refusal');await click('dialog-cancel');
 }
 passed('Cancellation after replayed commits preserves the observed history and recovery tip; refreshed state does not claim a rollback');
 for(const f of fixtures)assert.equal(hash(path.join(f.repo,'.git/config')),f.config,'Repository config changed');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const sensitive of ['PRIVATE_','検索',FILE])assert(!payload.includes(sensitive),'Diagnostic payload leak: '+sensitive);
 for(const op of focused?['rebase-preview','rebase']:['rebase-preview','rebase','rebase-continue','rebase-skip','rebase-abort']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,op);for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired '+op);}
 for(const step of ['review_plan','revalidate_preview','save_recovery_reference','classify_outcome'])assert(payload.includes(step),'Missing step '+step);
 for(const code of focused?['recovery_branch_moved']:['stale_preview','conflicts_remain','empty_change','dirty_worktree','unsupported_rebase','unsupported_index','index_locked','command_failed','recovery_branch_moved'])assert(records.some(r=>r.failure_code===code),'Missing failure '+code);
 assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;passed('Named preview/revalidate/recovery/execute/refresh outcomes, paired diagnostic spans, fixed failures, zero dropped events and no private payloads');report.passed=true;
}catch(error){if(client)try{await capture('failure');}catch{}throw error;}
finally{
 release();fs.writeFileSync('work/rebase-last-result.txt',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});
