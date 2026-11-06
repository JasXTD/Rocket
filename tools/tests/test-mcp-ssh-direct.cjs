// Focused SSH manager acceptance. Real Windows OpenSSH; disposable local keys.
// Never modifies ~/.ssh/config, existing keys, or unrelated agent identities.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict'),net=require('node:net');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/ssh-direct-資料-'+crypto.randomBytes(5).toString('hex'));
const repo=path.join(root,'repo'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/ssh';
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'ssh-home'));fs.mkdirSync(path.join(root,'empty-template'));
fs.writeFileSync(path.join(root,'empty.config'),'');fs.writeFileSync(path.join(root,'rocket-ssh-fixture'),'disposable-only-v1');
const oldConfig=path.join(root,'ssh-home/config');fs.writeFileSync(oldConfig,'# existing user config must survive byte for byte\r\nHost old-account\r\n  HostName example.invalid\r\n');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0'});
const git=a=>cp.execFileSync('git',['-C',repo,...a],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const sshAdd=a=>cp.spawnSync('C:/Windows/System32/OpenSSH/ssh-add.exe',a,{env,windowsHide:true,encoding:'utf8',timeout:10000});
const agent=()=>{const o=sshAdd(['-l','-E','sha256']);if(o.status===2)return [];assert([0,1].includes(o.status),'Cannot inspect existing agent state');return o.stdout.split(/\r?\n/).filter(Boolean).map(l=>l.split(/\s+/)[1]).filter(x=>x?.startsWith('SHA256:')).sort()};
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const initialAgent=agent();
git(['init','--initial-branch='+branch]);for(const [k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'資料.txt'),'base\n');git(['add','-A']);git(['commit','-m','Synthetic fixture']);
fs.writeFileSync(path.join(repo,'資料.txt'),'staged\n');git(['add','-A']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');
fs.writeFileSync(path.join(repo,'untracked'),'preserve');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const cfg=path.join(repo,'.git/config'),originalConfig=fs.readFileSync(cfg);
const before={head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'untracked')),sshConfig:hash(oldConfig)};
const secret='Rocket fixture '+crypto.randomBytes(12).toString('hex')+'!',wrong='Wrong disposable passphrase!';
const report={passed:false,root,cases:[],cancellation:[]};let service,client,state,port,generated=[];
const delay=ms=>new Promise(r=>setTimeout(r,ms)),pass=s=>{report.cases.push(s);console.error('PASS '+s)};
async function until(fn,label='state'){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-start<45000,label+' timeout '+JSON.stringify({ssh:state.ssh,dialog:state.dialog?.kind,busy:state.busy}));await delay(60)}}
const control=k=>state.controls?.find(c=>c.id===k);
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.ssh?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled')}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2},'layout')}
async function scroll(region,direction,pixels){await client.tool('rocket_scroll',{target:region,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable()}
async function reveal(k){await stable();const visible=()=>control(k)&&control(k).rect[3]-control(k).rect[1]>18;if(visible())return;
 if(control('ssh-scroll')){for(let i=0;i<5&&!visible();i++)await scroll('ssh-scroll','up',2000);for(let i=0;i<20&&!visible();i++)await scroll('ssh-scroll','down',250)}
 assert(visible(),'unreachable '+k)}
async function click(k,wait=true){await reveal(k);assert(control(k)?.enabled,'disabled '+k);await client.tool('rocket_click',{target:k,expected_tab:state.active_tab});await until(s=>!s.input_pending);if(wait)await settled()}
async function type(k,text){await reveal(k);await client.tool('rocket_type',{target:k,text,expected_tab:state.active_tab});await until(s=>!s.input_pending)}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const i=r.content.find(c=>c.type==='image');fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(i.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2))}
// SSH keys is a Preferences page (Edit → Preferences → SSH keys).
async function openPage(){await click('menu-edit');await click('preferences');await click('pref-page-ssh');
 await until(s=>s.preferences?.page==='ssh'&&s.ssh.loaded&&!s.ssh.busy);await settled()}
