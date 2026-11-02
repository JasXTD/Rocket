require('node:fs').mkdirSync(require('node:path').resolve('work/results/git-preferences-0.80'), {recursive:true});
// Only this task's disposable configuration/repositories are writable.
const fs=require('fs'),path=require('path'),cp=require('child_process'),crypto=require('crypto'),assert=require('assert/strict'),net=require('net');
const {RocketHttpClient}=require('../../../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/git-preferences-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'repo 文'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/preferences';
const exe=path.resolve('target/release/rocket-mcp.exe'),sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.mkdirSync(path.join(root,'ssh-home'));
const global=path.join(root,'empty.config');fs.writeFileSync(global,'# KEEP GLOBAL COMMENT\n[core]\n autocrlf = false\n');fs.writeFileSync(path.join(root,'system.config'),'');
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:global,GIT_CONFIG_SYSTEM:path.join(root,'system.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const ps=code=>cp.execFileSync('pwsh.exe',['-NoProfile','-Command',code],{env:{...env,ROCKET_GLOBAL_FIXTURE:global},windowsHide:true,encoding:'utf8'}).trim();
ps('$a=Get-Acl -LiteralPath $env:ROCKET_GLOBAL_FIXTURE; $a.SetAccessRuleProtection($true,$true); Set-Acl -LiteralPath $env:ROCKET_GLOBAL_FIXTURE -AclObject $a');
const acl=()=>ps('(Get-Acl -LiteralPath $env:ROCKET_GLOBAL_FIXTURE).Sddl'), originalAcl=acl();
const at=(where,args)=>cp.execFileSync('git',['-C',where,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd(),git=args=>at(repo,args);
function init(where){fs.mkdirSync(where,{recursive:true});at(where,['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Fixture Local'],['user.email','local@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])at(where,['config',k,v]);}
init(repo);fs.writeFileSync(path.join(repo,'資料 文.txt'),'base\n');git(['add','.']);git(['commit','-m','Fixture base']);const first=git(['rev-parse','HEAD']);
const sub=path.join(root,'source-sub');init(sub);fs.writeFileSync(path.join(sub,'a.txt'),'sub fixture\n');at(sub,['add','.']);at(sub,['commit','-m','Submodule']);
git(['-c','protocol.file.allow=always','submodule','add',sub,'modules/模組']);git(['commit','-am','Add submodule']);const head=git(['rev-parse','HEAD']);
git(['branch','rocket-ui-test/別の',first]);
fs.writeFileSync(path.join(repo,'資料 文.txt'),'staged PRIVATE_PREFS\n');git(['add','--','資料 文.txt']);fs.appendFileSync(path.join(repo,'資料 文.txt'),'unstaged\n');fs.writeFileSync(path.join(repo,'keep.txt'),'untracked\n');
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const fingerprint=()=>({head:git(['rev-parse','HEAD']),index:git(['ls-files','--stage','-z']),file:sha(path.join(repo,'資料 文.txt')),untracked:sha(path.join(repo,'keep.txt')),config:sha(path.join(repo,'.git/config')),modules:sha(path.join(repo,'.gitmodules'))});
const before=fingerprint(),report={passed:false,root,cases:[],exe_hash:sha(exe)};
let service,client,state;const delay=ms=>new Promise(r=>setTimeout(r,ms)),control=id=>state.controls?.find(c=>c.id===id);
const pass=name=>{report.cases.push(name);console.log('PASS '+name)};
async function until(fn,label='state'){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-start<45000,label+' timeout');await delay(60);}}
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.git_settings?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',n=0;await until(s=>{const now=JSON.stringify(s.controls);n=now===last?n+1:0;last=now;return n>=2},'layout');}
async function click(id,button='left'){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab,button});await settled();}
async function type(id,text){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settled();}
async function scroll(id,direction,pixels=650){await client.tool('rocket_scroll',{target:id,direction,pixels,expected_tab:state.active_tab});await settled();await stable();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const img=r.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settled();}
async function prefs(page){await click('menu-edit');await click('preferences');await click('pref-page-'+page);await settled();}
async function command(index,expected){assert(control('context-command-'+index)?.enabled);await click('context-command-'+index);assert.deepEqual(state.dialog.run_command.arguments,expected);await capture('command-'+index);const previous=state.activities.length;await click('dialog-submit');assert.equal(state.activities.length,previous+1);assert.equal(state.activities.at(-1).operation,'custom-command');assert.equal(state.activities.at(-1).command_succeeded,true);}
async function run(){try{
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>120)throw e;await delay(100)}}
 report.version=(await client.initialize()).serverInfo.version;assert.equal(report.version,'0.80.0');await client.tool('rocket_open',{path:repo});await settled();await resize(1440,1000);
 await prefs('git');assert(state.git_settings.loaded&&state.git_settings.identity_loaded);assert.equal(state.git_settings.error,null);assert.equal(fs.readFileSync(global,'utf8'),'# KEEP GLOBAL COMMENT\n[core]\n autocrlf = false\n');await capture('git-preferences-desktop');pass('Opening Git Preferences reads identity without changing config');
 await click('git-check');const executable=state.git_settings.checked.executable;assert(executable);await click('git-automatic');await type('git-program',executable);assert(!control('git-save')?.enabled);await click('git-check');await click('git-save');assert.match(state.git_settings.notice,/Restart/);assert(!state.git_settings.dirty);pass('Explicit Git is checked before save and activates after restart');
 await scroll('prefs-scroll','down');await type('git-name','Global Fixture 文');await type('git-email','fixture@example.invalid');await click('git-identity-save');assert.equal(state.git_settings.error,null);assert.equal(at(root,['config','--global','user.name']),'Global Fixture 文');assert.equal(at(root,['config','--global','user.email']),'fixture@example.invalid');assert.equal(git(['config','user.name']),'Fixture Local');assert(fs.readFileSync(global,'utf8').includes('# KEEP GLOBAL COMMENT'));assert(fs.readFileSync(global,'utf8').includes('autocrlf = false'));assert.equal(acl(),originalAcl,'Global config access controls preserved');pass('Global identity saves Unicode defaults while local overrides, access controls and unrelated settings survive');
 await type('git-name','Stale value');fs.appendFileSync(global,'\n# external edit\n');const external=sha(global);await click('git-identity-save');assert.match(state.git_settings.error,/changed/);assert.equal(sha(global),external);await capture('identity-external-edit');await click('git-reload');pass('External global edits reject stale Save and Reload restores observed values');
 const savedIdentity=sha(global);await type('git-name','Unsaved fixture');await click('dialog-close');assert(state.git_settings.dirty&&state.dialog);await click('git-reload');assert.equal(sha(global),savedIdentity);pass('Unsaved identity edits keep Preferences open until Save or explicit discard');
 let cancelled=false;const cancelBefore=sha(global);
 for(let attempt=0;attempt<3&&!cancelled;attempt++){
   await stable();await client.tool('rocket_click',{target:'git-reload',expected_tab:state.active_tab});
   await until(s=>!s.input_pending,'reload input');
   if(state.git_settings.busy&&control('git-cancel')?.enabled){
     await client.tool('rocket_click',{target:'git-cancel',expected_tab:state.active_tab});await settled();
     cancelled=/cancel/i.test(state.git_settings.error??'');
   }else await settled();
 }
 assert(cancelled,'Observed cancellation of an actual Git identity reload');assert.equal(sha(global),cancelBefore);await click('git-reload');pass('Visible Cancel stops a Git identity read and preserves config');
 await resize(780,620);await capture('git-preferences-compact');await resize(1440,1000);await click('pref-page-commands');await click('pref-manage-commands');
 const defs=path.join(root,'commands.json'),kinds=['repository','commit','file','branch','submodule'],args=['--version','log\n-1\n--format=%H\n{commit}','ls-files\n--\n{file}','rev-parse\n--verify\n{branch}','-C\n{submodule}\nstatus\n--short'];
 fs.writeFileSync(defs,JSON.stringify({version:2,commands:kinds.map((scope,i)=>({name:'Inspect '+scope,scope,program:executable,arguments:args[i],group:'Fixture',inputs:[]}))}));
 await click('dialog-commands-show-import');await type('dialog-commands-share-path',defs);await click('dialog-commands-import');await until(s=>control('dialog-commands-import-accept')?.enabled,'import review');await scroll('dialog-scroll','down',900);await click('dialog-commands-import-accept');await click('dialog-submit');assert(control('pref-manage-commands')?.enabled,'Returned to Preferences commands page');assert.deepEqual(state.commands.map(c=>c.scope),kinds);pass('Scoped command imports require review and save, and return to Preferences');
 await click('pref-manage-commands');await capture('command-editor-desktop');await resize(780,620);await capture('command-editor-compact');await resize(1440,1000);await click('dialog-cancel');await click('dialog-close');
 await click('menu-repository');await scroll('repository-menu-scroll','up',2000);
 for(let n=0;n<8;n++){const c=control('repository-custom-commands');if(c&&c.rect[3]-c.rect[1]>=28)break;await scroll('repository-menu-scroll','down',250);}
 await click('repository-custom-commands');assert(control('command-run-0')?.enabled);for(let n=1;n<5;n++)assert(!control('command-run-'+n));
 await click('command-run-0');assert.deepEqual(state.dialog.run_command.arguments,['--version']);await click('dialog-submit');assert.equal(state.activities.at(-1).command_succeeded,true);pass('Repository menu runs its command and excludes selection-specific commands');
 await click('view-commits');await click('commit-'+first,'right');assert(control('context-command-1'));assert(!control('context-command-3'));await command(1,['log','-1','--format=%H',first]);pass('Commit context runs the clicked older commit, not HEAD');
 await click('view-branches');let gotBranch=false;for(const c of state.controls.filter(c=>c.id.startsWith('branch-row-'))){await click(c.id,'right');await click('context-command-3');const actual=state.dialog.run_command.arguments;if(actual[2]==='refs/heads/rocket-ui-test/別の'){await capture('branch-context');await click('dialog-submit');gotBranch=true;break}await click('dialog-cancel');}assert(gotBranch);pass('Branch context captures a branch that is not checked out');
 await click('view-changes');const fileRow='change-unstaged-'+Buffer.from('資料 文.txt').toString('hex');await click(fileRow,'right');await command(2,['ls-files','--',path.join(repo,'資料 文.txt')]);pass('File context preserves the exact Unicode filename as one argument');
 await click('view-submodules');await until(s=>control('dialog-submodule-row-0')?.enabled);await click('dialog-submodule-row-0','right');await command(4,['-C',path.join(repo,'modules/模組'),'status','--short']);pass('Submodule context captures the initialized nested Unicode directory');
 assert.deepEqual(fingerprint(),before);pass('Read-only custom commands preserve HEAD, index, working files and repository configuration');
 await prefs('git');await scroll('prefs-scroll','up',2000);assert(!state.git_settings.automatic);await click('dialog-close');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window));await client.tool('rocket_open',{path:repo});await settled();await prefs('git');assert.equal(state.git_settings.path,executable);assert(!state.git_settings.automatic);assert.deepEqual(state.commands.map(c=>c.scope),kinds);pass('Restart retains the selected Git and all five command types');await capture('reopened-git');
 report.passed=true;
}catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client)try{await capture('failure')}catch{}}
finally{
 if(client){try{if(state?.dialog){if(control('git-reload')?.enabled)await click('git-reload');await click(control('dialog-cancel')?'dialog-cancel':'dialog-close');}await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}await client.raw('POST','/shutdown');}catch(e){report.cleanup_error=e.message;}await client.finish();}
 if(service){for(let i=0;i<80&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null)cp.spawnSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});}
 const access=path.join(profile,'access-token');if(fs.existsSync(access))fs.unlinkSync(access);report.cleanup={service_exit:service?.exitCode,access_token_removed:!fs.existsSync(access),error:report.cleanup_error??null};
 if(report.cleanup.error||report.cleanup.service_exit!==0){report.passed=false;process.exitCode=1;}fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.resolve('work/results/git-preferences-0.80/native-latest.json'),JSON.stringify(report,null,2));console.log('Evidence: '+root);
}}
run();
