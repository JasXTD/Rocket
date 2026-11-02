// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/external-tools-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/external-tools';
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
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function submit(){await click('dialog-submit');await settle();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}

const NAME='src/検索/表示 🚀 é.rs';
const marker=path.join(root,'owned-processes');
const helper=path.join(root,'tool with spaces 文.exe');
const external=()=>state.external_tools;
function snapshot(f){return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),file:fs.readFileSync(path.join(f.repo,NAME)).toString('base64')}}
function conflict(name){const f=fixture(name);f.write(NAME,'base\n');f.commit('base');f.git(['checkout','-b','rocket-ui-test/other']);f.write(NAME,'theirs\n');f.commit('theirs');f.git(['checkout',branch]);f.write(NAME,'ours\n');f.commit('ours');assert.throws(()=>f.git(['merge','rocket-ui-test/other']));return f;}
async function change(name,staged=false){await click('view-changes');await click('change-'+(staged?'staged':'unstaged')+'-'+Buffer.from(name).toString('hex'));}
async function reveal(key){if(control(key))return;await scroll('tools-scroll','up',2000);for(let n=0;!control(key)&&n<14;n++)await scroll('tools-scroll','down',240);assert(control(key),'Missing control '+key);}
// Profiles are edited in Preferences → External tools; a launch has its own dialog, which Edit profiles returns to.
const inPreferences=()=>!!control('pref-page-tools');
async function editProfiles(){if(!inPreferences()){if(control('tools-edit-profiles'))await click('tools-edit-profiles');else{await click('menu-edit');await click('preferences');await click('pref-page-tools');}}await until(()=>inPreferences(),'Preferences → External tools is open');}
async function saveProfiles(){if(external().settings_dirty)await click('tools-save-settings');assert(!external().error,external().error);assert.equal(external().settings_dirty,false);}
async function doneProfiles(){await click('dialog-submit');await until(()=>!inPreferences(),'Preferences closed');}
async function configure(mode,kind=external().kind,stay=false){await editProfiles();await reveal('tools-executable');await type('tools-executable',helper);await reveal('tools-arguments');await type('tools-arguments',mode+'\n'+({Editor:'{file}',Diff:'{left}\n{right}',Merge:'{base}\n{ours}\n{theirs}\n{result}'}[kind]));await saveProfiles();if(!stay)await doneProfiles();}
async function launch(){await reveal('tools-start');await click('tools-start');await until(s=>!external().running&&!s.input_pending,'External tool completion');}
async function mergePreview(f,mode='merge'){await open(f);await change(NAME);await click('conflict-open');await until(s=>!active().conflict_editor.running&&active().conflict_editor.bound);await click('conflict-external');await configure(mode);}
// Close ends a finished, refused or stopped launch; Hide keeps a running tool or an unreviewed merge result for the status bar.
async function close(){await reveal('tools-dismiss');await click('tools-dismiss');assert(!external().open);}
async function hide(){await reveal('tools-back');await click('tools-back');assert(!external().open);}
async function importDraft(){await reveal('tools-import');await click('tools-import');await until(s=>!external().running&&!s.input_pending);}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 fs.copyFileSync(path.resolve('target/release/examples/external_tool_fixture.exe'),helper);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:{...env,ROCKET_EXTERNAL_MARKER:marker},windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}assert.equal((await client.initialize()).serverInfo.version,version);

 const f=fixture('PRIVATE_DIFF_文');f.write(NAME,'base\n');f.commit('base');f.write(NAME,'index\n');f.git(['add','--',NAME]);f.write(NAME,'working\n');const before=snapshot(f);
 await open(f);await change(NAME);await click('diff-tools-menu');await click('diff-external');await configure('diff',undefined,true);await capture('settings-diff');await doneProfiles();await launch();assert.equal(external().success,true,external().error);assert.equal(fs.readFileSync(path.join(external().workspace,'observed-left'),'utf8'),'index\n');assert.equal(fs.readFileSync(path.join(external().workspace,'observed-right'),'utf8'),'working\n');assert.deepEqual(snapshot(f),before);
 await close();await change(NAME,true);await click('diff-tools-menu');await click('diff-external');await launch();assert.equal(fs.readFileSync(path.join(external().workspace,'observed-left'),'utf8'),'base\n');assert.equal(fs.readFileSync(path.join(external().workspace,'observed-right'),'utf8'),'index\n');assert.deepEqual(snapshot(f),before);passed('Native staged and unstaged external diffs use read-only copies and preserve repository bytes');
 await close();await click('view-commits');await click('detail-files');await click('diff-tools-menu');await click('diff-external');await launch();assert.equal(external().success,true,external().error);assert.deepEqual(snapshot(f),before);passed('Commit external diff uses pinned committed versions');
 await close();await change(NAME);await click('diff-tools-menu');await click('diff-editor');await configure('editor');await launch();assert.equal(external().success,true);assert.equal(fs.readFileSync(path.join(f.repo,NAME),'utf8'),'edited locally 世界\n');assert.equal(snapshot(f).index,before.index);assert.equal(snapshot(f).head,before.head);assert.match(external().notice,/Repository changes will refresh automatically/);await close();assert.equal(fs.readFileSync(path.join(f.repo,NAME),'utf8'),'edited locally 世界\n');passed('Editor explicitly edits only the chosen Unicode working file; index and HEAD remain unchanged, and closing its session does not claim to undo saves');

 const m=conflict('PRIVATE_MERGE_文');await mergePreview(m);const original=snapshot(m);await launch();assert.equal(external().success,true,external().error);assert.equal(external().result,'resolved 世界 🚀 é\n');assert.deepEqual(snapshot(m),original);await reveal('tools-import');await capture('merge-review');await importDraft();assert.equal(active().conflict_editor.draft,'resolved 世界 🚀 é\n');assert(active().conflict_editor.dirty);assert.deepEqual(snapshot(m),original);
 await click('conflict-stage');await click('dialog-cancel');assert.deepEqual(snapshot(m),original);assert(active().conflict_editor.dirty);await click('conflict-stage');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(fs.readFileSync(path.join(m.repo,NAME),'utf8'),'resolved 世界 🚀 é\n');assert.equal(m.git(['show',':'+NAME]),'resolved 世界 🚀 é');assert.equal(snapshot(m).head,original.head);
 const recovery=fs.readdirSync(path.join(m.repo,'.git')).filter(n=>n.includes('recovery'));assert(recovery.length);passed('Merge stays isolated until review and confirmed save; cancelled save preserves draft; staged resolution has recovery copies');

 const stale=conflict('PRIVATE_STALE_文');await mergePreview(stale);await launch();const staleBefore=snapshot(stale);stale.write(NAME,'external editor changed this\n');await importDraft();assert.match(external().error,/changed outside/);assert.equal(external().notice,null,'Finished import must not show a stale checking notice');assert.equal(fs.readFileSync(path.join(stale.repo,NAME),'utf8'),'external editor changed this\n');assert.equal(snapshot(stale).index,staleBefore.index);await capture('merge-external-edit');passed('External working-file edits reject merge import without overwriting or staging');

 await close();const newer=conflict('PRIVATE_DRAFT_文');await mergePreview(newer);await launch();await hide();await type('conflict-result','newer Rocket draft 世界\n');await click('status-tool');assert(external().open);await importDraft();assert.match(external().error,/draft changed/);assert.equal(external().notice,null);assert.equal(active().conflict_editor.draft,'newer Rocket draft 世界\n');passed('A newer Rocket draft cannot be replaced by an older external result kept with Hide');

 for(const mode of ['fail','binary','large','missing']) {await close();const bad=conflict('PRIVATE_'+mode);await mergePreview(bad,mode);const original=snapshot(bad);await launch();assert.equal(external().success,false,mode);assert(external().error);assert.equal(external().result,null);assert.deepEqual(snapshot(bad),original);if(mode==='fail'){assert.match(external().error,/7/);await capture('merge-tool-failure');}}
 passed('Nonzero exit, binary, oversized and missing results give visible errors and preserve originals');

 await close();const cancelled=conflict('PRIVATE_CANCEL_文');await mergePreview(cancelled,'wait');const cancelBefore=snapshot(cancelled);await reveal('tools-start');await click('tools-start');await until(()=>external().running&&fs.existsSync(marker),'Owned process started');const pids=fs.readFileSync(marker,'utf8').split(/\s+/).map(Number);await reveal('tools-cancel');const started=performance.now();await click('tools-cancel');await until(()=>!external().running,'Cancelled external process');report.cancel_ms=Math.round(performance.now()-started);assert.equal(external().success,false);assert.match(external().error,/cancel/);assert.deepEqual(snapshot(cancelled),cancelBefore);for(const pid of pids){assert.throws(()=>process.kill(pid,0),'Owned process is still alive '+pid);}passed('Cancellation terminates the owned parent and child, retains the workspace and never imports');

 await close();await configure('merge','Merge',true);await resize(780,620);await capture('tools-small');await resize(1480,920);
 // Test development scope independently of the normal user's executable picker.
 await reveal('tools-executable');await type('tools-executable','C:\\Windows\\System32\\notepad.exe');await saveProfiles();await doneProfiles();await click('conflict-external');await reveal('tools-start');await assert.rejects(()=>client.tool('rocket_click',{target:'tools-start',expected_tab:state.active_tab}),/fixture root/);await configure('merge');passed('Localhost MCP rejects external programs outside its fixture root');
 await closeWindow();await client.tool('rocket_open',{path:cancelled.repo});await settle();await click('menu-edit');await click('preferences');await click('pref-page-tools');await click('tools-kind-2');assert.equal(external().executable,helper);assert.equal(external().arguments,'merge\n{base}\n{ours}\n{theirs}\n{result}');passed('Tool configuration survives native window close/reopen in the isolated profile');

 await doneProfiles();await mergePreview(cancelled);const lateOriginal=snapshot(cancelled);await launch();await importDraft();cancelled.write(NAME,'late outside edit after draft import\n');await click('conflict-stage');await submit();assert.equal(latest().command_succeeded,false);assert.equal(fs.readFileSync(path.join(cancelled.repo,NAME),'utf8'),'late outside edit after draft import\n');assert.equal(snapshot(cancelled).index,lateOriginal.index);passed('A late external edit between draft import and confirmed save is rejected without staging');
 const missing=conflict('PRIVATE_MISSING_EXECUTABLE');await mergePreview(missing);await editProfiles();await reveal('tools-executable');await type('tools-executable',path.join(root,'missing-tool.exe'));await saveProfiles();await doneProfiles();const noLaunch=snapshot(missing);await launch();assert.equal(external().success,false);assert.match(external().error,/inspect external executable/);assert.deepEqual(snapshot(missing),noLaunch);passed('A missing configured executable gives a clear launch preparation error and leaves the repository unchanged');
 await close();await mergePreview(missing,'wait');if(fs.existsSync(marker))fs.unlinkSync(marker);await reveal('tools-start');await click('tools-start');await until(()=>external().running&&fs.existsSync(marker),'Tool running before quit');const quitPids=fs.readFileSync(marker,'utf8').split(/\s+/).map(Number);await closeWindow();for(const pid of quitPids)assert.throws(()=>process.kill(pid,0),'Quit left an owned process running');assert.deepEqual(snapshot(missing),noLaunch);passed('Quitting the native window cancels and reaps the owned tool and child before closing, preserving the original conflict');
 await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const secret of ['PRIVATE_',NAME,'resolved 世界',helper,'external editor changed this'])assert(!payload.includes(secret),'Diagnostic payload leak');for(const op of ['external-editor','external-diff','external-merge','external-prepare','external-process','external-result','external-import'])assert(records.some(r=>r.operation===op&&r.event==='request_end'),'Missing log '+op);assert(records.some(r=>r.failure_code==='external_tool_exit'&&r.exit_code===7));assert(records.some(r=>r.outcome==='cancelled'));assert(records.some(r=>r.failure_code==='conflict_file_changed'));assert(records.some(r=>r.failure_code==='external_merge_draft_changed'));assert(records.some(r=>r.operation==='external-settings'));assert(records.some(r=>r.operation==='external-cancel'));report.log_records=records.length;passed('Logs identify phases, cancellation, exit code and external-edit failure without tested private payloads or dropped records');report.passed=true;
 }catch(error){report.failure=error.stack;if(client)try{await capture('failure')}catch{}throw error;}
 finally{fs.writeFileSync('work/external-tools-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(150);if(service.exitCode===null)service.kill()}if(client)await client.finish()}}
run().catch(error=>{console.error(error);process.exitCode=1});
