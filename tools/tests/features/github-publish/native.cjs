require('node:fs').mkdirSync(require('node:path').resolve('work/results/github-publish'), {recursive:true});
// Owned disposable repositories; synthetic GitHub API; no credentials or real network Git.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto');
const http=require('node:http'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../../../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/github-publish-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'資料 project'),profile=path.join(root,'mcp-profile'),remotes=path.join(root,'publish-remotes');
const branch='rocket-ui-test/publish',token='DISPOSABLE_PUBLISH_'+crypto.randomBytes(12).toString('hex');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|GCM_|SSH_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(remotes);fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0'});
const git=(a,at=repo)=>cp.execFileSync('git',['-C',at,...a],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const sha=b=>crypto.createHash('sha256').update(b).digest('hex'),hash=p=>sha(fs.readFileSync(p));
git(['init','--initial-branch='+branch]);for(const [k,v]of[['user.name','Synthetic Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'資料.txt'),'committed history\n');git(['add','--all']);git(['commit','-m','Synthetic publication']);
fs.writeFileSync(path.join(repo,'資料.txt'),'staged draft\n');git(['add','--','資料.txt']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged edit\n');fs.writeFileSync(path.join(repo,'untracked.txt'),'PRIVATE unpublished data');
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const unchanged=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'untracked.txt')),global:hash(path.join(root,'empty.config'))});
const before=unchanged(),apiCalls=[],created=new Map();let mode='ok',afterCreate,service,client,state,apiPort,closedAt,requestsStarted=0;
const report={passed:false,root,cases:[],cancellation:[]};const delay=ms=>new Promise(r=>setTimeout(r,ms));
const server=http.createServer(async(req,res)=>{
 let data='';for await(const b of req)data+=b;
 const route=new URL(req.url,'http://127.0.0.1').pathname;
 const send=(code,body)=>{res.writeHead(code,{'Content-Type':'application/json'});res.end(JSON.stringify(body));};
 apiCalls.push({method:req.method,path:route});
 if(route==='/github/rocket/browser-sign-in'){assert.equal(req.method,'POST');return send(200,{access_token:token});}
 assert.equal(req.headers.authorization,'Bearer '+token);
 if(route==='/github/user')return send(200,{login:'fixture'});
 if(route==='/github/orgs/team')return send(200,{login:'team'});
 if(req.method==='GET'){
   if(mode==='slow-read'){requestsStarted++;res.on('close',()=>{closedAt=performance.now()});return;}
   const name=route.split('/').at(-1);return created.has(name)?send(200,created.get(name)):send(404,{message:'not found'});
 }
 assert(req.method==='POST');const draft=JSON.parse(data);assert.equal(draft.auto_init,false);assert.equal(typeof draft.private,'boolean');assert(!('gitignore_template' in draft));assert(!('license_template' in draft));
 if(mode==='forbidden')return send(403,{message:token+' PRIVATE server response'});
 if(mode==='rejected')return send(422,{message:token+' PRIVATE server response'});
 const owner=route.includes('/orgs/')?'team':'fixture';const made={id:200+created.size,full_name:owner+'/'+draft.name,private:draft.private,clone_url:'https://wrong-destination.invalid/never.git'};
 created.set(draft.name,made);requestsStarted++;
 if(afterCreate){const action=afterCreate;afterCreate=null;action();}
 if(mode==='slow-create'){res.on('close',()=>{closedAt=performance.now()});return;}
 send(201,made);
});
const pass=(name)=>{report.cases.push(name);console.log('PASS '+name);};
async function until(fn,label='state',ms=60000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return state;assert(performance.now()-start<ms,label+' timeout '+JSON.stringify({hosting:state.hosting,dialog:state.dialog?.kind}));await delay(60);}}
const control=k=>state.controls?.find(c=>c.id===k);
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.hosting?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',n=0;await until(s=>{const now=JSON.stringify(s.controls);n=now===last?n+1:0;last=now;return n>=2;},'stable');}
async function scroll(id,dir,pixels=300){await client.tool('rocket_scroll',{target:id,direction:dir,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(id){await stable();const visible=()=>control(id)&&control(id).rect[3]-control(id).rect[1]>18;if(visible())return;
 if(control('hosting-scroll')){await scroll('hosting-scroll','up',2000);for(let i=0;i<32&&!visible();i++)await scroll('hosting-scroll','down',180);}
 assert(visible(),'Control not reachable: '+id);}
async function click(id,wait=true){await reveal(id);assert(control(id).enabled,'Disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(wait)await settled();}
async function type(id,text){await reveal(id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settled();}
async function capture(name){await stable();const v=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!v.isError);const im=v.content.find(c=>c.type==='image');assert(im);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(im.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(w,h){await client.tool('rocket_resize',{width:w,height:h});await until(s=>s.viewport[0]===w&&s.viewport[1]===h&&!s.input_pending);await settled();}
async function openHosting(){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);await click('repository-remotes-menu');await stable();for(let i=0;i<15&&!(control('repository-hosting')&&control('repository-hosting').rect[3]-control('repository-hosting').rect[1]>24);i++)await scroll('repository-menu-scroll','down',190);await click('repository-hosting');await settled();}
async function destination(name,remote=name){git(['init','--bare',path.join(remotes,name+'.git')]);if(state.hosting.publish?.report)await click('hosting-publish-reset');if(state.hosting.publish?.reviewing)await click('hosting-publish-back');await type('hosting-publish-name',name);await type('hosting-publish-remote',remote);if(state.hosting.publish.upstream&&report.cases.length)await click('hosting-publish-upstream');}
async function review(){await click('hosting-publish-review');assert(state.hosting.publish.reviewing,state.hosting.error);assert.equal(state.hosting.publish.commit,before.head);}
async function verifyLocal(){assert.deepEqual(unchanged(),before);assert(!fs.existsSync(path.join(root,'rocket-ui-state/hosting-accounts.dpapi')));}
function checkRemote(name,exists=true){const result=cp.spawnSync('git',['-C',path.join(remotes,name+'.git'),'rev-parse','--verify','refs/heads/'+branch],{env,windowsHide:true,encoding:'utf8'});if(exists){assert.equal(result.status,0,result.stderr);assert.equal(result.stdout.trim(),before.head);}else assert.notEqual(result.status,0);}
async function cancel(kind){
 const started=requestsStarted;closedAt=null;
 if(kind==='preview')await click('hosting-publish-review',false);else await click('hosting-publish-create',false);
 await until(s=>s.hosting.busy&&requestsStarted>started,'HTTP gate');
 const at=performance.now();await client.tool('rocket_click',{target:'hosting-cancel',expected_tab:state.active_tab});const ack=performance.now()-at;
 await until(s=>!s.input_pending);const feedback=state.hosting.stopping||!state.hosting.busy;assert(feedback,'No stopping feedback');await settled();
 for(let i=0;i<50&&closedAt===null;i++)await delay(20);assert(closedAt!==null,'HTTP connection was not closed');report.cancellation.push({kind,input_ack_ms:Math.round(ack),socket_stop_ms:Math.round(closedAt-at),settled_ms:Math.round(performance.now()-at)});
}
async function run(){try{
 await new Promise(r=>server.listen(0,'127.0.0.1',r));apiPort=server.address().port;fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+apiPort);
 const exe=path.resolve(process.env.ROCKET_PUBLISH_EXE||'target/release/rocket-mcp.exe');report.sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>100)throw e;await delay(100);}}
 report.version=(await client.initialize()).serverInfo.version;await client.tool('rocket_open',{path:repo});await settled();await resize(1420,980);if(control('console-close'))await click('console-close');await openHosting();
 assert(!state.hosting.remember);await click('hosting-browser');assert(state.hosting.connected,state.hosting.error);await click('hosting-tab-publish');
 await destination('demo','origin');assert(state.hosting.publish.private);await type('hosting-publish-description','PRIVATE synthetic description 界');await review();await capture('private-review');await click('hosting-publish-create');
 assert.deepEqual([state.hosting.publish.report.creation,state.hosting.publish.report.remote_added,state.hosting.publish.report.push],['created',true,'confirmed_success']);assert.deepEqual(state.hosting.publish.report.errors,[]);checkRemote('demo');await verifyLocal();await capture('published');pass('private creation, reviewed first push, upstream and local Unicode edits preserved');
 await until(s=>s.tabs.some(t=>t.id===s.active_tab&&t.remotes.some(r=>r.name==='origin')&&t.upstream==='origin/'+branch),'published remote and upstream refresh',10000);pass('published remote and upstream appear without manual refresh');
 if(process.env.ROCKET_PUBLISH_REFRESH_ONLY==='1'){report.passed=true;return;}
 assert.equal(git(['remote','get-url','origin']),path.join(remotes,'demo.git').replaceAll('\\','/'));pass('API clone URL ignored; transport stayed in the local fixture');
 await destination('competing');const competing=git(['commit-tree',git(['rev-parse','HEAD^{tree}']),'-p',before.head,'-m','Concurrent fixture history']);git(['push',path.join(remotes,'competing.git'),competing+':refs/heads/'+branch]);await review();await click('hosting-publish-create');assert.equal(state.hosting.publish.report.creation,'created');assert.equal(state.hosting.publish.report.push,'rejected');assert.equal(git(['rev-parse','refs/heads/'+branch],path.join(remotes,'competing.git')),competing);await verifyLocal();pass('first push rejects a competing remote branch and retains recovery configuration');
 await destination('collision','origin');const posts=apiCalls.filter(x=>x.method==='POST').length;await click('hosting-publish-review');assert(state.hosting.error.includes('publish_remote_exists'),state.hosting.error);assert.equal(apiCalls.filter(x=>x.method==='POST').length,posts);await verifyLocal();pass('existing remote collision refused before creation');
 await type('hosting-publish-branch','main');await reveal('hosting-publish-review');const denied=await client.request('tools/call',{name:'rocket_click',arguments:{target:'hosting-publish-review',expected_tab:state.active_tab}});assert(denied.isError,'Unmarked branch must be refused');assert.equal(apiCalls.filter(x=>x.method==='POST').length,posts);await type('hosting-publish-branch',branch);pass('MCP refuses publishing an unmarked branch');
 await destination('stale');await review();const config=path.join(repo,'.git/config');fs.appendFileSync(config,'\n# external config edit must survive\n');await click('hosting-publish-create');assert(state.hosting.error.includes('publish_stale'),state.hosting.error);assert(!created.has('stale'));assert(fs.readFileSync(config,'utf8').includes('external config edit'));checkRemote('stale',false);await verifyLocal();pass('external config edit invalidates review without creation');
 await destination('forbidden');mode='forbidden';await review();await click('hosting-publish-create');assert.equal(state.hosting.publish.report.creation,'rejected');assert(state.hosting.publish.report.errors.some(x=>x.includes('hosting_forbidden')));assert(!state.hosting.publish.report.remote_added);checkRemote('forbidden',false);await capture('permission-rejected');await verifyLocal();mode='ok';pass('creation permission rejection has safe error and no local mutation');
 await destination('partial');await review();afterCreate=()=>fs.appendFileSync(config,'\n# external after creation\n');await click('hosting-publish-create');assert.equal(state.hosting.publish.report.creation,'created');assert(!state.hosting.publish.report.remote_added);assert(state.hosting.publish.report.errors.some(x=>x.includes('publish_stale')));checkRemote('partial',false);await verifyLocal();pass('creation retained and explained when external edits block attachment');
 await destination('cancel-read');mode='slow-read';await cancel('preview');assert(!state.hosting.publish.reviewing);assert(!created.has('cancel-read'));await verifyLocal();mode='ok';pass('preview cancellation stops promptly without a create request');
 await destination('cancel-create');await review();mode='slow-create';await cancel('create');assert.equal(state.hosting.publish.report.creation,'unknown');assert(!state.hosting.publish.report.remote_added);assert(!state.hosting.publish.report.push);checkRemote('cancel-create',false);await capture('cancelled-unknown');const creates=apiCalls.filter(x=>x.method==='POST').length;await delay(300);await settled();assert.equal(apiCalls.filter(x=>x.method==='POST').length,creates);await verifyLocal();mode='ok';pass('submitted creation cancellation reports unknown, keeps result and never retries');
 await destination('public-org');await type('hosting-publish-owner','team');await click('hosting-publish-private');await reveal('hosting-publish-review');assert(!control('hosting-publish-review').enabled);await click('hosting-publish-public-ack');await click('hosting-publish-ssh');await review();await capture('public-org-review');await click('hosting-publish-create');assert.equal(state.hosting.publish.report.push,'confirmed_success');assert.equal(created.get('public-org').private,false);assert.equal(created.get('public-org').full_name,'team/public-org');checkRemote('public-org');await verifyLocal();pass('organization/public acknowledgement and SSH choice use the reviewed branch');
 await click('hosting-publish-reset');await resize(780,620);await reveal('hosting-publish-name');assert(control('hosting-publish-name').rect[2]<=state.viewport[0]-15,'Compact field clips at the right edge');await capture('compact');await reveal('hosting-publish-review');assert(control('hosting-publish-review'));await verifyLocal();pass('compact publishing form fits and stays reachable');
 let events=[];for(const folder of fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')))for(const n of fs.readdirSync(path.join(root,folder)).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n))){const data=fs.readFileSync(path.join(root,folder,n),'utf8');for(const sentinel of [token,'PRIVATE synthetic','PRIVATE unpublished','wrong-destination.invalid','fixture@example.invalid'])assert(!data.includes(sentinel),'Diagnostic leak: '+sentinel);events.push(...data.trim().split('\n').filter(Boolean).map(x=>JSON.parse(x)));}
 const outcomes=events.filter(e=>e.event==='github_publish_outcome');assert(outcomes.some(e=>e.creation==='unknown'));assert(outcomes.some(e=>e.creation==='rejected'));assert(outcomes.some(e=>e.remote_outcome==='confirmed_success'));assert(events.some(e=>e.operation==='github-publish-preview'&&e.event==='request_start'));assert(events.every(e=>e.dropped_before===0));const starts=events.filter(e=>e.event==='request_start');for(const start of starts)assert(events.some(e=>e.event==='request_end'&&e.session===start.session&&e.request===start.request),'unpaired request');report.logs={records:events.length,publish_outcomes:outcomes.length,paired_requests:starts.length};pass('named phase/outcome logs contain no token or private payload');
 report.passed=true;
}catch(e){report.error=e.stack;console.error(e.stack);if(client){try{await capture('failure')}catch{}}process.exitCode=1;
}finally{
 report.api_calls=apiCalls;fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.resolve('work/results/github-publish/native-latest.json'),JSON.stringify(report,null,2));
 if(client){try{if(state?.hosting?.busy)await click('hosting-cancel');if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>s.window==='closed'||s.window==='not_started'||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.window==='closed'||s.window==='not_started');}await client.raw('POST','/shutdown')}catch(e){report.cleanup_error=e.message;}await client.finish();}
 if(service){for(let i=0;i<60&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null)cp.spawnSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});}
 server.closeAllConnections();await new Promise(r=>server.close(r));const access=path.join(profile,'access-token');if(access.startsWith(root+path.sep)&&fs.existsSync(access))fs.unlinkSync(access);report.cleanup={service_exit:service?.exitCode,access_token_removed:!fs.existsSync(access),error:report.cleanup_error??null};fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.resolve('work/results/github-publish/native-latest.json'),JSON.stringify(report,null,2));console.log('Evidence: '+root);
}}
run();
