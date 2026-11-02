// Focused native acceptance. All writable repos, profiles and runtime damage are disposable.
'use strict';
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const version='2.56.0.windows.4',branch='rocket-ui-test/managed-git';
const root=path.resolve('tools/maintenance/managed-git-20261030/resumed/runs/native-'+crypto.randomUUID());
const profile=path.join(root,'http-profile'),repo=path.join(root,'研究 repo'),storage=path.join(root,'rocket-ui-state');
let bundle=path.join(root,'Rocket portable 文'),service,client,state,port;
const selection=path.join(storage,'git-selection'),result={passed:false,root,cases:[],captures:[]};
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const normal=p=>path.resolve(p.replace(/^\\\\\?\\/,'' )).toLowerCase();
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const systemGit=cp.execFileSync('where.exe',['git.exe'],{windowsHide:true,encoding:'utf8'}).trim().split(/\r?\n/)[0];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(storage);fs.mkdirSync(bundle);
for(const folder of ['home','home/AppData/Local','home/AppData/Roaming','empty-template','tmp'])fs.mkdirSync(path.join(root,folder),{recursive:true});
fs.writeFileSync(path.join(root,'empty.config'),'');fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)||key.toLowerCase()==='path')delete env[key];
Object.assign(env,{HOME:path.join(root,'home'),USERPROFILE:path.join(root,'home'),APPDATA:path.join(root,'home/AppData/Roaming'),LOCALAPPDATA:path.join(root,'home/AppData/Local'),TEMP:path.join(root,'tmp'),TMP:path.join(root,'tmp'),
 GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GCM_INTERACTIVE:'never',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
// The packaged app/CLI must work without Git on PATH. Explicit system selection is tested too.
const installedRoot=path.basename(path.dirname(systemGit)).toLowerCase()==='cmd'?path.dirname(path.dirname(systemGit)).toLowerCase()+path.sep:null;
const portablePath=process.env.PATH.split(path.delimiter).filter(p=>path.isAbsolute(p)&&!(installedRoot&&(path.resolve(p).toLowerCase()+path.sep).startsWith(installedRoot))&&!['git.exe','git-lfs.exe','git-credential-manager.exe'].some(name=>fs.existsSync(path.join(p,name)))).join(path.delimiter);
env.PATH=path.dirname(systemGit)+path.delimiter+portablePath;
const run=(exe,args,extra={})=>cp.execFileSync(exe,args,{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe'],timeout:90000,...extra}).trimEnd();
const git=args=>run(systemGit,['-C',repo,...args]);
git(['init','--initial-branch='+branch]);
for(const[k,v]of[['user.name','Rocket Runtime Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',path.join(repo,'.git/hooks')]])git(['config',k,v]);
const file=path.join(repo,'資料 文.txt');fs.writeFileSync(file,'base\n');git(['add','.']);git(['commit','-m','Fixture base']);
fs.writeFileSync(file,'base\nPRIVATE_RUNTIME_SENTINEL changed\n');fs.writeFileSync(path.join(repo,'keep.txt'),'untracked stays intact\n');
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
fs.mkdirSync(path.join(repo,'.git/hooks'),{recursive:true});
fs.writeFileSync(path.join(repo,'.git/hooks/pre-commit'),"#!/bin/sh\nprintf 'portable shell hook ran\\n' > .git/runtime-hook-ran\n");
const fingerprint=()=>['.git/HEAD','.git/index','.git/config','資料 文.txt','keep.txt'].map(p=>hash(path.join(repo,p)));
const before=fingerprint();fs.writeFileSync(selection,'ROCKET-GIT-1\n'+systemGit);
const ctl=id=>state?.controls?.find(c=>c.id===id),g=()=>({...state.git_settings,managed_available:state.git_settings.bundled?.available,managed_verified:state.git_settings.bundled?.verified});
const pass=name=>{result.cases.push(name);console.log('PASS '+name)};
function within(p){const canonicalRoot=fs.realpathSync(root)+path.sep;const parent=fs.realpathSync(path.dirname(p))+path.sep;assert(parent.toLowerCase().startsWith(canonicalRoot.toLowerCase()),'Move outside fixture');assert(!fs.lstatSync(path.dirname(p)).isSymbolicLink());}
function move(from,to){within(from);within(to);assert(!fs.existsSync(to));fs.renameSync(from,to);}
async function until(fn,label='state',timeout=60000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-start<timeout,label+' timed out '+JSON.stringify({window:state.window,busy:state.busy,settings:state.git_settings,startup:state.startup_error}));await pause(40);}}
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.git_settings?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let prior='',count=0;await until(s=>{const next=JSON.stringify(s.controls);count=next===prior?count+1:0;prior=next;return count>=2&&!s.input_pending},'stable controls');}
async function click(id,wait=true){await stable();assert(ctl(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(wait)await settled();}
async function type(id,text){await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settled();}
async function scroll(direction,pixels=900){await client.tool('rocket_scroll',{target:'prefs-scroll',direction,pixels,expected_tab:state.active_tab});await settled();}
async function capture(name){await stable();const v=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!v.isError);const img=v.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));result.captures.push(name);}
async function prefs(){await click('menu-edit');await click('preferences');await click('pref-page-git');await settled();}
async function start(expectedStartupError=null){const exe=path.join(bundle,'rocket-mcp.exe');
 if(!fs.existsSync(profile)){run(exe,['--init-http',profile,root]);const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');result.port=port;}
 const fd=fs.openSync(path.join(root,'service.log'),'a');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>150)throw e;assert(service.exitCode===null,'MCP exited');await pause(100);}}
 await client.initialize();await client.tool('rocket_open',{path:repo});
 if(expectedStartupError){await until(s=>!!s.startup_error,'expected startup rejection');assert.match(state.startup_error,expectedStartupError);return;}
 await until(s=>{assert(!s.startup_error,s.startup_error);return s.tabs?.length>0&&s.git_settings?.active_executable&&!s.tabs.some(t=>t.loading);},'repository startup');
 await settled();await client.tool('rocket_resize',{width:1440,height:1000});await settled();await prefs();}
