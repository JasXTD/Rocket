// Focused native acceptance: three conflict columns + explicit AI branch reviews.
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
 const claude=path.join(stubDir,'claude.exe'),codex=path.join(stubDir,'codex.exe');for(const p of [claude,codex])fs.copyFileSync(path.resolve('target/release/examples/claude_cli_fixture.exe'),p);
 fs.writeFileSync(path.join(stubDir,'reply'),'PRIVATE_REVIEW finding: check edited.rs:1 for a missing error path.');
 const calls=()=>fs.readdirSync(stubDir).filter(n=>/^request-\d+\.txt$/.test(n)).length;
 await open(r);await click('view-review');await type('review-base','rocket-ui-test/base');await type('review-target','rocket-ui-test/target');await type('review-program',claude);
 await click('review-prepare');assert.equal(review().error,null);assert.equal(calls(),0);assert.equal(review().preview.commits,1);assert.equal(review().preview.files,3);assert.equal(review().preview.binary_files,1);
 assert(review().payload.includes('new 文.rs'));for(const secret of ['PRIVATE_STAGED','PRIVATE_UNSTAGED','PRIVATE_UNTRACKED','PRIVATE_BASE_ONLY'])assert(!review().payload.includes(secret));await capture('06-review-preview');
 await click('review-send');assert.equal(calls(),1);assert.match(review().result,/PRIVATE_REVIEW/);assert.deepEqual(preserved(r,['local.txt','untracked.txt']),unchanged);await capture('07-claude-review');pass('Preview is local, pins a merge base, includes Unicode renames/binary counts, and Claude sends only the consented committed payload');
 await click('review-provider-codex');await type('review-program',codex);await click('review-send');assert.equal(calls(),2);assert.equal(review().error,null);assert.match(fs.readFileSync(path.join(stubDir,'arguments-2.txt'),'utf8'),/^--disable\r?\nshell_tool$/m);pass('Codex review uses the same pinned preview and existing restricted CLI flags');
 fs.unlinkSync(privateMarker);await until(s=>s.private_state_opt_in===false);assert.equal(review().payload,undefined);assert.equal(review().result,undefined);assert.equal(review().preview.files,3);fs.writeFileSync(privateMarker,'synthetic-content-only-v1\n');await until(s=>s.private_state_opt_in===true);
 const ids=review().result_ids.slice(),oldResult=review().result;await type('review-target','missing');assert.equal(control('review-send').enabled,false);await click('review-prepare');assert.equal(review().failure,'git_exit');assert.equal(calls(),2);assert.equal(review().result,oldResult);
 await type('review-target','rocket-ui-test/target');await click('review-prepare');const p=review().preview;r.git(['update-ref','refs/heads/rocket-ui-test/target',p.base,p.target]);await click('review-send');assert.equal(review().failure,'review_refs_changed');assert.equal(calls(),2);r.git(['update-ref','refs/heads/rocket-ui-test/target',p.target,p.base]);assert.deepEqual(review().result_ids,ids);
 await click('review-prepare');pass('MCP defaults redact private payload/findings; invalid inputs and externally moved refs block sending while keeping prior findings');
 for(const action of ['cancel','navigate']){
  const oldPayload=review().payload,callCount=calls();if(fs.existsSync(marker))fs.unlinkSync(marker);fs.writeFileSync(gate,'merge-base');
  await client.tool('rocket_click',{target:'review-prepare',expected_tab:state.active_tab});await until(()=>review().preparing&&fs.existsSync(marker),'Gated preview');
  const owned=fs.readFileSync(marker,'utf8').split('\n').map(Number);
  await client.tool('rocket_click',{target:action==='cancel'?'review-cancel':'view-changes',expected_tab:state.active_tab});await settle();fs.unlinkSync(gate);
  await until(()=>owned.every(pid=>{try{process.kill(pid,0);return false}catch{return true}}),'Preview process stopped');
  if(action==='navigate')await click('view-review');
  assert.equal(review().payload,oldPayload);assert.equal(review().result,oldResult);assert.equal(calls(),callCount);assert.equal(control('review-send').enabled,false);assert.match(review().notice,/Nothing was sent/);
  await click('review-prepare');
 }
 pass('Cancelling or leaving a live preview stops its Git process tree and invalidates the retained old preview without sending any AI request');
 for(const mode of ['signin','garbage']){fs.writeFileSync(path.join(stubDir,'mode'),mode);await click('review-send');assert.equal(review().failure,mode==='signin'?'draft_sign_in_required':'draft_invalid_reply');assert.equal(review().result,oldResult)}await capture('08-review-error');pass('Authentication and malformed CLI replies have clear errors, preserving earlier findings');
 fs.writeFileSync(path.join(stubDir,'mode'),'slow');await stable();await client.tool('rocket_click',{target:'review-send',expected_tab:state.active_tab});await until(()=>review().running&&fs.existsSync(path.join(stubDir,'owned-processes')),'AI child started');
 const pids=fs.readFileSync(path.join(stubDir,'owned-processes'),'utf8').split(' ').map(Number),start=performance.now();await client.tool('rocket_click',{target:'review-cancel',expected_tab:state.active_tab});await until(s=>!s.input_pending);report.timings.cancel_feedback_ms=Math.round(performance.now()-start);assert(review().cancelling||/cancelled/.test(review().notice||''));await settle();report.timings.cancel_settled_ms=Math.round(performance.now()-start);
 for(const pid of pids){let alive=false;try{process.kill(pid,0);alive=true}catch{}assert(!alive,'Owned CLI process still alive')}assert.equal(review().result,oldResult);assert.match(review().notice,/provider may already/);assert.deepEqual(preserved(r,['local.txt','untracked.txt']),unchanged);pass('Cancellation stops the actual synthetic CLI process tree, reports stopping promptly and retains findings/local state');
 fs.unlinkSync(path.join(stubDir,'owned-processes'));await client.tool('rocket_click',{target:'review-send',expected_tab:state.active_tab});await until(()=>review().running&&fs.existsSync(path.join(stubDir,'owned-processes')),'Second AI child started');
 const navigationPids=fs.readFileSync(path.join(stubDir,'owned-processes'),'utf8').split(' ').map(Number);await client.tool('rocket_click',{target:'view-changes',expected_tab:state.active_tab});await settle();
 await until(()=>navigationPids.every(pid=>{try{process.kill(pid,0);return false}catch{return true}}),'Navigation stopped AI');await click('view-review');assert.equal(review().result,oldResult);assert.deepEqual(review().result_ids,ids);pass('Leaving AI review stops the owned process tree and keeps earlier findings bound to their original commit IDs');
 fs.writeFileSync(path.join(stubDir,'mode'),'ok');await resize(780,620);await capture('09-review-compact');await click('toolbar-console');
 await scroll('review-page-scroll','down',700);await stable();const bottom=control('console-close').rect[1];
 for(const id of ['review-send','review-show-preview','review-show-result']){const c=control(id);assert(c?.enabled&&c.rect[1]>=115&&c.rect[3]<bottom,'Compact control must be reachable above console: '+id)}
 assert(control('review-text-scroll').rect[3]-control('review-text-scroll').rect[1]>=80,'Readable compact preview');await capture('10-review-compact-console');await click('console-close');await resize(1480,920);await scroll('review-page-scroll','up',2000);
 await type('review-program',path.join(stubDir,'missing.exe'));await click('review-send');assert.equal(review().failure,'draft_codex_missing');await type('review-program',codex);await click('review-send');assert.equal(review().error,null);pass('Missing CLI is explained and retry works; compact review and console layouts captured');
 await close();await client.raw('POST','/shutdown');const records=readLogs();assert(records.every(r=>r.dropped_before===0));const serialized=JSON.stringify(records);for(const secret of ['PRIVATE_',conflictPath,'new 文.rs',r.target])assert(!serialized.includes(secret),'Private payload leaked into logs');
 for(const op of ['branch-review-preview','branch-review','branch-review-cancel'])assert(records.some(r=>r.operation===op),'Missing '+op);for(const failure of ['review_refs_changed','draft_sign_in_required','draft_invalid_reply'])assert(serialized.includes(failure),'Missing failure code '+failure);
 for(const start of records.filter(r=>r.event==='request_start'&&r.operation.startsWith('branch-review')))assert(records.some(r=>r.event==='request_end'&&r.request===start.request&&r.session===start.session),'Unpaired span');report.log_records=records.length;pass('Paired review diagnostics, typed failures, no tested payload leaks and zero dropped events');report.passed=true;
}catch(e){report.failure=e.stack;if(client)try{await capture('failure')}catch{}throw e}
finally{if(fs.existsSync(gate))fs.unlinkSync(gate);fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync('work/conflict-review-last-result',path.join(root,'result.json'));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{await close();await client.raw('POST','/shutdown')}catch{}await delay(250);if(service.exitCode===null)service.kill()}if(client)await client.finish()}}
run().catch(e=>{console.error(e);process.exitCode=1});

