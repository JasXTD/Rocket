// Native acceptance through authenticated loopback MCP, using only owned local fixtures.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('work/mcp-fixtures/bisect-'+crypto.randomUUID().replaceAll('-','')),profile=path.join(root,'http-profile'),branch='rocket-ui-test/bisect';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report={passed:false,version,root,cases:[],ui_actions:[]},fixtures=[],FILE='src/検索/結果 🚀 é.rs';
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const relay=gitRelay(root,env,'tools/helpers/productivity/git-relay.rs'),{gate,marker}=relay;
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'),delay=ms=>new Promise(r=>setTimeout(r,ms));
function fixture(name,count=7){
 const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
 const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trimEnd();
 const write=(name,body)=>{const p=path.join(repo,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,body)};
 const commit=message=>{git(['add','.']);git(['commit','-m',message]);return git(['rev-parse','HEAD'])};
 git(['init','--initial-branch='+branch]);for(const [k,v]of [['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',k,v]);
 write('.gitignore','ignored/\nignored.txt\n');const ids=[];for(let i=0;i<count;i++){write(FILE,'version '+i+'\n');ids.push(commit('PRIVATE_BISECT version '+i));}
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
async function open(f){if(state?.dialog)await click('dialog-cancel');console.error('OPEN '+path.basename(f.repo));await client.tool('rocket_open',{path:f.repo});await settle();relay.check(state);await resize(1480,920);if(control('console-close'))await click('console-close');await menu();}
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
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
 const port=await new Promise(resolve=>{const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})});fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
 // The MCP process runs Git with core.hooksPath=/dev/null (SECURITY.md), so no hook can hold a checkout.
 // The owned relay pauses after a gated Git command has finished, as a slow post-checkout hook did.
 const log=fs.openSync(path.join(root,'service.stderr.log'),'a');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 const normal=fixture('PRIVATE_NORMAL 中文');normal.write('untracked 文','PRIVATE_UNTRACKED');normal.write('ignored/output','PRIVATE_IGNORED');const original=snapshot(normal);
 await open(normal);await type('dialog-bisect-good-input','PRIVATE_MISSING_REF');await type('dialog-bisect-bad-input',normal.ids[6]);await click('dialog-preview-refresh');assert(state.dialog.error.includes('bisect_revision_unavailable'));assert.deepEqual(snapshot(normal),original);
 await reviewRange(normal);assert.deepEqual(snapshot(normal),original);await capture('bisect-review-wide');await resize(780,620);
 for(const id of ['dialog-submit','dialog-preview-refresh','dialog-cancel']){const r=control(id).rect;assert(r[0]>=0&&r[1]>=0&&r[2]<=780&&r[3]<=620,id+' clipped')}
 await capture('bisect-review-small');await resize(1480,920);await apply();assert.equal(session().phase,'Testing');
 await capture('bisect-session');const candidate=session().target;
 await click('dialog-cancel');await closeWindow();await client.tool('rocket_open',{path:normal.repo});await settle();await menu();assert.equal(session().target,candidate);assert.equal(session().phase,'Testing');
 for(let n=0;n<8&&session().phase!=='Found';n++){
  const index=normal.ids.indexOf(session().target);assert(index>=0);
  assert.equal(normal.git(['rev-parse','HEAD']),session().target);
  assert.equal(fs.readFileSync(path.join(normal.repo,FILE),'utf8'),'version '+index+'\n');
  await step(index<4?'good':'bad');assert(latest().command_succeeded,JSON.stringify(latest()));
 }
 assert.equal(session().phase,'Found');assert.equal(session().target,normal.ids[4]);await capture('bisect-found');await restore();assert.deepEqual(snapshot(normal),original);
 assert.equal(fs.readFileSync(path.join(normal.repo,'untracked 文'),'utf8'),'PRIVATE_UNTRACKED');assert.equal(fs.readFileSync(path.join(normal.repo,'ignored/output'),'utf8'),'PRIVATE_IGNORED');
 passed('Native range review, narrow/wide layout, reopen durable session, Good/Bad exact culprit and original branch restoration preserve Unicode and local files');
 const skipped=fixture('PRIVATE_SKIP',4);await open(skipped);await reviewRange(skipped);await apply();
 for(let n=0;n<10&&session().phase!=='Inconclusive';n++)await step('skip');
 assert.equal(session().phase,'Inconclusive');assert(session().candidate_count>1);assert(session().decisions.filter(d=>d.action==='bisect-skip').length>=2);assert(!control('dialog-bisect-good')?.enabled);await capture('bisect-inconclusive');await restore();
 passed('Skip produces an explicit inconclusive result with candidates; no false single culprit');
 // Keep another unclassified commit after the external Skip; Git returns 2
 // when every remaining candidate was skipped, which belongs to the prior case.
 const dirty=fixture('PRIVATE_DIRTY',4);await open(dirty);await reviewRange(dirty);dirty.write(FILE,'staged');dirty.git(['add','.']);dirty.write(FILE,'unstaged PRIVATE edit');const edits=snapshot(dirty);
 await apply();assert(!latest().command_succeeded);assert(state.dialog.bisect.notice.includes('bisect_dirty_worktree'));assert.deepEqual(snapshot(dirty),edits);assert(!fs.existsSync(path.join(dirty.repo,'.git/rocket-bisect')));await capture('bisect-dirty-refused');
 dirty.git(['restore','--staged','--worktree','--',FILE]);await reviewRange(dirty);await apply();
 await click('dialog-bisect-good');dirty.git(['bisect','skip',session().target]);await apply();assert(!latest().command_succeeded);assert(state.dialog.bisect.notice.includes('bisect_stale_preview'));
 dirty.git(['update-ref','refs/heads/'+branch,dirty.ids[0]]);await click('dialog-bisect-restore');assert(state.dialog.error.includes('bisect_original_branch_moved'));assert.equal(dirty.git(['rev-parse',branch]),dirty.ids[0]);await capture('bisect-original-moved');
 dirty.git(['update-ref','refs/heads/'+branch,dirty.ids.at(-1)]);await restore();
 passed('Staged/unstaged edits, externally changed decisions and moved original branch are refused without resetting user work');
 const collision=fixture('PRIVATE_COLLISION',4);collision.git(['switch','--detach',collision.ids[3]]);collision.write('ignored.txt','historical');collision.git(['add','-f','ignored.txt']);collision.git(['commit','-m','PRIVATE_COLLISION historical']);const historical=collision.git(['rev-parse','HEAD']);collision.git(['switch',branch]);collision.write('ignored.txt','PRIVATE_IGNORED_COLLISION');
 await open(collision);await reviewRange(collision,collision.ids[3],historical);await apply();assert(!latest().command_succeeded);assert.equal(session().phase,'CheckoutPending');assert(state.dialog.bisect.notice.includes('bisect_checkout_blocked'));assert.equal(fs.readFileSync(path.join(collision.repo,'ignored.txt'),'utf8'),'PRIVATE_IGNORED_COLLISION');await capture('bisect-checkout-pending');
 const recorded=collision.git(['bisect','log']);
 fs.mkdirSync(path.join(collision.repo,'ignored'),{recursive:true});fs.renameSync(path.join(collision.repo,'ignored.txt'),path.join(collision.repo,'ignored/preserved'));await step('resume');assert.equal(session().phase,'Found');assert.equal(session().target,historical);assert.equal(collision.git(['bisect','log']),recorded);await restore();
 assert.equal(fs.readFileSync(path.join(collision.repo,'ignored/preserved'),'utf8'),'PRIVATE_IGNORED_COLLISION');
 passed('Ignored-file collision preserves contents; Resume completes the recorded step once');
 const cancelled=fixture('PRIVATE_CANCEL',3);await open(cancelled);await type('dialog-bisect-good-input',cancelled.ids[0]);await type('dialog-bisect-bad-input',cancelled.ids.at(-1));
 const untouched=snapshot(cancelled);await stable();await client.tool('rocket_click',{target:'dialog-preview-refresh',expected_tab:state.active_tab});await until(s=>s.dialog?.bisect?.loading&&s.controls.some(c=>c.id==='dialog-bisect-cancel-read'&&c.enabled));await client.tool('rocket_click',{target:'dialog-bisect-cancel-read',expected_tab:state.active_tab});await settle();assert.deepEqual(snapshot(cancelled),untouched);assert(!state.dialog.bisect.preview);
 await reviewRange(cancelled);const hold='after:switch';
 fs.writeFileSync(gate,hold);await stable();await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>fs.existsSync(marker)&&s.write&&s.controls.some(c=>c.id==='cancel-operation'&&c.enabled));
 let start=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.cancel_start_ms=Math.round(performance.now()-start);assert(!latest().command_succeeded);assert.equal(session().phase,'Testing');assert(fs.existsSync(path.join(cancelled.repo,'.git/rocket-bisect')));await capture('bisect-cancelled-checkout');
 fs.unlinkSync(gate);fs.unlinkSync(marker);await click('dialog-bisect-restore');fs.writeFileSync(gate,hold);await stable();await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>fs.existsSync(marker)&&s.write&&s.controls.some(c=>c.id==='cancel-operation'&&c.enabled));
 start=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.cancel_restore_ms=Math.round(performance.now()-start);assert(!latest().command_succeeded);assert.equal(cancelled.git(['symbolic-ref','--short','HEAD']),branch);assert(fs.existsSync(path.join(cancelled.repo,'.git/rocket-bisect')));fs.unlinkSync(gate);await restore();
 passed('Read cancellation makes no writes; cancelled checkout and restore reconcile changed HEAD, retain recovery state and safely retry cleanup');
 for(const f of fixtures)assert.equal(hash(path.join(f.repo,'.git/config')),f.config);
 await click('dialog-cancel');await click('menu-view');await click('activity-open-menu');await until(s=>s.activity_history.open&&!s.activity_history.loading&&s.activity_history.records>0);
 await type('activity-search','bisect');await until(s=>!s.activity_history.loading&&s.activity_history.rows.some(r=>r.operation==='bisect-restore'));
 const row=state.activity_history.rows.find(r=>r.operation==='bisect-restore');await click('activity-row-'+row.index);await capture('bisect-activity');assert.equal(state.activity_history.dropped,0);await click('activity-close');await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const text=JSON.stringify(records);for(const secret of ['PRIVATE_',FILE,branch,'検索'])assert(!text.includes(secret),'Diagnostic leak '+secret);
 for(const op of ['bisect-preview','bisect-start','bisect-good','bisect-bad','bisect-skip','bisect-resume','bisect-restore']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const s of starts)assert(records.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request),'Unpaired request');}
 for(const step of ['revalidate_bisect','save_return_checkout','record_bisect_decision','checkout_bisect_candidate','restore_checkout','clear_bisect_state','inspect_bisect_result'])assert(records.some(r=>r.step===step),'Missing step '+step);
 for(const code of ['bisect_dirty_worktree','bisect_stale_preview','bisect_original_branch_moved','bisect_checkout_blocked','bisect_revision_unavailable'])assert(records.some(r=>r.failure_code===code),'Missing '+code);
 assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;passed('Activity and private structured diagnostics record paired requests, each step, refusal codes and cancellation with zero dropped records');report.passed=true;
}catch(e){report.error=e.message;if(client)try{await capture('failure')}catch{}throw e}finally{
 fs.writeFileSync('work/bisect-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(200);if(service.exitCode===null)service.kill()}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});
