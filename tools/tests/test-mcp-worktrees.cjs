// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/worktrees-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/worktrees';
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
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);

 const f=setup('PRIVATE_WORKTREES');
 f.write(FILE,'PRIVATE_STAGED\n');f.git(['add','--',FILE]);f.write(FILE,'PRIVATE_WORKING\n');f.write('untracked','PRIVATE_UNTRACKED\n');f.write('ignored/cache','PRIVATE_IGNORED\n');const original=snapshot(f);
 const topic='rocket-ui-test/topic',dest=destination(f,'PRIVATE_検索 🚀 é',topic);
 await open(f);const mainTab=state.active_tab;await browse();assert.equal(wt().entries.length,1);await select(f.repo);assert(wt().entries[0].main);assert(!control('dialog-worktree-remove').enabled);await capture('list-wide');
 await resize(780,620);await capture('list-narrow');await closeDialog();await click('menu-terminal');await click('terminal-new');await createPlan(dest,topic,'HEAD~');assert(wt().preview_loaded);assert.equal(wt().preview.target,f.base);await capture('create-narrow-terminal');assert(control('dialog-submit').rect[3]<=620);await closeDialog();assert(!fs.existsSync(dest));assert.equal(ref(f,'refs/heads/'+topic),null);assert.deepEqual(snapshot(f),original);await resize(1480,920);
 await createPlan(dest,topic,'HEAD~');await type('dialog-worktree-target','HEAD');assert(!control('dialog-submit').enabled);await click('dialog-preview-refresh');assert.equal(wt().preview.target,f.tip);await capture('create-wide');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(wt().creating,false);assert.equal(at(dest,['rev-parse','HEAD']),f.tip);assert.equal(fs.readFileSync(path.join(dest,FILE),'utf8'),'second\n');assert.deepEqual(snapshot(f),original);await select(dest);await capture('created-list');
 passed('Real Unicode worktree creation and cancelled preview, full commit review, draft invalidation, wide/compact terminal layouts; source HEAD/index/config and edits preserved');

 await click('dialog-worktree-open');assert.equal(normal(active().path),normal(dest));const linkedTab=state.active_tab;assert.notEqual(linkedTab,mainTab);await browse();await select(dest);assert(!control('dialog-worktree-remove').enabled);await click('dialog-worktree-open');assert.equal(state.active_tab,linkedTab);assert.equal(state.tabs.length,2);await click('tab-'+mainTab);await browse();await select(dest);assert(!control('dialog-worktree-remove').enabled);await capture('open-tab-removal-blocked');await closeDialog();await click('close-tab-'+linkedTab);assert(!state.tabs.some(t=>t.id===linkedTab));
 await removePlan(dest);assert(wt().preview_loaded);await resize(780,620);await capture('remove-narrow-terminal');await closeDialog();assert(fs.existsSync(dest));await resize(1480,920);
 await removePlan(dest);fs.writeFileSync(path.join(dest,'external.txt'),'PRIVATE_EXTERNAL');await submit();await expectError('worktree_dirty');assert.equal(fs.readFileSync(path.join(dest,'external.txt'),'utf8'),'PRIVATE_EXTERNAL');fs.unlinkSync(path.join(dest,'external.txt'));await closeDialog();
 fs.mkdirSync(path.join(dest,'ignored'));fs.writeFileSync(path.join(dest,'ignored/cache'),'PRIVATE_IGNORED');await removePlan(dest);await expectError('worktree_dirty');fs.unlinkSync(path.join(dest,'ignored/cache'));fs.rmdirSync(path.join(dest,'ignored'));await closeDialog();
 fs.writeFileSync(path.join(dest,FILE),'PRIVATE_STAGED_LINKED');at(dest,['add','--',FILE]);await removePlan(dest);await expectError('worktree_dirty');assert.equal(fs.readFileSync(path.join(dest,FILE),'utf8'),'PRIVATE_STAGED_LINKED');await closeDialog();at(dest,['restore','--staged','--worktree','--',FILE]);
 f.git(['worktree','lock','--reason','PRIVATE_LOCK 中文',dest]);await browse();await select(dest);assert(wt().entries.find(e=>normal(e.path)===normal(dest)).locked.includes('PRIVATE_LOCK'));assert(!control('dialog-worktree-remove').enabled);await capture('locked-worktree');await closeDialog();f.git(['worktree','unlock',dest]);
 const admin=at(dest,['rev-parse','--absolute-git-dir']);fs.writeFileSync(path.join(admin,'index.lock'),'PRIVATE_OTHER_LOCK');await removePlan(dest);await expectError('worktree_locked');assert.equal(fs.readFileSync(path.join(admin,'index.lock'),'utf8'),'PRIVATE_OTHER_LOCK');await closeDialog();fs.unlinkSync(path.join(admin,'index.lock'));
 await removePlan(dest);assert(wt().preview_loaded);const recovery=wt().preview.recovery_reference;await capture('remove-wide');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert(!fs.existsSync(dest));assert.equal(ref(f,'refs/heads/'+topic),f.tip);assert.equal(ref(f,recovery),f.tip);assert.deepEqual(snapshot(f),original);await closeDialog();
 passed('Open and tab deduplication; open-tab/main protections; cancelled removal and fresh refusal of external, staged, ignored files and locks; clean removal keeps branch and recovery tip');

 const existingDest=destination(f,'PRIVATE_EXISTING',topic);await createPlan(existingDest,topic,'ignored',true);assert(wt().preview_loaded);await submit();assert(latest().command_succeeded,JSON.stringify(latest()));await closeDialog();await removePlan(existingDest);await submit();assert(latest().command_succeeded);await closeDialog();
 for(const[name,b,target,existing,code]of [['bad-name','bad name','HEAD',false,'worktree_invalid_branch'],['duplicate',topic,'HEAD',false,'worktree_branch_exists'],['missing','rocket-ui-test/missing','HEAD',true,'worktree_branch_unavailable'],['in-use',branch,'HEAD',true,'worktree_branch_in_use'],['bad-target','rocket-ui-test/badtarget','not-a-commit',false,'worktree_target_unavailable']]){
  await createPlan(destination(f,'PRIVATE_'+name,b),b,target,existing);await expectError(code);await closeDialog();
 }
 const collision=destination(f,'PRIVATE_COLLISION','rocket-ui-test/collision');await createPlan(collision,'rocket-ui-test/collision');assert(wt().preview_loaded);fs.mkdirSync(collision);fs.writeFileSync(path.join(collision,'keep'),'PRIVATE_EXISTING');await submit();await expectError('worktree_destination_exists');assert.equal(fs.readFileSync(path.join(collision,'keep'),'utf8'),'PRIVATE_EXISTING');assert.equal(ref(f,'refs/heads/rocket-ui-test/collision'),null);await closeDialog();
 const c=setup('PRIVATE_PUBLISH_RACE'),publish=destination(c,'PRIVATE_PUBLISH_DEST','rocket-ui-test/publish');await open(c);await createPlan(publish,'rocket-ui-test/publish');const beforePublish=snapshot(c);
 fs.writeFileSync(path.join(c.hooks,'post-checkout'),'#!/bin/sh\nmkdir '+sh(publish.replaceAll('\\','/'))+'\nprintf PRIVATE_EXISTING > '+sh(path.join(publish,'keep').replaceAll('\\','/'))+'\n');
 await submit();await expectError('worktree_destination_exists');assert.equal(fs.readFileSync(path.join(publish,'keep'),'utf8'),'PRIVATE_EXISTING');assert(latest().details.some(s=>s.includes('Retained preparation folder')),JSON.stringify(latest()));assert.equal(c.git(['worktree','list','--porcelain']).split('worktree ').length-1,2);assert.deepEqual(snapshot(c),beforePublish);await closeDialog();
 passed('Existing branch reuse, explicit branch/target errors, pre-write destination collision and collision during checkout preserve existing files and retain prepared checkout');

 const neighbor=setup('PRIVATE_NEIGHBOR'),np=path.join(root,'PRIVATE_OTHER_TREE');neighbor.git(['worktree','add','--relative-paths','-b','rocket-ui-test/neighbor',np]);neighbor.config=hash(path.join(neighbor.repo,'.git/config'));writeExisting(path.join(np,'.git'),'gitdir: missing-unrelated-metadata\n');const independent=destination(neighbor,'PRIVATE_INDEPENDENT','rocket-ui-test/independent');await open(neighbor);await createPlan(independent,'rocket-ui-test/independent');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(fs.readFileSync(path.join(np,'.git'),'utf8'),'gitdir: missing-unrelated-metadata\n');await closeDialog();
 const linkLock=setup('PRIVATE_LINK_LOCK'),lld=destination(linkLock,'PRIVATE_LINK_LOCK_DEST','rocket-ui-test/linklock');await open(linkLock);await createPlan(lld,'rocket-ui-test/linklock');fs.writeFileSync(path.join(linkLock.hooks,'post-checkout'),'#!/bin/sh\nadmin=$(git rev-parse --absolute-git-dir)\nprintf PRIVATE_LINK_LOCK > "$admin/gitdir.lock"\n');await submit();assert(!latest().command_succeeded);assert(fs.existsSync(path.join(lld,FILE)));const lladmin=at(lld,['rev-parse','--absolute-git-dir']);assert.equal(fs.readFileSync(path.join(lladmin,'gitdir.lock'),'utf8'),'PRIVATE_LINK_LOCK');assert(latest().details.some(s=>s.includes('Retained preparation folder')));await capture('relink-lock-preserved');await closeDialog();
 passed('Creation leaves unrelated broken worktree links untouched; a relink lock is preserved and the published checkout remains available after failure');

 const cancel=setup('PRIVATE_CANCEL'),cancelDest=destination(cancel,'PRIVATE_CANCEL_DEST','rocket-ui-test/cancel');await open(cancel);await createPlan(cancelDest,'rocket-ui-test/cancel');const beforeCancel=snapshot(cancel),ready=path.join(cancel.hooks,'ready');
 fs.writeFileSync(path.join(cancel.hooks,'post-checkout'),'#!/bin/sh\nprintf ready > '+sh(ready.replaceAll('\\','/'))+'\nsleep 30\n');
 await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.write&&fs.existsSync(ready),'Paused checkout');const cancelStart=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.create_cancel_ms=performance.now()-cancelStart;assert(report.create_cancel_ms<10000);assert(!latest().command_succeeded);assert(!fs.existsSync(cancelDest));assert(latest().details.some(s=>s.includes('Retained preparation folder')));assert.deepEqual(snapshot(cancel),beforeCancel);await capture('cancelled-checkout');await closeDialog();
 const rc=setup('PRIVATE_RECOVERY_CANCEL'),rd=destination(rc,'PRIVATE_RECOVERY_DEST','rocket-ui-test/recovery');await open(rc);await createPlan(rd,'rocket-ui-test/recovery');await submit();assert(latest().command_succeeded);await closeDialog();await removePlan(rd);const rr=wt().preview.recovery_reference,rrReady=path.join(rc.hooks,'ready');
 fs.writeFileSync(path.join(rc.hooks,'reference-transaction'),'#!/bin/sh\nlines=$(cat)\nif [ "$1" = "committed" ] && printf "%s" "$lines" | grep -q "refs/rocket/removed-worktrees/"; then printf ready > '+sh(rrReady.replaceAll('\\','/'))+'; sleep 30; fi\n');
 await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.write&&fs.existsSync(rrReady),'Paused recovery transaction');const removeStart=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report.remove_cancel_ms=performance.now()-removeStart;assert(report.remove_cancel_ms<10000);assert(!latest().command_succeeded);assert(fs.existsSync(rd));assert.equal(ref(rc,rr),rc.tip);assert(latest().details.some(s=>s.includes(rr)),JSON.stringify(latest()));await capture('cancelled-recovery-ref');await closeDialog();
 await client.tool('rocket_click',{target:'view-worktrees',expected_tab:state.active_tab});await until(s=>s.dialog?.worktrees?.loading);const readStart=performance.now();await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();report.read_cancel_ms=performance.now()-readStart;assert.equal(state.dialog,null);assert(report.read_cancel_ms<10000);
 passed('Real cancellation during checkout and after recovery-ref commit, bounded responsiveness, retained preparation/observed recovery state, cancellable reads');

 const rt=setup('PRIVATE_REFTABLE');rt.git(['refs','migrate','--ref-format=reftable']);rt.config=hash(path.join(rt.repo,'.git/config'));const rtd=destination(rt,'PRIVATE_REFTABLE_DEST','rocket-ui-test/reftable');await open(rt);await createPlan(rtd,'rocket-ui-test/reftable');await submit();assert(latest().command_succeeded);await closeDialog();await removePlan(rtd);const rtAdmin=at(rtd,['rev-parse','--absolute-git-dir']);await submit();assert(!fs.existsSync(rtd));
 if(fs.existsSync(rtAdmin)){assert(!latest().command_succeeded);assert(latest().details.some(s=>s.includes('Git metadata remains at')),JSON.stringify(latest()));assert(latest().details.some(s=>s.includes('registration: absent. Folder currently absent.')));await capture('reftable-partial-cleanup');}else{assert(latest().command_succeeded);}
 await closeDialog();passed('Reftable removal reports any remaining Git metadata independently of the removed folder, preserving the installed Git error');

 const unmarked=setup('PRIVATE_UNMARKED'),ud=path.join(root,'PRIVATE_UNMARKED_DEST');await open(unmarked);await createPlan(ud,'rocket-ui-test/unmarked');assert(wt().preview_loaded);const denied=await client.request('tools/call',{name:'rocket_click',arguments:{target:'dialog-submit',expected_tab:state.active_tab}});assert(denied.isError);assert(!fs.existsSync(ud));await closeDialog();
 const linked=rd;await open(rc);await browse();await select(linked);const marker=linked+'.rocket-worktree-fixture',saved=fs.readFileSync(marker);fs.unlinkSync(marker);const openDenied=await client.request('tools/call',{name:'rocket_click',arguments:{target:'dialog-worktree-open',expected_tab:state.active_tab}});assert(openDenied.isError);fs.writeFileSync(marker,'rocket-ui-test/wrong-branch\n'+rc.repo);const branchDenied=await client.request('tools/call',{name:'rocket_click',arguments:{target:'dialog-worktree-open',expected_tab:state.active_tab}});assert(branchDenied.isError);fs.writeFileSync(marker,saved);await closeDialog();
 for(const fixture of fixtures)assert.equal(hash(path.join(fixture.repo,'.git/config')),fixture.config,'Repository config changed');
 assert.deepEqual(snapshot(f),original);assert.equal(fs.readFileSync(path.join(f.repo,'untracked'),'utf8'),'PRIVATE_UNTRACKED\n');assert.equal(fs.readFileSync(path.join(f.repo,'ignored/cache'),'utf8'),'PRIVATE_IGNORED\n');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const secret of ['PRIVATE_','検索',FILE,root,'rocket-ui-test/'])assert(!payload.includes(secret),'Diagnostic leak '+secret);
 for(const op of ['worktrees','worktree-preview','worktree-open','worktree-create','worktree-remove']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired '+op);}
 for(const step of ['list_worktrees','validate_worktree_open','review_create_worktree','review_remove_worktree','revalidate_worktree','prepare_worktree','publish_worktree','relink_worktree','observe_worktrees','verify_worktree_result','retain_worktree_commit','observe_worktree_recovery','remove_worktree'])assert(payload.includes(step),'Missing step '+step);
 for(const code of ['worktree_dirty','worktree_locked','worktree_destination_exists','worktree_invalid_branch','worktree_branch_in_use','worktree_branch_exists','worktree_branch_unavailable','worktree_target_unavailable'])assert(records.some(r=>r.failure_code===code),'Missing error '+code);
 assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;passed('MCP refuses unmarked creation/open; source state/config preserved; paired diagnostic requests, named phases, typed errors, cancellations, zero dropped events and no private payloads');report.passed=true;
}catch(error){if(client)try{await capture('failure');}catch{}throw error;}
finally{
 fs.writeFileSync('work/worktrees-last-result.txt',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});


