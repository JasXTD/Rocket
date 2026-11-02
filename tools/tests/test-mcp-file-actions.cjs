// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root=path.resolve('work/mcp-fixtures/file-actions-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/file-actions';
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
const ORIGINAL='// PRIVATE_BASE\nfn main() {\n    println!("Hello");\n}\n';
const STAGED='// PRIVATE_STAGED\nfn main() {\n    println!("staged");\n}\n';
const WORKING='// PRIVATE_WORKING\nfn main() {\n    println!("你好 🚀 é");\n    println!("你好");\n}\n';
function dialog(){return state.dialog?.file_edit;}
function file(f,p=FILE){return fs.readFileSync(path.join(f.repo,p));}
function invariant(f){return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),sentinel:hash(path.join(f.repo,'sentinel'))};}
async function change(name=FILE,staged=false){await click('view-changes');const key='change-'+(staged?'staged':'unstaged')+'-'+Buffer.from(name).toString('hex');for(let n=0;!control(key)&&n<20;n++)await scroll('changes-scroll');await click(key);}
async function closeDialog(){if(state.dialog){assert(!dialog()?.applying);await click('dialog-cancel');}if(control('console-close'))await click('console-close');}
async function backups(){await closeDialog();await menuItem('repository-file-backups');assert.equal(state.dialog.kind,'file-edit');}
async function restore(path,revision){await backups();await type('dialog-file-path',path);await type('dialog-file-revision',revision);await click('dialog-file-review');}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 const f=fixture('PRIVATE_FILES_検索 🚀');f.write(FILE,ORIGINAL);f.write('sentinel','PRIVATE_UNTOUCHED');f.old=f.commit('PRIVATE_INITIAL');f.write(FILE,STAGED);f.git(['add','--',FILE]);f.write(FILE,WORKING);let base=invariant(f);
 await open(f);await change();await type('diff-search','你好');assert.equal(active().diff_tools.matches,2);assert.equal(active().diff_tools.current,0);await click('diff-search-next');assert.equal(active().diff_tools.current,1);await click('diff-search-prev');assert.equal(active().diff_tools.current,0);await click('diff-split');await capture('diff-split-search');await click('diff-syntax');assert(!active().diff_tools.syntax);await click('diff-syntax');await click('diff-unified');await type('diff-search','PRIVATE_NO_MATCH');assert.equal(active().diff_tools.matches,0);await type('diff-search','é');assert.equal(active().diff_tools.matches,1);await click('diff-search-clear');
 const line=state.controls.find(c=>c.id.startsWith('diff-line-')&&c.enabled);assert(line);await click(line.id);assert.equal(active().selected_lines,1);await type('diff-search','你好');assert.equal(active().selected_lines,1);await capture('unicode-diff-wide');assert.deepEqual(file(f),Buffer.from(WORKING));assert.deepEqual(invariant(f),base);
 passed('Literal Unicode diff search, next/previous/no-match, unified/split, syntax toggle; raw line selection and repository data unchanged');

 await click('change-discard');assert(dialog().preview_loaded);assert.equal(dialog().preview.before_bytes,Buffer.byteLength(WORKING));await capture('discard-review-wide');await click('dialog-submit');assert(dialog().notice?.includes('verified'));assert.deepEqual(file(f),Buffer.from(STAGED));assert.deepEqual(invariant(f),base);await backups();assert.equal(dialog().backups.length,1);await click('dialog-file-backup-0');await click('dialog-submit');assert.deepEqual(file(f),Buffer.from(WORKING));assert.deepEqual(invariant(f),base);await capture('recovered');await closeDialog();
 passed('Whole-file discard returns to the staged version; recovery restores exact working bytes and preserves HEAD/index/config/sentinel');

 await change();await click('change-discard');f.write(FILE,'PRIVATE_EXTERNAL_EDIT\n');await click('dialog-submit');assert(state.dialog.error.includes('file_edit_stale'));assert.deepEqual(file(f),Buffer.from('PRIVATE_EXTERNAL_EDIT\n'));await capture('stale-preview');await closeDialog();
 await restore(FILE,'PRIVATE_missing_revision');assert(state.dialog.error.includes('file_edit_revision'));assert(!control('dialog-submit').enabled);await type('dialog-file-revision',f.old);await click('dialog-preview-refresh');assert.equal(dialog().preview.commit,f.old);await click('dialog-submit');assert.deepEqual(file(f),Buffer.from(ORIGINAL));assert.deepEqual(invariant(f),base);await closeDialog();
 await click('view-tree');await type('tree-revision',f.old);await click('tree-load');for(const name of ['src','検索','結果 🚀 é.rs']){const i=active().repository_tree.entries.findIndex(e=>e.name===name);assert(i>=0);await click('tree-entry-'+i);}f.write(FILE,'PRIVATE_NEWER\n');await click('tree-restore');assert(dialog().preview_loaded);assert.equal(dialog().preview.commit,f.old);await click('dialog-submit');assert.deepEqual(file(f),Buffer.from(ORIGINAL));await closeDialog();
 passed('External edits reject a stale review; invalid revisions are clear; commit and repository-tree restore use a pinned source without staging');

 fs.unlinkSync(path.join(f.repo,FILE));await refresh();await change();await click('change-discard');assert.equal(dialog().preview.before_bytes,null);await click('dialog-submit');assert.deepEqual(file(f),Buffer.from(STAGED));await backups();assert.equal(dialog().backups[0].present,false);await click('dialog-file-backup-0');await click('dialog-submit');assert(!fs.existsSync(path.join(f.repo,FILE)));await closeDialog();
 const binary='untracked 中文.bin';f.write(binary,Buffer.from([0,255,66,10]));await refresh();await change(binary);await click('change-discard');assert.equal(dialog().preview.after_bytes,null);await click('dialog-submit');assert(!fs.existsSync(path.join(f.repo,binary)));await backups();await click('dialog-file-backup-0');await click('dialog-submit');assert.deepEqual(file(f,binary),Buffer.from([0,255,66,10]));await closeDialog();assert.deepEqual(invariant(f),base);
 passed('Missing-file restoration and recovery preserve absence; untracked binary discard is recoverable with exact bytes');

 f.write(FILE,WORKING);await refresh();await change();await stable();await client.tool('rocket_click',{target:'change-discard',expected_tab:state.active_tab});await until(s=>!s.input_pending&&dialog()?.loading,'Active preview');await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();assert(dialog().notice.includes('cancelled'));assert(!dialog().preview_loaded);assert.deepEqual(file(f),Buffer.from(WORKING));await click('dialog-preview-refresh');assert(dialog().preview_loaded);await stable();await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>!s.input_pending&&s.write&&dialog()?.applying,'Active file write');const start=performance.now();await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();report.cancel_ms=performance.now()-start;assert(!dialog().applying);assert(state.dialog.error||dialog().notice);assert.deepEqual(file(f),Buffer.from(WORKING));assert.deepEqual(invariant(f),base);await capture('cancelled-write');await closeDialog();
 passed('Real preview and write cancellation preserve working contents, leave no stale enabled preview and report the observed outcome');

 await backups();await resize(780,680);await capture('backups-narrow');assert(dialog().backups.length>=8);await scroll('dialog-file-backups-scroll');await capture('backups-narrow-scrolled');await closeDialog();await resize(1480,920);
 await restore('.git/config','HEAD');assert(state.dialog.error.includes('file_edit_unsupported'));await closeDialog();await restore(FILE,'HEAD');await resize(780,680);await capture('restore-narrow');await closeDialog();await click('view-changes');await change();await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal?.running&&s.terminal.tail.includes('PS '),'Ready PowerShell');await resize(780,680);await type('diff-search','你好');await capture('diff-narrow-terminal');
 const terminalTop=control('terminal-hide').rect[1]-6;assert(control('diff-search').rect[3]<terminalTop);assert(state.controls.some(c=>c.id.startsWith('diff-line-')&&c.rect[3]-c.rect[1]>=18&&c.rect[3]<=terminalTop),'No visible diff line above terminal');
 await click('change-file-actions');await click('change-discard');await capture('review-narrow-terminal');
 for(const c of state.controls.filter(c=>c.id.startsWith('dialog-')))assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=state.viewport[0]+1&&c.rect[3]<=state.viewport[1]+1,'Clipped control '+JSON.stringify(c));await closeDialog();if(control('terminal-hide'))await click('terminal-hide');await resize(1480,920);
 passed('Git metadata path refused; compact native review and Unicode diff layout verified with a ready terminal');

 // A selection changes only display state until the explicit stage action is used.
 await change();await click('diff-search-clear');const selected=state.controls.find(c=>c.id.startsWith('diff-line-')&&c.enabled);await click(selected.id);await click('diff-apply-selection');assert.notEqual(hash(path.join(f.repo,'.git/index')),base.index);assert.deepEqual(file(f),Buffer.from(WORKING));assert.equal(hash(path.join(f.repo,'sentinel')),base.sentinel);
 passed('Existing partial staging still changes only the selected index lines after search/highlighting integration');

 const dense=fixture('PRIVATE_DENSE');dense.write('dense.rs','fn base() {}\n');dense.commit('PRIVATE_DENSE baseline');dense.write('dense.rs',('fn 你好 123; '.repeat(500)+'\n').repeat(45));await open(dense);await change('dense.rs');await type('diff-search','你好 123');assert.equal(active().diff_tools.matches,20000);assert(active().diff_tools.limited);await click('diff-search-next');await click('diff-split');await click('diff-syntax');await capture('dense-search');await type('diff-search','PRIVATE_ABSENT');assert.equal(active().diff_tools.matches,0);assert(!active().diff_tools.limited);await click('diff-search-clear');
 passed('Dense long Unicode lines remain controllable with syntax/search overlays; 20,000-match limit is explicit and new queries replace old hits');

 await closeWindow();await client.raw('POST','/shutdown');const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 const payload=JSON.stringify(records);assert(records.length);assert(records.every(r=>r.dropped_before===0));for(const secret of ['PRIVATE_',FILE,root,'你好'])assert(!payload.includes(secret),'Diagnostic leak '+secret);
 for(const operation of ['file-preview','file-discard','file-restore','file-recover','file-backups','diff-search']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===operation);assert(starts.length,'No request '+operation);for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Missing request end '+operation);}
 for(const step of ['review-file','revalidate-file','prepare-backup','revalidate-before-publish','publish-file','verify-file','read-backups'])assert(records.some(r=>r.event==='step_start'&&r.step===step),'Missing step '+step);
 for(const start of records.filter(r=>r.event==='step_start'&&r.operation.startsWith('file-')))assert(records.some(r=>r.event==='step_end'&&r.session===start.session&&r.id===start.id),'Missing phase outcome '+start.step);
 assert(payload.includes('file_edit_stale'));assert(payload.includes('file_edit_revision'));assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;passed('Paired operation diagnostics, explicit stale/invalid/cancelled outcomes, zero dropped records and no private payloads');report.passed=true;
}catch(error){if(client)try{await capture('failure')}catch{}throw error;}
finally{fs.writeFileSync('work/file-actions-last-result.txt',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();}}
run().catch(e=>{console.error(e);process.exitCode=1});