async function newForm(alias){await click('ssh-new');await type('ssh-alias',alias);await type('ssh-host','github.com');await type('ssh-user','git');await type('ssh-port','22')}
async function send(text=secret){await type('ssh-passphrase',text);await click('ssh-send',false)}
async function generate(alias,rsa=false){
 await newForm(alias);if(rsa)await click('ssh-rsa');await click('ssh-create',false);
 await until(s=>s.ssh.prompt==='New'||s.ssh.error,'new key prompt');assert.equal(state.ssh.prompt,'New',state.ssh.error);
 await send();await until(s=>s.ssh.prompt==='Confirm'||s.ssh.error,'confirmation');assert.equal(state.ssh.prompt,'Confirm',state.ssh.error);
 await send();await settled();assert(!state.ssh.error,state.ssh.error);
 const p=state.ssh.profiles.find(x=>x.alias===alias);assert(p?.encrypted);generated.push(path.join(root,'ssh-home/rocket-keys',alias,rsa?'id_rsa':'id_ed25519'));
 return p;
}
async function cancel(label){await reveal('ssh-cancel');const at=performance.now();await client.tool('rocket_click',{target:'ssh-cancel',expected_tab:state.active_tab});await until(s=>s.ssh.stopping||!s.ssh.busy);const ack=performance.now()-at;await settled();report.cancellation.push({label,feedback_ms:Math.round(ack),settled_ms:Math.round(performance.now()-at)});}
async function close(){if(state?.ssh?.busy)await cancel('cleanup');if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');
 if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window))}}
