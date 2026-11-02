// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const root=path.resolve('work/mcp-fixtures/patch-tools-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/patch-tools';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const focused=process.argv.includes('--preview-only');
const report={passed:false,version,root,focused,cases:[]},fixtures=[];
fs.mkdirSync(root,{recursive:true});
// Explicit local consent: this runner creates only synthetic shell/editor data.
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function fixture(name){
  const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
  const git=args=>cp.execFileSync('git',['-c','diff.autoRefreshIndex=false','-C',repo,...args],{env,windowsHide:true,encoding:'utf8'}).trimEnd();
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
const BASE='// PRIVATE_BASE\nfn main() {\n    println!("Hello");\n}\n';
const EDIT='// PRIVATE_BASE\nfn main() {\n    println!("你好 🚀");\n    trace!("KEEP");\n}\n';
function dialog(){return state.dialog?.patch;}
const file=(f,p=FILE)=>fs.readFileSync(path.join(f.repo,p));
function invariant(f){return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),sentinel:hash(path.join(f.repo,'sentinel'))};}
async function closeDialog(){if(state.dialog){assert(!dialog()?.applying);await click('dialog-cancel');}if(control('console-close'))await click('console-close');}
async function patchDialog(mode,fileName){await closeDialog();await menuItem('repository-patch-'+mode);assert.equal(state.dialog.kind,'patch-'+mode);await type('dialog-patch-path',fileName);}
async function preview(){await click('dialog-patch-review');assert(dialog().preview_loaded,JSON.stringify(state.dialog));}
async function change(name=FILE,staged=false){await click('view-changes');const key='change-'+(staged?'staged':'unstaged')+'-'+Buffer.from(name).toString('hex');for(let n=0;!control(key)&&n<20;n++)await scroll('changes-scroll');await click(key);}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 const f=fixture('PRIVATE_PATCHES_検索 🚀');f.write(FILE,BASE);f.write('other.txt','base\n');f.write('sentinel','PRIVATE_UNTOUCHED');f.commit('PRIVATE_INITIAL');
 f.write('other.txt','staged\n');f.git(['add','--','other.txt']);f.write('other.txt','staged plus working\n');f.write(FILE,EDIT);const original=invariant(f);
 const output=path.join(root,'changes.patch');await open(f);await patchDialog('export',output);await preview();await capture('export-wide');assert.equal(dialog().paths.length,2);await submit();assert(dialog().notice.includes('saved'));assert(fs.existsSync(output));assert.deepEqual(invariant(f),original);const patchBytes=fs.readFileSync(output);
 passed('Export unstaged changes through the UI; Unicode paths and existing repository state preserved');

 await patchDialog('export',path.join(root,'staged.patch'));await click('dialog-patch-source-staged');await preview();assert.deepEqual(dialog().paths,['other.txt']);await submit();assert(fs.existsSync(path.join(root,'staged.patch')));
 await patchDialog('export',path.join(root,'commit.patch'));await click('dialog-patch-source-commit');await type('dialog-patch-revision','HEAD');await preview();await submit();assert(fs.existsSync(path.join(root,'commit.patch')));assert.deepEqual(invariant(f),original);
 passed('Staged and commit export previews produce patch files without changing staging or history');

 f.write(FILE,BASE);f.write('other.txt','staged\n');await patchDialog('import',output);await preview();assert.deepEqual(file(f),Buffer.from(BASE));await capture('import-wide');await submit();assert(dialog().notice.includes('verified'));assert.deepEqual(file(f),Buffer.from(EDIT));assert.equal(file(f,'other.txt').toString(),'staged plus working\n');assert.deepEqual(invariant(f),original);
 passed('Multi-file import previews and applies through the real UI; staged changes, HEAD and unrelated files stay intact');

 await closeDialog();await menuItem('repository-file-backups');const b=state.dialog.file_edit.backups.findIndex(b=>b.path===FILE);assert(b>=0);await click('dialog-file-backup-'+b);await submit();assert.deepEqual(file(f),Buffer.from(BASE));assert.deepEqual(invariant(f),original);
 passed('Imported file can be recovered through the existing backup browser');

 f.write('other.txt','staged\n');await patchDialog('import',output);await preview();f.write(FILE,'PRIVATE_EXTERNAL\n');await submit();assert(state.dialog.error.includes('file_edit_stale'));assert.deepEqual(file(f),Buffer.from('PRIVATE_EXTERNAL\n'));assert.equal(file(f,'other.txt').toString(),'staged\n');assert.deepEqual(invariant(f),original);await capture('external-edit-rejected');
 passed('External edit after import review rejects the entire pending batch before the first write');

 f.write(FILE,EDIT);f.write('other.txt','staged plus working\n');await patchDialog('import',output);await click('dialog-patch-review');assert(!dialog().preview_loaded);assert(state.dialog.error.includes('patch_conflict'));assert(!control('dialog-submit').enabled);await capture('conflict-rejected');assert.deepEqual(invariant(f),original);
 passed('Conflicting import has a clear refusal and disabled apply; no conflict markers or reject files');

 f.write(FILE,BASE);f.write('other.txt','staged\n');await patchDialog('import',output);await preview();fs.writeFileSync(output,Buffer.concat([patchBytes,Buffer.from('PRIVATE_CHANGED_SOURCE\n')]));await submit();assert(state.dialog.error.includes('patch_source_changed'));assert.deepEqual(file(f),Buffer.from(BASE));fs.writeFileSync(output,patchBytes);
 f.write(FILE,EDIT);const collision=path.join(root,'collision.patch');await patchDialog('export',collision);await preview();fs.writeFileSync(collision,'PRIVATE_KEEP_EXISTING');await submit();assert(state.dialog.error.includes('patch_output_exists'));assert.equal(fs.readFileSync(collision,'utf8'),'PRIVATE_KEEP_EXISTING');assert.deepEqual(invariant(f),original);
 passed('Changed patch source and output filename races are rejected without overwriting existing data');

 await closeDialog();await refresh();await change();const raw=f.git(['--no-optional-locks','diff','--no-ext-diff','--no-textconv','--unified=3','--',FILE]).split('\n');
 const lines=raw.flatMap((line,i)=>(line==='-    println!("Hello");'||line==='+    println!("你好 🚀");')?[i]:[]);assert.equal(lines.length,2);
 for(const i of lines)await click('diff-line-'+i);assert.equal(active().selected_lines,2);await click('diff-discard-selection');assert(dialog().preview_loaded);await capture('discard-selected-wide');await submit();const retained=BASE.replace('}\n','    trace!("KEEP");\n}\n');assert.deepEqual(file(f),Buffer.from(retained));assert.deepEqual(invariant(f),original);
 await closeDialog();await change();const hunk=state.controls.find(c=>c.id.startsWith('diff-hunk-')&&c.enabled);assert(hunk);await click(hunk.id);await click('diff-discard-selection');assert(dialog().preview_loaded);await submit();assert.deepEqual(file(f),Buffer.from(BASE));assert.deepEqual(invariant(f),original);
 passed('Selected changed lines and a whole hunk discard only their changes, retaining unselected edits and staged contents');

 await closeDialog();f.write(FILE,EDIT);await refresh();await change();await click('diff-split');const row=state.controls.find(c=>c.id.startsWith('diff-line-')&&c.enabled);assert(row);await click(row.id);await click('diff-discard-selection');assert(dialog().preview_loaded);await click('dialog-cancel');assert.deepEqual(file(f),Buffer.from(EDIT));
 await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal?.running&&s.terminal.visible&&s.terminal.tail.includes('PS '),'Ready PowerShell');await resize(820,620);await change();await capture('diff-narrow-terminal');await patchDialog('import',output);await capture('import-narrow-terminal');assert(control('dialog-cancel').rect[3]<620);await closeDialog();if(control('terminal-hide'))await click('terminal-hide');await resize(1480,920);if(control('console-close'))await click('console-close');
 passed('Split-diff selection and closing review preserve bytes; compact layout keeps actions accessible with terminal open');

 f.write(FILE,BASE);f.write('other.txt','staged\n');await patchDialog('import',output);await stable();await client.tool('rocket_click',{target:'dialog-patch-review',expected_tab:state.active_tab});await until(s=>!s.input_pending&&dialog()?.loading,'Active import preview');await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();assert(dialog().notice.includes('cancelled'));assert(!dialog().preview_loaded);assert.deepEqual(file(f),Buffer.from(BASE));
 await preview();await stable();await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>!s.input_pending&&s.write&&dialog()?.applying,'Active import write');const start=performance.now();await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();report.cancel_ms=performance.now()-start;assert(!dialog().applying);assert(state.dialog.error||dialog().notice);assert.deepEqual(file(f),Buffer.from(BASE));assert.equal(file(f,'other.txt').toString(),'staged\n');assert.deepEqual(invariant(f),original);await capture('cancelled-import');
 f.write(FILE,EDIT);const cancelledOutput=path.join(root,'cancelled-output.patch');await patchDialog('export',cancelledOutput);await preview();await stable();await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>!s.input_pending&&s.write&&dialog()?.applying,'Active export write');await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();assert(!fs.existsSync(cancelledOutput));assert.deepEqual(file(f),Buffer.from(EDIT));assert.deepEqual(invariant(f),original);assert(state.dialog.error||dialog().notice);
 passed('Real import-preview, import-write and export-write cancellation clears stale apply state and preserves files before publication');

 const g=fixture('PRIVATE_PARTIAL');g.write('a-first.txt','base first\n');g.write('z-last.txt','base last\n');g.write('sentinel','PRIVATE_PARTIAL_SENTINEL');g.commit('PRIVATE_PARTIAL_BASE');const stableIndex=invariant(g);
 g.write('a-first.txt','next first\n');g.write('z-last.txt','next last\n');const late=path.join(root,'late-cancel.patch');fs.writeFileSync(late,g.git(['diff','--no-ext-diff','--no-textconv','--no-renames'])+'\n');g.write('a-first.txt','base first\n');g.write('z-last.txt','base last\n');await open(g);await patchDialog('import',late);await preview();await stable();await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});
 await until(s=>!s.input_pending&&s.write&&file(g,'a-first.txt').toString()==='next first\n','First file published');await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();assert(state.dialog.error.includes('after changing 1 file'));assert.equal(file(g,'a-first.txt').toString(),'next first\n');assert.equal(file(g,'z-last.txt').toString(),'base last\n');assert.deepEqual(invariant(g),stableIndex);await capture('cancelled-partway');
 await closeDialog();await menuItem('repository-file-backups');const restore=state.dialog.file_edit.backups.findIndex(b=>b.path==='a-first.txt');assert(restore>=0);await click('dialog-file-backup-'+restore);await submit();assert.equal(file(g,'a-first.txt').toString(),'base first\n');assert.deepEqual(invariant(g),stableIndex);
 passed('Cancellation after the first replacement reports a partial batch, preserves the remaining file and restores through backups');

 await patchDialog('import',path.resolve('Cargo.toml'));await stable();await assert.rejects(client.tool('rocket_click',{target:'dialog-patch-review',expected_tab:state.active_tab}),/fixture root/);await closeDialog();
 await patchDialog('export',path.resolve('work/outside-fixture.patch'));await stable();await assert.rejects(client.tool('rocket_click',{target:'dialog-patch-review',expected_tab:state.active_tab}),/fixture root/);await closeDialog();
 passed('Localhost MCP rejects patch input/output outside its configured temporary fixture root');

 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 const payload=JSON.stringify(records);assert(records.length);assert(records.every(r=>r.dropped_before===0));for(const privateText of ['PRIVATE_',FILE,root,'你好'])assert(!payload.includes(privateText),'Diagnostic leak '+privateText);
 for(const operation of ['patch-preview','patch-export-preview','patch-import','patch-export','discard-preview','discard-selection']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===operation);assert(starts.length,'Missing request '+operation);for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired request '+operation);}
 for(const step of ['parse-patch','snapshot-targets','check-patch','build-patch-result','cleanup-preview','revalidate-patch','apply-patch-files','prepare-backup','publish-file','verify-patch-files','publish-export'])assert(records.some(r=>r.event==='step_start'&&r.step===step),'Missing phase '+step);
 for(const start of records.filter(r=>r.event==='step_start'&&(r.operation.startsWith('patch-')||r.operation.startsWith('discard-'))))assert(records.some(r=>r.event==='step_end'&&r.session===start.session&&r.id===start.id),'Unpaired phase '+start.step);
 for(const code of ['file_edit_stale','patch_conflict','patch_source_changed','patch_output_exists'])assert(payload.includes(code),'Missing error code '+code);assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;
 assert(records.some(r=>r.event==='patch_outcome'&&r.changed_files===1&&r.complete===false&&r.verified===true),'Missing verified partial outcome');
 passed('Paired phase logs and typed failure codes, zero dropped records, and no tested private paths or patch contents');report.passed=true;
 }catch(error){if(client)try{await capture('failure')}catch{}throw error;}
 finally{fs.writeFileSync('work/patch-tools-last-result.txt',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();}}
run().catch(error=>{console.error(error);process.exitCode=1;});
