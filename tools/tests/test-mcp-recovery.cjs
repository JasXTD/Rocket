// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root=path.resolve('work/mcp-fixtures/recovery-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/recovery';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const focused=process.argv.includes('--preview-only');
const report={passed:false,version,root,focused,cases:[]},fixtures=[];
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
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
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function submit(){await click('dialog-submit');await settle();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}

const FILE='src/検索/結果 🚀 é.rs';
function setup(name){const f=fixture(name);f.write(FILE,'base\n');f.write('.gitignore','ignored/\n');f.base=f.commit('PRIVATE_BASE 中文');f.write(FILE,'second\n');f.write('second','second\n');f.tip=f.commit('PRIVATE_SECOND 🚀');return f;}
function snapshot(f){return {head:f.git(['rev-parse','HEAD']),index:f.git(['ls-files','--stage']),file:fs.readFileSync(path.join(f.repo,FILE),'utf8'),config:hash(path.join(f.repo,'.git/config'))};}
async function browse(){await menuItem('repository-recovery');assert(state.dialog?.recovery?.entries,JSON.stringify(state.dialog));}
async function select(id){const i=state.dialog.recovery.entries.findIndex(e=>e.id===id);assert(i>=0,'Commit missing from reflog '+id);for(let n=0;!control('dialog-reflog-row-'+i)&&n<20;n++)await scroll('dialog-recovery-scroll');await click('dialog-reflog-row-'+i);assert.equal(state.dialog.recovery.target,id);}
async function plan(f,mode='soft',target=f.base){await browse();await select(target);await click('dialog-recovery-reset');if(mode!=='soft')await click('dialog-reset-'+mode);await stable();}
async function finishDialog(){if(state.dialog)await click('dialog-cancel');}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);

 if(process.argv.includes('--selection-only')){
  const f=setup('PRIVATE_SELECTION');for(let i=0;i<205;i++)f.git(['update-ref','-m','PRIVATE_EVENT_'+i,'HEAD',i%2?f.tip:f.base]);f.git(['update-ref','-m','PRIVATE_CURRENT','HEAD',f.tip]);const original=snapshot(f);
  await open(f);await browse();assert.equal(state.dialog.recovery.entries.length,200);await click('dialog-reflog-row-3');const selected=state.dialog.recovery.entries[3];assert.equal(state.dialog.recovery.selected_row,3);assert.equal(state.dialog.recovery.target,selected.id);await capture('single-reflog-highlight');
  await click('dialog-reflog-more');assert(state.dialog.recovery.entries.length>200);assert.equal(state.dialog.recovery.entries[state.dialog.recovery.selected_row].subject,selected.subject);
  f.git(['update-ref','-m','PRIVATE_EXTERNAL_1','HEAD',f.base]);f.git(['update-ref','-m','PRIVATE_EXTERNAL_2','HEAD',f.tip]);await click('dialog-preview-refresh');assert.equal(state.dialog.recovery.target,selected.id);assert.equal(state.dialog.recovery.entries[state.dialog.recovery.selected_row].subject,selected.subject);assert.equal(state.dialog.recovery.selected_row,5);await capture('shifted-reflog-highlight');
  await resize(780,620);await capture('single-selection-narrow');await click('dialog-recovery-branch');assert.equal(state.dialog.recovery.preview.target,selected.id);await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(f.git(['rev-parse','recovery/'+selected.id.slice(0,9)]),selected.id);assert.deepEqual(snapshot(f),original);
  passed('One highlighted event among duplicate commit IDs; Show more and external reflog shifts retain its event and pinned commit; actual recovery branch uses that commit');report.passed=true;return;
 }

 const recovery=setup('PRIVATE_RECOVERY');const original=snapshot(recovery);await open(recovery);await browse();assert(state.dialog.recovery.entries.some(e=>e.subject.includes('PRIVATE_BASE 中文')));assert(!control('dialog-recovery-reset').enabled);await capture('reflog-wide');await select(recovery.base);await resize(780,620);await capture('reflog-narrow');await click('dialog-recovery-branch');await capture('branch-review-narrow');await click('dialog-cancel');assert.deepEqual(snapshot(recovery),original);
 await click('menu-terminal');await click('terminal-new');await browse();await select(recovery.base);await click('dialog-recovery-reset');await click('dialog-reset-hard');assert(state.dialog.recovery.preview_loaded);await capture('reset-narrow-terminal');await click('dialog-cancel');assert.deepEqual(snapshot(recovery),original);await resize(1480,920);
 if(focused){passed('Reflog and recovery/reset previews at wide and compact sizes with the native terminal; cancellation preserves all repository data');report.passed=true;return;}
 await browse();await select(recovery.base);await click('dialog-recovery-branch');await type('dialog-recovery-name','recovery/検索-🚀');assert(!control('dialog-submit').enabled);await click('dialog-preview-refresh');assert(state.dialog.recovery.preview_loaded);await capture('recovery-branch-review');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(recovery.git(['rev-parse','recovery/検索-🚀']),recovery.base);assert.deepEqual(snapshot(recovery),original);
 await browse();await select(recovery.base);await click('dialog-recovery-branch');await type('dialog-recovery-name','recovery/検索-🚀');await click('dialog-preview-refresh');assert(state.dialog.error.includes('[branch_exists]'));await capture('branch-exists');await finishDialog();
 passed('Native reflog browse, Unicode recovery branch, no branch switch, duplicate refusal, wide/compact/terminal layouts and cancelled confirmations');

 const soft=setup('PRIVATE_SOFT');soft.write(FILE,'PRIVATE_STAGED_ONLY\n');soft.git(['add','--',FILE]);soft.write(FILE,'PRIVATE_WORKING_ONLY\n');soft.write('untracked','PRIVATE_UNTRACKED\n');soft.write('ignored/cache','PRIVATE_IGNORED\n');const softBefore=snapshot(soft);await open(soft);await plan(soft);assert(state.dialog.recovery.preview_loaded);await capture('soft-review');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(soft.git(['rev-parse','HEAD']),soft.base);assert.equal(soft.git(['ls-files','--stage']),softBefore.index);assert.equal(fs.readFileSync(path.join(soft.repo,FILE),'utf8'),softBefore.file);assert.equal(soft.git(['rev-parse','refs/rocket/recovery/reset/'+soft.tip]),soft.tip);assert.equal(fs.readFileSync(path.join(soft.repo,'untracked'),'utf8'),'PRIVATE_UNTRACKED\n');assert.equal(fs.readFileSync(path.join(soft.repo,'ignored/cache'),'utf8'),'PRIVATE_IGNORED\n');
 await plan(soft,'mixed',soft.tip);assert(state.dialog.error.includes('[staged_changes]'));assert(!control('dialog-submit').enabled);await capture('staged-reset-refusal');await click('dialog-reset-hard');assert(state.dialog.error.includes('[staged_changes]'));await finishDialog();
 passed('Soft reset preserves staged-only bytes, working files and unrelated local content; mixed/hard clearly refuse staged changes');

 const mixed=setup('PRIVATE_MIXED');mixed.write(FILE,'PRIVATE_UNSTAGED\n');await open(mixed);await plan(mixed,'hard');assert(state.dialog.error.includes('[dirty_worktree]'));await capture('dirty-hard-refusal');await click('dialog-reset-mixed');assert(state.dialog.recovery.preview_loaded);await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(mixed.git(['rev-parse','HEAD']),mixed.base);assert.equal(mixed.git(['diff','--cached','--name-only']),'');assert.equal(fs.readFileSync(path.join(mixed.repo,FILE),'utf8'),'PRIVATE_UNSTAGED\n');assert(fs.existsSync(path.join(mixed.repo,'second')));
 const hard=setup('PRIVATE_HARD');hard.write('local','PRIVATE_LOCAL\n');hard.write('ignored/cache','PRIVATE_IGNORED\n');await open(hard);await plan(hard,'hard');await capture('hard-review-wide');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(hard.git(['rev-parse','HEAD']),hard.base);assert.equal(fs.readFileSync(path.join(hard.repo,FILE),'utf8'),'base\n');assert(!fs.existsSync(path.join(hard.repo,'second')));assert.equal(fs.readFileSync(path.join(hard.repo,'local'),'utf8'),'PRIVATE_LOCAL\n');assert.equal(fs.readFileSync(path.join(hard.repo,'ignored/cache'),'utf8'),'PRIVATE_IGNORED\n');
 passed('Mixed reset keeps unstaged files; clean hard reset restores tracked content while preserving unrelated untracked/ignored files and original tips');

 const collision=setup('PRIVATE_COLLISION');collision.git(['reset','--hard',collision.base]);collision.write('second','PRIVATE_COLLISION_DATA\n');await open(collision);await plan(collision,'hard',collision.tip);assert(state.dialog.error.includes('[local_file_collision]'));assert(!control('dialog-submit').enabled);await capture('local-collision');await finishDialog();assert.equal(fs.readFileSync(path.join(collision.repo,'second'),'utf8'),'PRIVATE_COLLISION_DATA\n');assert.equal(collision.git(['rev-parse','HEAD']),collision.base);
 const stale=setup('PRIVATE_STALE');stale.write(FILE,'PRIVATE_FIRST_EXTERNAL\n');await open(stale);await plan(stale);const old=snapshot(stale);stale.write(FILE,'PRIVATE_SECOND_EXTERNAL\n');await submit();assert(state.dialog.error.includes('[stale_preview]'));assert.equal(stale.git(['rev-parse','HEAD']),old.head);assert.equal(fs.readFileSync(path.join(stale.repo,FILE),'utf8'),'PRIVATE_SECOND_EXTERNAL\n');await capture('stale-preview');await finishDialog();
 const locked=setup('PRIVATE_LOCKED');await open(locked);await plan(locked,'hard');fs.writeFileSync(path.join(locked.repo,'.git/index.lock'),'PRIVATE_OTHER_CLIENT');await submit();assert(!latest().command_succeeded);assert(latest().details.some(t=>t.includes('[index_locked]')),JSON.stringify(latest()));assert.equal(locked.git(['rev-parse','HEAD']),locked.tip);assert.equal(fs.readFileSync(path.join(locked.repo,'.git/index.lock'),'utf8'),'PRIVATE_OTHER_CLIENT');await capture('index-lock-error');await finishDialog();
 passed('Local-file collision, same-status external edit and index-lock failures are explicit; no clobbered files, reset branch or deleted locks');

 const readCancel=setup('PRIVATE_READ_CANCEL');await open(readCancel);await click('menu-repository');await click('repository-history-menu');await stable();await client.tool('rocket_click',{target:'repository-recovery',expected_tab:state.active_tab});await until(s=>s.dialog?.recovery?.loading);const readStart=performance.now();await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();report.read_cancel_ms=performance.now()-readStart;assert(report.read_cancel_ms<10000);assert.equal(state.dialog,null);assert.equal(readCancel.git(['rev-parse','HEAD']),readCancel.tip);
 for(const phase of ['prepared','committed']){
  const f=setup('PRIVATE_CANCEL_'+phase.toUpperCase());const before=snapshot(f);await open(f);await plan(f);
  fs.writeFileSync(path.join(f.hooks,'reference-transaction'),'#!/bin/sh\nlines=$(cat)\nif [ "$1" = "'+phase+'" ] && printf "%s" "$lines" | grep -q "refs/heads/'+branch+'$"; then echo ready > cancel-ready; sleep 30; fi\n');
  await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.write&&fs.existsSync(path.join(f.repo,'cancel-ready')),'Hook pause '+phase);const start=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report[phase+'_cancel_ms']=performance.now()-start;assert(report[phase+'_cancel_ms']<10000);assert(!latest().command_succeeded);assert.equal(f.git(['rev-parse','HEAD']),phase==='prepared'?before.head:f.base);assert.equal(f.git(['ls-files','--stage']),before.index);assert.equal(fs.readFileSync(path.join(f.repo,FILE),'utf8'),before.file);assert.equal(f.git(['rev-parse','refs/rocket/recovery/reset/'+f.tip]),f.tip);await capture('cancel-'+phase);await finishDialog();
 }
 passed('Cancelled reflog read and in-flight reset before/after branch update; reconciled actual state, preserved index/files and recovery reference, no automatic retry');

 const paging=setup('PRIVATE_PAGING');for(let i=0;i<205;i++)paging.git(['update-ref','-m','PRIVATE_POSITION_'+i,'HEAD',i%2?paging.tip:paging.base]);paging.git(['update-ref','HEAD',paging.tip]);await open(paging);await browse();assert.equal(state.dialog.recovery.entries.length,200);assert(state.dialog.recovery.more);await select(paging.tip);await click('dialog-reflog-more');assert(state.dialog.recovery.entries.length>200);assert.equal(state.dialog.recovery.target,paging.tip);await capture('reflog-expanded');await finishDialog();
 passed('Growing reflog window loads older entries and retains selection by immutable commit ID');
 for(const f of fixtures)assert.equal(hash(path.join(f.repo,'.git/config')),f.config,'Repository config changed');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const secret of ['PRIVATE_','検索',FILE])assert(!payload.includes(secret),'Diagnostic leak '+secret);
 for(const op of ['reflog','recovery-preview','recover-branch','reset-soft','reset-mixed','reset-hard']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired '+op);}
 for(const step of ['read_reflog','review_recovery','revalidate_preview','save_recovery_reference','revalidate_before_reset','verify_recovery'])assert(payload.includes(step),'Missing step '+step);
 for(const code of ['branch_exists','staged_changes','dirty_worktree','local_file_collision','stale_preview','index_locked'])assert(records.some(r=>r.failure_code===code),'Missing error '+code);
 assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;passed('Diagnostic spans paired, every recovery phase present, fixed failure codes and cancellations, zero dropped events, no private paths/messages');report.passed=true;
}catch(error){if(client)try{await capture('failure');}catch{}throw error;}
finally{
 fs.writeFileSync('work/recovery-last-result.txt',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});