async function run(){
 try{
 const exe=path.resolve(process.env.ROCKET_SSH_EXE||'target/release/rocket-mcp.exe');report.sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));port=probe.address().port;await new Promise(r=>probe.close(r));report.port=port;
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const out=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>100)throw e;await delay(100)}}
 await client.initialize();await client.tool('rocket_open',{path:repo});await settled();
 await client.tool('rocket_resize',{width:1280,height:800});await settled();if(control('console-close'))await click('console-close');await openPage();
 const sshgen=path.join(process.env.WINDIR,'System32/OpenSSH/ssh-keygen.exe');
 const key=path.join(root,'ssh-home','id_資料'),other=path.join(root,'ssh-home','id_other');
 for(const name of [key,other]){generated.push(name);cp.execFileSync(sshgen,['-q','-t','ed25519','-a','16','-N',secret,'-f',name],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});}
 const originalKey=hash(key),otherKey=hash(other);await click('ssh-scan');
 const index=state.ssh.discovery.keys.findIndex(k=>k.name==='id_資料');assert(index>=0);
 await click('ssh-found-'+index);await click('ssh-mode-direct');assert.equal(state.ssh.authentication,'direct');
 await click('ssh-review');assert(state.ssh.creating);await type('ssh-alias','rocket-direct');await type('ssh-host','github.com');await click('ssh-create');
 assert.equal(state.ssh.authentication,'direct');assert.equal(state.ssh.profiles.length,1);assert(!state.ssh.profiles[0].passphrase_saved);assert(!state.ssh.error,state.ssh.error);
 await capture('01-direct-selected');pass('Unicode discovered key selection preserved through direct-profile creation');
 const profileDir=path.join(root,'ssh-home/rocket-keys/rocket-direct'),vault=path.join(profileDir,'passphrase.dpapi');
 await click('ssh-remember',false);await until(s=>s.ssh.prompt==='Remember');await capture('02-passphrase-prompt');await cancel('direct passphrase prompt');assert(!fs.existsSync(vault));
 await client.tool('rocket_resize',{width:780,height:620});await settled();
 await click('ssh-remember',false);await until(s=>s.ssh.prompt==='Remember');await reveal('ssh-cancel');await capture('02b-compact-prompt');await cancel('compact passphrase prompt');assert(!fs.existsSync(vault));
 await client.tool('rocket_resize',{width:1280,height:800});await settled();
 pass('Stop cancels the passphrase request without saving or changing agent state');
 await click('ssh-remember',false);await until(s=>s.ssh.prompt==='Remember');await send(wrong);await settled();assert.match(state.ssh.error,/ssh_unlock_failed/);assert(!fs.existsSync(vault));
 pass('wrong passphrase rejected by real OpenSSH; no credential published');
 await click('ssh-remember',false);await until(s=>s.ssh.prompt==='Remember');await send();await settled();assert(!state.ssh.error,state.ssh.error);assert(state.ssh.profiles[0].passphrase_saved);
 assert(!fs.readFileSync(vault).includes(Buffer.from(secret)));assert.deepEqual(agent(),initialAgent);await click('ssh-check');assert.match(state.ssh.notice,/agent disabled.*host verification required/);await capture('03-remembered');
 pass('correct passphrase verified and Windows-encrypted with no ssh-agent changes');
 await click('ssh-review');assert(state.ssh.review);await capture('04-direct-review');await click('ssh-back');assert.deepEqual(fs.readFileSync(cfg),originalConfig);
 await click('ssh-review');fs.appendFileSync(cfg,'\n# external edit\n');const external=fs.readFileSync(cfg);await click('ssh-apply');assert.match(state.ssh.error,/ssh_external_change/);assert.deepEqual(fs.readFileSync(cfg),external);
 await click('ssh-review');await click('ssh-apply');assert(!state.ssh.error,state.ssh.error);assert.match(git(['config','--get','core.sshCommand']),/rocket-ssh.exe.*--profile/);
 await click('ssh-restore');await click('ssh-apply');assert.deepEqual(fs.readFileSync(cfg),external);
 pass('review/back, external config edits, direct binding and exact restoration');
 await client.tool('rocket_resize',{width:780,height:620});await settled();await reveal('ssh-forget');await capture('05-compact-direct');
 await click('ssh-forget');assert(!state.ssh.profiles[0].passphrase_saved);assert(!fs.existsSync(vault));
 await click('ssh-review');assert.match(state.ssh.error,/ssh_passphrase_unavailable/);
 pass('compact controls reachable; forgetting a passphrase disables encrypted direct use');
 await click('ssh-mode-agent');await click('ssh-review');assert(state.ssh.review);await click('ssh-back');
 assert.equal(state.ssh.authentication,'agent');pass('existing agent binding preview remains available');
 assert.equal(git(['rev-parse','HEAD']),before.head);assert.equal(hash(path.join(repo,'.git/index')),before.index);assert.equal(hash(path.join(repo,'資料.txt')),before.file);assert.equal(hash(path.join(repo,'untracked')),before.untracked);assert.equal(hash(oldConfig),before.sshConfig);assert.equal(hash(key),originalKey);assert.equal(hash(other),otherKey);assert.deepEqual(agent(),initialAgent);
 pass('existing keys/config, agent identities, HEAD, index and working files preserved');
 await close();await client.raw('POST','/shutdown');
 const logs=[];function walk(d){for(const e of fs.readdirSync(d,{withFileTypes:true})){const p=path.join(d,e.name);if(e.isDirectory())walk(p);else if(e.name==='rocket.jsonl')logs.push(fs.readFileSync(p,'utf8'))}}walk(root);
 const text=logs.join('\n'),records=text.split(/\r?\n/).filter(Boolean).map(JSON.parse);assert(records.some(r=>r.operation==='ssh-remember'));assert(records.some(r=>r.operation==='ssh-forget'));assert(records.every(r=>!r.dropped_before));
 const strings=[];function collect(value){if(typeof value==='string')strings.push(value);else if(value&&typeof value==='object')Object.values(value).forEach(collect)}records.forEach(collect);
 for(const value of [secret,wrong,'BEGIN OPENSSH PRIVATE KEY',root])assert(!strings.some(s=>s.includes(value)),'private diagnostic payload');
 assert(!fs.readFileSync(path.join(root,'service.log'),'utf8').includes(secret));
 pass('clear diagnostic phases with no passphrases, key payloads or dropped events');report.passed=true;
 }catch(e){report.error=e.stack;console.error(e.stack);if(client){try{await capture('failure')}catch{}}}
 finally{
 if(client){try{if(!['closed','not_started'].includes(state?.window))await close()}catch(e){report.cleanup_warning=String(e)}try{await client.raw('POST','/shutdown')}catch{}try{await client.finish()}catch{}}
 if(service){await Promise.race([new Promise(r=>service.once('exit',r)),delay(5000)]);if(service.exitCode===null)service.kill();}
 try{assert.deepEqual(agent(),initialAgent);report.agent_preserved=true}catch(e){report.passed=false;report.cleanup_error=String(e)}
 // Delete only the exact disposable key pair paths this runner created. Keep
 // screenshots, diagnostics and the synthetic repository as review evidence.
 try{for(const key of generated){const resolved=path.resolve(key);assert(resolved.startsWith(root+path.sep));for(const p of [resolved,resolved+'.pub'])if(fs.existsSync(p)){assert(fs.lstatSync(p).isFile()&&!fs.lstatSync(p).isSymbolicLink());fs.unlinkSync(p);}}
 const vault=path.join(root,'ssh-home/rocket-keys/rocket-direct/passphrase.dpapi');if(fs.existsSync(vault)){assert(fs.lstatSync(vault).isFile()&&!fs.lstatSync(vault).isSymbolicLink());fs.unlinkSync(vault);}report.disposable_keys_removed=true;
 }catch(e){report.passed=false;report.cleanup_error=String(e)}
 fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.error('Results: '+path.join(root,'result.json'));if(!report.passed)process.exitCode=1;
 }
}
run();
