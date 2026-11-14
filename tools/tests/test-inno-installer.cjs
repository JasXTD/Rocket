'use strict';
// Real Inno lifecycle with a distinct AppId, Unicode temp folders and synthetic state.
// Pass the result.json from Package-Installer.ps1 and the already audited ISCC.exe.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const f=require('./app-update-fixtures.cjs');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
assert.equal(process.platform,'win32','Native Windows acceptance only');
const [buildFile,compiler]=process.argv.slice(2);assert(buildFile&&compiler,'Supply build result.json and ISCC.exe');
const build=JSON.parse(fs.readFileSync(buildFile)),id=crypto.randomBytes(6).toString('hex');
assert.equal(f.sha(fs.readFileSync(compiler)),build.compilerSha256);
assert.equal(f.sha(fs.readFileSync(build.installer)),build.sha256);
assert.equal(f.sha(fs.readFileSync('tools/windows/rocket.iss')),build.installerScriptSha256);
assert(!build.testIdentity,'Acceptance derives its separate registration from a production package');
const root=path.resolve('work/inno-'+id),install=path.join(root,'Rocket 中文'),app=path.join(install,'app');
const profile=path.join(root,'profile'),repo=path.join(root,'資料 repository');
const report={schema:1,passed:false,root,sourceSetup:build.sha256,cases:[]};
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(profile);fs.mkdirSync(path.join(root,'empty-template'));
fs.writeFileSync(path.join(root,'global'),'');fs.writeFileSync(path.join(root,'system'),'');
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};
for(const key of Object.keys(env))if(/^(GIT_|SSH_|GH_TOKEN$|GITHUB_TOKEN$)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'global'),GIT_CONFIG_SYSTEM:path.join(root,'system'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_TERMINAL_PROMPT:'0',LOCALAPPDATA:path.join(root,'local'),APPDATA:path.join(root,'roaming')});
for(const dir of [env.LOCALAPPDATA,env.APPDATA])fs.mkdirSync(dir);
const pwsh='C:/Program Files/PowerShell/7/pwsh.exe',children=new Set();
const run=(exe,args,options={})=>cp.execFileSync(exe,args,{env,windowsHide:true,encoding:'utf8',maxBuffer:4*1024*1024,...options});
// A child's console code page can differ from Node's. Keep native query results
// ASCII JSON on the pipe, including Unicode shortcut targets and user paths.
const psValue=code=>JSON.parse(run(pwsh,['-NoProfile','-Command',`& { ${code} } | ConvertTo-Json -Compress -EscapeHandling EscapeNonAscii`]));
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const pass=name=>{report.cases.push(name);console.log('PASS '+name)};
async function until(fn,label,ms=60000){const end=performance.now()+ms;while(!await fn()){assert(performance.now()<end,label+' timeout');await delay(80)}}
function spawn(exe,args){const p=cp.spawn(exe,args,{env,windowsHide:true,stdio:['pipe','pipe','pipe']});p.stdout.resume();p.stderr.resume();children.add(p);return p}
async function finish(p){await until(()=>p.exitCode!==null,'process exit',180000);children.delete(p);return p.exitCode}
async function setup(exe,dir,name){const p=spawn(exe,['/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/DIR='+dir,'/LOG='+path.join(root,name+'.log')]);return finish(p)}
function quote(s){return '"'+s.replaceAll('"','""').replaceAll('{','{{')+'"'}
function compile(payload,version,name,manifestHash){
  const inv=f.inventory(payload),include=path.join(root,name+'.iss');
  fs.writeFileSync(include,'\uFEFF'+Object.keys(inv).map(n=>{
    const dir=path.posix.dirname(n),dest='{code:GetStageDir}'+(dir==='.'?'':'\\'+dir.replaceAll('/','\\'));
    return 'Source: '+quote(path.join(payload,n))+'; DestDir: "'+dest+'"; Flags: ignoreversion uninsneveruninstall';
  }).join('\n'));
  const args=['/DRocketVersion='+version,'/DPayloadInclude='+include,'/DPayloadManifestHash='+(manifestHash??f.sha(fs.readFileSync(path.join(payload,'SHA256SUMS.txt')))),'/DOutputDirectory='+root,'/DOutputBase='+name,'/DProductId=Rocket.InstallerTest.'+id,'/DProductName=Rocket Installer Test '+id,path.resolve('tools/windows/rocket.iss')];
  fs.writeFileSync(path.join(root,name+'-compile.log'),run(compiler,args));return path.join(root,name+'.exe');
}
const gitExe=path.join(build.payload,'git/2.56.0.windows.4/cmd/git.exe');
const git=(...args)=>run(gitExe,['-C',repo,...args]).trimEnd();
git('init','-b','rocket-ui-test/installer');git('config','user.name','Installer fixture');git('config','user.email','fixture@example.invalid');git('config','commit.gpgsign','false');
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),'rocket-ui-test/installer');
fs.writeFileSync(path.join(repo,'日本語.md'),'original\n');git('add','.');git('commit','-m','Installer fixture');
fs.appendFileSync(path.join(repo,'日本語.md'),'local unstaged change\n');fs.writeFileSync(path.join(repo,'untracked.md'),'preserve');
const settings=path.join(profile,'user-settings-sentinel.json');fs.writeFileSync(settings,'{"theme":"clay","draft":"keep this"}\n');
const keyDir=path.join(root,'synthetic-ssh');fs.mkdirSync(keyDir);fs.writeFileSync(path.join(keyDir,'config'),'# disposable fixture; preserve\n');
const protectedState=()=>({repo:f.inventory(repo),settings:f.sha(fs.readFileSync(settings)),keys:f.inventory(keyDir),head:git('rev-parse','HEAD'),status:git('status','--porcelain=v2','-z')});
const before=protectedState();
const registration=()=>psValue(`Test-Path -LiteralPath 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Rocket.InstallerTest.${id}_is1'`);
const shortcut=path.join(psValue("[Environment]::GetFolderPath('Programs')"),'Rocket Installer Test '+id+'.lnk');
const shortcutTarget=()=>psValue(`$s=New-Object -ComObject Shell.Application; $s.NameSpace('${path.dirname(shortcut).replaceAll("'","''")}').ParseName('${path.basename(shortcut)}').GetLink.Path`);
let client,service;
async function closeUi(){
  if(client&&service?.exitCode===null){
    await client.tool('rocket_close');
    await until(async()=>{const s=await client.tool('rocket_state');if(s.dialog?.kind==='quit')await client.tool('rocket_click',{target:'dialog-submit',expected_tab:s.active_tab});return ['closed','not_started'].includes(s.window)},'test window close');
    await client.raw('POST','/shutdown');await finish(service);
  }
  if(client)await client.finish();client=null;
  const token=path.join(profile,'access-token');if(fs.existsSync(token))fs.unlinkSync(token);
}
async function nativeSmoke(){
  const exe=path.join(app,'rocket-mcp.exe');run(exe,['--init-http',profile,root]);
  const portProbe=net.createServer();await new Promise(r=>portProbe.listen(0,'127.0.0.1',r));const port=portProbe.address().port;await new Promise(r=>portProbe.close(r));
  fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
  service=spawn(exe,['--http-profile',profile]);client=new RocketHttpClient(profile);
  await until(async()=>{try{return (await client.raw('GET','/health')).status===200}catch{return false}},'installed MCP starts');
  assert.equal((await client.initialize()).serverInfo.version,build.version);
  await client.tool('rocket_open',{path:repo});
  await until(async()=>{const s=await client.tool('rocket_state');return s.window==='running'&&!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading)},'installed UI opens repo');
  const s=await client.tool('rocket_state');assert(s.tabs.length>0);
  await closeUi();pass('Installed helper opens a dirty Unicode repo through localhost MCP; closes cleanly');
}
async function update(mode){
  const stage=path.join(install,'.rocket-next-'+mode),backup=path.join(install,'.rocket-previous-update-'+mode),lock=path.join(install,'.app.rocket-update-lock'),job=path.join(root,'update-'+mode);
  fs.mkdirSync(job);f.writeBundle(stage,f.bundle('9.0.0',f.probe(root,'9.0.0')));
  const old=f.inventory(app),incoming=f.inventory(stage),parent=spawn(process.execPath,['-e','process.stdin.once("data",()=>process.exit(0));process.stdin.resume()']);
  fs.writeFileSync(lock,job);fs.writeFileSync(path.join(job,'install.json'),JSON.stringify({schema:1,target:app,stage,backup,lock,old,new:incoming,parent:parent.pid,version:'9.0.0',repositories:[]}));
  // The production updater copies itself outside app before replacing app.
  const helper=spawn(path.resolve('target/release/rocket.exe'),['--rocket-apply-update',path.join(job,'install.json')]);
  await until(()=>fs.existsSync(path.join(job,'ready'))||helper.exitCode!==null,'updater ready');assert.equal(helper.exitCode,null);
  if(mode==='cancel')fs.writeFileSync(path.join(job,'cancel'),'cancel');
  else fs.writeFileSync(path.join(job,'approved.json'),JSON.stringify({install:true}));
  if(mode!=='cancel'){parent.stdin.end('done\n');await finish(parent)}
  const code=await finish(helper),receipt=JSON.parse(fs.readFileSync(path.join(job,'result.json')));
  if(mode==='cancel'){assert.notEqual(code,0);assert.equal(receipt.code,'cancelled');assert.deepEqual(f.inventory(app),old);parent.stdin.end('done\n');await finish(parent)}
  else {assert.equal(code,0);assert.equal(receipt.code,'installed');assert.deepEqual(f.inventory(app),incoming);assert.deepEqual(f.inventory(backup),old);await until(()=>fs.existsSync(path.join(job,'restarted')),'updated app restarted')}
  assert(!fs.existsSync(lock));assert(fs.existsSync(path.join(install,'uninstall/unins000.exe')));
  pass(mode==='cancel'?'Cancelling an in-app update preserves installed files and uninstall metadata':'In-app version update swaps only app, retains backup and restarts');
}
async function uninstall(name){return setup(path.join(install,'uninstall/unins000.exe'),install,name)}

