// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root=path.resolve('work/mcp-fixtures/history-recovery-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/history-recovery';
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
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running||t.file_history.running||t.repository_tree.running));}
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

const DIR='src/検索 [x] 🚀 é',FILE=DIR+'/one.rs';
const fh=()=>active().file_history;
function snapshot(f){return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),file:hash(path.join(f.repo,FILE)),untracked:hash(path.join(f.repo,'untracked 文.txt'))};}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 console.error('Preparing private local history fixture');
 const f=fixture('PRIVATE_HISTORY 中文'),ids=[];
 f.write(FILE,'base\n');ids.push(f.commit('PRIVATE_FOLDER baseline'));
 for(let i=0;i<102;i++){f.write(FILE,'revision '+i+'\n');ids.push(f.commit('PRIVATE_FOLDER '+i));}
 const tip=ids.at(-1);f.git(['checkout','-b','lost/検索']);f.write(FILE,'lost\n');const lost=f.commit('PRIVATE_LOST_BRANCH');f.git(['checkout',branch]);f.git(['branch','-D','lost/検索']);
 f.git(['update-ref','--create-reflog','-m','PRIVATE_UNVISITED_REF','refs/remotes/origin/検索',lost]);
 f.write(FILE,'staged local\n');f.git(['add','--',FILE]);f.write(FILE,'unstaged local\n');f.write('untracked 文.txt','preserve');
 const before=snapshot(f);await open(f);
 await menuItem('repository-recovery');assert.equal(state.dialog.recovery.scope,'Head');assert(!state.dialog.recovery.entries.some(e=>e.subject==='PRIVATE_UNVISITED_REF'));
 await click('dialog-reflog-scope-all');assert.equal(state.dialog.recovery.scope,'AllRefs');const entries=state.dialog.recovery.entries;assert(entries.some(e=>e.subject==='PRIVATE_UNVISITED_REF'));assert.equal(state.dialog.recovery.target,null);
 await capture('all-ref-recovery-wide');await resize(780,620);await capture('all-ref-recovery-small');await resize(1480,920);
 const index=state.dialog.recovery.entries.findIndex(e=>e.id===lost);assert(index>=0&&index<20);await click('dialog-reflog-row-'+index);await click('dialog-recovery-branch');await type('dialog-recovery-name','recovered/検索');await click('dialog-preview-refresh');assert.equal(state.dialog.recovery.preview.target,lost);await submit();assert(latest().command_succeeded);assert.equal(f.git(['rev-parse','recovered/検索']),lost);assert.deepEqual(snapshot(f),before);
 if(state.dialog)await click('dialog-cancel');
 passed('All-ref scope, Unicode deleted-branch recovery and narrow review preserve HEAD, index, edits and config');
 await click('view-filehistory');await type('file-path',DIR);await click('file-directory');await click('file-load');
 assert.equal(fh().directory,true);assert.equal(fh().tip,tip);assert.equal(fh().entries.length,100);assert(fh().more);assert.equal(fh().mode,'Diff');assert(!control('file-blame'));assert(!fh().error);assert(fh().patch_added>0);
 await capture('folder-history-wide');await click('file-more');assert.equal(fh().entries.length,103);assert(!fh().more);assert.deepEqual(fh().entries.map(e=>e.id),ids.toReversed());await resize(780,620);await capture('folder-history-small');await resize(1480,920);
 // Selecting file mode changes the next request, not the retained folder result.
 await click('file-directory');assert(!fh().directory_input);assert(fh().directory);await type('file-path',FILE);await click('file-load');assert(!fh().directory);await click('file-blame');assert(fh().blame?.lines.length);assert.equal(fh().blame.lines[0].content,'revision 101');
 passed('Pinned folder history pages beyond 100 commits, renders diffs, and switching back to file blame remains correct');
 // Browse a historic directory, then open its history at that exact commit.
 await click('view-tree');await type('tree-revision',ids[20]);await click('tree-load');
 let tree=active().repository_tree;const src=tree.entries.find(e=>e.name==='src');assert(src);await click('tree-entry-'+src.index);
 tree=active().repository_tree;const dir=tree.entries.find(e=>e.name==='検索 [x] 🚀 é');assert(dir);await click('tree-entry-'+dir.index);await click('tree-history');
 assert.equal(active().view,'FileHistory');assert.equal(fh().tip,ids[20]);assert.equal(fh().path,DIR);assert(fh().directory);assert.equal(fh().entries.length,21);
 assert.deepEqual(snapshot(f),before);passed('Tree folder history stays pinned to the browsed older commit while local staged/unstaged files remain untouched');
 // Cancel a real new history read; previously loaded history must remain usable.
 await stable();await client.tool('rocket_click',{target:'file-load',expected_tab:state.active_tab});await until(s=>active().file_history.history_running&&control('file-cancel')?.enabled,'History worker running');
 const cancelStart=performance.now();await client.tool('rocket_click',{target:'file-cancel',expected_tab:state.active_tab});await settle();report.cancel_ms=performance.now()-cancelStart;assert(report.cancel_ms<10000);assert(!fh().running);assert(fh().notice?.includes('cancelled'));assert.equal(fh().tip,ids[20]);assert.equal(fh().entries.length,21);
 await type('file-path','../outside');await click('file-load');assert(fh().error?.includes('repository-relative'));assert.equal(fh().tip,ids[20]);await capture('invalid-path-retained-history');assert.deepEqual(snapshot(f),before);
 passed('Native read cancellation and invalid paths retain prior results without writes');
 await click('menu-view');await click('activity-open-menu');await until(s=>s.activity_history.open&&!s.activity_history.loading&&s.activity_history.records>0);
 await type('activity-search','directory-history');await until(s=>!s.activity_history.loading&&s.activity_history.rows.some(r=>r.operation==='directory-history'));const row=state.activity_history.rows.find(r=>r.operation==='directory-history'&&r.status==='Succeeded');assert(row);await click('activity-row-'+row.index);await capture('history-diagnostics');
 assert(!JSON.stringify(state.activity_history.selected).includes('PRIVATE_'));assert(!JSON.stringify(state.activity_history.selected).includes(DIR));
 assert.equal(state.activity_history.dropped,0);await click('activity-close');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const privateText of ['PRIVATE_',DIR,'recovered/検索'])assert(!payload.includes(privateText));
 for(const op of ['reflog','recovery-preview','recover-branch','directory-history']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const s of starts)assert(records.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request),'Unpaired '+op);}
 assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;passed('Activity viewer shows history operations and private paired diagnostics with cancellation and zero dropped events');report.passed=true;
}catch(e){report.error=e.message;if(client)try{await capture('failure');}catch{}throw e;}
finally{
 fs.writeFileSync('work/history-recovery-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(200);if(service.exitCode===null)service.kill();}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});

// The harness above matches the other isolated native MCP acceptance runners.
