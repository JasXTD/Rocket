// Real native controls over authenticated loopback MCP. All Git mutations stay in owned fixtures.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/lost-commits-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/lost-commits',FILE='src/検索/結果 🚀 é.rs';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report={passed:false,version,root,cases:[]},fixtures=[];
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'),delay=ms=>new Promise(r=>setTimeout(r,ms));
function fixture(name,count=2){
 const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
 const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trimEnd();
 const write=(name,body)=>{const p=path.join(repo,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,body)};
 const commit=message=>{git(['add','.']);git(['commit','-m',message]);return git(['rev-parse','HEAD'])};
 git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks],['gc.auto','0']])git(['config',k,v]);
 write('.gitignore','ignored/\n');write(FILE,'base\n');const base=commit('PRIVATE_BASE'),ids=[];
 if(count>2){
  let input='';for(let i=1;i<=count;i++){const m='PRIVATE_BULK '+i;input+='commit refs/heads/lost/bulk\nmark :'+i+'\ncommitter Rocket Fixture <fixture@example.invalid> '+(1700000000+i)+' +0000\ndata '+Buffer.byteLength(m)+'\n'+m+'\nfrom '+(i===1?base:':'+(i-1))+'\n\n'}
  cp.execFileSync('git',['-C',repo,'fast-import','--quiet'],{env,windowsHide:true,input,stdio:['pipe','pipe','pipe']});
  ids.push(...git(['rev-list','lost/bulk','^'+base]).split('\n').reverse());git(['branch','-D','lost/bulk']);
 }else if(count){
  git(['switch','-c','lost/検索']);write(FILE,'base\nlost parent\n');ids.push(commit('PRIVATE_LOST_PARENT'));write(FILE,'base\nlost parent\nlost tip\n');ids.push(commit('PRIVATE_LOST_TIP'));git(['switch',branch]);git(['branch','-D','lost/検索']);
 }
 // Fixture setup only; Rocket's scan never expires logs.
 git(['reflog','expire','--expire=now','--all']);
 fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
 const f={repo,hooks,git,write,commit,base,ids,config:hash(path.join(repo,'.git/config'))};fixtures.push(f);return f;
}
function snapshot(f){return{head:f.git(['rev-parse','HEAD']),symbolic:f.git(['symbolic-ref','HEAD']),index:hash(path.join(f.repo,'.git/index')),file:hash(path.join(f.repo,FILE)),config:hash(path.join(f.repo,'.git/config'))};}
let client,service,state;
const passed=text=>{report.cases.push(text);console.error('PASS '+text)};
const control=id=>state.controls.find(c=>c.id===id),latest=()=>state.activities.at(-1),recovery=()=>state.dialog?.recovery;
async function until(predicate,label='UI state'){
 const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<90000,label+' timed out; '+JSON.stringify({dialog:state.dialog,write:state.write,notice:state.notice}));await delay(100)}
}
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!s.write&&!s.dialog?.recovery?.loading&&!s.dialog?.recovery?.applying&&!(s.tabs||[]).some(t=>t.loading||t.search.running))}
async function stable(){let last='',n=0;await until(s=>{const next=JSON.stringify(s.controls);n=next===last?n+1:0;last=next;return n>=3},'Stable controls')}
async function scroll(target,direction='down',pixels=500){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settle();await stable()}
async function visible(id){
 await stable();if(control(id)?.enabled)return;
 if(state.dialog?.kind==='recovery'){await scroll('dialog-recovery-scroll','up',5000);for(let i=0;i<10&&!control(id)?.enabled;i++)await scroll('dialog-recovery-scroll','down',350)}
 assert(control(id)?.enabled,'Missing/disabled '+id);
}
async function click(target){await visible(target);report.last_action={target,rect:control(target)?.rect};await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle()}
async function type(target,text){await visible(target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle()}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const img=r.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2))}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();await stable()}
async function menu(){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);await click('repository-history-menu');await stable();for(let n=0;n<8;n++){const c=control('repository-recovery');if(c?.enabled&&c.rect[3]-c.rect[1]>=28)break;await stable()}await click('repository-recovery');assert.equal(state.dialog.kind,'recovery')}
async function open(f){if(state?.dialog)await click('dialog-cancel');console.error('OPEN '+path.basename(f.repo));await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await menu()}
async function scan(){await click('dialog-reflog-scope-unreachable');assert.equal(recovery().scope,'Unreachable');assert(!state.dialog.error,JSON.stringify(state.dialog))}
async function review(id,name){
 const row=recovery().entries.findIndex(e=>e.id===id);assert(row>=0);await click('dialog-reflog-row-'+row);await click('dialog-recovery-branch');
 if(name){await type('dialog-recovery-name',name);assert(!recovery().preview_loaded);await click('dialog-preview-refresh')}
 assert(recovery().preview_loaded,JSON.stringify(state.dialog));assert(recovery().preview.lost_commit);
}
async function closeWindow(){
 if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');
 await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');
 if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window))}
}
async function cancelRead(target){
 await stable();await client.tool('rocket_click',{target,expected_tab:state.active_tab});
 await until(s=>s.dialog?.recovery?.loading&&s.controls.some(c=>c.id==='dialog-recovery-cancel-read'&&c.enabled),'Cancellable read');
 await client.tool('rocket_click',{target:'dialog-recovery-cancel-read',expected_tab:state.active_tab});await settle();
 assert(recovery().read_notice?.includes('cancelled'));assert(!recovery().preview_loaded);
}
async function run(){try{
 const exe=path.resolve(process.env.ROCKET_TEST_EXE||'target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
 const port=await new Promise(resolve=>{const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})});fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const log=fs.openSync(path.join(root,'service.stderr.log'),'a');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 const normal=fixture('PRIVATE_NORMAL 中文');normal.write(FILE,'PRIVATE_STAGED');normal.git(['add','--',FILE]);normal.write(FILE,'PRIVATE_UNSTAGED');normal.write('untracked 文','PRIVATE_UNTRACKED');normal.write('ignored/local','PRIVATE_IGNORED');const original=snapshot(normal);
 await open(normal);await scan();assert.equal(recovery().scan_total,2);assert(recovery().entries.some(e=>e.id===normal.ids[0]));assert(!control('dialog-recovery-reset').enabled);await capture('lost-list-wide');
 await review(normal.ids[1],'recovery/検索-🚀');assert.deepEqual(recovery().preview.parents,[normal.ids[0]]);assert(recovery().preview.patch_bytes>0);assert.deepEqual(snapshot(normal),original);await capture('lost-review-wide');
 await resize(780,620);for(const id of ['dialog-submit','dialog-preview-refresh','dialog-cancel']){const r=control(id)?.rect;assert(r&&r[0]>=0&&r[1]>=0&&r[2]<=780&&r[3]<=620,id+' clipped')}await capture('lost-review-small');await resize(1480,920);
 // Editing local content after review must refuse the write.
 normal.write(FILE,'PRIVATE_EXTERNAL_EDIT');const external=snapshot(normal);await click('dialog-submit');assert(!latest().command_succeeded);assert(state.dialog.error.includes('stale_preview'));assert.deepEqual(snapshot(normal),external);await capture('lost-stale-refusal');
 await click('dialog-preview-refresh');await click('dialog-submit');assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(normal.git(['rev-parse','recovery/検索-🚀']),normal.ids[1]);assert.deepEqual(snapshot(normal),external);
 assert.equal(fs.readFileSync(path.join(normal.repo,'untracked 文'),'utf8'),'PRIVATE_UNTRACKED');assert.equal(fs.readFileSync(path.join(normal.repo,'ignored/local'),'utf8'),'PRIVATE_IGNORED');assert(!fs.existsSync(path.join(normal.repo,'.git/lost-found')));
 await menu();await scan();assert.equal(recovery().scan_total,0);await capture('lost-empty');
 passed('Native scan finds ancestors; Unicode review, small/wide layouts, stale edit refusal and recovery branch preserve checkout, raw index, config and local files');
 const collision=fixture('PRIVATE_COLLISION');await open(collision);await scan();await review(collision.ids[1],'recovery/collision');
 collision.git(['branch','recovery/collision',branch]);const unchanged=snapshot(collision);await click('dialog-submit');assert(state.dialog.error.includes('branch_exists'));assert.equal(collision.git(['rev-parse','recovery/collision']),collision.base);assert.deepEqual(snapshot(collision),unchanged);
 await type('dialog-recovery-name','recovery/invalid name');await click('dialog-preview-refresh');assert(state.dialog.error);assert(!control('dialog-submit').enabled);
 await type('dialog-recovery-name','recovery/cancelled');await cancelRead('dialog-preview-refresh');assert.deepEqual(snapshot(collision),unchanged);
 await click('dialog-recovery-back');await cancelRead('dialog-preview-refresh');assert.equal(recovery().scan_total,2);assert(recovery().read_notice.includes('retained'));assert.equal(recovery().entries.length,2);await capture('lost-scan-cancelled');
 passed('Existing/invalid branch names are refused; cancelled preview and rescan publish no writes and retain the previous complete scan');
 // Missing content after review: prune only this disposable fixture.
 await review(collision.ids[1],'recovery/pruned');collision.git(['prune','--expire=now']);await click('dialog-submit');assert(state.dialog.error.includes('lost_commit_unavailable'));assert(!latest().command_succeeded);assert.deepEqual(snapshot(collision),unchanged);await capture('lost-pruned-refusal');
 await click('dialog-recovery-back');const corrupt=path.join(collision.repo,'.git/objects/aa');fs.mkdirSync(corrupt,{recursive:true});fs.writeFileSync(path.join(corrupt,'1'.repeat(38)),'corrupt fixture object');await click('dialog-preview-refresh');assert(state.dialog.error.includes('lost_scan_failed'));assert(recovery().read_notice.includes('previous'));assert.equal(recovery().scan_total,2);await capture('lost-failed-scan');
 passed('Pruned objects refuse recovery; failed scans retain prior results and never report false empty success');
 const bulk=fixture('PRIVATE_BULK',205);await open(bulk);await scan();assert.equal(recovery().entries.length,200);assert.equal(recovery().scan_total,205);assert(recovery().more);await click('dialog-reflog-more');assert.equal(recovery().entries.length,205);assert(!recovery().more);passed('Show more uses the completed scan and exposes lost ancestors beyond the first 200');
 const cancelled=fixture('PRIVATE_WRITE_CANCEL');await open(cancelled);await scan();await review(cancelled.ids[1],'recovery/cancel-write');const before=snapshot(cancelled);
 const hook=path.join(cancelled.hooks,'reference-transaction'),ready=path.join(cancelled.repo,'.git/lost-ready');
 fs.writeFileSync(hook,'#!/bin/sh\nif [ "$1" = committed ]; then printf ready > .git/lost-ready; sleep 30; fi\n');
 await stable();await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>fs.existsSync(ready)&&s.write&&s.controls.some(c=>c.id==='cancel-operation'&&c.enabled),'Committed hook barrier');
 const started=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.cancel_write_ms=Math.round(performance.now()-started);fs.unlinkSync(hook);
 assert(!latest().command_succeeded);assert(latest().details.some(s=>s.includes('Verified recovery branch recovery/cancel-write at '+cancelled.ids[1])));assert(latest().summary.includes('instead of retrying'));assert.equal(cancelled.git(['rev-parse','recovery/cancel-write']),cancelled.ids[1]);assert.deepEqual(snapshot(cancelled),before);await capture('lost-cancelled-write');
 passed('Cancellation after a real ref transaction reports the existing branch, reconciles state and preserves all local edits');
 for(const f of fixtures)assert.equal(hash(path.join(f.repo,'.git/config')),f.config);
 await click('dialog-cancel');await click('menu-view');await click('activity-open-menu');await until(s=>s.activity_history.open&&!s.activity_history.loading&&s.activity_history.records>0);
 await type('activity-search','lost-commits');await until(s=>!s.activity_history.loading&&s.activity_history.rows.some(r=>r.operation==='lost-commits'));await capture('lost-activity');assert.equal(state.activity_history.dropped,0);await click('activity-close');await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const text=JSON.stringify(records);for(const secret of ['PRIVATE_',FILE,branch,'検索'])assert(!text.includes(secret),'Diagnostic leak '+secret);
 for(const op of ['lost-commits','lost-recovery-preview','recover-branch']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const s of starts)assert(records.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request),'Unpaired request')}
 for(const step of ['check_lost_history','scan_lost_objects','read_lost_commits','review_lost_commit','revalidate_lost_content','verify_recovery'])assert(records.some(r=>r.step===step),'Missing step '+step);
 for(const code of ['stale_preview','branch_exists','lost_commit_unavailable','lost_scan_failed'])assert(records.some(r=>r.failure_code===code),'Missing '+code);
 assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;
 passed('Activity and diagnostics show paired operations, scan/review/write steps, typed refusals and cancellation, with no private payloads or dropped events');report.passed=true;
}catch(e){report.error=e.message;if(client)try{await capture('failure')}catch{}throw e}finally{
 fs.writeFileSync('work/lost-commits-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(200);if(service.exitCode===null)service.kill()}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});
