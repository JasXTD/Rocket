'use strict';
// Actual signed MSIX deployment + package activation. No user's Rocket profile or login.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const assert=require('node:assert/strict'),net=require('node:net'),http=require('node:http');
const f=require('./app-update-fixtures.cjs');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {readDraftPayload}=require('./state-fixture.cjs');
const {installedTools}=require('./msix-installed-tools.cjs');
assert.equal(process.platform,'win32');
const [firstRecord,secondRecord,signingRecord]=process.argv.slice(2);
assert(firstRecord&&secondRecord&&signingRecord,'Supply two MSIX build records and the disposable signing record');
const first=JSON.parse(fs.readFileSync(firstRecord)),second=JSON.parse(fs.readFileSync(secondRecord));
assert(first.compatibilityTest&&second.compatibilityTest);assert.equal(first.identity,second.identity);
assert.match(first.identity,/^Rocket\.CompatibilityTest\.[a-f0-9]{12}$/);
assert.notEqual(first.packageVersion,second.packageVersion);
const id=first.identity.split('.').at(-1),pwsh='C:/Program Files/PowerShell/7/pwsh.exe';
const runId=id+'-'+require('node:crypto').randomBytes(4).toString('hex');
const root=path.join(process.env.LOCALAPPDATA,'Rocket-MSIX-Tests',runId);
assert(!fs.existsSync(root),'Refusing to reuse test data');
const repo=path.join(root,'資料 repository'),profile=path.join(root,'profile'),ssh=path.join(root,'ssh-home');
const evidence=path.resolve('tools/maintenance/msix-20261117',runId);fs.mkdirSync(evidence,{recursive:true});
for(const dir of [repo,profile,ssh,path.join(root,'template')])fs.mkdirSync(dir,{recursive:true});
for(const n of ['global','system'])fs.writeFileSync(path.join(root,n),'');
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GH_TOKEN$|GITHUB_TOKEN$)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'global'),GIT_CONFIG_SYSTEM:path.join(root,'system'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TEMPLATE_DIR:path.join(root,'template'),GIT_TERMINAL_PROMPT:'0'});
const run=(exe,args,options={})=>cp.execFileSync(exe,args,{env,windowsHide:true,encoding:'utf8',timeout:180000,maxBuffer:4*1024*1024,...options});
const host=path.join(__dirname,'MSIX-TestHost.ps1');
function operation(action,record=firstRecord,extra=[]){return run(pwsh,['-NoProfile','-File',host,'-Action',action,'-Record',path.resolve(record),'-SigningRecord',path.resolve(signingRecord),...extra]).trim()}
function inspect(){const raw=operation('Inspect');return raw?JSON.parse(raw):[]}
function prepareRepository(gitExe){
  const git=(...args)=>run(gitExe,['-C',repo,...args]).trimEnd();
  git('init','-b','rocket-ui-test/msix');for(const[k,v]of[['user.name','MSIX fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false']])git('config',k,v);
  fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),'rocket-ui-test/msix');
  fs.writeFileSync(path.join(repo,'日本語.md'),'original\n');git('add','.');git('commit','-m','MSIX fixture');
  fs.appendFileSync(path.join(repo,'日本語.md'),'uncommitted local change\n');fs.writeFileSync(path.join(repo,'untracked.md'),'preserve');
}
fs.writeFileSync(path.join(ssh,'config'),'# disposable SSH configuration; preserve\n');
run(path.join(process.env.WINDIR,'System32/OpenSSH/ssh-keygen.exe'),['-q','-t','ed25519','-N','','-C','Rocket MSIX disposable fixture','-f',path.join(ssh,'id_ed25519')]);
// Hash only; never print, export, upload or modify the user's credentials.
function personal(){const result={};for(const dir of [path.join(process.env.USERPROFILE,'.ssh'),path.join(process.env.LOCALAPPDATA,'Rocket')]){
  if(!fs.existsSync(dir))continue;
  const visit=p=>{const st=fs.lstatSync(p);if(st.isSymbolicLink())return;if(st.isDirectory()){for(const n of fs.readdirSync(p))visit(path.join(p,n))}else if(st.isFile())result[p]=f.sha(fs.readFileSync(p))};visit(dir);
}return result}
const personalBefore=personal(),protectedBefore={keys:f.inventory(ssh)};
let client,state,servicePid,api,requests=0,owned=false;
const report={schema:1,passed:false,root,identity:first.identity,cases:[]};
const pass=s=>{report.cases.push(s);console.log('PASS '+s)},delay=ms=>new Promise(r=>setTimeout(r,ms));
async function wait(fn,label,ms=60000){const end=performance.now()+ms;while(!await fn()){assert(performance.now()<end,label+' timeout');await delay(80)}}
async function stateUntil(fn,label='UI state',checkErrors=true){await wait(async()=>{state=await client.tool('rocket_state');if(checkErrors){assert(!state.startup_error,state.startup_error);assert(!state.state_error,state.state_error)}return fn(state)},label)}
const control=id=>state.controls?.find(c=>c.id===id);
async function settle(){await stateUntil(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading));}
async function stable(){let prev='',n=0;await stateUntil(s=>{const text=JSON.stringify(s.controls);n=text===prev?n+1:0;prev=text;return n>=2})}
async function click(id){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});await settle()}
async function type(id,text){await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle()}
async function capture(name){const r=await client.tool('rocket_capture');const img=r.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(evidence,name+'.png'),Buffer.from(img.data,'base64'))}
function unchanged(){assert.deepEqual(f.inventory(repo),protectedBefore.repo);assert.deepEqual(f.inventory(ssh),protectedBefore.keys);assert.deepEqual(personal(),personalBefore)}
async function start(packageVersion=first.packageVersion){
  servicePid=Number(operation('Activate',firstRecord,['-Profile',profile]));assert(servicePid>0);
  client=new RocketHttpClient(profile);await wait(async()=>{try{return(await client.raw('GET','/health')).status===200}catch{return false}},'Activated MCP host');
  assert.equal((await client.initialize()).serverInfo.version,first.version);
  await client.tool('rocket_open',{path:repo});await settle();assert.equal(state.window,'running');
  assert.equal(state.updates.status,'windows_managed','Native process did not have MSIX identity');assert.equal(state.updates.enabled,false);
  assert(state.git_settings.active_executable.toLowerCase().includes('windowsapps'),'Git was not loaded from the installed MSIX');
  assert(state.git_settings.active_executable.includes('_'+packageVersion+'_'),'Activation did not use the expected installed package version');
}
async function close(){
  if(client){
    await client.tool('rocket_close');await stateUntil(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit','Window close',false);
    if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await stateUntil(s=>['closed','not_started'].includes(s.window),'Window close',false)}
    await client.raw('POST','/shutdown');await client.finish();client=null;
  }
  if(servicePid)await wait(()=>{try{process.kill(servicePid,0);return false}catch{return true}},'MCP process exit',20000);
  servicePid=null;
}
(async()=>{
 try{
  assert.equal([inspect()].flat().filter(Boolean).length,0,'Test identity already installed');
  fs.writeFileSync(path.join(evidence,'unsigned-rejection.log'),operation('ExpectRejection',firstRecord,['-Candidate',path.resolve(first.package)]));
  assert.equal([inspect()].flat().filter(Boolean).length,0);pass('Windows refuses the unsigned package without bypassing signature enforcement');
  api=http.createServer((q,r)=>{requests++;r.writeHead(500);r.end('Unexpected updater request')});await new Promise(r=>api.listen(0,'127.0.0.1',r));
  fs.writeFileSync(path.join(root,'rocket-update-api'),'http://127.0.0.1:'+api.address().port+'\n');
  operation('Install');owned=true;
  const tools=installedTools(inspect(),first);
  prepareRepository(tools.git);
  protectedBefore.repo=f.inventory(repo);
  run(tools.mcp,['--init-http',profile,root]);
  const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));
  fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
  await start();pass('Signed package installs and activates its MCP host with real Windows package identity and bundled Git');
  await click('menu-help');await click('help-updates');await stateUntil(s=>s.updates.open);assert(control('update-store')?.enabled);assert(!control('update-download'));assert(!control('update-install'));
  await click('update-store');assert(state.github.opened_urls.includes('ms-windows-store://downloadsandupdates'));
  await client.tool('rocket_resize',{width:780,height:620});await stable();for(const id of ['update-store','update-close','update-later']){const r=control(id);assert(r?.enabled);assert(r.rect.every(Number.isFinite));const [w,h]=state.viewport;assert(r.rect[0]>=0&&r.rect[1]>=0&&r.rect[2]<=w&&r.rect[3]<=h,'Control outside compact window: '+id);}
  await capture('managed-updates-compact');await click('update-later');assert.equal(requests,0);pass('Launch and Help use Windows-managed updates; no GitHub request or portable installation control');
  await client.tool('rocket_resize',{width:1280,height:800});await settle();await click('view-changes');await type('commit-message','MSIX saved draft 日本語');
  await click('menu-edit');await click('preferences');await click('pref-page-appearance');await click('rows-compact');await click('dialog-submit');await settle();assert.equal(state.preferences.row_height,29);
  await click('menu-view');await click('view-goto');await click('goto-integrations');
  if(!state.integrations.playground.open){if(!control('int-open-playground'))await click('int-developer');await click('int-open-playground')}
  await stateUntil(s=>s.integrations.playground.loaded&&!s.integrations.playground.busy);
  await type('int-play-source',"return function() return ui.text('MSIX_LUA_OK') end");await click('int-play-run');
  await stateUntil(s=>!s.integrations.playground.busy&&(s.integrations.playground.has_preview||s.integrations.playground.error));
  assert(!state.integrations.playground.error,state.integrations.playground.error);assert(state.integrations.playground.text.labels.includes('MSIX_LUA_OK'));
  pass('Packaged Lua helper runs inside the existing AppContainer sandbox');
  await click('view-changes');await click('menu-terminal');await click('terminal-new');await stateUntil(s=>s.terminal.running&&s.terminal.tail.includes('PS '));const shellPid=state.terminal.pid;
  await type('terminal-command',"Write-Output ('ROCKET_' + 'TERMINAL_OK')");await click('terminal-send');await stateUntil(s=>s.terminal.tail.includes('ROCKET_TERMINAL_OK'));assert.equal(state.terminal.pid,shellPid);
  await capture('packaged-terminal');await close();unchanged();
  assert(readDraftPayload(path.join(root,'rocket-ui-state')).includes(Buffer.from('MSIX saved draft 日本語')));
  pass('Terminal helper works and real drafts/preferences persist outside package virtualization');
  const saved=f.inventory(path.join(root,'rocket-ui-state'));
  const signed=path.join(path.dirname(secondRecord),'compatibility-signed.msix'),bad=path.join(evidence,'corrupted.msix');
  // Target the signature rather than an arbitrary payload block, which Windows
  // may reconstruct/reuse from the already installed version during an update.
  fs.copyFileSync(signed,bad,fs.constants.COPYFILE_EXCL);
  run(pwsh,['-NoProfile','-Command',[
    "$ErrorActionPreference='Stop'",
    "$zip=[IO.Compression.ZipFile]::Open('"+bad.replaceAll("'","''")+"',[IO.Compression.ZipArchiveMode]::Update)",
    "try { $entry=$zip.GetEntry('AppxSignature.p7x'); if(-not $entry -or $entry.Length -lt 16 -or $entry.Length -gt 1048576){throw 'Unexpected test signature'}; $stream=$entry.Open(); try { [void]$stream.Seek(-1,[IO.SeekOrigin]::End); $last=$stream.ReadByte(); [void]$stream.Seek(-1,[IO.SeekOrigin]::End); $stream.WriteByte($last -bxor 1) } finally {$stream.Dispose()} } finally {$zip.Dispose()}"
  ].join('\n')]);
  fs.writeFileSync(path.join(evidence,'corrupted-rejection.log'),operation('ExpectRejection',secondRecord,['-Candidate',bad]));assert.equal([inspect()].flat()[0].Version,first.packageVersion);unchanged();
  pass('Corrupted upgrade is refused and the installed version and local data remain intact');
  operation('Install',secondRecord);assert.equal([inspect()].flat()[0].Version,second.packageVersion);assert.deepEqual(f.inventory(path.join(root,'rocket-ui-state')),saved);
  await start(second.packageVersion);assert.equal(state.preferences.row_height,29);assert(state.tabs[0].draft_present);await close();unchanged();pass('MSIX upgrade launches the new package version and preserves draft, preferences, dirty Unicode repository and disposable SSH key');
  const afterUpgrade=f.inventory(path.join(root,'rocket-ui-state'));operation('Uninstall');owned=false;
  assert.equal([inspect()].flat().filter(Boolean).length,0);assert.deepEqual(f.inventory(path.join(root,'rocket-ui-state')),afterUpgrade);unchanged();
  pass('Uninstall removes only the package; AppData settings/draft, repositories, SSH files and existing user data survive');
  report.passed=true;
 }catch(e){report.error=e.message;if(state?.startup_error)report.startupError=state.startup_error;if(state?.integrations?.playground?.error)report.luaError=state.integrations.playground.error;console.error(e);console.error('Evidence: '+evidence);process.exitCode=1}
 finally{
  try{await close()}catch(e){
    report.cleanupError=e.message;process.exitCode=1;
    if(client){try{await client.raw('POST','/shutdown');await client.finish();client=null}catch(error){report.shutdownError=error.message}}
    if(servicePid){try{await wait(()=>{try{process.kill(servicePid,0);return false}catch{return true}},'Failed host exit',20000);servicePid=null}catch(error){report.exitError=error.message}}
  }
  if(owned&&!servicePid){try{operation('Uninstall');owned=false}catch(e){report.uninstallError=e.message;process.exitCode=1}}
  if(api)await new Promise(r=>api.close(r));
  if(fs.existsSync(path.join(profile,'access-token')))fs.unlinkSync(path.join(profile,'access-token'));
  if(!servicePid){
    for(const name of ['id_ed25519','id_ed25519.pub']){
      const file=path.join(ssh,name);
      if(f.sha(fs.readFileSync(file))===protectedBefore.keys[name])fs.unlinkSync(file);
      else {report.keyCleanupError='Disposable key changed; preserved for inspection';process.exitCode=1}
    }
  }
  report.packageStillInstalled=owned;report.updateRequests=requests;fs.writeFileSync(path.join(evidence,'report.json'),JSON.stringify(report,null,2));
 }
})();