async function stop(){if(!client)return;await settled();if(state.dialog){await click('dialog-close');}await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit','close');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window),'closed');}
 await client.raw('POST','/shutdown');await client.finish();client=null;const deadline=Date.now()+10000;while(service.exitCode===null&&Date.now()<deadline)await pause(25);assert.equal(service.exitCode,0,'owned MCP service exit');service=null;}
async function busy(){await until(s=>s.git_settings.busy&&!s.input_pending
 &&s.controls.some(c=>c.id==='git-cancel'&&c.enabled)
 &&s.git_settings.bundled.progress[0]>0,'verification hashing with visible Stop');}
async function main(){try{
 const source=path.resolve(process.env.ROCKET_MANAGED_BUILD||'target/release');
 for(const name of ['rocket.exe','rocket-mcp.exe','rocket-cli.exe','rocket-lua-host.exe','rocket-rebase-editor.exe','rocket-terminal-host.exe'])fs.copyFileSync(path.join(source,name),path.join(bundle,name),fs.constants.COPYFILE_EXCL);
 fs.cpSync(path.join(source,'conpty'),path.join(bundle,'conpty'),{recursive:true,errorOnExist:true,force:false});
 console.log('Preparing one disposable runtime bundle...');
 run('C:/Program Files/PowerShell/7/pwsh.exe',['-NoProfile','-File',path.resolve('tools/Assemble-GitRuntime.ps1'),'-Destination',bundle],{env:{...env,PATH:process.env.PATH},timeout:180000});
 result.executable_sha256=hash(path.join(bundle,'rocket-mcp.exe'));result.driver_sha256=hash(__filename);
 const rejected=path.join(root,'not-git.cmd'),ran=path.join(root,'untrusted-program-ran');fs.writeFileSync(rejected,'@echo off\r\necho executed > "'+ran+'"\r\n');fs.writeFileSync(selection,'ROCKET-GIT-1\n'+rejected);
 await start(/Automation may use only bundled Git or the installed Git/);assert(!fs.existsSync(ran),'Untrusted program executed');await capture('untrusted-program-rejected');await stop();fs.writeFileSync(selection,'ROCKET-GIT-1\n'+systemGit);
 pass('MCP rejects an arbitrary saved program before execution');
 await start();assert.equal(normal(g().active_executable),normal(systemGit));assert.equal(g().error,null);assert(g().managed_available);await capture('installed-selection');
 pass('Existing explicit installed-Git selection is retained even when bundled Git is available');
 const runtime=path.join(bundle,'git',version),parked=path.join(bundle,'git','parked-runtime');
 await stop();move(runtime,parked);await start();assert(!g().managed_available);assert(!ctl('git-managed')?.enabled);await capture('missing-runtime');await stop();move(parked,runtime);await start();
 pass('Missing runtime is clearly unavailable and does not change the selected installed Git');
 const readme=path.join(runtime,'README.portable'),original=fs.readFileSync(readme);fs.appendFileSync(readme,'\nfixture alteration\n');
 try{await click('git-managed-verify');assert.match(g().error,/changed|verify/i);assert.equal(fs.readFileSync(selection,'utf8'),'ROCKET-GIT-1\n'+systemGit);await capture('changed-file');}finally{fs.writeFileSync(readme,original);}
 pass('Modified bundled file is rejected before execution and keeps the saved choice');
 await client.tool('rocket_resize',{width:780,height:620});await settled();
 await click('git-managed-verify',false);await busy();const startTime=performance.now();assert(ctl('git-cancel')?.enabled);
 await client.tool('rocket_click',{target:'git-cancel',expected_tab:state.active_tab});await until(s=>s.git_settings.stopping||!s.git_settings.busy,'stop feedback');result.cancel_feedback_ms=Math.round(performance.now()-startTime);
 await settled();result.cancel_completion_ms=Math.round(performance.now()-startTime);assert.match(g().error??g().notice??'',/cancel|stopp/i);assert.equal(fs.readFileSync(selection,'utf8'),'ROCKET-GIT-1\n'+systemGit);await capture('cancelled');
 pass('Visible Stop at compact size cancels a real full-inventory verification and preserves selection');
 await client.tool('rocket_resize',{width:1440,height:1000});await settled();
 await click('git-managed',false);await busy();fs.writeFileSync(selection,'ROCKET-GIT-1\n');await settled();assert.match(g().error,/changed externally/i);assert.equal(fs.readFileSync(selection,'utf8'),'ROCKET-GIT-1\n');await capture('external-selection-edit');await click('git-reload');
 pass('External selection changes during verification are not overwritten');
 await client.tool('rocket_resize',{width:780,height:620});await settled();await capture('compact');
 await client.tool('rocket_resize',{width:1440,height:1000});await settled();await scroll('up',2000);
 await click('git-managed');assert.equal(g().error,null);assert(g().managed&&g().managed_verified);assert.equal(normal(g().checked.executable),normal(path.join(runtime,'cmd/git.exe')));assert.equal(normal(g().active_executable),normal(systemGit));assert.equal(fs.readFileSync(selection,'utf8'),'ROCKET-GIT-2\nmanaged');assert.deepEqual(fingerprint(),before);await capture('verified-selection');
 pass('Verified bundled selection saves a relocatable choice and preserves repository data');
 await stop();env.PATH=portablePath;
 assert.notEqual(cp.spawnSync('where.exe',['git.exe'],{env,windowsHide:true}).status,0,'Fixture PATH must have no Git');
 assert.notEqual(cp.spawnSync('where.exe',['git-lfs.exe'],{env,windowsHide:true}).status,0,'Fixture PATH must have no LFS');
 await start();assert.equal(normal(g().active_executable),normal(path.join(bundle,'git',version,'cmd/git.exe')));assert(g().managed_verified&&g().managed);await capture('bundled-restarted');
 // A live client already holds a cached verified lease. Adding a file must block
 // the next helper operation, without requiring restart or manual Verify files.
 const added=path.join(bundle,'git',version,'cmd','rocket-security-extra.dll');
 try{
  fs.writeFileSync(added,'owned inert fixture',{flag:'wx'});
  await click('dialog-close');await click('menu-view');await click('view-goto');await click('goto-lfs');
  await until(s=>!s.dialog?.lfs?.running,'LFS refusal after runtime addition');
  assert.match(state.dialog?.error??'',/Runtime contents changed after verification/);
  assert.deepEqual(fingerprint(),before);await capture('added-runtime-file');await click('dialog-cancel');
 }finally{fs.rmSync(added,{force:true});}
 await prefs();
 pass('Existing bundled client rejects an added runtime DLL before its next helper launch and recovers after removal');
 await click('dialog-close');await click('menu-view');await click('view-goto');await click('goto-lfs');
 await until(s=>!s.dialog?.lfs?.running,'LFS status');assert.equal(state.dialog?.error,null);assert.equal(state.dialog?.lfs?.status?.version,'Git LFS 3.8.1');assert.deepEqual(fingerprint(),before);await capture('bundled-lfs');await click('dialog-cancel');
 pass('Native LFS status discovers the UCRT helper without system Git or LFS on child PATH');
 await prefs();
 await click('dialog-close');await click('view-changes');await capture('unicode-changes');await click('stage-all');await type('commit-message','Commit using bundled Git');await click('commit');
 assert.equal(git(['log','-1','--format=%s']),'Commit using bundled Git');assert(!fs.existsSync(path.join(repo,'.git/runtime-hook-ran')),'MCP hook isolation must remain enabled');assert.equal(fs.readFileSync(file,'utf8'),'base\nPRIVATE_RUNTIME_SENTINEL changed\n');
 pass('Native UI reads Unicode paths, stages and commits using bundled Git; MCP hook isolation remains enabled');
 await prefs();await click('git-automatic');await stop();await start();assert(g().automatic);assert.equal(normal(g().active_executable),normal(path.join(bundle,'git',version,'cmd/git.exe')));pass('Automatic uses bundled Git without a separate Git installation');
 await click('git-managed');await stop();const relocated=path.join(root,'Moved Rocket 空間');move(bundle,relocated);bundle=relocated;await start();assert.equal(normal(g().active_executable),normal(path.join(bundle,'git',version,'cmd/git.exe')));assert(g().managed);await capture('relocated');
 pass('Moving the complete app to a Unicode folder preserves bundled selection after restart');
 const binary=path.join(bundle,'git',version,'cmd/git.exe');result.helpers={git:run(binary,['--version']),lfs:run(binary,['lfs','version']),credentials:run(binary,['credential-manager','--version'])};assert.match(result.helpers.git,/2\.56\.0\.windows\.4/);assert.match(result.helpers.lfs,/git-lfs/);await stop();
 run(binary,['-C',repo,'commit','--allow-empty','-m','Disposable portable-shell compatibility check']);assert.equal(fs.readFileSync(path.join(repo,'.git/runtime-hook-ran'),'utf8'),'portable shell hook ran\n');
 pass('Independent bundled-Git hook compatibility check uses its portable shell without system Git on PATH');
 const cli=JSON.parse(run(path.join(bundle,'rocket-cli.exe'),['--repo',repo,'--json','status']));assert.equal(cli.ok,true);
 const cliLfs=JSON.parse(run(path.join(bundle,'rocket-cli.exe'),['--repo',repo,'--json','lfs','status']));assert.equal(cliLfs.ok,true);assert.equal(cliLfs.data.version,'Git LFS 3.8.1');
 pass('Packaged CLI status/LFS status and included credential-helper version probes work with isolated empty credentials');
 const logs=[];function collect(dir){if(!fs.existsSync(dir))return;for(const item of fs.readdirSync(dir,{withFileTypes:true})){const file=path.join(dir,item.name);if(item.isDirectory())collect(file);else if(item.name.endsWith('.jsonl'))logs.push(file);}}
 collect(profile);collect(storage);collect(path.join(root,'home'));
 for(const entry of fs.readdirSync(root,{withFileTypes:true}))if(entry.isDirectory()&&entry.name.startsWith('mcp-logs-'))collect(path.join(root,entry.name));
 const records=logs.flatMap(file=>fs.readFileSync(file,'utf8').split(/\r?\n/).filter(Boolean).map(line=>JSON.parse(line)));
 assert(records.length>0,'Diagnostic records must exist');assert(records.some(r=>r.operation==='managed-git-verify'),'Verification spans');assert(records.some(r=>r.outcome==='cancelled'),'Cancelled outcome');
 for(const record of records){assert.equal(record.dropped??0,0);assert(!JSON.stringify(record).includes('PRIVATE_RUNTIME_SENTINEL'));}
 result.diagnostic_records=records.length;result.logs=logs.map(p=>path.relative(root,p));pass('Diagnostics include verification and cancellation without repository payloads or dropped events');result.passed=true;
 }catch(error){result.error=error.stack;console.error(error.stack);process.exitCode=1;if(client)try{await capture('failure')}catch{}}
 finally{try{await stop()}catch(error){result.cleanup_error=error.message;process.exitCode=1;result.passed=false;}
 if(service&&service.exitCode===null){cp.spawnSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});result.passed=false;process.exitCode=1;}
 for(const name of ['access-token','service']){const p=path.join(profile,name);if(fs.existsSync(p)&&fs.statSync(p).isFile())fs.unlinkSync(p);}
 result.bundle=bundle;result.cleaned_access_token=!fs.existsSync(path.join(profile,'access-token'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(result,null,2));
 fs.writeFileSync(path.resolve('tools/maintenance/managed-git-20261030/resumed/native-latest.json'),JSON.stringify(result,null,2));console.log('Evidence: '+root);}}
main();
