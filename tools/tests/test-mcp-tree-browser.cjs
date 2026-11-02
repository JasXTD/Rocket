// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root=path.resolve('work/mcp-fixtures/tree-browser-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/tree-browser';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const focused=process.argv.includes('--preview-only');
const report={passed:false,version,root,focused,cases:[]},fixtures=[];
fs.mkdirSync(root,{recursive:true});
// Explicit local consent: this runner creates only synthetic shell/editor data.
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
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
async function menuItem(id){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);if(SUBMENU[id]){await click(SUBMENU[id]);await stable();}for(let i=0;i<8;i++){const c=control(id);if(c&&c.rect[3]-c.rect[1]>=28)break;await scroll('repository-menu-scroll','down',250);}await click(id);}
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function submit(){await click('dialog-submit');await settle();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}

const FILE='src/検索/結果 🚀 é.rs';
function tree(){return active().repository_tree;}
function gi(f,args,input){return cp.execFileSync('git',['-C',f.repo,...args],{env,windowsHide:true,encoding:'utf8',input}).trimEnd();}
function blob(f,input){return gi(f,['hash-object','-w','--stdin'],input);}
function snapshot(f){const files={};function walk(dir){for(const item of fs.readdirSync(dir,{withFileTypes:true})){if(item.name==='.git')continue;const p=path.join(dir,item.name);if(item.isDirectory())walk(p);else files[path.relative(f.repo,p)]=hash(p);}}walk(f.repo);return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),index_entries:f.git(['ls-files','--stage','-z']),config:hash(path.join(f.repo,'.git/config')),files};}
async function load(revision){await type('tree-revision',revision);await click('tree-load');}
async function choose(name){const i=tree().entries.findIndex(e=>e.name===name);assert(i>=0,'Missing '+name+' in '+JSON.stringify(tree().entries));if(control('tree-toggle-list')&&!tree().show_list)await click('tree-toggle-list');for(let n=0;!control('tree-entry-'+i)&&n<35;n++)await scroll('tree-list-scroll');await click('tree-entry-'+i);}
async function rootFolder(){await click('tree-root');}
async function unicodeFile(){await choose('src');await choose('検索');await choose('結果 🚀 é.rs');}
function preview(){return tree().content?.preview.join('\n')??'';}
function expectError(part){assert(tree().error?.includes(part),JSON.stringify(tree()));}
function setup(name){const f=fixture(name);f.write(FILE,'// PRIVATE_你好 🚀 é\nfn old() {}\n');f.write('empty','');f.write('binary.bin',Buffer.from([0,255,10]));f.write('large.txt',Buffer.alloc(2*1024*1024+1,65));f.write('scroll.txt',Array.from({length:3000},(_,i)=>'line '+i+' 界').join('\n'));f.old=f.commit('PRIVATE_OLD');f.git(['tag','-a','snapshot','-m','PRIVATE_TAG']);f.write(FILE,'// PRIVATE_NEW\nfn new() {}\n');f.tip=f.commit('PRIVATE_NEW');return f;}

