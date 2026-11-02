// Only Compare and its affected navigation/diff neighbours. Authenticated numeric
// localhost MCP, native pointer/keyboard events, synthetic repositories, no remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict'),zlib=require('node:zlib');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/compare-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/compare';
const exe=path.resolve(process.env.ROCKET_COMPARE_EXE||'target/release/rocket-mcp.exe');
const report={passed:false,root,exe,cases:[],timings:{}};
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const hash=b=>crypto.createHash('sha256').update(b).digest('hex');
const hashFile=p=>hash(fs.readFileSync(p));
const delay=ms=>new Promise(r=>setTimeout(r,ms));
function fixture(name){
 const repo=path.join(root,name);fs.mkdirSync(repo);const hooks=path.join(root,name+'-hooks');fs.mkdirSync(hooks);
 const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',maxBuffer:32*1024*1024}).trimEnd();
 const write=(name,data)=>{const p=path.join(repo,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,data)};
 const commit=message=>{git(['add','--all','--']);git(['commit','-m',message]);return git(['rev-parse','HEAD'])};
 git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Rocket Fixture'],['user.email','rocket@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',k,v]);
 fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);return{repo,git,write,commit};
}
function crc32(b){let c=0xffffffff;for(const v of b){c^=v;for(let k=0;k<8;k++)c=(c>>>1)^((c&1)?0xedb88320:0)}return(c^0xffffffff)>>>0}
function png(red){const chunks=[];const chunk=(type,data)=>{const name=Buffer.from(type),head=Buffer.alloc(4),crc=Buffer.alloc(4);head.writeUInt32BE(data.length);crc.writeUInt32BE(crc32(Buffer.concat([name,data])));chunks.push(head,name,data,crc)};const ihdr=Buffer.alloc(13);ihdr.writeUInt32BE(1);ihdr.writeUInt32BE(1,4);ihdr[8]=8;ihdr[9]=6;chunk('IHDR',ihdr);chunk('IDAT',zlib.deflateSync(Buffer.from([0,red,80,160,255])));chunk('IEND',Buffer.alloc(0));return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),...chunks])}
const old='src/検索/旧名 🚀 é.rs',renamed='src/検索/表示 🚀 é.rs',feature='rocket-ui-test/検索';
function prepared(){
 const f=fixture('PRIVATE_COMPARE_文');
 f.write(old,'pub fn greeting() -> String {\n    "你好".into()\n}\n');f.write('literal [x].txt','before\n');f.write('remove.md','remove\n');f.write('assets/画像.png',png(40));f.write('binary.bin',Buffer.from([0,1,2]));f.write('local.txt','committed\n');
 const base=f.commit('PRIVATE_BASE');f.git(['tag','review-base',base]);f.git(['branch','rocket-ui-test/before',base]);
 f.git(['mv',old,renamed]);f.write('literal [x].txt','after\n');f.write('literal x.txt','must not match literal [x]\n');f.write('docs/新增.md','new content\n');f.write('assets/画像.png',png(200));f.write('binary.bin',Buffer.from([0,3,4]));f.git(['rm','remove.md']);
 for(let i=0;i<24;i++)f.write('docs/list-'+String(i).padStart(2,'0')+'.md','synthetic row '+i+'\n');
 const target=f.commit('PRIVATE_TARGET');f.git(['branch',feature,target]);
 f.write('docs/future.md','new branch tip\n');const future=f.commit('PRIVATE_FUTURE');f.git(['branch','rocket-ui-test/future',future]);
 // This is a fresh synthetic repository created above, before any user edits.
 f.git(['reset','--hard',target]);f.write('local.txt','staged local\n');f.git(['add','--','local.txt']);f.write('local.txt','unstaged local\n');f.write('untracked 文.txt','keep me\n');
 return Object.assign(f,{base,target,future});
}
function preserved(f){return{head:f.git(['rev-parse','HEAD']),index:hashFile(path.join(f.repo,'.git/index')),config:hashFile(path.join(f.repo,'.git/config')),local:hashFile(path.join(f.repo,'local.txt')),untracked:hashFile(path.join(f.repo,'untracked 文.txt'))}}
let client,service,state;
function tab(){return state.tabs?.find(t=>t.id===state.active_tab)}
function comparison(){return tab()?.compare}
function control(id){return state.controls?.find(c=>c.id===id)}
function passed(message){report.cases.push(message);console.log('PASS '+message)}
async function until(predicate,label='UI state',timeout=60000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<timeout,label+' timed out: '+JSON.stringify({busy:state.busy,compare:comparison(),dialog:state.dialog?.kind}));await delay(50)}}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running),'Idle UI')}
async function stable(){let old='',same=0;await until(s=>{const n=JSON.stringify(s.controls);same=n===old?same+1:0;old=n;return !s.input_pending&&same>=2},'Stable controls')}
async function click(id,args={}){await stable();assert(control(id)?.enabled,'Missing or disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab,...args});await settle()}
async function type(id,text){await stable();assert(control(id)?.enabled,'Missing or disabled input '+id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle()}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const img=r.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2))}
async function resize(w,h){await client.tool('rocket_resize',{width:w,height:h});await until(s=>s.viewport[0]===w&&s.viewport[1]===h&&!s.input_pending);await settle()}
async function scroll(target,direction,pixels){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settle();await stable()}
async function revealTool(id){if(control(id))return;await scroll('tools-scroll','up',2000);for(let i=0;!control(id)&&i<12;i++)await scroll('tools-scroll','down',220);assert(control(id),'Missing '+id)}
async function open(f){await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-compare')}
async function compare(base,target){await type('compare-base',base);await type('compare-target',target);await click('compare-load');assert.equal(comparison().error,null);assert.equal(comparison().running,false)}
async function select(name){const i=comparison().loaded.files.findIndex(f=>f.path===name);assert(i>=0,'Missing '+name);if(!control('compare-file-'+i)){await type('compare-filter',name);await stable()}await click('compare-file-'+i);assert.equal(comparison().selected,i);assert.equal(comparison().error,null);assert(tab().patch_files?.length)}
async function closeWindow(){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window))}}
async function cancelRead(kind){
 // The real Git listing provides an observable cancellation window.
 // No sleeps or fake Git helpers are added to Rocket's implementation.
 await stable();await client.tool('rocket_click',{target:kind,expected_tab:state.active_tab});
 await until(()=>comparison().running,'Comparison started');const start=performance.now();
 await client.tool('rocket_click',{target:'compare-cancel',expected_tab:state.active_tab});
 await until(s=>!s.input_pending,'Cancel delivered');
 report.timings[kind+'_feedback_ms']=Math.round(performance.now()-start);
 assert(comparison().cancelling||/cancelled/i.test(comparison().notice||''),'Cancellation feedback missing');
 await until(()=>!comparison().running,'Comparison stopped');report.timings[kind+'_settled_ms']=Math.round(performance.now()-start);
 assert.match(comparison().notice,/cancelled/i);await settle();
}
async function run(){try{
 report.executable_sha256=hashFile(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(n===100)throw e;await delay(100)}}
 report.version=(await client.initialize()).serverInfo.version;
 const f=prepared(),unchanged=preserved(f);await open(f);await compare('review-base',feature);
 assert.equal(comparison().loaded.base,f.base);assert.equal(comparison().loaded.target,f.target);assert.equal(comparison().loaded.count,31);
 const rename=comparison().loaded.files.find(x=>x.path===renamed);assert.equal(rename.original_path,old);assert.equal(rename.change,'Renamed');assert.equal(rename.similarity,100);
 await scroll('compare-files-scroll','down',2000);assert(control('compare-file-'+rename.index)?.enabled,'Last virtualized file must be reachable');
 await select(renamed);assert.equal(tab().patch_files[0].path,renamed);await capture('01-unicode-rename');
 await type('compare-filter','旧名');await stable();assert(control('compare-file-'+rename.index));await type('compare-filter','');
 await select('literal [x].txt');assert.equal(tab().patch_files.length,1);assert.equal(tab().patch_added,1);assert.equal(tab().patch_removed,1);await type('diff-search','after');await until(()=>!tab().diff_tools.searching&&tab().diff_tools.matches===1,'Diff search results');
 assert.deepEqual(preserved(f),unchanged);passed('Tag/branch comparison, Unicode rename paths, old-name filter, literal pathspecs, text diff/search and untouched local changes');

 const helper=path.join(root,'diff fixture 文.exe');fs.copyFileSync(path.resolve('target/release/examples/external_tool_fixture.exe'),helper);
 await click('diff-tools-menu');await click('diff-external');await click('tools-edit-profiles');await revealTool('tools-executable');await type('tools-executable',helper);await revealTool('tools-arguments');await type('tools-arguments','diff\n{left}\n{right}');await revealTool('tools-save-settings');await click('tools-save-settings');await click('dialog-submit');await revealTool('tools-start');await click('tools-start');await until(s=>!s.external_tools.running&&!s.input_pending,'External diff complete');assert.equal(state.external_tools.success,true,state.external_tools.error);
 assert.equal(fs.readFileSync(path.join(state.external_tools.workspace,'observed-left'),'utf8'),'before\n');assert.equal(fs.readFileSync(path.join(state.external_tools.workspace,'observed-right'),'utf8'),'after\n');assert.deepEqual(preserved(f),unchanged);await revealTool('tools-dismiss');await click('tools-dismiss');passed('External diff receives the pinned committed versions as read-only copies, not local edits');

 await select('assets/画像.png');await until(()=>tab().image_diff.loaded,'Pinned image loaded');assert.equal(tab().image_diff.before.width,1);assert.equal(tab().image_diff.after.height,1);await capture('02-image');
 await select('binary.bin');assert.equal(tab().patch_added,0);assert.equal(tab().patch_removed,0);passed('Selected image and binary changes reuse the native read-only diff viewer');

 await type('compare-base','PRIVATE_missing_revision');await click('compare-load');assert.match(comparison().error,/Could not compare.*git_exit/);assert.equal(comparison().loaded.target,f.target);await capture('03-error');
 await compare('review-base','review-base');assert.equal(comparison().loaded.count,0);assert.equal(tab().patch_files,null);await capture('04-empty');
 await compare('review-base',feature);await click('compare-swap');assert(comparison().changed_inputs);await click('compare-load');assert.equal(comparison().loaded.base,f.target);assert.equal(comparison().loaded.target,f.base);assert(comparison().loaded.files.some(x=>x.path===old&&x.original_path===renamed));
 await compare(f.base,f.target);passed('Invalid revisions retain the reviewed result; identical revisions are empty; swap reverses rename direction; full commit IDs work');

 await click('compare-target-pick');await type('compare-target-pick-filter','検索');await click('compare-target-pick-0');assert.equal(comparison().target,'refs/heads/'+feature);await click('compare-load');
 const before=comparison().loaded.target;f.git(['update-ref','refs/heads/'+feature,f.future,f.target]);await click('refresh');assert.equal(comparison().loaded.target,before);await select(renamed);assert.equal(comparison().loaded.target,before);await click('compare-load');assert.equal(comparison().loaded.target,f.future);assert(comparison().loaded.files.some(x=>x.path==='docs/future.md'));assert.deepEqual(preserved(f),unchanged);passed('Filtered branch picker works; external ref changes do not alter pinned comparisons until an explicit new comparison');

 const pinned=JSON.stringify(comparison().loaded);await cancelRead('compare-load');assert.equal(JSON.stringify(comparison().loaded),pinned);await capture('05-cancelled');await click('compare-load');assert.equal(comparison().error,null);assert.equal(comparison().notice,null);passed('Actual comparison cancellation reports stopping/cancelled, retains the previous result and supports retry');
 await type('compare-filter',renamed);await stable();const renameIndex=comparison().loaded.files.find(x=>x.path===renamed).index;await cancelRead('compare-file-'+renameIndex);assert.equal(tab().patch_files,null);await click('compare-retry');assert.equal(comparison().error,null);assert.equal(tab().patch_files[0].path,renamed);passed('Selected-file diff cancellation also leaves the index/worktree intact and Retry reloads the pinned file');

 await select(renamed);await resize(780,620);if(comparison().show_files)await click('compare-toggle-files');await capture('06-compact-diff');await click('compare-toggle-files');await capture('07-compact-files');await click('toolbar-console');await capture('08-compact-console');await click('console-close');await resize(1480,920);passed('Compact window switches between file list and diff, including with the console open');

 await click('view-commits');await click('commit-'+f.target);await click('commit-'+f.base,{modifiers:'ctrl'});assert.equal(tab().selected_commits.length,2);await click('menu-view');await click('menu-compare-selected');assert.equal(comparison().loaded.base,f.base);assert.equal(comparison().loaded.target,f.target);
 await click('view-branches');const bi=tab().branches.findIndex(b=>b.reference==='refs/heads/'+feature);assert(bi>=0);await click('branch-row-'+bi);await click('branch-compare');assert.equal(comparison().loaded.base,f.target);assert.equal(comparison().loaded.target,f.future);assert.deepEqual(preserved(f),unchanged);passed('Two selected commits and branch Compare with HEAD open the same reviewed read-only view');

 // Supersede a real read by leaving; no finished patch may leak into Changes.
 await stable();await client.tool('rocket_click',{target:'compare-load',expected_tab:state.active_tab});await until(()=>comparison().running,'Comparison running before navigation');await client.tool('rocket_click',{target:'view-changes',expected_tab:state.active_tab});await settle();assert.equal(tab().view,'Changes');assert.equal(comparison().running,false);await click('view-compare');await settle();assert.equal(comparison().loaded.target,f.future);assert.deepEqual(preserved(f),unchanged);passed('Leaving Compare cancels its owned read and late results cannot overwrite another view');

 const empty=fixture('PRIVATE_EMPTY_文');await open(empty);await type('compare-base','HEAD');await type('compare-target','HEAD');await click('compare-load');assert.match(comparison().error,/Could not compare/);assert.equal(comparison().loaded,null);passed('Unborn repository shows a clear unavailable-revision error');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));for(const op of ['compare','compare-base','compare-target','compare-files','compare-diff','compare-cancel'])assert(records.some(r=>r.operation===op),'Missing log '+op);
 assert(records.some(r=>r.operation==='compare'&&r.event==='request_end'&&r.outcome==='cancelled'));assert(records.some(r=>r.operation==='compare'&&r.event==='request_end'&&r.outcome==='git_error'));assert(records.some(r=>r.operation==='compare'&&r.event==='request_end'&&r.outcome==='ok'));
 const payload=JSON.stringify(records);for(const secret of ['PRIVATE_',old,renamed,feature,'unstaged local',f.base,f.target])assert(!payload.includes(secret),'Diagnostic payload leak');
 report.log_records=records.length;passed('Logs identify resolution/list/file/cancel steps and failures, with zero drops and no tested private paths/revisions/content');report.passed=true;
 }catch(error){report.failure=error.stack;if(client)try{await capture('failure')}catch{}throw error}
 finally{fs.writeFileSync('work/compare-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(250);if(service.exitCode===null)service.kill()}if(client)await client.finish()}
}
run().catch(error=>{console.error(error);process.exitCode=1});
