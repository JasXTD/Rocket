// Native Map acceptance via authenticated localhost MCP, synthetic local Git only.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/repository-map-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/repository-map';
const evidence=path.resolve('tools/maintenance/repository-map-redesign-20261105');
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const edgesOnly=process.argv.includes('--edges-only'),privateIds=[];
const report={passed:false,version,root,scope:edgesOnly?'edges-only':'full-map',cases:[]};
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
async function click(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();await stable();}
async function type(target,text){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();await stable();}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const img=result.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-map');assert.equal(active().view,'Map');}
// The revision box is in the revision picker's list: open it first.
async function load(revision){await click('map-revision-pick');await type('map-revision',revision);await click('map-load');}
async function closeWindow(){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit'&&!s.input_pending);if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}}
function snapshot(f){const files={};function walk(dir){for(const e of fs.readdirSync(dir,{withFileTypes:true})){if(e.name==='.git')continue;const p=path.join(dir,e.name);if(e.isDirectory())walk(p);else files[path.relative(f.repo,p)]=hash(p);}}walk(f.repo);return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),files};}
function checkBounds(){for(const c of state.controls.filter(c=>c.id.startsWith('map-')))assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=state.viewport[0]+1&&c.rect[3]<=state.viewport[1]+1,'Offscreen '+JSON.stringify(c));}
const FILE='应用/src/結果 🚀 é.rs';
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
 fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 if(!edgesOnly){
 const f=fixture('PRIVATE_MAP_検索 🚀');
 f.write('Cargo.toml',"[workspace]\nmembers=['应用','core']\n");f.write('应用/Cargo.toml',"[package]\nname='界面'\n[dependencies]\ncore={path='../core'}\n");f.write('core/Cargo.toml',"[package]\nname='engine'\n");
 f.write(FILE,'// PRIVATE_INITIAL 界\n');f.write('.mailmap','PRIVATE_林安 <canonical@example.invalid> PRIVATE_Alias <alias@example.invalid>\n');f.commit('PRIVATE_STRUCTURE');
 f.git(['switch','-c','feature-map']);f.write('feature.txt','feature\n');f.side=f.commit('PRIVATE_SIDE');f.git(['switch',branch]);
 f.git(['config','user.name','PRIVATE_Béa']);f.git(['config','user.email','bea@example.invalid']);f.write(FILE,'// PRIVATE_SHARED\n');f.commit('PRIVATE_MAIN');f.git(['merge','--no-ff','feature-map','-m','PRIVATE_MERGE']);
 f.git(['switch','-c','unmerged-map']);f.write('unmerged.txt','unmerged\n');f.unmerged=f.commit('PRIVATE_UNMERGED');f.git(['switch',branch]);f.tip=f.git(['rev-parse','HEAD']);
 privateIds.push(f.tip);
 f.write(FILE,'PRIVATE_STAGED\n');f.git(['add','--',FILE]);f.write(FILE,'PRIVATE_UNSTAGED\n');f.write('PRIVATE_UNTRACKED','unchanged');const before=snapshot(f);
 await open(f);assert.equal(map().data.tip,f.tip);assert(map().alternate);let nodes=map().data.nodes;
 const ui=nodes.find(n=>n.label==='界面'),core=nodes.find(n=>n.label==='engine');assert(ui&&core);assert(map().data.links.some(e=>e.from===ui.index&&e.to===core.index&&e.kind==='DependsOn'));
 assert(nodes.some(n=>n.path===FILE));assert(!nodes.some(n=>n.path==='PRIVATE_UNTRACKED'));await capture('code-packages-wide');
 await click('map-node-'+ui.index);assert.equal(map().selected,ui.index);await capture('code-package-details');
 assert(control('map-details-scroll'),'wide maps keep the details card');assert(control('map-link-0'),'the package card lists what it uses');
 await click('map-show-folders');assert(!map().alternate);assert.equal(map().selected,ui.index);assert(!map().collapsed.includes(map().data.nodes[ui.index].label));await capture('code-folders-from-package');
 // Folders: a file counts on its folder's card; search selects that folder.
 await type('map-search','結果 🚀 é');const src=map().data.nodes.find(n=>n.path==='应用/src');assert(map().visible.includes(src.index));assert.equal(map().selected,src.index);assert(!map().visible.some(i=>map().data.nodes[i].kind==='File'));await capture('code-unicode-search');
 // A file row in the folder card shows the commits that changed it.
 await click('map-path-0');assert.equal(map().kind,'History');assert.equal(map().path_scope,FILE);await until(s=>!map().running,'path history');assert(map().data.nodes.every(n=>n.label!=='PRIVATE_MERGE'||n.count===2));await capture('history-file-scope');
 await type('map-path-scope','');await click('map-apply-scope');await until(s=>!map().running,'whole history');assert.equal(map().path_scope,'');await click('map-view-code');
 await click('map-clear');await load('PRIVATE_UNKNOWN');assert(map().error.includes('Check the revision'));assert.equal(map().data.tip,f.tip);await capture('error-retains-code');await load('HEAD');assert(!map().error);
 await click('map-view-history');nodes=map().data.nodes;const merge=nodes.find(n=>n.label==='PRIVATE_MERGE');assert.equal(merge.count,2);assert.equal(map().data.links.filter(e=>e.to===merge.index&&e.kind==='Parent').length,2);assert(!nodes.some(n=>n.commit===f.unmerged));assert.equal(map().condensed,true);assert.equal(map().line,0);assert(control('map-commit-0'),'a merge line lists its merged commits');assert(control('map-inspect-commit'));await capture('history-merge-wide');
 await click('map-all-branches');assert(map().data.nodes.some(n=>n.commit===f.unmerged));assert(control('map-tip-0'),'a branch ahead of the main line shows above it');await click('map-tip-0');assert.equal(map().data.nodes[map().selected].commit,f.unmerged);await capture('history-branch-ahead');
 await click('map-mode-commits');assert(map().visible.includes(map().data.nodes.find(n=>n.label==='PRIVATE_MERGE').index));await capture('history-every-commit');
 await type('map-search','PRIVATE_UNMERGED');await click('map-node-'+map().data.nodes.find(n=>n.commit===f.unmerged).index);await click('map-inspect-commit');
 await until(s=>s.review_windows.length===1&&!s.review_windows[0].loading,'Out-of-list commit review');const review=state.review_windows[0];assert.equal(review.commit,f.unmerged);assert(review.patch_files.includes('unmerged.txt'));assert(!review.error);await client.tool('rocket_review_click',{window:review.id,target:'review-close'});await until(s=>s.review_windows.length===0&&!s.review_input_pending);
 await type('map-search','PRIVATE_STRUCTURE');const commit=map().data.nodes.find(n=>n.label==='PRIVATE_STRUCTURE');await click('map-node-'+commit.index);await click('map-inspect-commit');assert.equal(active().view,'Commits');assert.equal(active().selected_commit,commit.commit);
 await click('view-map');await click('map-view-people');nodes=map().data.nodes;const a=nodes.find(n=>n.label==='PRIVATE_林安'),b=nodes.find(n=>n.label==='PRIVATE_Béa');assert(a&&b);assert.equal(a.count,3);assert.equal(b.count,2);assert(nodes.every(n=>n.label!=='PRIVATE_Alias'));
 assert(map().data.links.some(e=>e.kind==='SharedFiles'&&[e.from,e.to].includes(a.index)&&[e.from,e.to].includes(b.index)));await capture('people-areas-wide');
 const cell=state.controls.find(c=>c.id.startsWith('map-cell-'+a.index+'-'));assert(cell,'a person has cells in the grid');await click(cell.id);assert.deepEqual(map().cell,cell.id.split('-').slice(2).map(Number));assert(control('map-history')&&control('map-path-0'));await capture('people-cell');
 await click('map-mode-shared');await capture('people-shared-files');await click('map-node-'+a.index);await click('map-history');assert.equal(map().kind,'History');assert.equal(map().query,'PRIVATE_林安');assert(map().visible.some(i=>map().data.nodes[i].label==='PRIVATE_STRUCTURE'));
 assert.deepEqual(snapshot(f),before);passed('Real committed Cargo edges, Unicode paths, pinned revision errors, merge parents, all branches, canonical authors, shared-file evidence and cross-view navigation; local changes preserved');

 for(const kind of ['history','people']){
  await click('map-view-'+kind);const tip=map().data.tip;await stable();await client.tool('rocket_click',{target:'map-load',expected_tab:state.active_tab});
  await until(s=>!s.input_pending&&map().running&&control('map-cancel')?.enabled,'In-flight '+kind+' read');
  const start=performance.now();await client.tool('rocket_click',{target:'map-cancel',expected_tab:state.active_tab});await settle();
  report[kind+'_stop_ms']=performance.now()-start;assert(map().notice?.includes('stopped'));assert.equal(map().data.tip,tip);assert(report[kind+'_stop_ms']<10000);
 }
 passed('First-click History and People cancellation retains their completed results');

 await click('map-view-code');await type('map-search','結果 🚀 é');await resize(780,620);await capture('code-compact');checkBounds();
 const folder=map().data.nodes.find(n=>n.path==='应用/src');if(map().show_details)await click('map-close-details');await click('map-focus');await click('map-node-'+folder.index);assert(map().show_details);await capture('code-compact-details');checkBounds();await click('map-close-details');
 const zoom=map().zoom;await click('map-zoom-in');assert(map().zoom>zoom);await click('map-fit');await click('map-focus');
 for(const kind of ['history','people']){await click('map-view-'+kind);if(map().show_details)await click('map-close-details');await capture(kind+'-compact');checkBounds();if(kind==='history'){await click('map-mode-topology');await click('map-line-0');}else{await click('map-mode-grid');await click('map-node-'+map().data.nodes.find(n=>n.kind==='Person').index);}assert(map().show_details);await capture(kind+'-compact-details');checkBounds();await click('map-close-details');}
 await type('map-search','PRIVATE_NO_MATCH');assert.equal(map().visible.length,0);await capture('empty-search');await click('map-clear');
 await resize(1480,920);await click('map-view-code');await click('map-clear');
 await click('map-export');assert.equal(map().export.name,'rocket-folders.mmd');assert(control('map-export-copy')?.enabled&&control('map-export-save')?.enabled);await capture('export-dialog');
 await click('map-export-save');await until(s=>!map().export.running,'export saved');assert.match(map().export.notice||'',/Saved rocket-folders\.mmd/,map().export.error);const mermaid=path.join(map().export.dir,'rocket-folders.mmd');assert(fs.readFileSync(mermaid,'utf8').includes('flowchart'));
 await click('map-export-save');await until(s=>!map().export.running,'second save');assert.match(map().export.error||'',/already exists/);
 await type('map-export-name','map.txt');await click('map-export-save');assert.match(map().export.error||'',/\.mmd/);await capture('export-errors');
 await client.tool('rocket_escape');await settle();assert(!map().export,'Esc closes the export dialog');fs.unlinkSync(mermaid);
 const old=map().data.tip;
 f.write('external 新.rs','// PRIVATE_EXTERNAL\n');f.git(['add','--','external 新.rs']);f.git(['commit','--only','-m','PRIVATE_EXTERNAL','--','external 新.rs']);const afterExternal=snapshot(f);
 await click('refresh');assert.equal(map().data.tip,old);assert(map().stale);await click('map-load');assert.notEqual(map().data.tip,old);assert(map().data.nodes.some(n=>n.path==='external 新.rs'));assert.deepEqual(snapshot(f),afterExternal);
 passed('780×620 layouts, readable Unicode nodes and detail drawers, zoom/focus, empty search, Mermaid export dialog, external-change detection and explicit snapshot refresh');

 const kinds=fixture('PRIVATE_REFERENCES');const link=kinds.git(['hash-object','-w','--stdin'],'PRIVATE_DO_NOT_FOLLOW\n');const ref=kinds.git(['rev-parse','HEAD']);
 kinds.git(['update-index','--add','--cacheinfo','120000',link,'Cargo.toml']);kinds.git(['update-index','--add','--cacheinfo','120000',link,'.mailmap']);kinds.git(['update-index','--add','--cacheinfo','160000',ref,'module']);kinds.git(['commit','-m','PRIVATE_REFERENCES']);
 kinds.git(['config','diff.external','false']);kinds.git(['config','filter.test.smudge','false']);kinds.git(['config','filter.test.required','true']);kinds.write('.gitattributes','* filter=test\n');const beforeKinds=snapshot(kinds);
 await open(kinds);assert(map().data.nodes.some(n=>n.path==='Cargo.toml'&&n.detail.includes('not followed')));assert(!map().data.nodes.some(n=>n.kind==='Crate'));assert(map().data.nodes.some(n=>n.path==='module'&&n.detail.includes('Submodule')));await click('map-view-history');assert(!map().error);await click('map-view-people');assert(!map().error);assert.deepEqual(snapshot(kinds),beforeKinds);
 passed('Committed symlinks and submodules remain references; configured filters/external diff are not executed; missing working files and index remain unchanged');
 }

 const empty=fixture('PRIVATE_EMPTY',true);await open(empty);for(const kind of ['code','history','people']){await click('map-view-'+kind);assert.equal(map().data.nodes.length,0);assert(!map().running);await capture('empty-'+kind);}
 const oldRepo=fixture('PRIVATE_OLD');const oldTree=oldRepo.git(['rev-parse','HEAD^{tree}']);
 const oldCommit=cp.execFileSync('git',['-C',oldRepo.repo,'commit-tree',oldTree],{env:{...env,GIT_AUTHOR_DATE:'2000-01-01T00:00:00Z',GIT_COMMITTER_DATE:'2000-01-01T00:00:00Z'},input:'PRIVATE_OLD_COMMIT\n',windowsHide:true,encoding:'utf8'}).trim();oldRepo.git(['update-ref','refs/heads/'+branch,oldCommit]);
 await open(oldRepo);await click('map-view-people');assert(map().data.nodes.length>0);await click('map-period-90');assert.equal(map().data.nodes.length,0);assert.equal(map().data.scanned,1);await capture('people-empty-period');
 const longRepo=fixture('PRIVATE_LONG');const longTree=longRepo.git(['rev-parse','HEAD^{tree}']);const longCommit=longRepo.git(['commit-tree',longTree,'-p',longRepo.git(['rev-parse','HEAD'])],'PRIVATE_LONG_'+ '界🚀'.repeat(4000)+'\n');longRepo.git(['update-ref','refs/heads/'+branch,longCommit]);
 await open(longRepo);await click('map-view-history');const longNode=map().data.nodes.find(n=>n.commit===longCommit);assert(Buffer.byteLength(longNode.label)<=4096);assert(!longNode.label.includes('\ufffd'));assert(map().data.notes.some(n=>n.includes('shortened')));await capture('bounded-long-label-wide');await resize(780,620);await capture('bounded-long-label-compact');checkBounds();
 const big=fixture('PRIVATE_LARGE');const blob=big.git(['hash-object','-w','--stdin'],'PRIVATE_SMALL\n');let records='';for(let i=0;i<50000;i++)records+=`100644 blob ${blob}\tfile-${String(i).padStart(6,'0')}.rs\0`;
 const tree=big.git(['mktree','-z'],records),large=big.git(['commit-tree',tree],'PRIVATE_BIG_TREE');const beforeBig=snapshot(big);
 await open(big);await load(large);assert.equal(map().data.scanned,50000);assert.equal(map().data.node_count,4096);assert(map().data.limited);assert.equal(map().visible.length,0,'files have no cards of their own');assert.deepEqual(map().file_groups,[{folder:null,files:map().data.node_count}]);assert(map().zoom>=0.75);await capture('bounded-large-code');await click('map-fit');assert(map().zoom>=0.75&&map().zoom<=1);await capture('bounded-large-code-fit');
 if(edgesOnly){await load('PRIVATE_BAD_REVISION');assert(map().error?.includes('Check the revision'));assert.equal(map().data.tip,large);await load(large);}
 await stable();await client.tool('rocket_click',{target:'map-load',expected_tab:state.active_tab});await until(s=>!s.input_pending&&map().running&&control('map-cancel')?.enabled,'In-flight map read');
 // Exactly one Stop click; no retry conceals a missed cancellation.
 const started=performance.now();await client.tool('rocket_click',{target:'map-cancel',expected_tab:state.active_tab});await until(s=>!s.input_pending&&(map().cancelling||!map().running),'Immediate Stop acknowledgement');report.stop_feedback_ms=performance.now()-started;
 await settle();report.stop_settled_ms=performance.now()-started;assert(map().notice?.includes('stopped'));assert.equal(map().data.tip,large);assert(report.stop_settled_ms<10000);await capture('stopped-retains-code');
 await stable();await client.tool('rocket_click',{target:'map-load',expected_tab:state.active_tab});await until(s=>!s.input_pending&&map().running,'Map reload running');
 // Refresh becomes Stop while reading: stop the big read, then read HEAD. The stopped read must never publish.
 await client.tool('rocket_click',{target:'map-revision-pick',expected_tab:state.active_tab});await until(s=>!s.input_pending);await client.tool('rocket_type',{target:'map-revision',text:'HEAD',expected_tab:state.active_tab});await until(s=>!s.input_pending);await client.tool('rocket_click',{target:'map-cancel',expected_tab:state.active_tab});await until(s=>!s.input_pending&&!map().running,'stopped reload');await stable();await client.tool('rocket_click',{target:'map-load',expected_tab:state.active_tab});await until(s=>!s.input_pending&&!map().running,'HEAD read');await settle();assert.equal(map().data.tip,big.git(['rev-parse','HEAD']));assert.equal(map().data.node_count,1);await delay(250);await settle();assert.equal(map().data.node_count,1);assert.deepEqual(snapshot(big),beforeBig);
 passed('Empty views and date scope, bounded Unicode labels, readable large-tree focus and complete Fit, one-click Code cancellation, a stopped read that never publishes, and unchanged Git/files');

 await closeWindow();await client.raw('POST','/shutdown');
 const recordsLog=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(recordsLog.length);assert(recordsLog.every(r=>r.dropped_before===0));const payload=JSON.stringify(recordsLog);for(const secret of ['PRIVATE_','検索',FILE,root,blob,...privateIds])assert(!payload.includes(secret),'Diagnostic payload leak');
 for(const op of ['map-code','map-history','map-people']){const starts=recordsLog.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const start of starts)assert(recordsLog.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired '+op);}
 for(const start of recordsLog.filter(r=>r.event==='step_start'&&r.operation?.startsWith('map-')))assert(recordsLog.some(r=>r.event==='step_end'&&r.step===start.step&&r.session===start.session&&r.request===start.request),'Unpaired step');
 for(const op of (edgesOnly?['map-code']:['map-code','map-history','map-people']))assert(recordsLog.some(r=>r.operation===op&&r.outcome==='cancelled'),'Missing cancellation log '+op);assert(recordsLog.some(r=>r.operation==='map-code'&&r.outcome==='git_error'));report.log_records=recordsLog.length;
 passed('Paired request/step logs with named map operations, error/cancellation outcomes, zero dropped events and no private payloads');report.passed=true;
}catch(error){report.failure=error.stack;if(client)try{await capture('failure');}catch{}throw error;}
finally{
 fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(evidence,'mcp-last-result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}for(let n=0;n<40&&service.exitCode===null;n++)await delay(100);if(service.exitCode===null){report.forced_service_stop=true;service.kill();}}if(client)await client.finish();
 const token=path.join(profile,'access-token');if(fs.existsSync(token))fs.unlinkSync(token);
 report.cleanup={service_exit:service?.exitCode,token_removed:!fs.existsSync(token)};
 fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(evidence,'mcp-last-result.json'),JSON.stringify(report,null,2));
}}
run().catch(e=>{console.error(e);process.exitCode=1});
