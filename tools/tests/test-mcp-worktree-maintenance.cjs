// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {readDraftPayload}=require('./state-fixture.cjs');
const root=path.resolve('work/mcp-fixtures/worktree-maintenance-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/worktrees';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const timingOnly=process.argv.includes('--timing-only');
const report={passed:false,version,root,timing_only:timingOnly,cases:[]},fixtures=[];
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

async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function submit(){await click('dialog-submit');await settle();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}

const FILE='src/検索/結果 🚀 é.rs';
const sh=value=>"'"+value.replaceAll("'","'\\''")+"'";
// Windows Git marks .git hidden; truncating an existing handle preserves that
// attribute, whereas Node's CREATE_ALWAYS open mode refuses hidden files.
function writeExisting(file,text){const fd=fs.openSync(file,'r+');try{const bytes=Buffer.from(text);fs.writeSync(fd,bytes,0,bytes.length,0);fs.ftruncateSync(fd,bytes.length);}finally{fs.closeSync(fd);}}
const normal=p=>p.replace(/^\\\\\?\\/,'').replaceAll('/','\\').toLowerCase();
function setup(name){const f=fixture(name);f.write(FILE,'base\n');f.write('.gitignore','ignored/\n');f.base=f.commit('PRIVATE_BASE 中文');f.write(FILE,'second\n');f.tip=f.commit('PRIVATE_SECOND 🚀');return f;}
function snapshot(f){return{head:f.git(['rev-parse','HEAD']),index:f.git(['ls-files','--stage','-z']),file:fs.readFileSync(path.join(f.repo,FILE),'utf8'),config:hash(path.join(f.repo,'.git/config'))};}
function at(folder,args){return cp.execFileSync('git',['-C',folder.replace(/^\\\\\?\\/,''),...args],{env,windowsHide:true,encoding:'utf8'}).trimEnd();}
function destination(f,name,branchName){const dest=path.join(root,name);fs.writeFileSync(dest+'.rocket-worktree-fixture',branchName+'\n'+f.repo);return dest;}
function wt(){return state.dialog?.worktrees;}
async function browse(){if(state.dialog?.kind==='worktrees')return;if(control('view-worktrees'))await click('view-worktrees');else{await click('menu-view');await click('view-goto');await click('goto-worktrees')}assert(wt()?.entries,JSON.stringify(state.dialog));}
async function select(dest){const i=wt().entries.findIndex(e=>normal(e.path)===normal(dest));assert(i>=0,'Missing worktree '+dest);for(let n=0;!control('dialog-worktree-row-'+i)&&n<30;n++)await scroll('dialog-worktree-list-scroll');await click('dialog-worktree-row-'+i);assert.equal(normal(wt().selected),normal(dest));}
async function createPlan(dest,branchName,target='HEAD',existing=false){await browse();await click('dialog-worktree-create');await type('dialog-worktree-path',dest);await type('dialog-worktree-branch',branchName);await click(existing?'dialog-worktree-existing':'dialog-worktree-new');if(!existing)await type('dialog-worktree-target',target);assert(!control('dialog-submit').enabled);await click('dialog-preview-refresh');}
async function removePlan(dest){await browse();await select(dest);await click('dialog-worktree-remove');}
async function closeDialog(){for(let i=0;state.dialog&&i<3;i++)await click('dialog-cancel');assert.equal(state.dialog,null);}
function ref(f,name){const r=cp.spawnSync('git',['-C',f.repo,'rev-parse','--verify',name],{env,windowsHide:true,encoding:'utf8'});return r.status===0?r.stdout.trim():null;}
async function expectError(code){assert(state.dialog.error?.includes('['+code+']'),JSON.stringify({dialog:state.dialog,activity:latest()}));assert(!control('dialog-submit')?.enabled);await capture(code);}
async function run(){try{
 const holderSource=path.resolve('tools/helpers/hold-owned-mcp-ui.rs'),holderExe=path.resolve('work/hold-owned-mcp-ui.exe');if(!fs.existsSync(holderExe)||fs.statSync(holderExe).mtimeMs<fs.statSync(holderSource).mtimeMs)cp.execFileSync('rustc',['--edition','2024','-C','opt-level=1',holderSource,'-o',holderExe],{windowsHide:true});
 const exe=path.resolve(process.env.ROCKET_MAINTENANCE_EXE||'target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);

 const f=setup('PRIVATE_MAINTENANCE'); f.write(FILE,'PRIVATE_STAGED');f.git(['add','--',FILE]);f.write(FILE,'PRIVATE_WORKING');f.write('untracked','PRIVATE_UNTRACKED');const before=snapshot(f);
 if(!timingOnly){
 const topic='rocket-ui-test/maintenance',from=destination(f,'PRIVATE_旧 検索 🚀',topic),to=destination(f,'PRIVATE_新 検索 🚀 é',topic);
 f.git(['worktree','add','-b',topic,from,'HEAD']);const originalIndex=hash(path.join(at(from,['rev-parse','--absolute-git-dir']),'index'));
 async function plan(kind,source,dest){await browse();await select(source);for(let i=0;!control('dialog-worktree-'+kind)&&i<8;i++)await scroll('page-details-scroll','down',200);await click('dialog-worktree-'+kind);if(dest){await type('dialog-worktree-path',dest);assert(!control('dialog-submit').enabled);await click('dialog-preview-refresh');}}
 await open(f);await plan('move',from,to);assert(wt().preview_loaded);await capture('move-wide');await closeDialog();assert(fs.existsSync(from));assert(!fs.existsSync(to));
 await click('menu-terminal');await click('terminal-new');await resize(780,620);await plan('move',from,to);await capture('move-small-terminal');assert(control('dialog-submit').rect[3]<=620);await closeDialog();await resize(1480,920);
 await plan('move',from,to);fs.writeFileSync(path.join(from,'outside'),'PRIVATE_EXTERNAL');await submit();await expectError('worktree_dirty');assert.equal(fs.readFileSync(path.join(from,'outside'),'utf8'),'PRIVATE_EXTERNAL');fs.unlinkSync(path.join(from,'outside'));await closeDialog();
 await plan('move',from,to);await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert(!fs.existsSync(from));assert(fs.existsSync(to));assert.equal(hash(path.join(at(to,['rev-parse','--absolute-git-dir']),'index')),originalIndex);assert.deepEqual(snapshot(f),before);await closeDialog();
 passed('Move: Unicode paths, cancelled review, compact terminal layout, fresh dirty refusal, non-replacing folder move and preserved source changes');

 await browse();await select(to);await click('dialog-worktree-open');const linkedTab=state.active_tab;await browse();await select(to);assert(!control('dialog-worktree-move').enabled);await closeDialog();await click('close-tab-'+linkedTab);assert.equal(normal(active().path),normal(f.repo));
 const repaired=destination(f,'PRIVATE_repaired 文',topic),admin=at(to,['rev-parse','--absolute-git-dir']);fs.writeFileSync(path.join(to,FILE),'PRIVATE_STAGED_LINKED');at(to,['add','--',FILE]);fs.writeFileSync(path.join(to,FILE),'PRIVATE_WORKING_LINKED');fs.writeFileSync(path.join(to,'untracked'),'PRIVATE_LINKED_UNTRACKED');const dirtyIndex=hash(path.join(admin,'index'));fs.renameSync(to,repaired);
 await plan('repair',to,repaired);assert(wt().preview_loaded);await capture('repair-wide');await closeDialog();assert(wt()==null);assert(fs.readFileSync(path.join(admin,'gitdir'),'utf8').includes(path.basename(to)));
 await plan('repair',to,repaired);fs.writeFileSync(path.join(admin,'outside'),'PRIVATE_METADATA_EDIT');await submit();await expectError('worktree_stale_preview');assert.equal(fs.readFileSync(path.join(admin,'outside'),'utf8'),'PRIVATE_METADATA_EDIT');await closeDialog();
 await plan('repair',to,repaired);await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(hash(path.join(admin,'index')),dirtyIndex);assert.equal(fs.readFileSync(path.join(repaired,FILE),'utf8'),'PRIVATE_WORKING_LINKED');assert.equal(fs.readFileSync(path.join(repaired,'untracked'),'utf8'),'PRIVATE_LINKED_UNTRACKED');assert.deepEqual(snapshot(f),before);await closeDialog();
 passed('Repair: only the selected moved registration, open-tab protection, cancelled review and changed metadata refusal; staged and working files preserved');

 const saved=destination(f,'PRIVATE_saved 文',topic);fs.renameSync(repaired,saved);
 const otherBranch='rocket-ui-test/untouched',other=destination(f,'PRIVATE_OTHER',otherBranch);f.git(['worktree','add','-b',otherBranch,other,'HEAD']);const otherAdmin=at(other,['rev-parse','--absolute-git-dir']);fs.renameSync(other,path.join(root,'PRIVATE_OTHER_SAVED'));const otherLink=hash(path.join(otherAdmin,'gitdir'));
 await plan('prune',repaired);assert(wt().preview_loaded);await capture('prune-wide');await resize(780,620);await capture('prune-small-terminal');assert(control('dialog-submit').rect[3]<=620);await closeDialog();assert(fs.existsSync(admin));await resize(1480,920);
 await plan('prune',repaired);const ready=path.join(f.hooks,'prune-ready'),reference=wt().preview.recovery_reference;
 fs.writeFileSync(path.join(f.hooks,'reference-transaction'),'#!/bin/sh\nlines=$(cat)\nif [ "$1" = "committed" ] && printf "%s" "$lines" | grep -q "refs/rocket/removed-worktrees/"; then printf ready > '+sh(ready.replaceAll('\\','/'))+'; sleep 30; fi\n');
 await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.write&&fs.existsSync(ready),'Held recovery reference');const cancelStart=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.cancel_ms=performance.now()-cancelStart;assert(report.cancel_ms<10000);assert(!latest().command_succeeded);assert(fs.existsSync(admin));assert.equal(ref(f,reference),f.tip);assert(latest().details.some(s=>s.includes(reference)),JSON.stringify(latest()));await capture('prune-cancelled');await closeDialog();fs.unlinkSync(path.join(f.hooks,'reference-transaction'));
 await plan('prune',repaired);await submit();assert(latest().command_succeeded,JSON.stringify(latest()));const backupText=latest().details.find(s=>s.startsWith('Retained Git metadata: '));assert(backupText);const backup=backupText.split('Retained Git metadata: ')[1].split('. Inspect')[0];assert.equal(hash(path.join(backup,'registration/index')),dirtyIndex);assert(!fs.existsSync(admin));assert.equal(hash(path.join(otherAdmin,'gitdir')),otherLink);assert.equal(fs.readFileSync(path.join(saved,FILE),'utf8'),'PRIVATE_WORKING_LINKED');await closeDialog();
 passed('Prune: cancellation after recovery ref commit, accurate retained-tip result, one archived registration and index; unrelated missing worktree and all working files preserved');
 }else{await open(f);}

 // Induce one queue delay on this owned fixture window, without blocking HTTP.
 // This proves expiry diagnostics/no replay; it does not establish the cause of the earlier intermittent SSH stall.
 await click('view-changes');await type('commit-message','draft stays unchanged');const holdMarker=path.join(root,'ui-held'),draftDirectory=path.join(root,'rocket-ui-state');await until(s=>active().draft_present&&!s.state_pending&&fs.existsSync(path.join(draftDirectory,'state.bin'))&&readDraftPayload(draftDirectory).includes(Buffer.from('draft stays unchanged')),'Saved fixture draft');
 const holder=cp.spawn(path.resolve('work/hold-owned-mcp-ui.exe'),[String(service.pid),holdMarker],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});let holderError='';holder.stderr.on('data',b=>holderError+=b);const holderExit=new Promise((resolve,reject)=>{holder.on('exit',resolve);holder.on('error',reject)});
 const holdStart=performance.now();while(!fs.existsSync(holdMarker)){assert(performance.now()-holdStart<15000,'UI hold did not start: '+holderError);assert(holder.exitCode===null,'UI holder stopped: '+holderError);await delay(30)}
 const expired=await client.request('tools/call',{name:'rocket_type',arguments:{target:'commit-message',text:'PRIVATE_MUST_NOT_TYPE',expected_tab:state.active_tab}});assert(expired.isError,JSON.stringify(expired));assert.match(expired.content[0].text,/expired before input delivery/);const timing=expired._meta.rocket_request;assert.equal(timing.phase,'queued');assert.equal(timing.input_delivered,false);assert(timing.queue_ms>=20000);report.queue_expiry=timing;assert.equal(await holderExit,0,holderError);await settle();
 assert(active().draft_present);const savedDrafts=readDraftPayload(draftDirectory);assert(savedDrafts.includes(Buffer.from('draft stays unchanged')));assert(!savedDrafts.includes(Buffer.from('PRIVATE_MUST_NOT_TYPE')));await capture('mcp-expired-draft');await type('commit-message','');
 const receipt=await client.request('tools/call',{name:'rocket_click',arguments:{target:'view-commits',expected_tab:state.active_tab}});assert(!receipt.isError);assert.equal(receipt._meta.rocket_request.input_delivered,true);report.delivered_timing=receipt._meta.rocket_request;await settle();
 const stale=await client.request('tools/call',{name:'rocket_click',arguments:{target:'toolbar-push',expected_tab:999999}});assert(stale.isError);assert.equal(stale._meta.rocket_request.input_delivered,false);assert.equal(stale._meta.rocket_request.phase,'guard');
 passed('MCP timing: induced 21-second UI queue stall expires without input or replay; successful delivery and guard refusal have distinct payload-free phase receipts');

 await click('menu-view');await click('activity-open-menu');await until(s=>s.activity_history?.open&&!s.activity_history.loading);await type('activity-search','queued');await click('activity-refresh');await until(s=>!s.activity_history.loading&&s.activity_history.matched>0);
 const expiredRow=state.activity_history.rows.find(r=>r.operation==='mcp-ui'&&r.status==='Needs attention'&&!r.incomplete);assert(expiredRow,JSON.stringify(state.activity_history));await click('activity-row-'+expiredRow.index);
 const timingRecord=state.activity_history.selected.records.find(r=>r.native_request===report.queue_expiry.native_request);assert(timingRecord,JSON.stringify(state.activity_history.selected));assert(timingRecord.queue_ms>=20000);assert.equal(timingRecord.input_delivered,false);assert.equal(timingRecord.tool,'rocket_type');assert.equal(timingRecord.phase,'queued');await capture('mcp-timing-viewer');await click('activity-close');
 passed('Activity viewer: expired MCP request is a complete searchable operation with queue time and no input delivered');

 for(const fixture of fixtures)assert.equal(hash(path.join(fixture.repo,'.git/config')),fixture.config);assert.deepEqual(snapshot(f),before);await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const secret of ['PRIVATE_','検索',FILE,root,'rocket-ui-test/'])assert(!payload.includes(secret),'Private payload in diagnostics '+secret);
 if(!timingOnly){for(const operation of ['worktree-move','worktree-repair','worktree-prune'])assert(records.some(r=>r.event==='request_end'&&r.operation===operation&&r.outcome==='ok'),'Missing operation '+operation);
 for(const step of ['review_worktree_maintenance','backup_worktree_metadata','publish_worktree_maintenance','verify_worktree_result'])assert(records.some(r=>r.step===step),'Missing step '+step);}
 assert(records.some(r=>r.event==='mcp_request'&&r.native_request===report.queue_expiry.native_request&&r.phase==='queued'&&r.outcome==='refused'&&r.queue_ms>=20000&&!r.input_delivered));
 for(const start of records.filter(r=>r.event.endsWith('_start')&&r.id!==undefined))assert(records.some(r=>r.event===start.event.replace(/_start$/,'_end')&&r.session===start.session&&r.request===start.request&&r.id===start.id),'Unpaired diagnostic span');
 report.log_records=records.length;report.mcp_timing_records=records.filter(r=>r.event==='mcp_request').length;passed('Named operations/steps, retained recovery outcomes, paired spans, zero dropped events and no private payloads');report.passed=true;
}catch(error){report.error=String(error);if(client)try{await capture('failure');}catch{}throw error;}
finally{
 fs.writeFileSync('work/worktree-maintenance-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});