(async()=>{
  try {
    const oldPayload=path.join(root,'old-payload');f.writeBundle(oldPayload,f.bundle('0.88.0',f.probe(root,'0.88.0')));
    const oldSetup=compile(oldPayload,'0.88.0','old-test-setup'),newSetup=compile(build.payload,build.version,'new-test-setup');
    report.testSetupSha256=f.sha(fs.readFileSync(newSetup));assert(!registration());
    const occupied=path.join(root,'occupied');fs.mkdirSync(occupied);fs.writeFileSync(path.join(occupied,'keep.md'),'do not overwrite');
    assert.notEqual(await setup(newSetup,occupied,'occupied'),0);assert.deepEqual(fs.readdirSync(occupied),['keep.md']);pass('Setup refuses an unrelated nonempty folder');
    const cancelled=path.join(root,'cancelled');const wizard=spawn(oldSetup,['/SUPPRESSMSGBOXES','/SP-','/NOICONS','/DIR='+cancelled,'/LOG='+path.join(root,'cancel-wizard.log')]);
    run(pwsh,['-NoProfile','-File',path.resolve('tools/windows/Cancel-TestSetup.ps1'),'-SetupProcessId',String(wizard.pid)]);
    assert.notEqual(await finish(wizard),0);assert(!fs.existsSync(path.join(cancelled,'app')));assert(!registration());pass('Normal wizard cancellation creates no installed app or registration');
    assert.equal(await setup(oldSetup,install,'fresh-install'),0);assert(registration());assert(fs.existsSync(shortcut));assert.equal(shortcutTarget(),path.join(app,'rocket.exe'));assert.deepEqual(f.inventory(app),f.inventory(oldPayload));pass('Fresh per-user installation in a Unicode folder with its own Start menu shortcut');
    const invalidSetup=compile(oldPayload,'0.88.0','invalid-manifest-setup','0'.repeat(64));
    assert.notEqual(await setup(invalidSetup,install,'invalid-manifest'),0);assert.deepEqual(f.inventory(app),f.inventory(oldPayload));
    assert(fs.readFileSync(path.join(root,'invalid-manifest.log'),'utf8').includes('Rocket installation failed: Prepared installation changed.'));
    pass('Post-install manifest failure returns nonzero, explains the error and preserves the previous app');
    fs.writeFileSync(path.join(app,'personal.md'),'preserve');assert.notEqual(await setup(newSetup,install,'extra-file'),0);assert.equal(fs.readFileSync(path.join(app,'personal.md'),'utf8'),'preserve');
    assert.notEqual(await uninstall('extra-uninstall'),0);assert(fs.existsSync(path.join(app,'rocket.exe')));fs.unlinkSync(path.join(app,'personal.md'));pass('Upgrade and uninstall refuse extra personal files without deleting them');
    const original=fs.readFileSync(path.join(app,'資料/README.md'));fs.appendFileSync(path.join(app,'資料/README.md'),'external edit');
    assert.notEqual(await setup(newSetup,install,'changed-file'),0);assert.notEqual(await uninstall('changed-uninstall'),0);assert(fs.readFileSync(path.join(app,'資料/README.md'),'utf8').endsWith('external edit'));fs.writeFileSync(path.join(app,'資料/README.md'),original);pass('Changed installed files stop both upgrade and uninstall');
    const outside=path.join(root,'outside');fs.mkdirSync(outside);fs.writeFileSync(path.join(outside,'keep.md'),'preserve');fs.symlinkSync(outside,path.join(app,'linked'),'junction');
    assert.notEqual(await uninstall('linked-uninstall'),0);assert.equal(fs.readFileSync(path.join(outside,'keep.md'),'utf8'),'preserve');fs.unlinkSync(path.join(app,'linked'));pass('Linked directories cannot redirect uninstall into other files');
    const holder=spawn(path.join(root,'probe-0.88.0.exe'),['--hold-directory',app]);let held=false;holder.stdout.on('data',b=>{if(b.toString().includes('ready'))held=true});await until(()=>held,'test directory lock');
    assert.notEqual(await setup(newSetup,install,'locked-upgrade'),0);assert.deepEqual(f.inventory(app),f.inventory(oldPayload));holder.stdin.end('done\n');await finish(holder);pass('A locked installation refuses upgrade and preserves the old app');
    assert.equal(await setup(newSetup,install,'upgrade'),0);assert.deepEqual(f.inventory(app),f.inventory(build.payload));
    const previous=fs.readdirSync(install).filter(n=>n.startsWith('.rocket-setup-previous-'));assert.equal(previous.length,1);assert.deepEqual(f.inventory(path.join(install,previous[0])),f.inventory(oldPayload));
    const cli=JSON.parse(run(path.join(app,'rocket-cli.exe'),['--json','--version']));assert.equal(cli.version,build.version);pass('Setup upgrades the earlier version to the exact verified portable payload and keeps its backup');
    await nativeSmoke();await update('cancel');await update('install');
    const backups=Object.fromEntries(fs.readdirSync(install).filter(n=>n.startsWith('.rocket-previous-')||n.startsWith('.rocket-setup-previous-')).map(n=>[n,f.inventory(path.join(install,n))]));
    assert.equal(await uninstall('uninstall-after-update'),0);assert(!fs.existsSync(app));assert(!registration());assert(!fs.existsSync(shortcut));
    for(const [n,inv]of Object.entries(backups))assert.deepEqual(f.inventory(path.join(install,n)),inv);
    assert.deepEqual(protectedState(),before);pass('Uninstall after an in-app update preserves settings, dirty repo, synthetic SSH config and recovery backups');
    report.passed=true;
  }catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1}
  finally {
    try{await closeUi()}catch(e){report.cleanupError=e.message;report.passed=false;process.exitCode=1}
    for(const p of children)if(p.exitCode===null){report.ownedProcessLeft=p.pid;report.passed=false;process.exitCode=1}
    report.testRegistrationRemaining=registration();if(report.testRegistrationRemaining){report.passed=false;process.exitCode=1}
    fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log('Evidence: '+root);
  }
})();
