require('node:fs').mkdirSync(require('node:path').resolve('work/results/accounts-preferences'), {recursive:true});
// Accounts and SSH keys in Preferences: navigation and layout only. Isolated profile,
// repository and SSH folder; the loopback hosting API refuses every request, so any
// hosting call fails the run. No sign-in, keys or credentials are created.
// Usage: node tools/tests/test-mcp-accounts-preferences.cjs [path\to\rocket-mcp.exe]
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict'),http=require('node:http'),net=require('node:net');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/accounts-prefs-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'repo 文'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/accounts-prefs';
const exe=path.resolve(process.argv[2]||process.env.ROCKET_UI_EXE||'target/release/rocket-mcp.exe');
const results=path.resolve('work/results/accounts-preferences');
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.mkdirSync(path.join(root,'ssh-home'));
fs.writeFileSync(path.join(root,'empty.config'),'');fs.writeFileSync(path.join(root,'rocket-ssh-fixture'),'disposable-only-v1');fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const sshConfig=path.join(root,'ssh-home/config');fs.writeFileSync(sshConfig,'# Existing synthetic config\nHost fixture\n  HostName example.invalid\n');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'資料.txt'),'base\n');git(['add','--','資料.txt']);git(['commit','-m','Fixture']);
fs.writeFileSync(path.join(repo,'資料.txt'),'PRIVATE_PREFS staged\n');git(['add','--','資料.txt']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');fs.writeFileSync(path.join(repo,'untracked.txt'),'keep\n');
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const fingerprint=()=>({head:git(['rev-parse','HEAD']),index:git(['ls-files','--stage','-z']),file:sha(path.join(repo,'資料.txt')),untracked:sha(path.join(repo,'untracked.txt')),config:sha(path.join(repo,'.git/config')),ssh:sha(sshConfig)});
const before=fingerprint(),report={passed:false,root,cases:[],exe,exe_hash:sha(exe)};
let api,service,client,state,apiRequests=0;
const delay=ms=>new Promise(r=>setTimeout(r,ms)),control=id=>state.controls?.find(c=>c.id===id);
const pass=name=>{report.cases.push(name);console.log('PASS '+name)};
async function until(fn,label='state'){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-start<45000,label+' timeout');await delay(60);}}
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.hosting?.busy&&!s.ssh?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2},'layout');}
async function click(id){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});await settled();}
async function press(id){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});}
async function type(id,text){await stable();await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await until(s=>!s.input_pending,'typed '+id);}
async function scroll(target,direction,pixels){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settled();await stable();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const img=r.content.find(c=>c.type==='image');assert(img);for(const dir of [root,results])fs.writeFileSync(path.join(dir,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settled();}
function inView(id){const c=control(id);assert(c,'Missing '+id);assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=state.viewport[0]+1&&c.rect[3]<=state.viewport[1]+1,id+' outside the window');}
function actionsDoNotOverlap(){const rows=['hosting-forget','hosting-disconnect','hosting-browser','hosting-connect'].map(id=>{const c=control(id);assert(c&&c.rect[3]-c.rect[1]>=27,'Account action clipped: '+id);return c;});for(let i=0;i<rows.length;i++)for(let j=i+1;j<rows.length;j++){const a=rows[i].rect,b=rows[j].rect;assert(!(Math.max(a[0],b[0])<Math.min(a[2],b[2])&&Math.max(a[1],b[1])<Math.min(a[3],b[3])),'Account actions overlap: '+i+'/'+j);}}
async function preferences(page){await click('menu-edit');await click('preferences');await click('pref-page-'+page);await until(s=>s.preferences?.page===page,'Preferences '+page);await settled();}
async function openHosting(){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);
 await click('repository-remotes-menu');await stable();for(let i=0;i<12&&!(control('repository-hosting')&&control('repository-hosting').rect[3]-control('repository-hosting').rect[1]>25);i++)await scroll('repository-menu-scroll','down',230);
 await click('repository-hosting');await until(s=>s.dialog?.kind==='hosting'&&!s.hosting.busy,'Hosting');await settled();}
