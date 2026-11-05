// Focused native acceptance: three conflict columns + explicit AI reviews of a branch.
// Only disposable local repositories and compiled synthetic CLI fixtures.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('work/mcp-fixtures/conflict-review-'+crypto.randomUUID().replaceAll('-','')),profile=path.join(root,'http-profile'),branch='rocket-ui-test/review';
const exe=path.resolve(process.env.ROCKET_TEST_MCP_EXE||'target/release/rocket-mcp.exe');
const report={passed:false,root,cases:[],timings:{}},privateMarker=path.join(root,'rocket-private-state');
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');fs.writeFileSync(privateMarker,'synthetic-content-only-v1\n');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const relay=gitRelay(root,env,'tools/helpers/file-history-preview/git-relay.rs'),{gate,marker}=relay;
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'),delay=ms=>new Promise(r=>setTimeout(r,ms));
let client,service,state;
function fixture(label){
 const repo=path.join(root,label);fs.mkdirSync(repo);const git=a=>cp.execFileSync('git',['-C',repo,...a],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe'],maxBuffer:32*1024*1024}).trimEnd();
 const write=(p,b)=>{const dest=path.join(repo,p);fs.mkdirSync(path.dirname(dest),{recursive:true});fs.writeFileSync(dest,b)};const commit=m=>{git(['add','-A']);git(['commit','-m',m]);return git(['rev-parse','HEAD'])};
 git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Synthetic Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false'],['core.hooksPath',path.join(root,'empty-template')],['merge.conflictStyle','diff3']])git(['config',k,v]);write('.git/rocket-ui-fixture',branch);return{repo,git,write,commit};
}
const conflictPath='src/界面/解決 🚀 é.rs';
function conflicted(){
 const f=fixture('PRIVATE_CONFLICT_文');const content=(a,b)=>Array.from({length:280},(_,i)=>i===12?a:i===190?b:'// common line '+i+' 日本語').join('\n')+'\n';
 f.write(conflictPath,content('base one','base two'));f.write('keep.txt','PRIVATE_KEEP');f.commit('base');
 f.git(['branch','rocket-ui-test/base']);f.git(['checkout','-b','rocket-ui-test/other']);f.write(conflictPath,content('PRIVATE_THEIRS first','PRIVATE_THEIRS second'));f.commit('theirs');f.git(['checkout',branch]);f.write(conflictPath,content('PRIVATE_OURS first','PRIVATE_OURS second'));f.commit('ours');
 assert.throws(()=>f.git(['merge','rocket-ui-test/other']));return f;
}
function reviewFixture(){
 const f=fixture('PRIVATE_REVIEW_文');f.write('old 文.rs','fn hello() { println!("界 🚀"); }\n');f.write('edited.rs','fn change() { old(); }\n');f.write('local.txt','base\n');f.commit('base');f.git(['branch','rocket-ui-test/base']);
 f.git(['mv','old 文.rs','new 文.rs']);f.write('edited.rs','fn change() { new_call(); }\n');f.write('binary.bin',Buffer.from([0,1,2]));const target=f.commit('target');f.git(['branch','rocket-ui-test/target']);
 f.git(['checkout','rocket-ui-test/base']);f.write('base-only.txt','PRIVATE_BASE_ONLY');f.commit('base diverges');f.git(['checkout',branch]);
 f.write('local.txt','PRIVATE_STAGED\n');f.git(['add','local.txt']);f.write('local.txt','PRIVATE_UNSTAGED\n');f.write('untracked.txt','PRIVATE_UNTRACKED');
 return Object.assign(f,{target});
}
function preserved(f,names){return{head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),files:Object.fromEntries(names.map(n=>[n,hash(path.join(f.repo,n))]))}}
function tab(){return state.tabs?.find(t=>t.id===state.active_tab)}const editor=()=>tab()?.conflict_editor,review=()=>tab()?.branch_review,control=id=>state.controls?.find(c=>c.id===id);
function pass(s){report.cases.push(s);console.log('PASS '+s)}
async function until(predicate,label='UI',timeout=60000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return;assert(performance.now()-start<timeout,label+' timed out');await delay(50)}}
async function settle(){await until(s=>!s.busy&&!s.input_pending,'Idle')}
async function stable(){let old='',same=0;await until(s=>{const now=JSON.stringify(s.controls);same=now===old?same+1:0;old=now;return !s.input_pending&&same>=2},'Stable controls')}
async function click(id){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});await settle();await stable()}
async function type(id,text){await stable();assert(control(id)?.enabled,'Missing input '+id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle()}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle()}
async function scroll(target,direction,pixels){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settle();await stable()}
// AI review reads by itself: wait for typing to settle and the read (or a send that reads again) to end.
async function readDone(){await until(()=>review()&&!review().waiting&&!review().running,'Review read');await stable()}
async function sent(){await until(()=>review()&&!review().running,'Review sent');await stable()}
async function reveal(id){await stable();for(let i=0;i<12&&!(control(id)&&control(id).rect[3]-control(id).rect[1]>=16);i++){await client.tool('rocket_scroll',{target:'dialog-scroll',direction:'down',pixels:160,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}assert(control(id),'Not visible: '+id);}
async function revealPage(id,bottom){for(let i=0;i<12;i++){const c=control(id);if(c&&c.rect[1]>=115&&c.rect[3]<bottom)return;await scroll('review-page-scroll','down',120)}}
// Preferences → Commit messages holds each CLI's program, which AI review shares.
async function program(text,provider){await click('menu-edit');await click('preferences');await until(s=>s.dialog&&control('pref-page-commit'),'Preferences');await click('pref-page-commit');if(!state.drafting.enabled)await click('pref-ai-enabled');await reveal('pref-ai-provider-'+provider);await click('pref-ai-provider-'+provider);await reveal('pref-ai-executable');await type('pref-ai-executable',text);await click('dialog-submit');await until(s=>!s.dialog,'Preferences closed');}
async function capture(label){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,label+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,label+'.json'),JSON.stringify(state,null,2))}
async function open(f){await client.tool('rocket_open',{path:f.repo});await settle();relay.check(state);await resize(1480,920);if(control('console-close'))await click('console-close')}
async function close(){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window))}}
const readLogs=()=>fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
async function run(){try{
 report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const server=net.createServer();await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;await new Promise(r=>server.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 report.endpoint='http://127.0.0.1:'+port+'/mcp';const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore',log,log]});fs.closeSync(log);
 client=new RocketHttpClient(profile);for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}report.version=(await client.initialize()).serverInfo.version;
 const f=conflicted(),before=preserved(f,[conflictPath,'keep.txt']);await open(f);await click('view-changes');await click('change-unstaged-'+Buffer.from(conflictPath).toString('hex'));await click('conflict-open');
 assert.equal(editor().blocks,2);assert.equal(editor().three_columns,true);assert.equal(editor().linked,true);const draft=editor().draft;await capture('01-three-columns');
 await scroll('conflict-ours-scroll','down',380);assert(editor().scroll_y.every(v=>v>150),'All three panes must move');const y=editor().scroll_y;assert(Math.max(...y)-Math.min(...y)<130,'Common context should remain near-aligned');assert.equal(editor().draft,draft);
 await click('conflict-linked');const unlinked=editor().scroll_y.slice();await scroll('conflict-theirs-scroll','down',250);assert(editor().scroll_y[2]>unlinked[2]+100);assert(Math.abs(editor().scroll_y[1]-unlinked[1])<2);await click('conflict-linked');
 await click('conflict-next');assert.equal(editor().selected,1);assert(editor().scroll_y.every(v=>v>1500));await click('conflict-marker-0');assert.equal(editor().selected,0);assert(editor().scroll_y.every(v=>v<500));await capture('02-navigation');
 assert.deepEqual(preserved(f,[conflictPath,'keep.txt']),before);pass('Three real panes, linked/unlinked wheel scrolling, common-line alignment and marker/next navigation preserve editor bytes');
 await resize(780,620);assert.equal(editor().three_columns,false);await capture('03-compact');await click('toolbar-console');await capture('04-compact-console');await click('console-close');await resize(1480,920);
 await click('conflict-ours');await click('conflict-both');assert.equal(editor().blocks,0);const chosen=editor().draft;await click('conflict-save');await click('dialog-cancel');assert.equal(editor().draft,chosen);assert.deepEqual(preserved(f,[conflictPath,'keep.txt']),before);pass('Compact panes with console, Unicode block choices and cancelled save review preserve originals');
 f.write(conflictPath,'PRIVATE_EXTERNAL edit\n');const external=hash(path.join(f.repo,conflictPath));await click('conflict-save');await click('dialog-submit');assert.match(editor().error,/changed outside/);assert.equal(editor().draft,chosen);assert.equal(hash(path.join(f.repo,conflictPath)),external);await capture('05-external-edit');pass('External edit is refused and the unsaved resolution remains available');
 const r=reviewFixture(),unchanged=preserved(r,['local.txt','untracked.txt']);const stubDir=path.join(root,'AI 文');fs.mkdirSync(stubDir);
 const claude=path.join(stubDir,'claude.exe'),codex=path.join(stubDir,'codex.exe');cp.execFileSync('rustc',['--edition=2024','--crate-name=claude_cli_fixture','-C','opt-level=1',path.resolve('crates/rocket-git/examples/claude_cli_fixture.rs'),'-o',claude],{windowsHide:true,stdio:'pipe'});fs.copyFileSync(claude,codex);report.stub_sha256=hash(claude);
 const findingsReply=JSON.stringify({summary:'PRIVATE_REVIEW summary: one bug.',findings:[{kind:'bug',file:'edited.rs',line:1,title:'PRIVATE_REVIEW finding',why:'new_call() has no error path.',fix:'Handle its error.'},{kind:'check',file:'nowhere.rs',line:3,title:'PRIVATE_REVIEW check outside the changes',why:'',fix:''}],not_checked:'PRIVATE_REVIEW tests were not in the diff.'});
 fs.writeFileSync(path.join(stubDir,'reply'),findingsReply);
 const calls=()=>fs.readdirSync(stubDir).filter(n=>/^request-\d+\.txt$/.test(n)).length,dead=pids=>pids.every(pid=>{try{process.kill(pid,0);return false}catch{return true}});
 await open(r);await program(codex,'codex');await program(claude,'claude');await click('view-review');await stable();
 // The fixture's own branch and no main: nothing is read until a base is chosen.
 assert.equal(review().base,'');assert.equal(review().target,branch);assert.equal(review().preview,null);assert.equal(review().preparing,false);assert.equal(calls(),0);assert(control('review-choose'),'Choose branch');assert.equal(control('review-send').enabled,false);await capture('06-review-choose');
 await click('review-choose');await click('review-choose-0');await readDone();assert.equal(review().base,'rocket-ui-test/base');assert.equal(review().preview.commits,1);
 await type('review-target','rocket-ui-test/target');await readDone();assert.equal(review().target,'rocket-ui-test/target');assert.equal(review().error,null);assert.equal(review().changed_inputs,false);assert.equal(calls(),0);assert.equal(review().provider,'claude');
 assert.equal(review().preview.files,3);assert.equal(review().preview.binary_files,1);assert.deepEqual(review().preview.rows.map(x=>x.sent).sort(),['binary','diff','diff']);
 assert(review().payload.includes('new 文.rs'));for(const secret of ['PRIVATE_STAGED','PRIVATE_UNSTAGED','PRIVATE_UNTRACKED','PRIVATE_BASE_ONLY'])assert(!review().payload.includes(secret));
 await click('review-file-'+review().preview.rows.findIndex(x=>x.change==='M'));assert(control('review-diff-scroll'),'The selected file shows its diff as sent');assert(control('review-send').enabled);await capture('06-review-preview');
 await click('review-exact');assert.equal(review().show_exact,true);assert(control('review-exact-scroll'));await capture('06b-review-exact');await click('review-exact');assert.equal(review().show_exact,false);
 pass('The page reads by itself, only once a base is chosen; it pins a merge base, shows each file as sent with Unicode renames and binary files named only, and sends nothing');
 await click('review-send');await sent();assert.equal(calls(),1);assert.match(review().result,/PRIVATE_REVIEW/);assert.equal(review().findings,2);assert.equal(review().tab,'findings');assert.equal(review().reviewed_now,true);assert.deepEqual(preserved(r,['local.txt','untracked.txt']),unchanged);
 assert(control('review-finding-0'),'A finding in these changes links to them');assert.equal(control('review-finding-1'),undefined,'A file outside the changes has no link');assert(control('review-copy')?.enabled);await capture('07-claude-review');
 await click('review-finding-0');assert.equal(review().tab,'changes');assert.equal(review().selected,review().preview.rows.findIndex(x=>x.change==='M'));assert.equal(review().mark,1);await capture('07b-show-in-changes');
 pass('Claude Code gets only the shown payload; findings come back as cards bound to their commit IDs, and Show in Changes opens the file at its line');
 await click('review-provider-codex');assert.equal(review().provider,'codex');await click('review-send');await sent();assert.equal(calls(),2);assert.equal(review().error,null);assert.match(fs.readFileSync(path.join(stubDir,'arguments-2.txt'),'utf8'),/^--disable\r?\nshell_tool$/m);
 pass('Codex reviews the same pinned changes with the existing restricted CLI flags');
 fs.writeFileSync(path.join(stubDir,'reply'),'PRIVATE_REVIEW plain: check edited.rs:1 for a missing error path.');await click('review-send');await sent();assert.equal(calls(),3);assert.equal(review().plain,true);assert.equal(review().findings,null);
 const placeId=state.controls.find(c=>/^review-place-/.test(c.id))?.id;assert(placeId,'A file:line in plain text links to it');await capture('07c-plain-reply');await click(placeId);assert.equal(review().tab,'changes');assert.equal(review().mark,1);
 fs.writeFileSync(path.join(stubDir,'reply'),findingsReply);await click('review-provider-claude');
 pass('A reply that is not a list shows as written, with its file:line places linked to the changes');
 fs.unlinkSync(privateMarker);await until(s=>s.private_state_opt_in===false);assert.equal(review().payload,undefined);assert.equal(review().result,undefined);assert.equal(review().preview.files,3);fs.writeFileSync(privateMarker,'synthetic-content-only-v1\n');await until(s=>s.private_state_opt_in===true);
 const ids=review().result_ids.slice(),kept=review().result;await type('review-target','missing');await readDone();assert.equal(review().invalid,'target');assert.equal(review().failure,'review_target_not_found');assert.equal(control('review-send').enabled,false);assert.equal(calls(),3);assert.equal(review().result,kept);await capture('08-review-missing');
 await type('review-target','rocket-ui-test/target');await readDone();assert.equal(review().invalid,null);assert.equal(review().reviewed_now,true);
 const p=review().preview,tree=r.git(['rev-parse',p.target+'^{tree}']),moved=r.git(['commit-tree',tree,'-p',p.target,'-m','moved']);r.git(['update-ref','refs/heads/rocket-ui-test/target',moved,p.target]);
 await click('review-send');await readDone();assert.equal(calls(),3);assert.match(review().notice,/rocket-ui-test\/target moved, so Rocket read it again\. Nothing was sent\./);assert.equal(review().preview.target,moved);assert.equal(review().preview.commits,2);assert.deepEqual(review().result_ids,ids);await capture('08b-review-moved');
 r.git(['update-ref','refs/heads/rocket-ui-test/target',p.target,moved]);await click('review-read');await readDone();assert.equal(review().preview.target,p.target);assert.equal(review().reviewed_now,true);
 pass('MCP redacts the payload and findings by default; an unknown name is marked and blocks Send; a moved branch is read again instead of sent, and earlier findings keep their commit IDs');
 for(const action of ['cancel','navigate']){
  const oldPayload=review().payload,callCount=calls();if(fs.existsSync(marker))fs.unlinkSync(marker);fs.writeFileSync(gate,'merge-base');
  await client.tool('rocket_click',{target:'review-read',expected_tab:state.active_tab});await until(()=>review().preparing&&fs.existsSync(marker),'Gated read');
  const owned=fs.readFileSync(marker,'utf8').split('\n').map(Number);if(action==='cancel'){await until(()=>control('review-cancel'),'Stop while reading');await capture('08c-review-reading')}
  await client.tool('rocket_click',{target:action==='cancel'?'review-cancel':'view-changes',expected_tab:state.active_tab});await settle();fs.unlinkSync(gate);
  await until(()=>dead(owned),'Read process stopped');if(action==='navigate')await click('view-review');
  await until(()=>!review().running,'Read ended');assert.equal(review().payload,oldPayload);assert.equal(calls(),callCount);assert.match(review().notice,/Stopped reading\. Nothing was sent\./);assert(control('review-send').enabled,'The last preview is still the one shown');
 }
 pass('Stopping or leaving a running read stops its Git process tree, keeps the last preview and sends nothing');
 for(const mode of ['signin','garbage']){fs.writeFileSync(path.join(stubDir,'mode'),mode);await click('review-send');await sent();assert.equal(review().failure,mode==='signin'?'draft_sign_in_required':'draft_invalid_reply');assert.equal(review().result,kept);if(mode==='signin'){assert(control('review-details'),'Details');await click('review-details');await capture('08d-review-signin')}}
 pass('Sign-in and malformed replies say what to do, with the code under Details, and keep earlier findings');
 fs.writeFileSync(path.join(stubDir,'mode'),'slow');await stable();await client.tool('rocket_click',{target:'review-send',expected_tab:state.active_tab});await until(()=>review().sending&&fs.existsSync(path.join(stubDir,'owned-processes')),'AI child started');await capture('08e-review-sending');
 const pids=fs.readFileSync(path.join(stubDir,'owned-processes'),'utf8').split(' ').map(Number),start=performance.now();await client.tool('rocket_click',{target:'review-cancel',expected_tab:state.active_tab});await until(s=>!s.input_pending);report.timings.cancel_feedback_ms=Math.round(performance.now()-start);assert(review().cancelling||/may already have/.test(review().notice||''));await until(()=>!review().running,'Review stopped');report.timings.cancel_settled_ms=Math.round(performance.now()-start);
 assert(dead(pids),'Owned CLI process still alive');assert.equal(review().result,kept);assert.match(review().notice,/Claude Code may already have the preview/);assert.deepEqual(preserved(r,['local.txt','untracked.txt']),unchanged);await capture('08f-review-stopped');pass('Stop ends the actual synthetic CLI process tree promptly and keeps findings and local state');
 fs.unlinkSync(path.join(stubDir,'owned-processes'));await client.tool('rocket_click',{target:'review-send',expected_tab:state.active_tab});await until(()=>review().sending&&fs.existsSync(path.join(stubDir,'owned-processes')),'Second AI child started');
 const navigationPids=fs.readFileSync(path.join(stubDir,'owned-processes'),'utf8').split(' ').map(Number);await client.tool('rocket_click',{target:'view-changes',expected_tab:state.active_tab});await settle();
 await until(()=>dead(navigationPids),'Navigation stopped AI');await click('view-review');assert.equal(review().result,kept);assert.deepEqual(review().result_ids,ids);pass('Leaving AI review stops the owned process tree and keeps earlier findings bound to their commit IDs');
 fs.writeFileSync(path.join(stubDir,'mode'),'ok');await resize(780,620);await click('review-tab-changes');await capture('09-review-compact');
 assert(control('review-files')&&control('review-diff'),'A narrow window switches between files and diff');await click('review-files');assert.equal(review().show_files,true);await capture('09b-review-compact-files');await click('review-diff');
 await click('toolbar-console');await stable();const bottom=control('console-close').rect[1];
 for(const id of ['review-send','review-tab-changes','review-tab-findings']){await revealPage(id,bottom);const c=control(id);assert(c?.enabled&&c.rect[1]>=115&&c.rect[3]<bottom,'Compact control must be reachable above console: '+id)}
 await capture('10-review-compact-console');await click('console-close');await resize(1480,920);await scroll('review-page-scroll','up',2000);
 await program(path.join(stubDir,'missing.exe'),'claude');await stable();assert.equal(review().provider,'claude');assert.equal(control('review-send').enabled,false,'No Send while Claude Code is not found');assert(control('review-browse')?.enabled,'Not found offers Browse');await capture('11-review-not-found');
 await program(claude,'claude');await stable();assert(control('review-send').enabled);await click('review-send');await sent();assert.equal(review().error,null);assert.equal(review().findings,2);pass('A missing program shows Not found with Browse and blocks Send; with the program back, sending works; compact and console layouts captured');
 await close();await client.raw('POST','/shutdown');const records=readLogs();assert(records.every(r=>r.dropped_before===0));const serialized=JSON.stringify(records);for(const secret of ['PRIVATE_',conflictPath,'new 文.rs',r.target])assert(!serialized.includes(secret),'Private payload leaked into logs');
 for(const op of ['branch-review-preview','branch-review','branch-review-cancel'])assert(records.some(r=>r.operation===op),'Missing '+op);for(const failure of ['review_refs_changed','review_target_not_found','draft_sign_in_required','draft_invalid_reply'])assert(serialized.includes(failure),'Missing failure code '+failure);
 for(const start of records.filter(r=>r.event==='request_start'&&r.operation.startsWith('branch-review')))assert(records.some(r=>r.event==='request_end'&&r.request===start.request&&r.session===start.session),'Unpaired span');report.log_records=records.length;pass('Paired review diagnostics, typed failures, no tested payload leaks and zero dropped events');report.passed=true;
}catch(e){report.failure=e.stack;if(client)try{await capture('failure')}catch{}throw e}
finally{if(fs.existsSync(gate))fs.unlinkSync(gate);fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync('work/conflict-review-last-result',path.join(root,'result.json'));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{await close();await client.raw('POST','/shutdown')}catch{}await delay(250);if(service.exitCode===null)service.kill()}if(client)await client.finish()}}
run().catch(e=>{console.error(e);process.exitCode=1});

