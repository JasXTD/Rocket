require('node:fs').mkdirSync(require('node:path').resolve('work/results/file-accounts-0.78.1'), {recursive:true});
// File-menu navigation only: isolated profile/repository/SSH folder, no sign-in or keys.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict'),http=require('node:http'),net=require('node:net');
const {RocketHttpClient}=require('../../../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/file-accounts-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'repo 文'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/file-accounts';
const exe=path.resolve('target/release/rocket-mcp.exe');
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.mkdirSync(path.join(root,'ssh-home'));
fs.writeFileSync(path.join(root,'empty.config'),'');fs.writeFileSync(path.join(root,'rocket-ssh-fixture'),'disposable-only-v1');fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const sshConfig=path.join(root,'ssh-home/config');fs.writeFileSync(sshConfig,'# Existing synthetic config\nHost fixture\n  HostName example.invalid\n');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'資料.txt'),'base\n');git(['add','--','資料.txt']);git(['commit','-m','Fixture']);
fs.writeFileSync(path.join(repo,'資料.txt'),'PRIVATE_MENU staged\n');git(['add','--','資料.txt']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');fs.writeFileSync(path.join(repo,'untracked.txt'),'keep\n');
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const fingerprint=()=>({head:git(['rev-parse','HEAD']),index:git(['ls-files','--stage','-z']),file:sha(path.join(repo,'資料.txt')),untracked:sha(path.join(repo,'untracked.txt')),config:sha(path.join(repo,'.git/config')),ssh:sha(sshConfig)});
const before=fingerprint(),report={passed:false,root,cases:[],exe_hash:sha(exe)};
let api,service,client,state,apiRequests=0;
const delay=ms=>new Promise(r=>setTimeout(r,ms)),control=id=>state.controls?.find(c=>c.id===id);
const pass=name=>{report.cases.push(name);console.log('PASS '+name)};
async function until(fn,label='state'){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-start<45000,label+' timeout');await delay(60);}}
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.hosting?.busy&&!s.ssh?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2},'layout');}
async function click(id){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});await settled();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const img=r.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settled();}
function menuFits(){for(const id of ['file-ssh-keys','file-accounts']){const c=control(id);assert(c?.enabled);assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=state.viewport[0]&&c.rect[3]<=state.viewport[1]);}}
function actionsDoNotOverlap(){const rows=['hosting-forget','hosting-disconnect','hosting-browser','hosting-connect'].map(id=>{const c=control(id);assert(c&&c.rect[3]-c.rect[1]>=27,'Account action clipped: '+id);return c;});for(let i=0;i<rows.length;i++)for(let j=i+1;j<rows.length;j++){const a=rows[i].rect,b=rows[j].rect;assert(!(Math.max(a[0],b[0])<Math.min(a[2],b[2])&&Math.max(a[1],b[1])<Math.min(a[3],b[3])),rows[i].id+' overlaps '+rows[j].id);}assert(control('hosting-connect').rect[1]<=control('hosting-remember').rect[3]+20,'Account actions separated from the form');}
async function run(){try{
 api=http.createServer((req,res)=>{apiRequests++;req.resume();res.writeHead(500,{'Content-Type':'application/json'});res.end('{"error":"navigation must not contact hosting"}');});
 await new Promise(r=>api.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+api.address().port);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>120)throw e;await delay(100)}}
 report.version=(await client.initialize()).serverInfo.version;assert.equal(report.version,'0.78.1');await client.tool('rocket_open',{path:repo});await settled();await resize(1440,960);
 await click('menu-file');menuFits();await capture('file-menu-desktop');pass('File menu exposes both enabled shortcuts at desktop size');
 await click('file-accounts');assert.equal(state.dialog?.kind,'hosting');assert.equal(state.hosting.section,'Accounts');assert(state.hosting.accounts_loaded);assert.equal(state.hosting.accounts_count,0);actionsDoNotOverlap();await capture('accounts-desktop');pass('Accounts opens the actual account settings directly');
 await click('hosting-tab-pulls');assert.equal(state.hosting.section,'Pulls');await click('menu-file');await click('file-accounts');assert.equal(state.hosting.section,'Accounts');pass('Accounts returns to account settings from an already-open hosting section');
 // Accounts is an editable form. Close it before MCP navigates to another page.
 await click('dialog-cancel');await click('menu-file');await click('file-ssh-keys');assert.equal(state.dialog?.kind,'ssh-keys');assert(state.ssh.loaded);assert.equal(state.ssh.profiles.length,0);assert.equal(state.ssh.error,null);await capture('ssh-desktop');pass('Configure SSH Keys opens the existing OpenSSH manager');
 await click('dialog-cancel');await resize(780,620);await click('menu-file');menuFits();await capture('file-menu-compact');await click('file-accounts');await client.tool('rocket_scroll',{target:'hosting-scroll',direction:'down',pixels:200,expected_tab:state.active_tab});await settled();await stable();actionsDoNotOverlap();await capture('accounts-compact');assert.equal(state.hosting.section,'Accounts');await click('dialog-cancel');pass('Both File shortcuts fit at 780x620 and account actions do not overlap');
 assert.deepEqual(fingerprint(),before);assert.equal(apiRequests,0);assert(!state.hosting.connected);assert(!fs.existsSync(path.join(root,'ssh-home/rocket-keys')));assert(!fs.existsSync(path.join(root,'rocket-ui-state/hosting-accounts.dpapi')));pass('Navigation preserves staged/working files, Git/SSH config and credentials without hosting requests');
 report.passed=true;
}catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client)try{await capture('failure')}catch{}}
finally{
 if(client){try{if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}await client.raw('POST','/shutdown');}catch(e){report.cleanup_error=e.message;}await client.finish();}
 if(service){for(let i=0;i<80&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null)cp.spawnSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});}
 if(api)await new Promise(r=>api.close(r));const access=path.join(profile,'access-token');if(fs.existsSync(access))fs.unlinkSync(access);
 report.cleanup={service_exit:service?.exitCode,access_token_removed:!fs.existsSync(access),error:report.cleanup_error??null};report.hosting_requests=apiRequests;
 if(report.cleanup.error||report.cleanup.service_exit!==0){report.passed=false;process.exitCode=1;}
 fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.resolve('work/results/file-accounts-0.78.1/native-latest.json'),JSON.stringify(report,null,2));console.log('Evidence: '+root);
}}
run();
