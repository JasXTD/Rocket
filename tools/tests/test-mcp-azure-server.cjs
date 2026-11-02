// Focused Azure DevOps Server acceptance. Only an owned dirty repository,
// synthetic loopback APIs and a new protected profile. No hosted service access.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto');
const http=require('node:http'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/azure-server-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'PRIVATE_REPO'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/azure-server';
const token='fixture_AZURE_'+crypto.randomBytes(12).toString('hex'),collection='https://devops.example.com/tfs/DefaultCollection';
const prefix='/azure-server/tfs/DefaultCollection',repository='team/資料';
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0',http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:''});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const sha=b=>crypto.createHash('sha256').update(b).digest('hex'),hash=p=>sha(fs.readFileSync(p));
git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'資料.txt'),'committed\n');git(['add','-A']);git(['commit','-m','Synthetic Azure fixture']);
fs.writeFileSync(path.join(repo,'資料.txt'),'staged\n');git(['add','-A']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');
fs.writeFileSync(path.join(repo,'private-untracked.txt'),'preserve me');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const snapshot=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'private-untracked.txt'))});
const before=snapshot(),requests=[];let mode='ok',changed=false,created,service,client,state,closedAt=null,apiFailure;
const creationOnly=process.argv.includes('--creation-only');
const report={passed:false,phase:creationOnly?'creation':'all',root,cases:[],cancel:[]},delay=ms=>new Promise(r=>setTimeout(r,ms));
const identity={authenticatedUser:{id:'11111111-2222-3333-4444-555555555555',providerDisplayName:'Fixture User',isActive:true,isContainer:false}};
const item=(id,status='active')=>({pullRequestId:id,title:'PRIVATE 改善 Unicode 🚀',status,createdBy:{displayName:'Fixture User'},sourceRefName:'refs/heads/feature/界',targetRefName:'refs/heads/main'});
const server=http.createServer(async(req,res)=>{try{
 let body='';for await(const chunk of req){body+=chunk;assert(body.length<128*1024);}
 const url=new URL(req.url,'http://127.0.0.1'),cloud=url.pathname.startsWith('/azure/');
 const call={method:req.method,path:url.pathname,query:url.search,body,auth:req.headers.authorization==='Basic '+Buffer.from(':'+token).toString('base64')};requests.push(call);
 const send=(code,value,headers={})=>{res.writeHead(code,{'Content-Type':'application/json',...headers});res.end(JSON.stringify(value));};
 const hold=()=>{res.on('close',()=>{closedAt=performance.now();});};
 assert(call.auth);assert(cloud||url.pathname.startsWith(prefix+'/'),'wrong collection');
 const auth=url.pathname.endsWith('/_apis/connectionData');
 if(auth){
   assert.equal(req.method,'GET');assert.equal(url.search,cloud?'?api-version=7.1-preview.1':'');
   if(mode==='hold-auth')return hold();
   if(/^auth-\d+$/.test(mode))return send(Number(mode.slice(5)),{message:token+' PRIVATE ERROR'},mode==='auth-302'?{Location:'https://outside.invalid/login'}:mode==='auth-429'?{'Retry-After':'2'}:{});
   if(mode==='auth-html'){res.writeHead(200,{'Content-Type':'text/html'});return res.end('<html>PRIVATE '+token+'</html>');}
   if(mode==='auth-anonymous')return send(200,{authenticatedUser:{...identity.authenticatedUser,providerDisplayName:'anonymous'}});
   return send(200,identity);
 }
 assert(['6.0','7.0','7.1'].includes(url.searchParams.get('api-version')),'missing chosen API version');
 const isBranch=url.pathname.endsWith('/refs'),isCreate=req.method==='POST'&&url.pathname.endsWith('/pullrequests');
 if(isBranch){const main=url.searchParams.get('filter')==='heads/main';assert(main||url.searchParams.get('filter')==='heads/feature/界');return send(200,{value:[{name:'refs/heads/'+(main?'main':'feature/界'),objectId:(main?'b':changed?'c':'a').repeat(40)}]});}
 if(mode==='hold-list'&&!isCreate)return hold();
 if(mode==='hold-create'&&isCreate){created=item(99);return hold();}
 if(mode==='rejected'&&isCreate)return send(422,{message:token+' PRIVATE ERROR'});
 if(mode==='read-error')return send(503,{message:token+' PRIVATE ERROR'});
 if(url.pathname.endsWith('/wiql')){
   assert.equal(req.method,'POST');const q=JSON.parse(body).query;assert(q.includes("[System.TeamProject] = 'team'"));
   const ids=q.includes("[System.State] = 'Done'")?[]:q.includes('[System.Id] > 50')?[51]:Array.from({length:50},(_,i)=>i+1);
   return send(200,{workItems:ids.map(id=>({id}))});
 }
 if(url.pathname.endsWith('/workitems')){
   assert.equal(req.method,'GET');assert(url.searchParams.get('fields').includes('System.TeamProject'));
   const ids=url.searchParams.get('ids').split(',').map(Number);
   return send(200,{value:ids.map(id=>({id,fields:{'System.Title':'PRIVATE work item 界','System.State':'Active','System.TeamProject':mode==='wrong-project'?'other':'team','System.CreatedBy':{displayName:'Fixture User'}}}))});
 }
 assert(url.pathname.endsWith('/pullrequests'));assert(url.pathname.includes('/team/_apis/git/repositories/%E8%B3%87%E6%96%99')||url.pathname.includes('/team/_apis/git/repositories/unrelated'));
 if(isCreate){const d=JSON.parse(body);assert.equal(d.sourceRefName,'refs/heads/feature/界');assert.equal(d.targetRefName,'refs/heads/main');assert.equal(d.title,'PRIVATE 改善 Unicode 🚀');assert(d.description.includes('line'));created=item(43);return send(201,created);}
 assert.equal(req.method,'GET');const skip=Number(url.searchParams.get('$skip'));
 const rows=skip===50?[item(51)]:Array.from({length:50},(_,i)=>item(i+1));if(created&&skip===0)rows[0]=created;
 if(url.searchParams.get('searchCriteria.status')==='all')rows[1]=item(2,'completed');
 return send(200,{value:rows},mode==='unsafe-link'?{Link:'<http://127.0.0.1:1/never-follow>; rel="next"'}:{});
 }catch(e){apiFailure=e;res.writeHead(500);res.end('{}');}
});
const pass=s=>{report.cases.push(s);console.error('PASS '+s);};
async function until(fn,label='state'){const at=performance.now();for(;;){if(apiFailure)throw apiFailure;state=await client.tool('rocket_state');if(fn(state))return state;assert(performance.now()-at<45000,label+' timeout '+JSON.stringify({hosting:state.hosting,dialog:state.dialog?.kind}));await delay(65);}}
const control=k=>state.controls?.find(c=>c.id===k);
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.hosting?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2;},'stable layout');}
async function scroll(region,direction,pixels=300){await client.tool('rocket_scroll',{target:region,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(k){await stable();const visible=()=>control(k)&&control(k).rect[3]-control(k).rect[1]>18;if(visible())return;
 if(control('hosting-scroll')){for(let i=0;i<6&&!visible();i++){const old=JSON.stringify(state.controls);await scroll('hosting-scroll','up',2000);if(old===JSON.stringify(state.controls))break;}for(let i=0;i<50&&!visible();i++){const old=JSON.stringify(state.controls);await scroll('hosting-scroll','down',280);if(old===JSON.stringify(state.controls))break;}}
 assert(visible(),'unreachable '+k);}
async function click(k,wait=true){await reveal(k);assert(control(k)?.enabled,'disabled '+k);await client.tool('rocket_click',{target:k,expected_tab:state.active_tab});if(wait)await settled();}
async function type(k,text){await reveal(k);assert(control(k)?.enabled,'disabled '+k);await client.tool('rocket_type',{target:k,text,expected_tab:state.active_tab});await settled();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const image=r.content.find(x=>x.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(w,h){await client.tool('rocket_resize',{width:w,height:h});await until(s=>s.viewport[0]===w&&s.viewport[1]===h&&!s.input_pending);await settled();}
async function openHosting(){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);await click('repository-remotes-menu');await stable();for(let i=0;i<12&&!(control('repository-hosting')&&control('repository-hosting').rect[3]-control('repository-hosting').rect[1]>25);i++)await scroll('repository-menu-scroll','down',230);await click('repository-hosting');await until(s=>s.dialog?.kind==='hosting'&&!s.hosting.busy);await settled();}
async function account(version='7.0',remember=false){await click('hosting-tab-accounts');await click('hosting-provider-azure-server');await type('hosting-label','Fixture Azure Server');await type('hosting-server',collection);await click('hosting-azure-api-'+version.replace('.','-'));await type('hosting-token',token);if(state.hosting.remember!==remember)await click('hosting-remember');}
async function connect(){await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);assert(!state.hosting.error);}
async function pulls(){await click('hosting-tab-pulls');await type('hosting-repository',repository);await click('hosting-refresh');assert(!state.hosting.error,state.hosting.error);}
async function preview(){await click('hosting-tab-create');await type('hosting-repository',repository);await type('hosting-source','feature/界');await type('hosting-target','main');await type('hosting-title','PRIVATE 改善 Unicode 🚀');await type('hosting-body','Quoted "line"\nSecond line \\ path');await click('hosting-review');assert(state.hosting.reviewing,state.hosting.error);assert.equal(state.hosting.review.source_id,'a'.repeat(40));assert.equal(state.hosting.created,null,'a new review must clear the previous successful PR link');}
async function cancelRequest(kind,button,hold){mode=hold;closedAt=null;const count=()=>requests.filter(r=>kind!=='create'||r.method==='POST'&&r.path.endsWith('/pullrequests')).length;const n=count();await click(button,false);await until(s=>s.hosting.busy&&count()>n);await reveal('hosting-cancel');const at=performance.now();await client.tool('rocket_click',{target:'hosting-cancel',expected_tab:state.active_tab});await until(s=>s.hosting.stopping||!s.hosting.busy);const feedback=performance.now()-at;await settled();const finish=performance.now()-at;for(let i=0;i<60&&closedAt===null;i++)await delay(50);assert(closedAt!==null,'curl connection must close');report.cancel.push({kind,feedback_ms:feedback,settled_ms:finish,socket_closed_ms:Math.max(0,closedAt-at)});mode='ok';}
async function closeWindow(){if(state?.hosting?.busy)await click('hosting-cancel');if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}}
async function stopService(){if(!service)return;if(service.exitCode===null){await closeWindow();assert.equal((await client.raw('POST','/shutdown')).status,202);for(let i=0;i<100&&service.exitCode===null;i++)await delay(50);}assert.equal(service.exitCode,0);report.service_exit=service.exitCode;}
async function run(){try{
 await new Promise(r=>server.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+server.address().port);
 const exe=path.resolve(process.env.ROCKET_HOSTING_EXE||'target/release/rocket-mcp.exe');report.sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const out=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>100)throw e;await delay(100);}}
 report.version=(await client.initialize()).serverInfo.version;await client.tool('rocket_open',{path:repo});await settled();await resize(1480,920);if(control('console-close'))await click('console-close');await openHosting();
 let n=0;
 if(creationOnly){await account();await connect();}else{
 await account('7.0',true);assert(!control('hosting-username'));assert(!control('hosting-browser'));await capture('accounts-wide');
 await resize(780,620);await reveal('hosting-azure-api-7-1');await capture('accounts-compact');await reveal('hosting-connect');await resize(1480,920);
 n=requests.length;await type('hosting-server','https://dev.azure.com/organization');await click('hosting-connect');assert(state.hosting.error);assert.equal(requests.length,n);await type('hosting-server',collection);
 for(const[modeName,code]of[['401','unauthorized'],['403','forbidden'],['404','not_found'],['429','rate_limited'],['302','redirect_refused'],['503','server_unavailable'],['400','azure_api_rejected'],['410','azure_api_rejected'],['html','invalid_response'],['anonymous','azure_identity_unverified']]){
   mode='auth-'+modeName;await type('hosting-token',token);n=requests.length;await click('hosting-connect');assert(state.hosting.error?.includes('hosting_'+code),state.hosting.error);assert(!state.hosting.connected);assert.equal(state.hosting.accounts_count,0);assert.equal(requests.length,n+1);
 }
 await capture('auth-error-wide');await resize(780,620);await reveal('hosting-connect');await scroll('hosting-scroll','up',2000);await capture('auth-error-compact');await resize(1480,920);
 await type('hosting-token',token);await cancelRequest('connect','hosting-connect','hold-auth');assert(!state.hosting.connected);assert.equal(state.hosting.accounts_count,0);assert(state.hosting.notice.includes('Cancelled'));
 pass('Collection/auth/identity errors are explicit, no redirects or saves on failure; cancelled verification stops without connecting');
 // Keep Services and Server profiles distinct, with no changes to normal login.
 await click('hosting-new-account');await click('hosting-provider-azure');await type('hosting-label','Fixture Azure Services');await type('hosting-server','https://dev.azure.com/fixture');await type('hosting-token',token);if(!state.hosting.remember)await click('hosting-remember');await connect();
 await click('hosting-tab-accounts');await click('hosting-new-account');await account('6.0',true);await connect();assert.equal(state.hosting.accounts_count,2);assert.equal(state.hosting.azure_api_version,'6.0');
 for(const version of ['6.0','7.1','7.0']){
   if(state.hosting.azure_api_version!==version){await click('hosting-tab-accounts');await click('hosting-azure-api-'+version.replace('.','-'));assert(!state.hosting.connected);await type('hosting-token',token);if(state.hosting.remember)await click('hosting-remember');await connect();}
   n=requests.length;await pulls();assert.equal(state.hosting.count,50);assert(requests.slice(n).every(r=>r.query.endsWith('api-version='+version)));assert.equal(state.hosting.items[0].url,collection+'/team/_git/%E8%B3%87%E6%96%99/pullrequest/1');assert(control('hosting-details-0')?.enabled);assert(!control('hosting-tab-publish')?.enabled);
 }
 const vault=path.join(root,'rocket-ui-state/hosting-accounts.dpapi');assert(fs.existsSync(vault));assert(!fs.readFileSync(vault).includes(Buffer.from(token)));await click('hosting-tab-accounts');await click('hosting-load-accounts');await click('hosting-account-1');assert.equal(state.hosting.azure_api_version,'6.0');await connect();
 await click('hosting-tab-accounts');await click('hosting-azure-api-7-0');await type('hosting-token',token);await connect();
 pass('Three explicit API contracts, protected profile roundtrip and separate Services/Server accounts; unsupported GitHub-only actions absent');
 await pulls();const first=state.hosting.items;await cancelRequest('next-page','hosting-more','hold-list');assert.deepEqual(state.hosting.items,first);assert(state.hosting.more);await click('hosting-more');assert.equal(state.hosting.count,51);assert(!state.hosting.more);
 await click('hosting-state-closed');await click('hosting-refresh');assert.equal(state.hosting.count,1);assert.equal(state.hosting.items[0].state,'completed');await click('hosting-state-open');await click('hosting-refresh');
 mode='unsafe-link';n=requests.length;await click('hosting-more');assert(state.hosting.error.includes('hosting_invalid_pagination'));assert.equal(requests.length,n+1);assert.equal(state.hosting.count,50);mode='ok';await capture('pagination-error');
 await click('hosting-tab-issues');await type('hosting-work-state','Active');await click('hosting-refresh');assert.equal(state.hosting.count,50);assert(state.hosting.items[0].url.endsWith('/team/_workitems/edit/1'));await click('hosting-more');assert.equal(state.hosting.count,51);
 await type('hosting-work-state','Done');await click('hosting-refresh');assert.equal(state.hosting.count,0);await type('hosting-work-state','Active');await click('hosting-refresh');const work=state.hosting.items;
 await cancelRequest('work-items','hosting-refresh','hold-list');assert.deepEqual(state.hosting.items,work);mode='wrong-project';await click('hosting-refresh');assert(state.hosting.error.includes('hosting_invalid_response'));assert.deepEqual(state.hosting.items,work);mode='ok';await click('hosting-refresh');await capture('work-items');
 pass('Scoped PR/work-item pagination, exact state filters and Unicode links; failed or cancelled reads preserve prior results');
 }
 await preview();await capture('review-wide');await resize(780,620);await reveal('hosting-create');await capture('review-compact');await resize(1480,920);n=requests.filter(r=>r.method==='POST'&&r.path.endsWith('/pullrequests')).length;await click('hosting-back');assert.equal(requests.filter(r=>r.method==='POST'&&r.path.endsWith('/pullrequests')).length,n);
 await click('hosting-review');changed=true;await click('hosting-create');assert(state.hosting.error.includes('hosting_review_stale'));assert.equal(requests.filter(r=>r.method==='POST'&&r.path.endsWith('/pullrequests')).length,n);changed=false;
 await preview();mode='rejected';await click('hosting-create');assert(state.hosting.error.includes('hosting_rejected'));mode='ok';await preview();await click('hosting-create');assert.equal(state.hosting.created,43);assert(!state.hosting.uncertain);await capture('created');
 await preview();await cancelRequest('create','hosting-create','hold-create');assert(state.hosting.uncertain);assert(state.hosting.error.includes('hosting_outcome_unknown'));n=requests.filter(r=>r.method==='POST'&&r.path.endsWith('/pullrequests')).length;await delay(300);assert.equal(requests.filter(r=>r.method==='POST'&&r.path.endsWith('/pullrequests')).length,n);await capture('create-unknown');
 await click('hosting-tab-pulls');await type('hosting-repository','team/unrelated');await click('hosting-refresh');await reveal('hosting-acknowledge');assert(!control('hosting-acknowledge').enabled);await type('hosting-repository',repository);await click('hosting-refresh');assert(state.hosting.items.some(i=>i.number===99));await click('hosting-acknowledge');assert(!state.hosting.uncertain);
 pass('Reviewed creation checks branch tips; back/cancel, rejection and uncertain writes are safe with explicit remote-result recovery');
 if(!creationOnly){
 mode='auth-401';await click('hosting-tab-accounts');await click('hosting-load-accounts');await click('hosting-account-1');await click('hosting-connect');assert(!state.hosting.connected);assert.equal(state.hosting.accounts_count,2);mode='ok';await click('hosting-load-accounts');await click('hosting-account-1');await click('hosting-forget');assert.equal(state.hosting.accounts_count,1);await click('hosting-account-0');await click('hosting-connect');assert(state.hosting.connected);assert.equal(state.hosting.provider,'azure');
 pass('Revoked fixture PAT does not erase saved profiles; forgetting Server preserves the Services account');
 await closeWindow();fs.unlinkSync(path.join(root,'rocket-hosting-api'));n=requests.length;await client.tool('rocket_open',{path:repo});await settled();await resize(1480,920);if(control('console-close'))await click('console-close');await openHosting();assert(!state.hosting.enabled);const denied=await client.request('tools/call',{name:'rocket_click',arguments:{target:'hosting-provider-azure-server',expected_tab:state.active_tab}});assert(denied.isError);assert.equal(requests.length,n);
 }
 await stopService();assert.deepEqual(snapshot(),before);report.local_state_unchanged=true;
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(x=>/^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x=>fs.readFileSync(path.join(root,n,x),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const logs=JSON.stringify(records);
 for(const secret of [token,Buffer.from(':'+token).toString('base64'),'PRIVATE','Authorization','team/','Fixture User','devops.example.com','DefaultCollection','outside.invalid'])assert(!logs.includes(secret),'diagnostic leak '+secret);
 for(const op of ['hosting-connect','hosting-list','hosting-preview','hosting-create','hosting-accounts']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length);for(const s of starts)assert(records.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request));}
 const codes=creationOnly?['hosting_review_stale','hosting_rejected','hosting_outcome_unknown']:['hosting_unauthorized','hosting_forbidden','hosting_not_found','hosting_rate_limited','hosting_redirect_refused','hosting_server_unavailable','hosting_azure_api_rejected','hosting_azure_identity_unverified','hosting_invalid_response','hosting_invalid_pagination','hosting_review_stale','hosting_rejected','hosting_outcome_unknown'];
 for(const code of codes)assert(records.some(r=>r.failure_code===code),'missing '+code);
 assert(logs.includes('hosting_provider_azure_server'));for(const version of creationOnly?['7_0']:['6_0','7_0','7_1'])assert(logs.includes('hosting_azure_api_'+version));assert(requests.every(r=>r.auth));report.logs=records.length;report.requests=requests.length;pass('Unchanged dirty repository and paired redacted logs with precise failures and API contracts'+(creationOnly?'':'; default-off MCP policy'));report.passed=true;
 }catch(e){report.failure=e.stack;try{await capture('failure');}catch{}throw e;}
 finally{
  if(service?.exitCode===null){try{await stopService();}catch(e){report.cleanup_error=String(e);service.kill();}}
  if(client)await client.finish();server.closeAllConnections();await new Promise(r=>server.close(r));
  const access=path.join(profile,'access-token');if(fs.existsSync(access)){fs.unlinkSync(access);report.mcp_token_removed=true;}
  fs.writeFileSync('work/azure-server-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));console.log(JSON.stringify(report,null,2));
 }
}
run().catch(e=>{console.error(e);process.exitCode=1;});
