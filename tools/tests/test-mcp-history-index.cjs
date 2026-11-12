// Native acceptance of Speed up history through authenticated loopback MCP, using only
// owned local fixtures: a small repository for the dialog's states, and a generated one
// with 300,000 commits, where ordering history really is slow, for the notice, progress
// and Stop. Writes screenshots and result.json under work/mcp-fixtures/history-index-*.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/history-index-'+crypto.randomUUID().replaceAll('-','')),profile=path.join(root,'http-profile'),branch='rocket-ui-test/history-index';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report={passed:false,version,root,cases:[],timings:{}},BIG=Number(process.env.ROCKET_HISTORY_INDEX_COMMITS||300000);
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'),delay=ms=>new Promise(r=>setTimeout(r,ms));
function repository(name){
 const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
 const git=(args,input)=>cp.execFileSync('git',['-C',repo,...args],{env,input,windowsHide:true,encoding:'utf8',maxBuffer:1<<28,stdio:['pipe','pipe','pipe']}).trimEnd();
 git(['init','--initial-branch='+branch]);for(const [k,v]of [['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',k,v]);
 const info=path.join(repo,'.git/objects/info');
 return {repo,git,info,lock:path.join(info,'commit-graph.lock'),
  files:()=>({single:fs.existsSync(path.join(info,'commit-graph')),chain:fs.existsSync(path.join(info,'commit-graphs/commit-graph-chain'))}),
  // Everything a write of the index must leave alone.
  kept:()=>git(['for-each-ref'])+'\n'+git(['rev-parse','HEAD'])+'\n'+git(['status','--porcelain=v2','--branch']),
  setting:()=>{try{return git(['config','--local','--get','rocket.commitGraphOnFetch'])}catch{return null}},
  mark(){fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);this.config=hash(path.join(repo,'.git/config'));return this}};
}
function small(name,count=7){
 const f=repository(name);
 for(let i=0;i<count;i++){fs.mkdirSync(path.join(f.repo,'src'),{recursive:true});fs.writeFileSync(path.join(f.repo,'src/結果 é.rs'),'version '+i+'\n');f.git(['add','.']);f.git(['commit','-m','PRIVATE_HISTORY version '+i]);}
 return f.mark();
}
// One line of history with `commits` commits over 50 small files, through fast-import.
function large(name,commits){
 const f=repository(name),parts=[];let time=1500000000;
 for(let i=1;i<=commits;i++){
  time+=600;const who=`Author ${i%40} <author${i%40}@example.invalid> ${time} +0000`,message=`PRIVATE_HISTORY change ${i}\n`,body=`value ${i}\n`;
  parts.push(`commit refs/heads/${branch}\nauthor ${who}\ncommitter ${who}\ndata ${message.length}\n${message}M 100644 inline src/f${String(i%50).padStart(2,'0')}.txt\ndata ${body.length}\n${body}\n`);
 }
 const start=performance.now();f.git(['fast-import','--quiet'],parts.join(''));f.git(['reset','--hard','--quiet']);
 report.timings.generate_ms=Math.round(performance.now()-start);
 return f.mark();
}
let client,service,state;
const passed=text=>{report.cases.push(text);console.error('PASS '+text)};
async function until(predicate,label='UI state',limit=120000){
 const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<limit,label+' timed out; '+JSON.stringify({dialog:state.dialog?.kind,write:state.write,notice:state.notice,controls:state.controls.map(c=>c.id).filter(id=>/history|dialog|cancel/.test(id))}));await delay(100);}
}
const control=id=>state.controls.find(c=>c.id===id),has=id=>state.controls.some(c=>c.id===id),latest=()=>state.activities.at(-1);
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!s.write&&!(s.tabs||[]).some(t=>t.loading||t.search.running));}
async function stable(){let last='',n=0;await until(s=>{let next=JSON.stringify(s.controls);n=next===last?n+1:0;last=next;return n>=3},'Stable controls');}
async function scroll(target,direction='down',pixels=500){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settle();await stable();}
async function press(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});}
async function click(target){await press(target);await settle();await stable();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const img=r.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));}
// The same without waiting for the controls to rest, for a state that does not last.
async function snap(name){const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();await stable();}
// The dialog has read the index once it shows a decision: write, update or the result.
const decided=s=>s.dialog?.kind==='history-index'&&s.controls.some(c=>c.id==='dialog-submit'||c.id==='dialog-history-index-written');
async function menu(){
 await click('menu-repository');await scroll('repository-menu-scroll','up',2000);await click('repository-history-menu');
 for(let n=0;n<8;n++){const c=control('repository-history-index');if(c?.enabled&&c.rect[3]-c.rect[1]>=28)break;await stable();}
 await click('repository-history-index');assert.equal(state.dialog.kind,'history-index');await until(decided,'the dialog read the index');await stable();
}
async function open(f){
 if(state?.dialog)await click('dialog-cancel');console.error('OPEN '+path.basename(f.repo));
 await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');
 if(has('view-commits'))await click('view-commits');
}
// A write from the dialog, through to the refresh that follows it.
async function apply(target='dialog-submit'){await click(target);await until(decided,'the dialog after '+target);await stable();}
// A failed write opens the console pane behind the dialog; it can only be closed once the dialog is.
async function closeDialog(){await click('dialog-cancel');if(control('console-close'))await click('console-close');}
async function closeWindow(){
 if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');
 await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');
 if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}
}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
 const port=await new Promise(resolve=>{const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})});fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
 const log=fs.openSync(path.join(root,'service.stderr.log'),'a');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}
 assert.equal((await client.initialize()).serverInfo.version,version);

 // A small repository orders in milliseconds: no notice, and the dialog from the menu.
 const f=small('PRIVATE_SMALL 中文');let kept=f.kept();
 await open(f);assert(!has('history-speed-notice'),'a fast repository shows no notice');
 await menu();assert(control('dialog-submit').enabled&&has('dialog-history-index-keep'));assert(!has('dialog-history-index-remove')&&!has('dialog-history-index-state'));
 assert.deepEqual(f.files(),{single:false,chain:false});await capture('review');
 await resize(780,620);for(const id of ['dialog-submit','dialog-history-index-keep','dialog-cancel']){const r=control(id).rect;assert(r[0]>=0&&r[1]>=0&&r[2]<=780&&r[3]<=620,id+' clipped')}
 await capture('review-small');await resize(1480,920);
 await apply();assert(latest().command_succeeded,JSON.stringify(latest()));
 assert(has('dialog-history-index-written')&&has('dialog-history-index-remove')&&!has('dialog-submit'));await until(()=>has('dialog-history-index-after'),'the time with the index');
 assert.deepEqual(f.files(),{single:true,chain:false});assert.equal(f.setting(),'true');assert.equal(f.kept(),kept);f.git(['commit-graph','verify']);await capture('done');
 passed('Menu entry opens the review; Write index makes Git\'s single file, stores the fetch choice and changes no reference, HEAD or file state; narrow and wide layouts fit');
 await apply('dialog-history-index-remove');assert(latest().command_succeeded);assert(control('dialog-submit').enabled&&!has('dialog-history-index-written'));assert.deepEqual(f.files(),{single:false,chain:false});assert.equal(f.kept(),kept);
 await apply();await closeDialog();f.git(['commit','--allow-empty','-m','PRIVATE_HISTORY later']);kept=f.kept();
 await click('refresh');await menu();assert(has('dialog-history-index-state')&&has('dialog-history-index-keep')&&has('dialog-history-index-remove')&&control('dialog-submit').enabled);await capture('present');
 await apply();assert(latest().command_succeeded);assert.deepEqual(f.files(),{single:false,chain:true});f.git(['commit-graph','verify']);assert.equal(f.kept(),kept);
 await closeDialog();await menu();await apply('dialog-history-index-keep');assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(f.setting(),null);
 passed('Remove deletes the index; reopened, the dialog shows the index it finds, Update adds later commits as a layer, and the checkbox stores the fetch choice at once');
 fs.writeFileSync(f.lock,'another writer');await closeDialog();await menu();await apply();
 assert(!latest().command_succeeded);assert(has('dialog-history-index-failure')&&has('dialog-history-index-copy')&&control('dialog-submit').enabled);assert.equal(fs.readFileSync(f.lock,'utf8'),'another writer');assert.equal(f.kept(),kept);await capture('failure');
 fs.unlinkSync(f.lock);await apply();assert(latest().command_succeeded);assert(!has('dialog-history-index-failure'));f.git(['commit-graph','verify']);
 passed('Another writer\'s lock refuses the write with its reason, is left in place, and Try again succeeds once it is gone');

 // A large one: the notice, its ×, then progress, Stop and the measured result.
 const b=large('PRIVATE_LARGE',BIG);kept=b.kept();
 let start=performance.now();await open(b);report.timings.open_ms=Math.round(performance.now()-start);
 assert(!has('history-speed-notice'),'one slow load is not enough for the notice');await click('refresh');
 await until(()=>has('history-speed-notice'),'the notice for slow ordering (is this machine ordering '+BIG+' commits in under a second?)');
 assert(control('history-speed-open').enabled&&control('history-speed-hide').enabled);await capture('notice');
 await click('history-speed-open');await until(decided,'the dialog from the notice');await delay(4000);await capture('large-review');
 start=performance.now();await press('dialog-submit');
 // Git reports a count once it has read commits for a second: wait for one, of the total counted beforehand.
 await until(s=>(s.write?.progress||'').endsWith(' of '+BIG.toLocaleString('en-US')+' commits')&&s.write.fraction>0&&s.controls.some(c=>c.id==='cancel-operation'&&c.enabled),'commit counts while writing');
 report.timings.first_progress_ms=Math.round(performance.now()-start);report.progress_seen=state.write.progress;await snap('large-writing');
 await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();await until(decided,'the dialog after Stop');
 assert(!latest().command_succeeded);assert.deepEqual(b.files(),{single:false,chain:false});assert.equal(b.kept(),kept);assert(!has('dialog-history-index-failure'),'Stop is not shown as a failure');await capture('large-stopped');
 // The backend cannot prove ownership of a surviving lock. Only this isolated
 // fixture runner knows no external writer exists and may remove it to retry.
 if(fs.existsSync(b.lock)){
  const retained=fs.readFileSync(b.lock);await apply();assert(!latest().command_succeeded);assert(has('dialog-history-index-failure'));assert.deepEqual(fs.readFileSync(b.lock),retained);fs.unlinkSync(b.lock);
 }
 passed('Slow ordering shows progress; Stop preserves refs and files, and any retained lock blocks retry until the isolated fixture owner clears it');
 await closeDialog();assert(has('history-speed-notice'));await click('history-speed-hide');assert(!has('history-speed-notice'));
 await click('refresh');assert(!has('history-speed-notice'),'the closed notice came back after a refresh');await capture('notice-hidden');
 await menu();start=performance.now();await apply();report.timings.write_ms=Math.round(performance.now()-start);
 assert(latest().command_succeeded,JSON.stringify(latest()));await until(()=>has('dialog-history-index-after'),'the time with the index');await capture('large-done');
 assert.deepEqual(b.files(),{single:true,chain:false});assert.equal(b.kept(),kept);b.git(['commit-graph','verify']);report.index_bytes=fs.statSync(path.join(b.info,'commit-graph')).size;
 await closeDialog();start=performance.now();await click('refresh');report.timings.refresh_with_index_ms=Math.round(performance.now()-start);assert(!has('history-speed-notice'));
 passed('The closed notice stays closed across a refresh; the menu still opens the dialog, and the write completes with the index verified by Git');

 for(const fixture of [f,b])assert.notEqual(fixture.config,undefined);
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(x=>/^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x=>fs.readFileSync(path.join(root,n,x),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const text=JSON.stringify(records);for(const secret of ['PRIVATE_',branch,'結果','中文'])assert(!text.includes(secret),'Diagnostic leak '+secret);
 for(const op of ['history-index-read','history-index-count','history-index-write','history-index-remove','history-index-setting']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const s of starts)assert(records.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request),'Unpaired request');}
 assert(text.includes('"commit-graph"'),'the Git command is logged by its own name');assert(records.some(r=>r.outcome==='cancelled'));
 report.log_records=records.length;passed('Private structured diagnostics name each request and the Git command, pair every request, record the cancellation and leak no fixture name');report.passed=true;
}catch(e){report.error=e.message;if(client)try{await capture('failure-state')}catch{}throw e}finally{
 fs.writeFileSync('work/history-index-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(200);if(service.exitCode===null)service.kill()}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});
