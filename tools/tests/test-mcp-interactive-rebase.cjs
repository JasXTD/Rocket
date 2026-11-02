// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('work/mcp-fixtures/interactive-rebase-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/interactive-rebase';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const focused=process.argv.includes('--plan-only');
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
async function menuItem(id){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);if(SUBMENU[id]){await click(SUBMENU[id]);await stable();}for(let i=0;i<8;i++){const c=control(id);if(c&&c.rect[3]-c.rect[1]>=28)break;await scroll('repository-menu-scroll','down',250);}await click(id);}
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();relay.check(state);await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function submit(){await click('dialog-submit');await settle();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}


const FILE='src/検索/結果 🐦 é.rs';
function setup(name,conflict=false){
 const f=fixture(name);f.write(FILE,'base\n');f.write('.gitignore','ignored/\n');f.commit('PRIVATE_BASE 中文');f.git(['branch','main']);
 if(conflict){f.write(FILE,'feature\n');f.commit('PRIVATE_CONFLICT feature');f.write('later','later\n');f.commit('PRIVATE_LATER');f.git(['switch','main']);f.write(FILE,'main\n');f.commit('PRIVATE_DESTINATION');f.git(['switch',branch]);}
 else {for(const name of ['A','B','C','D','E','F']){f.write('検索/'+name+'.txt',name+'\n');f.commit('PRIVATE_SOURCE '+name);}}
 f.git(['branch','other']);f.config=hash(path.join(f.repo,'.git/config'));return f;
}
function snapshot(f){return {head:f.git(['rev-parse','HEAD']),index:f.git(['ls-files','--stage']),file:fs.readFileSync(path.join(f.repo,FILE),'utf8'),config:hash(path.join(f.repo,'.git/config'))};}
async function ensure(target){await settle();if(control(target)?.enabled)return;await scroll('dialog-rebase-scroll','up',2000);if(control(target)?.enabled)return;for(let i=0;i<14;i++){await scroll('dialog-rebase-scroll','down',240);if(control(target)?.enabled)return;}throw Error('Control not visible: '+target);}
async function plan(){await menuItem('repository-interactive-rebase');await type('dialog-rebase-target','main');await click('dialog-preview-refresh');await until(s=>s.dialog?.rebase_preview_loaded||s.dialog?.error);assert(state.dialog.rebase_preview_loaded,JSON.stringify(state.dialog));await stable();}
async function action(i,name){await ensure('dialog-rebase-action-'+i);await click('dialog-rebase-action-'+i);await click('dialog-rebase-action-'+i+'-'+name);assert.equal(state.dialog.rebase_plan.entries[i].action,name);}
async function message(i,value){await ensure('dialog-rebase-row-'+i);await click('dialog-rebase-row-'+i);await ensure('dialog-rebase-message');await type('dialog-rebase-message',value);}
async function review(){await submit();assert(state.dialog.rebase_plan.reviewed);}
async function recover(action){await click('operation-'+action);await until(s=>s.dialog?.rebase_preview_loaded||s.dialog?.error);await stable();}
async function finishDialog(){if(state.dialog)await click('dialog-cancel');}
// MCP runs Git with hooks off (SECURITY.md), so the relay holds the real command for Stop. The gate stays set until
// Stop has settled: a held command it failed to end would never finish, and settle would time out.
function hold(point){fs.rmSync(marker,{force:true});fs.writeFileSync(gate,point);}
function release(){fs.rmSync(gate,{force:true});}
function assertCleanJournal(f){assert(!fs.existsSync(path.join(f.repo,'.git/rocket-interactive-plan')),'Completed operation left its journal');}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);report.editor_sha256=hash(path.resolve('target/release/rocket-rebase-editor.exe'));cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);

 const linear=setup('PRIVATE_LINEAR');linear.write('private','PRIVATE_UNTRACKED\n');linear.write('ignored/cache','PRIVATE_IGNORED\n');const original=snapshot(linear),destination=linear.git(['rev-parse','main']);
 await open(linear);await plan();assert.equal(state.dialog.rebase_plan.entries.length,6);await capture('plan-wide');await resize(780,620);await capture('plan-narrow');await click('dialog-cancel');assert.deepEqual(snapshot(linear),original);
 await click('menu-terminal');await click('terminal-new');await plan();await capture('plan-narrow-terminal');await click('dialog-cancel');await resize(1480,920);await plan();
 await action(0,'fixup');assert(!control('dialog-submit').enabled);await capture('invalid-first-fixup');await action(0,'pick');await ensure('dialog-rebase-up-1');await click('dialog-rebase-up-1');
 await action(0,'reword');await message(0,'PRIVATE_REWORD 中文 🚀\n\n# literal message\n$(touch PRIVATE_INJECTION)');
 await action(2,'squash');await message(2,'PRIVATE_COMBINED 中文\n\nCombined A and C');await action(3,'fixup');await action(4,'drop');await review();await capture('review-wide');await resize(780,620);await capture('review-narrow-terminal');
 linear.git(['update-ref','refs/heads/main','HEAD~']);await submit();assert(state.dialog.error.includes('[stale_preview]'),JSON.stringify(state.dialog));assert.deepEqual(snapshot(linear),original);await capture('stale-preview');
 linear.git(['update-ref','refs/heads/main',destination]);await click('dialog-preview-refresh');await until(s=>s.dialog?.rebase_preview_loaded||s.dialog?.error);assert(state.dialog.rebase_preview_loaded);assert.equal(state.dialog.rebase_plan.entries[0].action,'reword');await review();await submit();
 assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(linear.git(['log','--reverse','--format=%s','main..HEAD']),'PRIVATE_REWORD 中文 🚀\nPRIVATE_COMBINED 中文\nPRIVATE_SOURCE F');assert.equal(linear.git(['rev-list','--count','main..HEAD']),'3');
 assert.equal(linear.git(['rev-parse','other']),original.head);assert.equal(linear.git(['rev-parse','refs/rocket/recovery/rebase/'+original.head]),original.head);assert(!fs.existsSync(path.join(linear.repo,'検索/E.txt')));assert(!fs.existsSync(path.join(linear.repo,'PRIVATE_INJECTION')));assert.equal(fs.readFileSync(path.join(linear.repo,'private'),'utf8'),'PRIVATE_UNTRACKED\n');assert.equal(fs.readFileSync(path.join(linear.repo,'ignored/cache'),'utf8'),'PRIVATE_IGNORED\n');assertCleanJournal(linear);
 passed('Native reorder, reword, squash/fixup and drop; two-step review, invalid actions, Unicode, stale destination and compact layout with terminal; original/unrelated data preserved');
 if(process.argv.includes('--plan-only')){report.passed=true;return;}

 const continued=setup('PRIVATE_CONTINUE',true);await open(continued);await plan();await action(0,'reword');await message(0,'PRIVATE_RESOLVED 中文');await review();await submit();assert(active().operation_state.rebase);assert(latest().details.some(d=>d.includes('[conflicts_remain]')));await capture('conflict-paused');
 const journal=path.join(continued.repo,'.git/rocket-interactive-plan'),savedJournal=fs.readFileSync(journal);fs.writeFileSync(journal,'PRIVATE_CHANGED_PLAN');await recover('abort');assert(state.dialog.error.includes('[rebase_plan_journal]'));await capture('changed-journal-refusal');await click('dialog-cancel');fs.writeFileSync(journal,savedJournal);
 const todo=path.join(continued.repo,'.git/rebase-merge/git-rebase-todo'),savedTodo=fs.readFileSync(todo);fs.writeFileSync(todo,'exec echo PRIVATE_UNEXPECTED_EXEC\n');await recover('skip');assert(state.dialog.error.includes('[unsupported_rebase]'));await click('dialog-cancel');fs.writeFileSync(todo,savedTodo);
 await closeWindow();await open(continued);assert(active().operation_state.rebase);await recover('continue');assert(state.dialog.error.includes('[conflicts_remain]'));await click('dialog-cancel');
 await click('view-changes');await click('change-unstaged-'+Buffer.from(FILE).toString('hex'));await click('conflict-open');await type('conflict-result','PRIVATE_RESOLUTION 中文 é 🚀\n');await click('conflict-stage');await submit();assert(latest().command_succeeded);await click('conflict-back');
 await recover('continue');assert(state.dialog.rebase_preview_loaded);const beforeContinue=snapshot(continued);await capture('continue-review');await click('dialog-cancel');assert.deepEqual(snapshot(continued),beforeContinue);await recover('continue');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(continued.git(['log','--reverse','--format=%s','main..HEAD']),'PRIVATE_RESOLVED 中文\nPRIVATE_LATER');assertCleanJournal(continued);
 passed('Conflict, changed journal/todo refusal, window restart, actual Unicode conflict-editor resolution, reviewed Continue and preserved reword message');

 for(const recovery of ['skip','abort']){
  const f=setup('PRIVATE_'+recovery.toUpperCase(),true),original=snapshot(f);await open(f);await plan();await review();await submit();assert(active().operation_state.rebase);await recover(recovery);await capture(recovery+'-review');await click('dialog-cancel');assert(active().operation_state.rebase);await recover(recovery);await submit();assert(latest().command_succeeded,JSON.stringify(latest()));if(recovery==='abort')assert.deepEqual(snapshot(f),original);else assert.equal(f.git(['log','--format=%s','main..HEAD']),'PRIVATE_LATER');assertCleanJournal(f);
 }
 passed('Native Skip and Abort confirmations, cancellation of recovery review, expected final history and journal cleanup');

 // Git refuses the reword itself once the editor has written a message equal to commit.template, the step a commit-msg
 // hook used to fail. Hooks are off under MCP, and emptying the file lifts the refusal as removing the hook did.
 const reviewed='PRIVATE_RETRY 中文 🚀\n\n# retained literal',template=path.join(root,'message-template');fs.writeFileSync(template,'');
 const messageStop=setup('PRIVATE_MESSAGE_STOP');messageStop.git(['config','commit.template',template.replaceAll('\\','/')]);messageStop.config=hash(path.join(messageStop.repo,'.git/config'));await open(messageStop);await plan();await action(0,'reword');await message(0,reviewed);await review();
 fs.writeFileSync(template,reviewed);await submit();assert(active().operation_state.rebase);assert(latest().details.some(d=>d.includes('[reviewed_message_not_saved]')),JSON.stringify(latest()));await capture('message-rejected');
 await recover('skip');assert(state.dialog.error.includes('[message_only_stop]'));await click('dialog-cancel');fs.writeFileSync(template,'');await recover('continue');assert.equal(state.dialog.rebase_preview.message_retry,true);await capture('message-retry-review');
 hold('after:--amend');await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.write&&fs.existsSync(marker));await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();release();assert(active().operation_state.rebase);const amended=messageStop.git(['rev-parse','HEAD']);await finishDialog();await recover('continue');assert.equal(state.dialog.rebase_preview.message_retry,false);await capture('message-already-saved-review');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(messageStop.git(['rev-parse','HEAD~5']),amended);assert.equal(messageStop.git(['log','-1','--format=%B',amended]),reviewed);assertCleanJournal(messageStop);
 passed('Clear refused-reword failure, message-only Skip refusal, reviewed retry, cancellation after amendment and resume without a second amendment');

 const fold=setup('PRIVATE_FOLD',true);await open(fold);await plan();await action(1,'squash');await message(1,'PRIVATE_FOLDED resolved');await review();await submit();await recover('skip');assert(state.dialog.error.includes('[skip_would_fold_base]'));await capture('skip-fold-refusal');await click('dialog-cancel');fold.write(FILE,'resolved fold\n');fold.git(['add','--',FILE]);await recover('continue');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(fold.git(['log','--format=%s','main..HEAD']),'PRIVATE_FOLDED resolved');assertCleanJournal(fold);
 passed('A skip that would fold into the destination is refused; resolved Continue completes the reviewed squash message');

 const cancelFixture=setup('PRIVATE_CANCEL');const cancelOriginal=snapshot(cancelFixture);await open(cancelFixture);await plan();await review();hold('before:rebase');await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.write&&fs.existsSync(marker));const start=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.cancel_ms=performance.now()-start;assert(report.cancel_ms<10000);assert.deepEqual(snapshot(cancelFixture),cancelOriginal);assert(!latest().command_succeeded);assertCleanJournal(cancelFixture);await capture('cancel-before-replay');await finishDialog();release();
 passed('In-flight cancellation before replay reconciles state, preserves original history and cleans the unused plan journal');

 // Hooks can't stop Git between moving the branch and clearing the rebase. A locked other branch fails --update-refs in
 // that gap, which leaves the same state, and the relay holds that result so Stop lands there.
 const late=setup('PRIVATE_LATE');const lateOriginal=late.git(['rev-parse','HEAD']);fs.writeFileSync(path.join(late.repo,'.git/refs/heads/other.lock'),'');await open(late);await plan();await ensure('dialog-rebase-update-refs');await click('dialog-rebase-update-refs');assert.equal(state.dialog.rebase_update_refs.choice,true);await review();hold('after:rebase');await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.write&&fs.existsSync(marker));const applied=late.git(['rev-parse','HEAD']);assert.notEqual(applied,lateOriginal);const lateStart=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.late_cancel_ms=performance.now()-lateStart;release();assert(report.late_cancel_ms<10000);assert.equal(late.git(['rev-parse','HEAD']),applied);assert(latest().details.some(d=>d.includes('[recovery_branch_moved]')));assert(fs.existsSync(path.join(late.repo,'.git/rocket-interactive-plan')));await capture('late-cancellation');await finishDialog();await stable();assert.equal(control('commit').enabled,false,'Continue rebase offered on a rebase only Git can finish');await recover('abort');assert(state.dialog.error.includes('[recovery_branch_moved]'));await capture('late-recovery-refusal');await click('dialog-cancel');
 passed('Cancellation after rewritten commits keeps the new history and original recovery tip; remaining journal/state is explicit and unsafe abort is refused');

 for(const f of fixtures)assert.equal(hash(path.join(f.repo,'.git/config')),f.config,'Repository config changed');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const sensitive of ['PRIVATE_','検索',FILE])assert(!payload.includes(sensitive),'Diagnostic payload leak: '+sensitive);
 for(const op of ['rebase-preview','interactive-rebase','rebase-continue','rebase-skip','rebase-abort']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,op);for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired '+op);}
 for(const step of ['review_plan','revalidate_preview','prepare_plan_editor','save_recovery_reference','classify_outcome','retry_reviewed_message','cleanup_plan_journal'])assert(payload.includes(step),'Missing step '+step);
 for(const code of ['stale_preview','conflicts_remain','unsupported_rebase','rebase_plan_journal','skip_would_fold_base','recovery_branch_moved','reviewed_message_not_saved','message_only_stop'])assert(records.some(r=>r.failure_code===code),'Missing failure '+code);
 assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;passed('Paired diagnostic spans, review/editor/recovery steps, fixed failures, cancellation, zero dropped events and no private messages/paths');report.passed=true;
}catch(error){if(client)try{await capture('failure');}catch{}throw error;}
finally{
 release();fs.writeFileSync('work/interactive-rebase-last-result.txt',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});