async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);

 const f=setup('PRIVATE_TREE_検索 🚀');f.write(FILE,'PRIVATE_STAGED\n');f.git(['add','--',FILE]);f.write(FILE,'PRIVATE_UNSTAGED\n');f.write('untracked','PRIVATE_UNTRACKED');const before=snapshot(f);
 await open(f);await click('commit-browse-files');assert.equal(active().view,'Tree');assert.equal(tree().tip,f.tip);assert(!tree().entries.some(e=>e.name.includes('結果')),'Only one folder level loads');
 await load('snapshot');assert.equal(tree().tip,f.old);await unicodeFile();assert(preview().includes('PRIVATE_你好 🚀 é'));assert(!preview().includes('PRIVATE_UNSTAGED'));assert.equal(tree().selected_path,FILE);await capture('unicode-file-wide');
 await rootFolder();await choose('empty');assert.equal(tree().content.line_count,0);await choose('binary.bin');assert.equal(tree().content.kind,'Binary');await choose('large.txt');expectError('2 MiB');await capture('large-file-error');
 await load('PRIVATE_UNKNOWN');expectError('Check the commit');assert.equal(tree().tip,f.old);await capture('invalid-revision');await load('--all');expectError('Check the commit');assert.deepEqual(snapshot(f),before);
 passed('Commit-list entry point, annotated-tag resolution, lazy Unicode folders, pinned historical contents, empty/binary/large-file states, invalid revisions, dirty files and index preserved');

 await load('snapshot');let contentCancelled=false;report.content_cancel_attempts=0;
 for(let attempt=0;attempt<5&&!contentCancelled;attempt++){
   report.content_cancel_attempts++;const i=tree().entries.findIndex(e=>e.name==='scroll.txt');assert(i>=0);await stable();await client.tool('rocket_click',{target:'tree-entry-'+i,expected_tab:state.active_tab});await until(s=>!s.input_pending&&tree().running&&!tree().directory_running&&control('tree-cancel')?.enabled,'In-flight blob read');
   const start=performance.now();const r=await client.request('tools/call',{name:'rocket_click',arguments:{target:'tree-cancel',expected_tab:state.active_tab}});if(r.isError){await settle();continue;}await settle();report.content_cancel_ms=performance.now()-start;contentCancelled=tree().notice?.includes('File read cancelled');
 }
 assert(contentCancelled,'Must observe cancellation of a real blob read');assert.equal(tree().content,null);assert(report.content_cancel_ms<10000);await capture('cancelled-content');await click('tree-retry');assert.equal(tree().content.line_count,3000);
 // MCP server settings open as a Preferences page over the tree; menus wait until it closes.
 await click('toolbar-mcp');assert(state.mcp.visible);const covered=await client.request('tools/call',{name:'rocket_click',arguments:{target:'menu-repository',expected_tab:state.active_tab}});assert(covered.isError&&covered.content[0].text.includes('active dialog'),JSON.stringify(covered));await click('mcp-back');assert(!state.mcp.visible);assert.equal(active().view,'Tree');await menuItem('repository-tree');assert(control('tree-load'));assert.equal(tree().tip,f.tip);
 passed('Real blob cancellation clears incomplete content and supports retry; MCP settings close back to the tree and Repository menu navigation then reloads it');

 await load('snapshot');await unicodeFile();f.git(['tag','-f','snapshot',f.tip]);await refresh();assert.equal(tree().tip,f.old);assert(preview().includes('PRIVATE_你好'));await rootFolder();await unicodeFile();assert(preview().includes('PRIVATE_你好'));await load('snapshot');assert.equal(tree().tip,f.tip);await unicodeFile();assert(preview().includes('PRIVATE_NEW'));
 await resize(780,680);await capture('unicode-file-narrow');assert(control('tree-toggle-list'));await click('tree-toggle-list');await capture('folders-narrow');await choose('結果 🚀 é.rs');
 await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal?.running&&s.terminal.tail.includes('PS '),'PowerShell prompt ready');await capture('unicode-file-narrow-terminal');
 for(const c of state.controls.filter(c=>c.id.startsWith('tree-')))assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=state.viewport[0]+1&&c.rect[3]<=state.viewport[1]+1,'Clipped control '+JSON.stringify(c));
 if(control('terminal-hide'))await click('terminal-hide');await resize(1480,920);await rootFolder();await choose('scroll.txt');assert.equal(tree().content.line_count,3000);await scroll('tree-source-scroll');await capture('scrolled-contents');
 passed('External tag movement and automatic/manual refresh preserve the snapshot; explicit reload follows the ref; compact layouts, terminal coexistence, Unicode rendering and virtualized source scrolling');

 const kinds=fixture('PRIVATE_KINDS'),outside=path.join(root,'PRIVATE_OUTSIDE');fs.writeFileSync(outside,'PRIVATE_OUTSIDE_DO_NOT_READ');const link=blob(kinds,outside),contents=blob(kinds,'PRIVATE_LINKED_CONTENT\n'),base=kinds.git(['rev-parse','HEAD']);
 kinds.git(['update-index','--add','--cacheinfo','120000',link,'shortcut']);kinds.git(['update-index','--add','--cacheinfo','160000',base,'module']);kinds.git(['update-index','--add','--cacheinfo','100755',contents,'executable']);kinds.git(['commit','-m','PRIVATE_TYPES']);
 kinds.git(['config','filter.danger.smudge','false']);kinds.git(['config','filter.danger.required','true']);kinds.git(['config','diff.danger.textconv','false']);kinds.write('.gitattributes','* filter=danger diff=danger\n');const beforeKinds=snapshot(kinds);
 await open(kinds);await click('view-tree');await choose('shortcut');assert.equal(preview(),outside);await capture('symlink-target');await choose('module');assert.equal(tree().content,null);assert.equal(tree().entries[tree().selected].kind,'Submodule');await capture('submodule');await choose('executable');assert(preview().includes('PRIVATE_LINKED_CONTENT'));assert.deepEqual(snapshot(kinds),beforeKinds);assert.equal(fs.readFileSync(outside,'utf8'),'PRIVATE_OUTSIDE_DO_NOT_READ');
 const missing='a'.repeat(40),missingTree=gi(kinds,['mktree','-z','--missing'],`100644 blob ${missing}\tPRIVATE_MISSING\0`),missingCommit=gi(kinds,['commit-tree',missingTree],'missing');await load(missingCommit);await choose('PRIVATE_MISSING');expectError('unavailable locally');await capture('missing-object');
 passed('Symlink text is previewed without following targets; submodules are references only; executable blobs bypass content filters; missing objects fail clearly without changing existing files');

 const big=fixture('PRIVATE_BIG'),object=blob(big,'PRIVATE_SMALL_BLOB\n');let records='';for(let i=0;i<50000;i++)records+=`100644 blob ${object}\tfile-${String(i).padStart(6,'0')}.txt\0`;
 const bigTree=gi(big,['mktree','-z'],records),bigCommit=gi(big,['commit-tree',bigTree],'PRIVATE_LARGE_TREE');await open(big);await click('view-tree');await load(bigCommit);assert.equal(tree().entry_count,50000);assert(tree().entries_limited);assert.equal(tree().entries.length,1000);await capture('large-folder');
 const beforeBig=snapshot(big);const previous=tree().tip;let cancelled=false;report.cancel_attempts=0;
 for(let attempt=0;attempt<5&&!cancelled;attempt++){
   report.cancel_attempts++;await stable();await client.tool('rocket_click',{target:'tree-load',expected_tab:state.active_tab});await until(s=>!s.input_pending&&tree().running&&control('tree-cancel')?.enabled,'In-flight tree read');
   const start=performance.now();const r=await client.request('tools/call',{name:'rocket_click',arguments:{target:'tree-cancel',expected_tab:state.active_tab}});if(r.isError){await settle();continue;}await settle();report.cancel_ms=performance.now()-start;cancelled=tree().notice?.includes('cancelled');
 }
 assert(cancelled,'Must observe a cancelled real Git directory read');assert.equal(tree().tip,previous);assert(report.cancel_ms<10000);await capture('cancelled-folder');assert.deepEqual(snapshot(big),beforeBig);
 await type('tree-revision',bigCommit);await client.tool('rocket_click',{target:'tree-load',expected_tab:state.active_tab});await until(s=>!s.input_pending&&tree().running);await client.tool('rocket_type',{target:'tree-revision',text:'HEAD',expected_tab:state.active_tab});await until(s=>!s.input_pending);await client.tool('rocket_click',{target:'tree-load',expected_tab:state.active_tab});await settle();assert.equal(tree().tip,big.git(['rev-parse','HEAD']));assert.equal(tree().entry_count,1);await delay(400);await settle();assert.equal(tree().entry_count,1);await capture('superseded-read');
 passed('50,000-entry folder uses bounded MCP state and virtualized rows; real read cancellation retains the prior snapshot; superseded reads cannot overwrite the newest revision');

 await open(f);await click('view-tree');assert.equal(tree().tip,f.tip);await unicodeFile();assert(preview().includes('PRIVATE_NEW'));assert.deepEqual(snapshot(f),before);assert.deepEqual(snapshot(kinds),beforeKinds);assert.deepEqual(snapshot(big),beforeBig);
 await closeWindow();await client.raw('POST','/shutdown');
 const recordsLog=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(recordsLog.length);assert(recordsLog.every(r=>r.dropped_before===0));const payload=JSON.stringify(recordsLog);for(const secret of ['PRIVATE_','検索',FILE,root,object,f.old])assert(!payload.includes(secret),'Diagnostic leak '+secret);
 for(const op of ['tree-list','tree-content']){const starts=recordsLog.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const start of starts)assert(recordsLog.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired '+op);}
 for(const step of ['resolve-revision','read-directory','parse-directory','check-blob-size','read-blob'])assert(payload.includes(step),'Missing step '+step);
 for(const start of recordsLog.filter(r=>r.event==='step_start'&&['tree-list','tree-content'].includes(r.operation)))assert(recordsLog.some(r=>r.event==='step_end'&&r.operation===start.operation&&r.step===start.step&&r.session===start.session&&r.request===start.request),'Unpaired step '+start.step);
 assert(recordsLog.some(r=>r.operation==='tree-list'&&r.outcome==='cancelled'));assert(recordsLog.some(r=>r.operation==='tree-content'&&r.outcome==='cancelled'));assert(recordsLog.some(r=>r.operation==='tree-content'&&r.outcome==='unsupported'));assert(recordsLog.some(r=>r.operation==='tree-content'&&r.outcome==='git_error'));report.log_records=recordsLog.length;
 passed('Per-tab snapshot restoration, unchanged source state/config/files, paired request/step diagnostics with failures/cancellation and zero dropped events, no private payloads');report.passed=true;
}catch(error){if(client)try{await capture('failure');}catch{}throw error;}
finally{
 fs.writeFileSync('work/tree-browser-last-result.txt',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});
