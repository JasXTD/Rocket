// Real GitHub acceptance: one private repository, synthetic data, no credential writes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const https=require('node:https'),net=require('node:net'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../../../mcp/mcp-http-client.cjs');
const base=path.resolve('work/live/github-publish'),planFile=path.join(base,'plan.json');
fs.mkdirSync(base,{recursive:true});
if(!fs.existsSync(planFile)){
 const id=crypto.randomBytes(16).toString('hex');
 fs.writeFileSync(planFile,JSON.stringify({id,owner:'JasXTD',account_id:268453345,name:'rocket-publish-test-'+id,branch:'rocket-ui-test/publish-'+id.slice(0,12),root:path.resolve('work/mcp-fixtures/github-publish-live-'+id)},null,2),{flag:'wx'});
}
const plan=JSON.parse(fs.readFileSync(planFile)),{root,branch,owner,name}=plan;
assert.equal(owner,'JasXTD');assert.match(name,/^rocket-publish-test-[a-f0-9]{32}$/);
assert.equal(branch,'rocket-ui-test/publish-'+plan.id.slice(0,12));
assert(path.resolve(root).startsWith(path.resolve('work/mcp-fixtures')+path.sep));
const repo=path.join(root,'資料 synthetic'),profile=path.join(root,'mcp-profile');
const fullName=owner+'/'+name,remote='git@github.com:'+fullName+'.git';
const sha=b=>crypto.createHash('sha256').update(b).digest('hex'),hash=p=>sha(fs.readFileSync(p));
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const env={...process.env};
for(const k of Object.keys(env))if(/^(GIT_|GCM_|SSH_|ROCKET_|CURL_|SSL_CERT_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$|^NETRC$/i.test(k))delete env[k];
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'),{recursive:true});
if(!fs.existsSync(path.join(root,'empty.config')))fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'ssh',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0'});
function git(a,{at=repo,input,extra={}}={}){return cp.execFileSync('git',['-C',at,...a],{env:{...env,...extra},input,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe'],timeout:30000}).trimEnd();}
if(!fs.existsSync(repo)){
 fs.mkdirSync(repo);git(['init','--initial-branch='+branch]);
 for(const [k,v]of [['user.name','Rocket synthetic test'],['user.email','rocket-test@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
 fs.writeFileSync(path.join(repo,'資料.txt'),'Synthetic GitHub publishing test\n');git(['add','--all']);git(['commit','-m','Synthetic publication']);
 fs.writeFileSync(path.join(repo,'資料.txt'),'staged synthetic draft\n');git(['add','--','資料.txt']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged synthetic edit\n');
 fs.writeFileSync(path.join(repo,'untracked.txt'),'PRIVATE unpublished synthetic data');
 fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
}
fs.writeFileSync(path.join(root,'rocket-github-publish-check'),`ROCKET-GITHUB-PUBLISH-CHECK-1\n${fullName}\n${branch}\nprivate\nssh\n`);
const resultFile=path.join(root,'result.json');
const report=fs.existsSync(resultFile)?JSON.parse(fs.readFileSync(resultFile)):{passed:false,root,fullName,branch,cases:[],cancellation:[],api_calls:[]};
delete report.error;delete report.cleanup_error;delete report.identity_error;report.passed=false;
const save=()=>{fs.writeFileSync(resultFile,JSON.stringify(report,null,2));fs.writeFileSync(path.join(base,'native-latest.json'),JSON.stringify(report,null,2));};
const pass=s=>{if(!report.cases.includes(s))report.cases.push(s);save();console.log('PASS '+s);};
const local=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'untracked.txt')),global:hash(path.join(root,'empty.config'))});
report.before??=local();
const personalPaths=['.gitconfig','.ssh/config','.ssh/known_hosts','.ssh/id_ed25519_github.pub'];
const personal=()=>Object.fromEntries(personalPaths.map(p=>{const f=path.join(process.env.USERPROFILE,p);return[p,fs.existsSync(f)?hash(f):null]}));
report.personal_before??=personal();
const sshAdd=path.join(process.env.WINDIR,'System32/OpenSSH/ssh-add.exe');
function agent(){return cp.execFileSync(sshAdd,['-l'],{env,windowsHide:true,encoding:'utf8',timeout:10000}).split(/\r?\n/).filter(Boolean).map(s=>s.split(' ')[1]).sort();}
report.agent_before??=agent();
const publicKey=fs.readFileSync(path.join(process.env.USERPROFILE,'.ssh/id_ed25519_github.pub'),'utf8').trim().split(/\s+/).slice(0,2).join(' ');
assert(publicKey.startsWith('ssh-ed25519 '));
fs.writeFileSync(path.join(root,'identity.pub'),publicKey+'\n');
const keygen=path.join(process.env.WINDIR,'System32/OpenSSH/ssh-keygen.exe');
const fingerprint=cp.execFileSync(keygen,['-lf',path.join(root,'identity.pub')],{env,windowsHide:true,encoding:'utf8'}).split(' ')[1];
assert(report.agent_before.includes(fingerprint),'Existing public identity is not loaded in Windows agent');
function credential(){
 const e={...env,GCM_INTERACTIVE:'never',GCM_GUI_PROMPT:'0'};
 const r=cp.spawnSync('git',['credential-manager','get'],{env:e,input:'protocol=https\nhost=github.com\nusername=JasXTD\n\n',encoding:'utf8',windowsHide:true,timeout:30000,maxBuffer:128*1024});
 const t=(r.stdout||'').split(/\r?\n/).find(s=>s.startsWith('password='))?.slice(9);
 assert(r.status===0&&t,'Existing noninteractive credential unavailable; no credential mutation attempted');return t;
}
let token=credential();
assert.equal(sha(token),JSON.parse(fs.readFileSync(path.join(base,'credential-before.json'))).credential_sha256);
const redact=s=>String(s).split(token).join('[REDACTED]');
function api(route){
 assert(route==='/user'||route==='/repos/'+fullName||route==='/repos/'+fullName+'/git/ref/heads/'+branch);
 return new Promise((resolve,reject)=>{
  const req=https.request({hostname:'api.github.com',port:443,path:route,method:'GET',headers:{Authorization:'Bearer '+token,Accept:'application/vnd.github+json','User-Agent':'Rocket-Publishing-Acceptance','X-GitHub-Api-Version':'2022-11-28'},timeout:15000},res=>{
   let data='';res.on('data',c=>{data+=c;if(data.length>256*1024)res.destroy(Error('Response exceeds bound'))});
   res.on('error',()=>reject(Error('Bounded GitHub read failed')));
   res.on('end',()=>{let body;try{body=JSON.parse(data)}catch{return reject(Error('Invalid GitHub JSON'))}
    report.api_calls.push({method:'GET',path:route,status:res.statusCode});resolve({status:res.statusCode,body,scopes:res.headers['x-oauth-scopes']});});
  });req.on('error',()=>reject(Error('GitHub verification read failed')));req.on('timeout',()=>req.destroy());req.end();
 });
}
let service,client,state;
async function until(fn,label='UI state',ms=60000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return state;assert(performance.now()-start<ms,label+' timeout '+JSON.stringify({hosting:state.hosting,dialog:state.dialog?.kind,busy:state.busy}));await delay(40);}}
const control=k=>state.controls?.find(c=>c.id===k);
const settled=()=>until(s=>!s.busy&&!s.input_pending&&!s.hosting?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');
async function stable(){let last='',n=0;await until(s=>{const now=JSON.stringify(s.controls);n=now===last?n+1:0;last=now;return n>=2;},'stable');}
async function scroll(id,dir,pixels=300){await client.tool('rocket_scroll',{target:id,direction:dir,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(id){await stable();const visible=()=>control(id)&&control(id).rect[3]-control(id).rect[1]>18;if(visible())return;if(control('hosting-scroll')){await scroll('hosting-scroll','up',2000);for(let i=0;i<32&&!visible();i++)await scroll('hosting-scroll','down',180);}assert(visible(),'Control not reachable: '+id);}
async function click(id,wait=true){await reveal(id);assert(control(id).enabled,'Disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(wait)await settled();}
async function type(id,text){await reveal(id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settled();}
async function denied(id){await reveal(id);const r=await client.request('tools/call',{name:'rocket_click',arguments:{target:id,expected_tab:state.active_tab}});assert(r.isError,'Scope should reject '+id);}
async function capture(name){await stable();const v=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!v.isError);const im=v.content.find(c=>c.type==='image');assert(im);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(im.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),redact(JSON.stringify(state,null,2)));}
async function openHosting(){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);await click('repository-remotes-menu');await stable();for(let i=0;i<15&&!(control('repository-hosting')&&control('repository-hosting').rect[3]-control('repository-hosting').rect[1]>24);i++)await scroll('repository-menu-scroll','down',190);await click('repository-hosting');}
async function form(remoteName='origin'){
 if(state.hosting.publish?.report)await click('hosting-publish-reset');
 if(state.hosting.publish?.reviewing)await click('hosting-publish-back');
 await type('hosting-publish-owner',owner);await type('hosting-publish-name',name);await type('hosting-publish-branch',branch);await type('hosting-publish-remote',remoteName);
 await type('hosting-publish-description','Disposable synthetic Rocket publication '+plan.id);
 if(!state.hosting.publish.private)await click('hosting-publish-private');
 if(!state.hosting.publish.ssh)await click('hosting-publish-ssh');
}
async function review(){await click('hosting-publish-review');assert(state.hosting.publish.reviewing,state.hosting.error);assert.equal(state.hosting.publish.commit,report.before.head);}
const sameLocal=()=>assert.deepEqual(local(),report.before);
function logs(){let events=[];for(const d of fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')))for(const n of fs.readdirSync(path.join(root,d)).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n))){const raw=fs.readFileSync(path.join(root,d,n),'utf8');for(const secret of [token,'PRIVATE unpublished','rocket-test@example.invalid'])assert(!raw.includes(secret),'Diagnostic payload leak');for(const line of raw.split('\n').filter(Boolean)){try{events.push(JSON.parse(line))}catch{}}}return events;}
async function verifyIdentity(){assert.deepEqual(personal(),report.personal_before);assert.deepEqual(agent(),report.agent_before);assert.equal(sha(credential()),sha(token));const u=await api('/user');assert.equal(u.status,200);assert.equal(u.body.login,owner);assert.equal(u.body.id,plan.account_id);assert(!u.scopes.split(/,\s*/).includes('delete_repo'));assert(!fs.existsSync(path.join(root,'rocket-ui-state/hosting-accounts.dpapi')));}
async function run(){try{
 const who=await api('/user');assert.equal(who.status,200);assert.equal(who.body.id,plan.account_id);
 const prior=await api('/repos/'+fullName);
 if(report.creation_intent&&!report.published&&prior.status!==404)throw Error('Previous creation outcome needs explicit reconciliation; no automatic retry');
 if(report.published){assert.equal(prior.status,200);assert.equal(prior.body.id,report.repository_id);assert.equal(prior.body.private,true);}else assert.equal(prior.status,404,'Disposable repository must not already exist');
 const exe=path.resolve('target/release/rocket-mcp.exe');report.sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const fd=fs.openSync(path.join(root,'service-'+Date.now()+'.log'),'w');service=cp.spawn(exe,['--http-profile',profile,'--windows-ssh-agent',path.join(root,'identity.pub')],{env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>100)throw e;await delay(100)}}
 report.version=(await client.initialize()).serverInfo.version;assert.equal(report.version,'0.75.1');
 await client.tool('rocket_open',{path:repo});await settled();await client.tool('rocket_resize',{width:1420,height:980});await until(s=>s.viewport[0]===1420&&s.viewport[1]===980&&!s.input_pending);await settled();if(control('console-close'))await click('console-close');await openHosting();
 assert(!state.hosting.remember);
 if(!report.cases.includes('real GitHub invalid credential error is clear; existing login untouched')){await type('hosting-token','ROCKET_DISPOSABLE_INVALID_CREDENTIAL');await click('hosting-connect');assert(!state.hosting.connected);assert.match(state.hosting.error,/unauthoriz|authentication|credential/i);await capture('authentication-rejected');pass('real GitHub invalid credential error is clear; existing login untouched');}
 await denied('hosting-remember');
 await type('hosting-token',token);await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);assert(!state.hosting.remember);pass('existing GitHub credential connected in memory only');
 await denied('hosting-tab-create');pass('live MCP rejects credential persistence and unrelated hosting writes');
 await click('hosting-tab-publish');await form();
 if(!report.published){
  await click('hosting-publish-private');await click('hosting-publish-public-ack');await denied('hosting-publish-review');await click('hosting-publish-private');
  await type('hosting-publish-name','rocket-auth-test');await denied('hosting-publish-review');await type('hosting-publish-name',name);
  await type('hosting-publish-branch','main');await denied('hosting-publish-review');await type('hosting-publish-branch',branch);pass('live MCP refuses public creation, other repository and unmarked branch');
  git(['remote','add','reserved',remote]);await type('hosting-publish-remote','reserved');await click('hosting-publish-review');assert.match(state.hosting.error,/publish_remote_exists/);git(['remote','remove','reserved']);sameLocal();assert.equal((await api('/repos/'+fullName)).status,404);pass('existing remote is not overwritten and no GitHub repository is created');
  await form();await click('hosting-publish-review',false);await until(s=>s.hosting.busy,'preview running');const at=performance.now();await client.tool('rocket_click',{target:'hosting-cancel',expected_tab:state.active_tab});const ack=performance.now()-at;await until(s=>!s.input_pending);assert(state.hosting.stopping||!state.hosting.busy);await settled();assert(!state.hosting.publish.reviewing);assert.equal((await api('/repos/'+fullName)).status,404);sameLocal();report.cancellation.push({phase:'publish-preview',input_ack_ms:Math.round(ack),settled_ms:Math.round(performance.now()-at)});pass('live preview cancellation gives stopping feedback and creates nothing');
  await review();fs.appendFileSync(path.join(repo,'.git/config'),'\n# external synthetic config edit preserved\n');await click('hosting-publish-create');assert.match(state.hosting.error,/publish_stale/);assert.equal((await api('/repos/'+fullName)).status,404);assert(fs.readFileSync(path.join(repo,'.git/config'),'utf8').includes('external synthetic config edit preserved'));sameLocal();pass('external config change invalidates review before real GitHub creation');
  await form();await review();await capture('private-review');report.creation_intent=true;save();await click('hosting-publish-create');
  const published=state.hosting.publish.report;report.publish_report=published;save();
  assert.deepEqual([published.creation,published.remote_added,published.push],['created',true,'confirmed_success']);assert.deepEqual(published.errors,[]);
  const actual=await api('/repos/'+fullName);assert.equal(actual.status,200);assert.equal(actual.body.private,true);assert.equal(actual.body.owner.id,plan.account_id);assert.equal(actual.body.full_name,fullName);assert.equal(actual.body.description,'Disposable synthetic Rocket publication '+plan.id);
  report.repository_id=actual.body.id;report.published=true;report.cleanup_required=true;save();
  const ref=await api('/repos/'+fullName+'/git/ref/heads/'+branch);assert.equal(ref.status,200);assert.equal(ref.body.object.sha,report.before.head);sameLocal();assert.equal(git(['remote','get-url','origin']),remote);assert.equal(git(['config','branch.'+branch+'.remote']),'origin');
  await until(s=>s.tabs.some(t=>t.id===s.active_tab&&t.remotes.some(r=>r.name==='origin')&&t.upstream==='origin/'+branch),'automatic published remote refresh');await capture('published');pass('real private GitHub creation and SSH first push succeeded with upstream and preserved local edits');
 }
 await form('second-origin');if(state.hosting.publish.upstream)await click('hosting-publish-upstream');await click('hosting-publish-review');assert.match(state.hosting.error,/publish_repository_exists/);assert.equal(git(['remote']),'origin');sameLocal();await capture('repository-exists');pass('existing GitHub destination is rejected without adding or overwriting remotes');
 await click('dialog-cancel');
 // Restrict subsequent recovery push to this exact owned branch, matching MCP policy.
 git(['config','remote.origin.fetch','+refs/heads/'+branch+':refs/remotes/origin/'+branch]);
 if(!report.cancel_source){
  const index=path.join(root,'cancel.index'),extra={GIT_INDEX_FILE:index};git(['read-tree','HEAD'],{extra});
  for(let i=0;i<8;i++){const blob=git(['hash-object','-w','--stdin'],{input:crypto.randomBytes(128*1024)});git(['update-index','--add','--cacheinfo','100644,'+blob+',synthetic-transfer-'+i+'.bin'],{extra});}
  const tree=git(['write-tree'],{extra}),commit=git(['commit-tree',tree,'-p',report.before.head,'-m','Synthetic cancellation transfer']);git(['update-ref','refs/heads/'+branch,commit,report.before.head]);report.cancel_source=commit;report.cancel_local=local();save();
 }
 assert.deepEqual(local(),report.cancel_local);await click('refresh');await settled();
 if(!report.push_cancelled){
  await click('toolbar-push');assert.equal(state.dialog.kind,'push');
  const oldStarts=logs().filter(e=>e.event==='git_start'&&e.operation==='push').length;
  await click('dialog-submit',false);
  await until(s=>s.write&&logs().filter(e=>e.event==='git_start'&&e.operation==='push').length>oldStarts,'real SSH push started');
  const at=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});const ack=performance.now()-at;
  await until(s=>!s.input_pending);report.cancel_feedback={busy:state.busy,write:state.write,notice:state.notice,progress:state.progress};await settled();
  const ended=logs().filter(e=>e.event==='git_end'&&e.operation==='push').at(-1);report.cancel_git_end=ended;
  assert(ended&&/cancel/i.test(JSON.stringify(ended)),'Push completed before cancellation: inspect and retry explicitly');
  const remoteRef=await api('/repos/'+fullName+'/git/ref/heads/'+branch);assert.equal(remoteRef.status,200);assert([report.before.head,report.cancel_source].includes(remoteRef.body.object.sha));report.cancel_remote_oid=remoteRef.body.object.sha;
  report.cancellation.push({phase:'SSH push',input_ack_ms:Math.round(ack),settled_ms:Math.round(performance.now()-at),cancel_to_stop_us:ended.cancel_to_stop_us,remote_oid:report.cancel_remote_oid});
  assert.deepEqual(local(),report.cancel_local);report.push_cancelled=true;save();await capture('push-cancelled');pass('real SSH push cancellation checked against actual GitHub branch result; local edits preserved');
 }
 if(report.cancel_remote_oid===report.before.head&&!report.recovery_pushed){
  if(state.dialog)await click('dialog-cancel');await click('toolbar-push');await click('dialog-submit');
  const after=await api('/repos/'+fullName+'/git/ref/heads/'+branch);assert.equal(after.status,200);assert.equal(after.body.object.sha,report.cancel_source);assert.deepEqual(local(),report.cancel_local);report.recovery_pushed=true;save();pass('reviewed retry after confirmed unchanged remote succeeds through MCP');
 }
 const events=logs(),starts=events.filter(e=>e.event==='request_start'),outcomes=events.filter(e=>e.event==='github_publish_outcome');
 assert(outcomes.some(e=>e.creation==='created'&&e.remote_outcome==='confirmed_success'));
 assert(events.every(e=>e.dropped_before===0));for(const start of starts)assert(events.some(e=>e.event==='request_end'&&e.session===start.session&&e.request===start.request),'Unpaired diagnostic request');
 report.logs={records:events.length,publish_outcomes:outcomes.length,paired_requests:starts.length};pass('publish phases, failures and cancellation logs are paired with no dropped events or secrets');
 await verifyIdentity();pass('existing credentials, personal Git/SSH config and loaded keys remain unchanged');
 report.passed=true;
}catch(e){report.error=redact(e.stack);console.error(report.error);if(client){try{await capture('failure')}catch{}}process.exitCode=1;
}finally{
 save();
 if(client){try{if(state?.hosting?.busy)await click('hosting-cancel');if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>s.window==='closed'||s.window==='not_started'||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.window==='closed'||s.window==='not_started');}await client.raw('POST','/shutdown')}catch(e){report.cleanup_error=redact(e.message)}await client.finish();}
 if(service){for(let i=0;i<60&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null)cp.spawnSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});}
 const access=path.join(profile,'access-token');if(access.startsWith(root+path.sep)&&fs.existsSync(access))fs.unlinkSync(access);
 report.cleanup={service_exit:service?.exitCode,access_token_removed:!fs.existsSync(access),repository_deletion:'User cleanup required; existing credential lacks delete_repo',error:report.cleanup_error??null};
 try{await verifyIdentity();report.login_preserved=true}catch(e){report.identity_error=redact(e.message);report.passed=false;process.exitCode=1}
 save();token='';console.log('Evidence: '+root);
}}
run();