async function run(){try{
 api=http.createServer((req,res)=>{apiRequests++;req.resume();res.writeHead(500,{'Content-Type':'application/json'});res.end('{"error":"navigation must not contact hosting"}');});
 await new Promise(r=>api.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+api.address().port);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>120)throw e;await delay(100)}}
 report.version=(await client.initialize()).serverInfo.version;assert.equal(report.version,fs.readFileSync('Cargo.toml','utf8').match(/^version\s*=\s*"([^"]+)"/m)[1]);
 await client.tool('rocket_open',{path:repo});await settled();await resize(1440,960);
 await click('menu-file');assert(!control('file-accounts')&&!control('file-ssh-keys'));await capture('file-menu');pass('File menu no longer lists Accounts or Configure SSH Keys');
 await click('menu-file');await preferences('accounts');assert(state.hosting.accounts_loaded);assert.equal(state.hosting.accounts_count,0);assert.equal(state.hosting.section,'Pulls');
 for(const id of ['pref-page-accounts','pref-page-ssh','hosting-provider-github','hosting-provider-gitea','hosting-connect'])inView(id);actionsDoNotOverlap();await capture('accounts-desktop');pass('Preferences → Accounts loads saved accounts and keeps the account controls');
 await click('hosting-provider-azure-server');for(const id of ['hosting-azure-api-6-0','hosting-azure-api-7-0','hosting-azure-api-7-1'])inView(id);await capture('accounts-azure-server');pass('Self-hosted providers show their own fields');
 await click('hosting-provider-github');await click('pref-page-ssh');await until(s=>s.preferences?.page==='ssh'&&s.ssh.loaded&&!s.ssh.busy,'SSH profiles');assert.equal(state.ssh.profiles.length,0);assert.equal(state.ssh.error,null);assert(control('ssh-new')?.enabled);assert(control('ssh-restore')?.enabled);await capture('ssh-desktop');pass('Preferences → SSH keys lists profiles from the marked SSH folder');
 // One disposable encrypted key through OpenSSH's prompts; no agent is needed to create it.
 await click('ssh-new');await capture('ssh-new-form');await type('ssh-alias','rocket-fixture');await type('ssh-host','github.com');await type('ssh-user','git');await type('ssh-port','22');
 const secret='Fixture-'+crypto.randomBytes(8).toString('hex');
 await press('ssh-create');await until(s=>s.ssh.prompt==='New','new passphrase prompt');await capture('ssh-passphrase');await type('ssh-passphrase',secret);await press('ssh-send');
 await until(s=>s.ssh.prompt==='Confirm','confirm prompt');await type('ssh-passphrase',secret);await press('ssh-send');
 await until(s=>!s.ssh.busy&&s.ssh.profiles.length===1,'profile created');await click('ssh-profile-0');assert.equal(state.ssh.error,null);await capture('ssh-profile');
 await click('ssh-review');assert(state.ssh.review);await capture('ssh-review');await click('ssh-back');assert(!state.ssh.review);pass('A new profile shows its prompts, details and repository review in Preferences');
 await click('dialog-cancel');assert.equal(state.dialog,null);
 await openHosting();assert.equal(state.hosting.section,'Pulls');assert.deepEqual(control('hosting-tab-accounts').rect,control('hosting-accounts').rect,'the old tab name reaches the header link');await capture('hosting-signed-out');
 await click('hosting-accounts');assert.equal(state.preferences?.page,'accounts');assert.equal(state.hosting.section,'Accounts');await capture('accounts-from-hosting');
 await click('dialog-cancel');assert.equal(state.dialog?.kind,'hosting');assert.equal(state.hosting.section,'Pulls');
 await click('hosting-go-accounts');assert.equal(state.preferences?.page,'accounts');await click('dialog-cancel');assert.equal(state.dialog?.kind,'hosting');pass('Hosting reaches Accounts in Preferences and returns when it closes');
 await click('dialog-cancel');await resize(780,620);await preferences('accounts');await scroll('hosting-scroll','down',400);actionsDoNotOverlap();await capture('accounts-compact');
 await click('pref-page-ssh');await until(s=>s.preferences?.page==='ssh'&&s.ssh.loaded,'SSH compact');await capture('ssh-compact');await click('dialog-cancel');pass('Both pages fit a 780x620 window');
 assert.deepEqual(fingerprint(),before);assert.equal(apiRequests,0);assert(!state.hosting.connected);assert(!fs.existsSync(path.join(root,'rocket-ui-state/hosting-accounts.dpapi')));pass('Navigation preserves staged/working files, Git/SSH config and credentials without hosting requests');
 report.passed=true;
}catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client)try{await capture('failure')}catch{}}
finally{
 if(client){try{if(state?.dialog)await click('dialog-cancel');if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}await client.raw('POST','/shutdown');}catch(e){report.cleanup_error=e.message;process.exitCode=1}}
 if(service&&service.exitCode===null)await Promise.race([new Promise(r=>service.once('exit',r)),delay(5000)]);if(service&&service.exitCode===null)service.kill();if(api)await new Promise(r=>api.close(r));
 fs.writeFileSync(path.join(results,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify({passed:report.passed,cases:report.cases.length,root}));
}}
run();
