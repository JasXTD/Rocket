// Hosting acceptance only. Synthetic local HTTP APIs + one owned temporary repo.
// No real accounts, credentials, SSH keys or provider writes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto');
const http=require('node:http'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/hosting-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'PRIVATE_REPO'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/hosting';
const token='fixture_HOSTING_'+crypto.randomBytes(12).toString('hex');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0',http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:''});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const sha=b=>crypto.createHash('sha256').update(b).digest('hex'),hash=p=>sha(fs.readFileSync(p));
git(['init','--initial-branch='+branch]);for(const [k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'資料.txt'),'committed\n');git(['add','-A']);git(['commit','-m','Synthetic fixture']);
fs.writeFileSync(path.join(repo,'資料.txt'),'staged\n');git(['add','-A']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');
fs.writeFileSync(path.join(repo,'private-untracked.txt'),'preserve me');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const before={head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'private-untracked.txt'))};
const requests=[],created={};let mode='ok',changed=false,service,client,state,apiPort,closedAt=null;
const phase=process.env.ROCKET_HOSTING_PHASE||"all";assert(["all","recovery"].includes(phase));
const report={passed:false,phase,root,cases:[],cancel:[]};const delay=ms=>new Promise(r=>setTimeout(r,ms));
const item=(p,id,title='PRIVATE 改善 Unicode 🚀')=>{
 const common={title,state:p==='gitlab'?'opened':p==='bitbucket'?'OPEN':'open'};
 if(p==='gitlab')return {...common,iid:id,author:{username:'fixture'},source_branch:'feature/界',target_branch:'main'};
 if(p==='bitbucket')return {...common,id,author:{display_name:'fixture'},source:{branch:{name:'feature/界'}},destination:{branch:{name:'main'}}};
 if(p==='azure')return {...common,pullRequestId:id,status:'active',createdBy:{displayName:'fixture'},sourceRefName:'refs/heads/feature/界',targetRefName:'refs/heads/main'};
 return {...common,number:id,user:{login:'fixture'},head:{ref:'feature/界'},base:{ref:'main'}};
};
const server=http.createServer(async(req,res)=>{
 let data='';for await(const chunk of req)data+=chunk;
 const url=new URL(req.url,'http://127.0.0.1'),p=url.pathname.split('/')[1],route=url.pathname.slice(p.length+1);
 const wanted=p==='azure'?'Basic '+Buffer.from(':'+token).toString('base64'):p==='bitbucket'?'Basic '+Buffer.from('fixture@example.invalid:'+token).toString('base64'):p==='gitea'?'token '+token:'Bearer '+token;
 const call={p,method:req.method,path:url.pathname,query:url.search,body:data,auth:req.headers.authorization===wanted||(p==='bitbucket'&&req.headers.authorization==='Bearer '+token),auth_kind:req.headers.authorization?.split(' ')[0]};requests.push(call);
 const send=(code,body,headers={})=>{res.writeHead(code,{'Content-Type':'application/json',...headers});res.end(JSON.stringify(body));};
 if(!call.auth)return send(401,{message:token});
 if(mode==='401')return send(401,{message:token});
 if(route==='/user'||route.includes('connectionData'))return send(200,p==='gitlab'?{username:'fixture'}:p==='bitbucket'?{display_name:'fixture'}:p==='azure'?{authenticatedUser:{providerDisplayName:'fixture'}}:{login:'fixture'});
 const isBranch=route.includes('/branches/')||route.endsWith('/refs');
 if(isBranch){
   const main=req.url.includes('main'),id=(main?'b':changed?'c':'a').repeat(40);
   return send(200,p==='github'?{commit:{sha:id}}:p==='gitlab'||p==='gitea'?{commit:{id}}:p==='bitbucket'?{target:{hash:id}}:{value:[{name:'refs/heads/'+(main?'main':'feature/界'),objectId:id}]});
 }
 if(mode==='slow'||mode==='slow-create'&&req.method==='POST'){
   if(mode==='slow-create')created[p]=item(p,99);
   res.on('close',()=>{closedAt=performance.now();});return;
 }
 if(mode==='redirect'){res.writeHead(302,{Location:'http://127.0.0.1:1/never'});return res.end();}
 if(/^[45]\d\d$/.test(mode))return send(Number(mode),{message:token+' PRIVATE SERVER BODY'},mode==='429'?{'Retry-After':'2'}:{});
 if(p==='azure'&&route.includes('/wit/wiql')){assert(req.method==='POST');const q=JSON.parse(data).query;assert(q.includes('[System.TeamProject]'));if(q.includes(" = 'Done'"))return send(200,{workItems:[]});return send(200,{workItems:[{id:18}]});}
 if(p==='azure'&&route.includes('/wit/workitems'))return send(200,{value:[{id:18,fields:{'System.Title':'PRIVATE work item 界','System.State':'New','System.CreatedBy':{displayName:'fixture'}}}]});
 if(req.method==='POST'){
    const body=JSON.parse(data);assert(body.title==='PRIVATE 改善 Unicode 🚀');assert(body.description?.includes('line')||body.body?.includes('line'));
    const source=p==='gitlab'?body.source_branch:p==='bitbucket'?body.source.branch.name:p==='azure'?body.sourceRefName:body.head;
    const target=p==='gitlab'?body.target_branch:p==='bitbucket'?body.destination.branch.name:p==='azure'?body.targetRefName:body.base;
    assert.equal(source,p==='azure'?'refs/heads/feature/界':'feature/界');assert.equal(target,p==='azure'?'refs/heads/main':'main');
    created[p]=item(p,43);return send(201,created[p]);
 }
 const issues=route.endsWith('/issues'),page=url.searchParams.get('cursor')==='PRIVATE_CURSOR+/2'?2:Number(url.searchParams.get('page')||1),closed=url.search.includes('closed')||url.search.includes('DECLINED');
 const pageSize=['gitlab','gitea'].includes(p)?2:50;
 let rows=[];
 if(!closed){
    if(issues)rows=[{...item(p,18,'PRIVATE issue 界'),source_branch:undefined,target_branch:undefined}];
    else if(page===1&&Number(url.searchParams.get('$skip')||0)===0)rows=Array.from({length:pageSize},(_,i)=>item(p,i+1)),rows[0]=created[p]||rows[0];
    else rows=[item(p,51,'PRIVATE page two')];
 }
 const nextUrl=new URL(req.url,'http://'+req.headers.host);
 if(p==='bitbucket'){nextUrl.searchParams.delete('page');nextUrl.searchParams.set('cursor','PRIVATE_CURSOR+/2');}
 else nextUrl.searchParams.set('page','2');
 if(['gitlab','gitea'].includes(p))nextUrl.searchParams.set(p==='gitlab'?'per_page':'limit','2');
 const next=mode==='unsafe-link'?'http://127.0.0.1:1/never-follow':(!issues&&rows.length===pageSize&&page===1?nextUrl.href:undefined);
 send(200,p==='bitbucket'?{values:rows,...(next?{next}:{})}:p==='azure'?{value:rows}:rows,next&&p!=='azure'&&p!=='bitbucket'?{'Link':'<'+next+'>; rel="next"'}:{});
});
const pass=s=>{report.cases.push(s);console.error('PASS '+s);};
async function until(fn,label='state'){const at=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return state;assert(performance.now()-at<45000,label+' timeout '+JSON.stringify({hosting:state.hosting,dialog:state.dialog?.kind}));await delay(65);}}
const control=k=>state.controls?.find(c=>c.id===k);
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.hosting?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2;},'stable layout');}
async function scroll(region,direction,pixels=300){await client.tool('rocket_scroll',{target:region,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(k){await stable();if(control(k)&&control(k).rect[3]-control(k).rect[1]>18)return;
 if(control('hosting-scroll')){const visible=()=>control(k)&&control(k).rect[3]-control(k).rect[1]>18;for(let i=0;i<6&&!visible();i++){const previous=JSON.stringify(state.controls);await scroll('hosting-scroll','up',2000);if(previous===JSON.stringify(state.controls))break;}for(let i=0;i<40&&!visible();i++){const previous=JSON.stringify(state.controls);await scroll('hosting-scroll','down',240);if(previous===JSON.stringify(state.controls))break;}}
 assert(control(k)&&control(k).rect[3]-control(k).rect[1]>18,'unreachable '+k);}
async function click(k,wait=true){await reveal(k);assert(control(k)?.enabled,'disabled '+k);await client.tool('rocket_click',{target:k,expected_tab:state.active_tab});if(wait)await settled();}
async function type(k,text){await reveal(k);assert(control(k)?.enabled,'disabled '+k);await client.tool('rocket_type',{target:k,text,expected_tab:state.active_tab});await settled();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const image=r.content.find(x=>x.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(w,h){await client.tool('rocket_resize',{width:w,height:h});await until(s=>s.viewport[0]===w&&s.viewport[1]===h&&!s.input_pending);await settled();}
async function openHosting(){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);await click('repository-remotes-menu');await stable();for(let i=0;i<12&&!(control('repository-hosting')&&control('repository-hosting').rect[3]-control('repository-hosting').rect[1]>25);i++)await scroll('repository-menu-scroll','down',230);await click('repository-hosting');await until(s=>s.dialog?.kind==='hosting'&&!s.hosting.busy);await settled();}
async function connect(p,remember=false,username=p==='bitbucket'?'':'fixture@example.invalid'){
 await click('hosting-tab-accounts');await click('hosting-provider-'+p);
 await type('hosting-label','Fixture '+p);
 if(p==='azure')await type('hosting-server','https://dev.azure.com/fixture');
 await type('hosting-username',username);await type('hosting-token',token);
 if(state.hosting.remember!==remember)await click('hosting-remember');
 await click('hosting-connect');assert(state.hosting.connected,JSON.stringify(state.hosting));assert(!state.hosting.error);
}
async function preview(){
 await click('hosting-tab-create');await type('hosting-repository','team/資料');await type('hosting-source','feature/界');await type('hosting-target','main');
 await type('hosting-title','PRIVATE 改善 Unicode 🚀');await type('hosting-body','Quoted "line"\nSecond line \\ path');
 await click('hosting-review');assert(state.hosting.reviewing,state.hosting.error);assert.equal(state.hosting.review.source_id,'a'.repeat(40));
}
async function closeWindow(){
 if(state?.hosting?.busy){await click('hosting-cancel');}
 if(state?.dialog)await click('dialog-cancel');
 await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));
 if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}
}
async function run(){
 try{
 await new Promise(r=>server.listen(0,'127.0.0.1',r));apiPort=server.address().port;fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+apiPort);
 const exe=path.resolve(process.env.ROCKET_HOSTING_EXE||'target/release/rocket-mcp.exe');report.sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const out=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>100)throw e;await delay(100);}}
 report.version=(await client.initialize()).serverInfo.version;
 await client.tool('rocket_open',{path:repo});await settled();await resize(1480,920);if(control('console-close'))await click('console-close');await openHosting();
 if(phase==='all'){
 for(const p of ['github','gitlab','bitbucket','azure','gitea']){
    await connect(p,p==='github');await type('hosting-repository','team/資料');await click('hosting-refresh');
    const firstCount=['gitlab','gitea'].includes(p)?2:50;
    assert.equal(state.hosting.count,firstCount);assert(state.hosting.more);
    await click('hosting-more');assert.equal(state.hosting.count,firstCount+1);assert(!state.hosting.more);
    if(p==='bitbucket')assert(requests.some(r=>r.p===p&&r.query.includes('cursor=PRIVATE_CURSOR%2B%2F2')));
    await type('hosting-filter','page two');assert.deepEqual(state.hosting.visible,[firstCount]);await type('hosting-filter','');
    await click('hosting-state-closed');assert.equal(state.hosting.count,0);await click('hosting-refresh');
    if(p!=='azure')assert.equal(state.hosting.count,0);
    await click('hosting-state-open');await click('hosting-refresh');
    await capture(p+'-pulls');
    const links=(state.github?.opened_urls||state.opened_urls||[]).length;
    await click('hosting-item-0');
    assert(state.github?.opened_urls?.length>links||state.opened_urls?.length>links,'recorded provider link');
    await click('hosting-tab-issues');
    if(p==='bitbucket'){assert(!control('hosting-refresh'));}
    else{await click('hosting-refresh');assert.equal(state.hosting.count,1);if(p==='azure'){await type('hosting-work-state','Done');await click('hosting-refresh');assert.equal(state.hosting.count,0);}}
    await preview();const beforePosts=requests.filter(x=>x.method==='POST'&&!x.path.includes('/wit/wiql')).length;
    await capture(p+'-review');await click('hosting-create');assert.equal(state.hosting.created,43);assert(!state.hosting.error);
    assert.equal(requests.filter(x=>x.method==='POST'&&!x.path.includes('/wit/wiql')).length,beforePosts+1);
    pass(p+': verified account, pages, state/text filters, issues capability and reviewed create');
 }
 // Concurrent branch change refuses before a POST.
 await preview();changed=true;const count=requests.filter(r=>r.method==='POST').length;await click('hosting-create');
 assert(state.hosting.error.includes('hosting_review_stale'));assert.equal(requests.filter(r=>r.method==='POST').length,count);changed=false;
 pass('Concurrent remote branch change invalidates review without a create request');
 await click('hosting-tab-pulls');
 for(const [m,code]of[['403','forbidden'],['404','not_found'],['429','rate_limited'],['redirect','invalid_response']]){
   mode=m;await click('hosting-refresh');assert(state.hosting.error.includes('hosting_'+code));assert(!state.hosting.error.includes(token));
 }mode='ok';await click('hosting-refresh');pass('Permission, missing resource, rate limit and redirect errors are clear and sanitized');
 await preview();mode='422';await click('hosting-create');assert(state.hosting.error.includes('hosting_rejected'));assert(!state.hosting.uncertain);mode='ok';
 pass('Server validation rejection preserves an editable form and does not claim creation');
 }else{await connect('github',true);await connect('gitea');await type('hosting-repository','team/資料');await click('hosting-refresh');}
 // Invalid continuation links retain earlier rows and expose a payload-free error.
 await click('hosting-tab-pulls');mode='ok';await click('hosting-refresh');const retained=state.hosting.items;
 mode='unsafe-link';const invalidSeen=requests.length;await click('hosting-refresh');
 assert(state.hosting.error.includes('hosting_invalid_pagination'));assert.deepEqual(state.hosting.items,retained);assert.equal(requests.length,invalidSeen+1);
 await capture('unsafe-pagination');mode='ok';await click('hosting-refresh');
 pass('Unsafe pagination link refused before follow-up, earlier rows preserved and clear error shown');
 // Cancellation of Load next page retains both rows and a retryable cursor.
 const beforeNext=state.hosting.items;mode='slow';closedAt=null;const seen=requests.length;await click('hosting-more',false);await until(s=>s.hosting.busy&&requests.length>seen);
 await reveal('hosting-cancel');let at=performance.now();await client.tool('rocket_click',{target:'hosting-cancel',expected_tab:state.active_tab});await until(s=>s.hosting.stopping||!s.hosting.busy);const ack=performance.now()-at;await settled();
 assert(!state.hosting.uncertain);assert(state.hosting.notice.includes('Cancelled'));assert.deepEqual(state.hosting.items,beforeNext);assert(state.hosting.more);for(let i=0;i<60&&closedAt===null;i++)await delay(50);assert(closedAt!==null);report.cancel.push({kind:'next-page',ack_ms:ack,disconnect_ms:closedAt-at});mode='ok';await click('hosting-more');assert(!state.hosting.more);
 // Submission cancellation after server receives and accepts it.
 await preview();mode='slow-create';closedAt=null;const posts=requests.filter(r=>r.method==='POST').length;await click('hosting-create',false);await until(s=>s.hosting.busy&&requests.filter(r=>r.method==='POST').length>posts);
 await reveal('hosting-cancel');at=performance.now();await client.tool('rocket_click',{target:'hosting-cancel',expected_tab:state.active_tab});await until(s=>s.hosting.stopping||!s.hosting.busy);const ack2=performance.now()-at;await settled();
 assert(state.hosting.uncertain);assert(state.hosting.error.includes('hosting_outcome_unknown'));assert(!control('hosting-review')?.enabled);
 for(let i=0;i<60&&closedAt===null;i++)await delay(50);assert(closedAt!==null);report.cancel.push({kind:'create',ack_ms:ack2,disconnect_ms:closedAt-at});mode='ok';await capture('unknown-outcome');
 await click('hosting-tab-pulls');await type('hosting-repository','team/unrelated');await click('hosting-refresh');
 await reveal('hosting-acknowledge');assert(!control('hosting-acknowledge').enabled,'wrong repository cannot clear uncertain outcome');
 await type('hosting-repository','team/資料');await click('hosting-refresh');assert(state.hosting.items.some(i=>i.number===99));await click('hosting-acknowledge');assert(!state.hosting.uncertain);
 pass('Cancellation stops curl, retains prior reads and reports uncertain submission; remote outcome must be checked before another review');
 // Native compact layout and remembered accounts.
 await click('hosting-tab-accounts');await resize(780,620);await reveal('hosting-connect');await capture('accounts-compact');await resize(1480,920);
 const vault=path.join(root,'rocket-ui-state/hosting-accounts.dpapi');assert(fs.existsSync(vault));assert(!fs.readFileSync(vault).includes(Buffer.from(token)));
 await click('hosting-load-accounts');assert.equal(state.hosting.accounts_count,1);await click('hosting-account-0');await click('hosting-connect');assert(state.hosting.connected);
 mode='401';await click('hosting-refresh');assert(state.hosting.error.includes('hosting_unauthorized'));assert(!state.hosting.connected);mode='ok';
 await click('hosting-load-accounts');await click('hosting-account-0');await click('hosting-forget');assert.equal(state.hosting.accounts_count,0);
 pass('Protected profile roundtrip, disconnect after unauthorized token and forgetting only Rocket profile; compact controls reachable');
 // Marker removed: no new window can call any hosting API.
 await closeWindow();fs.unlinkSync(path.join(root,'rocket-hosting-api'));const requestCount=requests.length;
 await client.tool('rocket_open',{path:repo});await settled();await resize(1480,920);if(control('console-close'))await click('console-close');await openHosting();assert(!state.hosting.enabled);
 const denied=await client.request('tools/call',{name:'rocket_click',arguments:{target:'hosting-provider-gitlab',expected_tab:state.active_tab}});assert(denied.isError);assert.equal(requests.length,requestCount);
 await closeWindow();await client.raw('POST','/shutdown');
 assert.deepEqual({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'private-untracked.txt'))},before);
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(x=>/^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x=>fs.readFileSync(path.join(root,n,x),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const logs=JSON.stringify(records);
 for(const secret of[token,'PRIVATE','Authorization','Bearer','team/','fixture@example.invalid','dev.azure.com','never-follow'])assert(!logs.includes(secret),'diagnostic leak '+secret);
 assert(records.some(r=>r.failure_code==='hosting_invalid_pagination'));
 for(const op of['hosting-connect','hosting-list','hosting-preview','hosting-create','hosting-accounts']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length);for(const s of starts)assert(records.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request));}
 const expectedCodes=phase==='all'?['hosting_unauthorized','hosting_forbidden','hosting_not_found','hosting_rate_limited','hosting_rejected','hosting_review_stale','hosting_outcome_unknown']:['hosting_unauthorized','hosting_outcome_unknown'];
 for(const code of expectedCodes)assert(records.some(r=>r.failure_code===code),'missing code '+code);
 assert(requests.every(r=>r.auth));report.logs=records.length;report.requests=requests.length;pass('Default-off loopback policy, unchanged repository snapshots, paired diagnostic spans with failure codes and no secrets');report.passed=true;
 }catch(e){report.failure=e.stack;try{await capture('failure');}catch{}throw e;}
 finally{
  fs.writeFileSync('work/hosting-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));
  fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));console.log(JSON.stringify(report,null,2));
  if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}
  if(client)await client.finish();server.closeAllConnections();server.close();
 }
}
run().catch(e=>{console.error(e);process.exitCode=1});
