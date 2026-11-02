// Focused SSH manager acceptance. Real Windows OpenSSH; disposable local keys.
// Never modifies ~/.ssh/config, existing keys, or unrelated agent identities.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict'),net=require('node:net');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/ssh-資料-'+crypto.randomBytes(5).toString('hex'));
const repo=path.join(root,'repo'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/ssh';
fs.mkdirSync(repo,{recursive:true});
// Explicit local consent: this runner creates only synthetic shell/editor data.
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.mkdirSync(path.join(root,'ssh-home'));fs.mkdirSync(path.join(root,'empty-template'));
fs.writeFileSync(path.join(root,'empty.config'),'');fs.writeFileSync(path.join(root,'rocket-ssh-fixture'),'disposable-only-v1');
const oldConfig=path.join(root,'ssh-home/config');fs.writeFileSync(oldConfig,'# existing user config must survive byte for byte\r\nHost old-account\r\n  HostName example.invalid\r\n');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0'});
const git=a=>cp.execFileSync('git',['-C',repo,...a],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const sshAdd=a=>cp.spawnSync('C:/Windows/System32/OpenSSH/ssh-add.exe',a,{env,windowsHide:true,encoding:'utf8',timeout:10000});
const agent=()=>{const o=sshAdd(['-l','-E','sha256']);assert([0,1].includes(o.status),'Windows SSH agent must be running for disposable key tests');return o.stdout.split(/\r?\n/).filter(Boolean).map(l=>l.split(/\s+/)[1]).filter(x=>x?.startsWith('SHA256:')).sort()};
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
 report.version=(await client.initialize()).serverInfo.version;await client.tool('rocket_open',{path:repo});await settled();
 await client.tool('rocket_resize',{width:1480,height:920});await settled();if(control('console-close'))await click('console-close');await openPage();

 await newForm('rocket-mismatch');await click('ssh-create',false);await until(s=>s.ssh.prompt==='New');await send();await until(s=>s.ssh.prompt==='Confirm');await send(wrong);await until(s=>s.ssh.prompt==='NewRetry'||s.ssh.error);
 assert.equal(state.ssh.prompt,'NewRetry',state.ssh.error);await capture('mismatch-guidance');await cancel('mismatched-key-passphrase');await click('ssh-form-cancel');
 assert(!fs.existsSync(path.join(root,'ssh-home/rocket-keys/rocket-mismatch')));
 pass('mismatched creation passphrases show guidance; cancellation preserves files');
 const personal=await generate('rocket-personal'),keyOne=generated[0],keyHash=hash(keyOne);
 const tool='C:/Windows/System32/OpenSSH/ssh-keygen.exe',ca=path.join(root,'fixture-ca');
 const call=a=>{const o=cp.spawnSync(tool,a,{env,windowsHide:true,encoding:'utf8',timeout:15000});assert.equal(o.status,0,'disposable certificate fixture');};
 call(['-q','-t','ed25519','-N','','-f',ca]);
 call(['-q','-s',ca,'-I','rocket-disposable','-n','git','-V','+1h',keyOne+'.pub']);
 assert(fs.existsSync(keyOne+'-cert.pub'));
 await click('ssh-unlock',false);await until(s=>s.ssh.prompt==='Unlock');await send(wrong);await until(s=>s.ssh.prompt==='UnlockRetry');await send();await settled();
 assert(!state.ssh.error,state.ssh.error);assert(state.ssh.profiles[0].loaded);assert.deepEqual(agent(),[...initialAgent,personal.fingerprint].sort());
 pass('encrypted key unlock retries correctly and excludes adjacent SSH certificate');
 await click('ssh-unload');assert.deepEqual(agent(),initialAgent);assert.equal(hash(keyOne),keyHash);pass('selected plain identity unload leaves pre-existing agent identities intact');
 await click('ssh-unlock',false);await until(s=>s.ssh.prompt==='Unlock');await cancel('unlock-prompt');assert.deepEqual(agent(),initialAgent);
 await capture('final-ssh-page');await click('dialog-cancel');
 await click('menu-terminal');await click('terminal-new');
 await until(s=>s.terminal?.running&&s.terminal.tail.includes('PS '),'normal PowerShell prompt');
 await client.tool('rocket_terminal_probe',{probe:'typing',expected_tab:state.active_tab});await until(s=>!s.input_pending&&s.terminal?.tail.includes('ROCKET_TYPING_OK'),'normal terminal typing');
 assert(!state.terminal.error,state.terminal.error);await capture('terminal-neighbor');
 await click('terminal-end');await until(s=>s.terminal?.closed);await click('terminal-hide');
 pass('shared ConPTY launcher still starts PowerShell and accepts typed commands');
 await openPage();await newForm('rocket-quit');await click('ssh-create',false);await until(s=>s.ssh.prompt==='New');
 await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');
 if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window),'quit drains SSH worker');}
 assert(!fs.existsSync(path.join(root,'ssh-home/rocket-keys/rocket-quit')));assert.deepEqual(agent(),initialAgent);
 pass('window exit cancels and drains private OpenSSH console without stranded keys');
 assert.equal(git(['rev-parse','HEAD']),before.head);assert.equal(hash(path.join(repo,'.git/index')),before.index);assert.equal(hash(path.join(repo,'資料.txt')),before.file);assert.equal(hash(path.join(repo,'untracked')),before.untracked);assert.equal(hash(oldConfig),before.sshConfig);assert.deepEqual(fs.readFileSync(cfg),originalConfig);
 const logPaths=[];function walk(d){for(const e of fs.readdirSync(d,{withFileTypes:true})){const p=path.join(d,e.name);if(e.isDirectory())walk(p);else if(e.name==='rocket.jsonl')logPaths.push(p)}}walk(root);
 const logs=logPaths.map(p=>fs.readFileSync(p,'utf8')).join('\n'),records=logs.split(/\r?\n/).filter(Boolean).map(l=>JSON.parse(l));
 for(const value of [secret,wrong,'BEGIN OPENSSH PRIVATE KEY',personal.fingerprint,'rocket-personal'])assert(!logs.includes(value),'private diagnostic content');
 const spans=new Map();for(const r of records){if(/_(start|end)$/.test(r.event)&&r.id){const key=r.event.replace(/_(start|end)$/,'')+':'+r.id;spans.set(key,(spans.get(key)||0)+(r.event.endsWith('_start')?1:-1))}assert(!r.dropped_before)}
 assert([...spans.values()].every(n=>n===0));report.diagnostic_records=records.length;report.stop_microseconds=records.filter(r=>r.event==='ssh_console_exit'&&r.cancel_to_stop_us!=null).map(r=>r.cancel_to_stop_us);
 pass('private balanced diagnostics, zero dropped events and repository preservation');
 report.passed=true;await client.raw('POST','/shutdown');
 }catch(e){report.error=e.stack;console.error(e.stack);if(client){try{await capture('failure')}catch{}}}
 finally{
  // Exact disposable public keys only; never ssh-add -D and never remove another identity.
  for(const key of generated)if(fs.existsSync(key+'.pub'))sshAdd(['-d',key+'.pub']);
  try{assert.deepEqual(agent(),initialAgent);report.agent_restored=true}catch(e){report.passed=false;report.cleanup_error=String(e)}
  if(client){try{if(!['closed','not_started'].includes(state?.window))await close()}catch(e){report.cleanup_warning=String(e)}try{await client.raw('POST','/shutdown')}catch{}try{await client.finish()}catch{}}
  if(service){await Promise.race([new Promise(r=>service.once('exit',r)),delay(5000)]);if(service.exitCode===null)service.kill();}
  fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.error('Results: '+path.join(root,'result.json'));if(!report.passed)process.exitCode=1;
 }
}
run();
