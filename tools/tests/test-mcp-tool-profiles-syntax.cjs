// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/tool-profiles-syntax-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/external-tools';
// The live tree's version: this runner drives target/release, not the frozen 0.63.0 copy.
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
async function reveal(key){if(control(key))return;await scroll("tools-scroll","up",2000);for(let n=0;!control(key)&&n<14;n++)await scroll("tools-scroll","down",240);assert(control(key),"Missing control "+key);}
// Profiles are edited in Preferences → External tools; a launch has its own dialog, which Edit profiles returns to.
const inPreferences=()=>!!control('pref-page-tools');
async function editProfiles(){if(!inPreferences()){if(control('tools-edit-profiles'))await click('tools-edit-profiles');else{await click('menu-edit');await click('preferences');await click('pref-page-tools');}}await until(()=>inPreferences(),'Preferences → External tools is open');}
async function doneProfiles(){await click('dialog-submit');await until(()=>!inPreferences(),'Preferences closed');}
async function configure(mode,kind=external().kind){await editProfiles();await reveal('tools-executable');await type('tools-executable',helper);await reveal('tools-arguments');await type('tools-arguments',mode+'\n'+({Editor:'{file}',Diff:'{left}\n{right}',Merge:'{base}\n{ours}\n{theirs}\n{result}'}[kind]));if(external().settings_dirty)await click('tools-save-settings');assert(!external().error,external().error);}
async function launch(){await reveal('tools-start');await click('tools-start');await until(s=>!external().running&&!s.input_pending,'External tool completion');}
async function mergePreview(f,mode='merge'){await open(f);await change(NAME);await click('conflict-open');await until(s=>!active().conflict_editor.running&&active().conflict_editor.bound);await click('conflict-external');await configure(mode);}
// Close ends a finished, refused or stopped launch.
async function close(){await reveal('tools-dismiss');await click('tools-dismiss');assert(!external().open);}
async function importDraft(){await reveal('tools-import');await click('tools-import');await until(s=>!external().running&&!s.input_pending);}
// Appended to the established external-tools fixture/MCP helpers by prepare script.
async function selectProfile(index){await reveal('tools-profile-picker');await click('tools-profile-picker');await click('tools-profile-'+index);}
async function saveProfiles(){await reveal('tools-save-settings');await click('tools-save-settings');assert.equal(external().error,null);assert.equal(external().settings_dirty,false);}
async function profileName(name){await reveal('tools-profile-name');await type('tools-profile-name',name);}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 fs.copyFileSync(path.resolve('target/release/examples/external_tool_fixture.exe'),helper);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:{...env,ROCKET_EXTERNAL_MARKER:marker},windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}assert.equal((await client.initialize()).serverInfo.version,version);
 const f=fixture('PRIVATE_PROFILES_文');
 const samples={
  [NAME]:['pub fn render(label: &str) -> String {\n    format!("Hello {label}")\n}\n','pub fn render(label: &str) -> String {\n    let greeting = r#"你好 🚀 é"#;\n    format!("{greeting} {label}")\n}\n','Rust'],
  'src/文/worker.py':['def old():\n    return 1\n','async def render(label: str):\n    text = """hello\n# this is still a string\n"""\n    return label\n','Python'],
  'src/文/view.ts':['const n = 1;\n','interface View { title: string }\nconst n = `hello\n// inside template`;\n','TypeScript'],
  'src/文/config.json':['{"name":"old"}\n','{"name":"你好", "count":42, "enabled":true}\n','JSON'],
  'src/文/plain.unknown':['before\n','fn text is plain\n',null],
 };
 for(const [name,[before]]of Object.entries(samples))f.write(name,before);f.commit('PRIVATE syntax baseline');
 for(const [name,[,after]]of Object.entries(samples))f.write(name,after);
 const original=snapshot(f);await open(f);
 for(const [name,[,,language]]of Object.entries(samples)){
  await change(name);const file=active().patch_files.find(p=>p.path===name);assert(file,'Missing rendered patch '+name);assert.equal(file.syntax_language,language);assert.equal(file.syntax_limited,false);assert(language?file.syntax_spans>0:file.syntax_spans===0);
  if(language==='Rust'){
   await until(()=>!active().word_diff.running,'Word overlay ready');assert(active().word_diff.spans>0);await type('diff-search','你好');await until(()=>!active().diff_tools.searching&&active().diff_tools.matches===1,'Unicode search overlay');
   await capture('rust-syntax-unified');await click('diff-split');await capture('rust-syntax-split');await click('diff-syntax');assert.equal(active().diff_tools.syntax,false);await capture('syntax-disabled');await click('diff-syntax');await click('diff-unified');await type('diff-search','');
  }
 }
 assert.deepEqual(snapshot(f),original);passed('Rust/Python/TypeScript/JSON and unknown files render through native MCP; Unicode search, word overlays, split/unified and syntax toggle preserve source/index');
 f.write(NAME,'// external edit 你好\n');await refresh();await change(NAME);assert.equal(active().patch_files[0].syntax_language,'Rust');await type('diff-search','external edit');await until(()=>!active().diff_tools.searching&&active().diff_tools.matches===1,'Refreshed syntax and search');f.write(NAME,samples[NAME][1]);await refresh();await change(NAME);await type('diff-search','');passed('An external file edit refreshes the patch and its highlighting/search without stale results');

 await click('diff-tools-menu');await click('diff-external');assert.deepEqual(external().profiles,['Default']);await editProfiles();await profileName('Compare 文');await configure('diff');
 await reveal('tools-profile-duplicate');await click('tools-profile-duplicate');await profileName('Alternate 🚀');await reveal('tools-arguments');await type('tools-arguments','fail-profile\n{left}\n{right}');await reveal('tools-profile-default');await click('tools-profile-default');await saveProfiles();assert.equal(external().default_profile,1);
 await doneProfiles();await selectProfile(0);await launch();assert.equal(external().success,true,external().error);assert.equal(external().default_profile,1);assert.equal(fs.readFileSync(path.join(external().workspace,'observed-right'),'utf8'),samples[NAME][1]);assert.deepEqual(snapshot(f),original);
 await close();await click('diff-tools-menu');await click('diff-external');assert.equal(external().selected_profile,1);await launch();assert.equal(external().success,false);assert.match(external().error,/9/);assert.deepEqual(snapshot(f),original);await capture('selected-profile-failure');passed('Two diff profiles launch the chosen configuration; temporary selection leaves the default intact and the next preview uses it; failed alternate reports exit 9 safely');
 await close();await editProfiles();await profileName('Compare 文');await reveal('tools-save-settings');await click('tools-save-settings');assert.match(external().error,/unique profile name/);await reveal('tools-discard-settings');await click('tools-discard-settings');assert.equal(external().settings_dirty,false);assert.equal(external().profiles[1],'Alternate 🚀');
 await reveal('tools-profile-add');await click('tools-profile-add');await until(()=>control('tools-known-empty'),'Add list');await click('tools-known-empty');await profileName('Unsaved profile');await reveal('tools-discard-settings');await click('tools-discard-settings');assert.equal(external().profiles.length,2);
 await reveal('tools-arguments');await type('tools-arguments','diff\n{left}\n{right}');await saveProfiles();await resize(780,620);await capture('profiles-small');await resize(1480,920);await capture('profiles-wide');passed('Duplicate names are rejected; discarded edits and additions restore saved profiles; settings fit narrow and wide native windows');
 await doneProfiles();await closeWindow();await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);await click('menu-edit');await click('preferences');await click('pref-page-tools');await click('tools-kind-1');assert.deepEqual(external().profiles,['Compare 文','Alternate 🚀']);assert.equal(external().selected_profile,1);assert.equal(external().default_profile,1);assert.equal(external().arguments,'diff\n{left}\n{right}');passed('Multiple Unicode profile names, configuration and default survive native window restart');
 await reveal('tools-profile-remove');await click('tools-profile-remove');await click('tools-profile-remove-cancel');assert.equal(external().profiles.length,2);await click('tools-profile-remove');await click('tools-profile-remove-confirm');assert.equal(external().profiles.length,1);assert.equal(external().default_profile,0);await saveProfiles();assert(fs.existsSync(helper));assert.deepEqual(snapshot(f),original);passed('Removal requires confirmation and save; deleting the default selects a remaining profile without deleting installed files or changing repositories');

 await doneProfiles();const m=conflict('PRIVATE_MERGE_PROFILE');await mergePreview(m);await profileName('Resolve 文');await saveProfiles();
 await reveal('tools-profile-duplicate');await click('tools-profile-duplicate');await profileName('Wait then cancel');await reveal('tools-arguments');await type('tools-arguments','wait\n{base}\n{ours}\n{theirs}\n{result}');await saveProfiles();
 await doneProfiles();const unchanged=snapshot(m);await reveal('tools-start');await click('tools-start');await until(()=>external().running&&fs.existsSync(marker),'Owned merge process running');const pids=fs.readFileSync(marker,'utf8').trim().split(/\s+/).map(Number);await reveal('tools-cancel');const started=performance.now();await click('tools-cancel');await until(()=>!external().running,'Merge profile cancellation');report.cancel_ms=Math.round(performance.now()-started);assert.equal(external().success,false);assert.match(external().error,/cancel/);for(const pid of pids)assert.throws(()=>process.kill(pid,0));assert.deepEqual(snapshot(m),unchanged);passed('A selected merge profile cancels its owned process tree and leaves originals/index intact');
 await close();await click('conflict-external');assert.equal(external().selected_profile,0);await launch();assert.equal(external().success,true);assert.deepEqual(snapshot(m),unchanged);await importDraft();assert.equal(active().conflict_editor.draft,'resolved 世界 🚀 é\n');assert.deepEqual(snapshot(m),unchanged);await click('conflict-stage');await click('dialog-cancel');assert.deepEqual(snapshot(m),unchanged);passed('Default merge profile imports only into a review draft; cancelling publication preserves original file and index');
 await click('conflict-external');await launch();m.write(NAME,'outside edit must survive\n');await importDraft();assert.match(external().error,/changed outside/);assert.equal(fs.readFileSync(path.join(m.repo,NAME),'utf8'),'outside edit must survive\n');assert.equal(snapshot(m).index,unchanged.index);passed('Selected-profile merge import still refuses external edits with a clear error');
 await close();await editProfiles();await reveal('tools-executable');await type('tools-executable','C:\\Windows\\System32\\notepad.exe');await saveProfiles();await doneProfiles();await click('conflict-external');await reveal('tools-start');await assert.rejects(()=>client.tool('rocket_click',{target:'tools-start',expected_tab:state.active_tab}),/fixture root/);passed('Every selected profile remains subject to localhost MCP executable-scope checks');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const secret of ['PRIVATE_',NAME,'Compare 文','Wait then cancel','outside edit must survive',helper])assert(!payload.includes(secret),'Diagnostic payload leak');
 for(const op of ['external-settings','external-diff','external-merge','external-import','external-cancel','diff-search','diff-words'])assert(records.some(r=>r.operation===op&&r.event==='request_end'),'Missing log '+op);
 assert(records.some(r=>r.failure_code==='external_tool_exit'&&r.exit_code===9));assert(records.some(r=>r.outcome==='cancelled'));assert(records.some(r=>r.failure_code==='conflict_file_changed'));report.log_records=records.length;passed('Affected logs identify settings, execution, cancellation, search and external-edit failure without private profile/path/content payloads or dropped records');report.passed=true;
 }catch(error){report.failure=error.stack;if(client)try{await capture('failure')}catch{}throw error;}
 finally{fs.writeFileSync('work/tool-profiles-syntax-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(150);if(service.exitCode===null)service.kill()}if(client)await client.finish()}
}
run().catch(error=>{console.error(error);process.exitCode=1});
