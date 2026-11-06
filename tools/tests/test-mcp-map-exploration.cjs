// Native Map acceptance via authenticated localhost MCP, synthetic local Git only.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/map-exploration-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/repository-map';
const evidence=path.resolve('tools/maintenance/map-exploration-20261104');
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const privateIds=[];
const report={passed:false,version,root,scope:'map-exploration',cases:[]};
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(evidence,{recursive:true});
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function fixture(name,empty=false){
 const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
 const git=(args,input)=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',input,maxBuffer:32*1024*1024}).trimEnd();
 const write=(name,body)=>{const p=path.join(repo,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,body);};
 const commit=message=>{git(['add','--all']);git(['commit','-m',message]);return git(['rev-parse','HEAD']);};
 git(['init','--initial-branch='+branch]);
 for(const [key,value]of [['user.name','PRIVATE_Alias'],['user.email','alias@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',key,value]);
 fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
 if(!empty){write('base.txt','base\n');commit('PRIVATE_BASE 中文');}
 return {repo,git,write,commit};
}
let client,service,state;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const passed=text=>{report.cases.push(text);console.error('PASS '+text);};
function active(){return state.tabs.find(t=>t.id===state.active_tab);}
function map(){return active().repository_map;}
function control(id){return state.controls.find(c=>c.id===id);}
async function until(predicate,label='UI state'){
 const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<60000,label+' timed out: '+JSON.stringify({dialog:state.dialog,notice:state.notice,map:active()?.repository_map?.notice}));await delay(60);}
}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running));}
async function stable(){let last='',count=0;await until(s=>{const next=JSON.stringify(s.controls);count=next===last?count+1:0;last=next;return count>=2;},'Stable controls');}
async function click(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();await stable();if(target==='map-clear')assert.equal(map().query,'','Clear must apply on the first click');}
async function type(target,text){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();await stable();}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const img=result.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-map');assert.equal(active().view,'Map');}
// The revision box is in the revision picker's list: open it first.
async function load(revision){await click('map-revision-pick');await type('map-revision',revision);await click('map-load');}
async function closeWindow(){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit'&&!s.input_pending);if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}}
function snapshot(f){const files={};function walk(dir){for(const e of fs.readdirSync(dir,{withFileTypes:true})){if(e.name==='.git')continue;const p=path.join(dir,e.name);if(e.isDirectory())walk(p);else files[path.relative(f.repo,p)]=hash(p);}}walk(f.repo);return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),files};}
function checkBounds(){for(const c of state.controls.filter(c=>c.id.startsWith('map-')))assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=state.viewport[0]+1&&c.rect[3]<=state.viewport[1]+1,'Offscreen '+JSON.stringify(c));}
async function finished(){await until(()=>!map().running&&!state.input_pending);assert(!map().error,map().error);await stable();}
function series(){
 const f=fixture('PRIVATE_SERIES_中文',true);let stream='';
 for(let i=0;i<605;i++){
  const message='PRIVATE_SERIES_'+String(i).padStart(3,'0'),file=i%3===0?'应用/[結果] 🚀 é.rs':'src/code.rs',body='synthetic '+i+'\n';
  stream+=`commit refs/heads/${branch}\nmark :${i+1}\nauthor PRIVATE_Person <synthetic@example.invalid> ${1700000000+i} +0000\ncommitter PRIVATE_Person <synthetic@example.invalid> ${1700000000+i} +0000\ndata ${Buffer.byteLength(message)}\n${message}\n`;
  if(i)stream+=`from :${i}\n`;
  stream+=`M 100644 inline ${JSON.stringify(file)}\ndata ${Buffer.byteLength(body)}\n${body}\n`;
 }
 f.git(['fast-import','--quiet'],stream);f.git(['reset','--hard',branch]);
 return {...f,ids:f.git(['rev-list','--reverse','HEAD']).split(/\r?\n/)};
}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}assert.equal((await client.initialize()).serverInfo.version,version);
 const f=fixture('PRIVATE_GROUPS_資料');f.write('ui/Cargo.toml',"[package]\nname='界面'\n[dependencies]\nengine={path='../core'}\n");f.write('core/Cargo.toml',"[package]\nname='engine'\n");f.write('ui/src/結果 🚀 é.rs','fn main() {}\n');f.write('core/src/lib.rs','pub fn run() {}\n');f.commit('PRIVATE_GROUPS');f.write('local-edit','keep');const before=snapshot(f);
 await open(f);await finished();if(map().alternate)await click('map-mode-folders');await click('map-collapse-all');
 const nodes=map().data.nodes,ui=nodes.find(n=>n.path==='ui').index,core=nodes.find(n=>n.path==='core').index;
 assert(map().collapsed.includes('ui'));assert(!map().visible.some(i=>nodes[i].path?.startsWith('ui/')));assert(map().projected_links.some(e=>e.kind==='DependsOn'&&e.from===ui&&e.to===core));await capture('groups-wide');
 await type('map-search','結果 🚀 é');assert(map().visible.includes(ui),'hidden Unicode match should reveal its group');await click('map-clear');await click('map-collapse-all');
 await click('map-fit');await click('map-node-'+ui);await click('map-neighbors');assert.equal(map().neighbor_anchor,'ui');assert(map().visible.length<map().data.nodes.length);await capture('neighbors-wide');await click('map-neighbors');
 passed('Collapsible Unicode folders, projected dependency connections, hidden-file search and direct neighbor focus');
 await click('map-export');assert(map().export);const cancelled=path.join(root,'cancelled.mmd');await type('map-export-folder',root);await type('map-export-name',path.basename(cancelled));await click('map-export-close');assert(!fs.existsSync(cancelled));
 await click('map-export');const existing=path.join(root,'existing.mmd');fs.writeFileSync(existing,'PRIVATE_KEEP_EXISTING');await type('map-export-folder',root);await type('map-export-name',path.basename(existing));await click('map-export-save');await until(()=>!map().export.running&&!!map().export.error);assert.equal(fs.readFileSync(existing,'utf8'),'PRIVATE_KEEP_EXISTING');await capture('export-existing-refused');
 const outside=path.join(root,'..','outside-map.mmd');await type('map-export-folder',path.join(root,'..'));await type('map-export-name',path.basename(outside));await click('map-export-save');await until(()=>!map().export.running&&!!map().export.error);assert(!fs.existsSync(outside));
 const exported=path.join(root,'資料 🚀.mmd');await type('map-export-folder',root);await type('map-export-name',path.basename(exported));await click('map-export-save');await until(()=>!map().export.running&&!!map().export.notice);assert(!map().export.error,map().export.error);const text=fs.readFileSync(exported,'utf8');assert(text.includes('flowchart LR\n')&&text.includes(map().data.tip));assert(!/\n(?:click|style|classDef|%%\{)/.test(text));await capture('export-success');await click('map-export-close');assert.deepEqual(snapshot(f),before);
 passed('Reviewed Mermaid text, cancelled review, scoped destination refusal, existing-file preservation and Unicode new-file export');
 const h=series();await open(h);await click('map-view-history');await finished();assert.equal(map().data.node_count,500);assert(map().can_more);const pinned=map().data.tip;
 await click('map-scope');await type('map-base',h.ids[500]);assert(!map().can_more,'edited scope must disable pinned continuation');await type('map-base','');assert(map().can_more);await click('map-scope');
 h.write('external-only','external');h.commit('PRIVATE_EXTERNAL_NEW');h.write('local-untracked','keep');const stableRepo=snapshot(h);
 await type('map-search','PRIVATE_SERIES_050');assert.equal(map().visible.length,0);await click('map-clear');
 await client.tool('rocket_click',{target:'map-more',expected_tab:state.active_tab});await until(()=>map().running&&!state.input_pending,'Load more in flight');const stopStart=performance.now();await client.tool('rocket_click',{target:'map-cancel',expected_tab:state.active_tab});await until(()=>!map().running&&!state.input_pending,'Cancelled load more');report.history_stop_ms=performance.now()-stopStart;assert.equal(map().data.node_count,500);assert.equal(map().data.tip,pinned);assert.match(map().notice,/stopped/i);assert(map().can_more);
 await click('map-more');await finished();assert.equal(map().data.node_count,605);assert.equal(map().data.tip,pinned);assert(!map().data.nodes.some(n=>n.label==='PRIVATE_EXTERNAL_NEW'));assert(!map().can_more);await click('map-mode-commits');await click('map-next-nodes');await click('map-next-nodes');assert.equal(map().display_offset,480);assert(map().visible.every(i=>i>=480));await capture('older-history-section');await type('map-search','PRIVATE_SERIES_050');assert.equal(map().display_offset,0);assert(map().visible.some(i=>map().data.nodes[i].label==='PRIVATE_SERIES_050'));await capture('older-history-search');await click('map-clear');
 await click('map-scope');await type('map-base',h.ids[500]);await type('map-path-scope','应用/[結果] 🚀 é.rs');await click('map-apply-scope');await finished();assert.equal(map().data.node_count,35);assert(map().data.boundary_parents>0);assert(!map().data.nodes.some(n=>n.label==='PRIVATE_EXTERNAL_NEW'));await capture('scoped-history');
 await type('map-path-scope','应用/*.rs');await click('map-apply-scope');await finished();assert.equal(map().data.node_count,0);await type('map-path-scope','应用/[結果] 🚀 é.rs');await click('map-apply-scope');await finished();
 await type('map-base','missing-PRIVATE-reference');await click('map-apply-scope');await until(()=>!map().running&&!!map().error);assert.equal(map().data.node_count,35);await capture('scope-error-retains-result');await type('map-base',h.ids[500]);await click('map-apply-scope');await finished();assert.deepEqual(snapshot(h),stableRepo);
 passed('Pinned Load more across external commits, first-click cancellation, older search, literal path/base scope and error retention');
 await resize(780,620);await stable();checkBounds();await capture('scope-compact');await click('map-scope');checkBounds();await capture('history-compact');await click('map-export');checkBounds();await capture('export-compact');await click('map-export-close');
 await click('map-view-people');await finished();assert(map().can_more);await click('map-more');await finished();assert.equal(map().data.scanned,606);await click('map-mode-shared');await click('map-neighbors');checkBounds();await capture('people-neighbors-compact');assert.deepEqual(snapshot(h),stableRepo);
 passed('Compact history/export/People controls and People load-more aggregation');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=[];for(const dir of fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-'))){for(const file of fs.readdirSync(path.join(root,dir)).filter(n=>n.endsWith('.jsonl'))){for(const line of fs.readFileSync(path.join(root,dir,file),'utf8').split(/\r?\n/).filter(Boolean))records.push(JSON.parse(line));}}
 const payload=JSON.stringify(records);assert(!payload.includes('PRIVATE_')&&!payload.includes('結果')&&!payload.includes(pinned)&&!payload.includes(root));assert(records.length);assert(records.every(e=>e.dropped_before===0));
 for(const operation of ['map-code','map-history','map-people','map-export']){const starts=records.filter(e=>e.event==='request_start'&&e.operation===operation);assert(starts.length,'Missing '+operation);for(const start of starts)assert(records.some(e=>e.event==='request_end'&&e.session===start.session&&e.request===start.request),'Unpaired '+operation);}
 for(const start of records.filter(e=>e.event==='step_start'&&e.operation?.startsWith('map-')))assert(records.some(e=>e.event==='step_end'&&e.session===start.session&&e.request===start.request&&e.step===start.step),'Unpaired step');
 assert(records.some(e=>e.operation==='map-history'&&e.outcome==='cancelled'));assert(records.some(e=>e.operation==='map-export'&&e.event==='request_end'&&e.outcome!=='ok'));report.log_records=records.length;passed('Paired map/export diagnostics, recorded cancellation/errors, no dropped events and no fixture payloads');
 report.passed=true;
 }catch(e){report.failure=e.stack;console.error(e.stack);try{state=await client.tool('rocket_state');await capture('failure')}catch{}throw e;
 }finally{
  if(service&&service.exitCode===null){try{state=await client.tool('rocket_state');if(!['closed','not_started'].includes(state.window))await closeWindow();await client.raw('POST','/shutdown');}catch{}for(let n=0;n<40&&service.exitCode===null;n++)await delay(100);if(service.exitCode===null){report.forced_service_stop=true;service.kill();}}
  if(client)await client.finish();
  const tokenPath=path.join(profile,'access-token');if(fs.existsSync(tokenPath))fs.unlinkSync(tokenPath);
  report.cleanup={service_exit:service?.exitCode,token_removed:!fs.existsSync(tokenPath)};fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(evidence,'mcp-result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
 }
}
run().catch(()=>{process.exitCode=1});
