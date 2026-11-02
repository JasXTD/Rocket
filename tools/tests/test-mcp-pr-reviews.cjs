// Focused GitHub reviews, merges and cross-fork creation via native UI + localhost MCP.
// Synthetic HTTP only; no personal account, hosted write or existing repository.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto');
const http=require('node:http'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const forkOnly=process.argv.includes('--fork-only');
const enterprise=process.argv.includes('--enterprise');
const provider=enterprise?'github-enterprise':'github',enterpriseOrigin='https://enterprise.example.invalid:8443';
const apiPrefix=enterprise?'/github-enterprise/api/v3':'/github';
const root=path.resolve('work/mcp-fixtures/'+(enterprise?'pr-ghes-':'pr-reviews-')+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'fixture-repo'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/pr-reviews';
const token='fixture_PR_'+crypto.randomBytes(12).toString('hex'),privateBody='PRIVATE review 界 "quoted"\nSecond line';
const cloudToken=token+'-public-fixture';
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0',http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:''});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
if(enterprise)git(['remote','add','origin',enterpriseOrigin+'/team/sample.git']);
fs.writeFileSync(path.join(repo,'資料.txt'),'committed\n');git(['add','-A']);git(['commit','-m','Synthetic fixture']);
fs.writeFileSync(path.join(repo,'資料.txt'),'staged\n');git(['add','-A']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');
fs.writeFileSync(path.join(repo,'private-untracked.txt'),'preserve me');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const local=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'private-untracked.txt'))}),before=local();
const requests=[],reviews=[];let mode='ok',head='a'.repeat(40),base='b'.repeat(40),merged=false,service,client,state,closedAt=null;
const report={passed:false,provider,scope:forkOnly?'fork-only':'all-pr-workflows',root,before,cases:[],cancel:[]},delay=ms=>new Promise(r=>setTimeout(r,ms));
const summary=()=>({number:42,title:'Preserve Unicode paths 界 🚀',body:'PRIVATE description\nRename paths without changing the checkout.',user:{login:'contributor'},state:merged?'closed':'open',draft:false,merged,mergeable:true,mergeable_state:mode==='merge-blocked'?'blocked':'clean',head:{sha:head,ref:'feature/界'},base:{sha:base,ref:'main'},commits:2,changed_files:2,additions:3,deletions:1});
const server=http.createServer(async(req,res)=>{
 try{
 let data='';for await(const chunk of req)data+=chunk;assert(data.length<128*1024);
 const url=new URL(req.url,'http://127.0.0.1');
 const isEnterprise=url.pathname.startsWith('/github-enterprise/api/v3/');
 const prefix=isEnterprise?'/github-enterprise/api/v3':'/github';
 assert(url.pathname.startsWith(prefix+'/'),'unexpected API base');
 const route=url.pathname.slice(prefix.length);
 const call={method:req.method,path:url.pathname,body:data};requests.push(call);
 const send=(code,body,headers={})=>{res.writeHead(code,{'Content-Type':'application/json',...headers});res.end(JSON.stringify(body));};
 const expectedToken=enterprise&&!isEnterprise?cloudToken:token;
 if(req.headers.authorization!=='Bearer '+expectedToken)return send(401,{message:token});
 if(isEnterprise)assert.equal(req.headers['x-github-api-version'],'2022-11-28');
 if(route==='/user'){
   if(mode==='auth-slow'){res.on('close',()=>{closedAt=performance.now();});return;}
   const codes={'auth-forbidden':403,'auth-missing':404,'auth-rate':429,'auth-redirect':302,'auth-service':503,'auth-version':410};
   if(codes[mode])return send(codes[mode],{message:token+' PRIVATE account response'},mode==='auth-redirect'?{Location:'http://127.0.0.1:1/forbidden-redirect'}:{});
   if(mode==='auth-malformed'){res.writeHead(200,{'Content-Type':'text/html'});return res.end('<html>PRIVATE '+token+'</html>');}
   return send(200,{login:'fixture'},isEnterprise?{'X-GitHub-Enterprise-Version':'3.21.0'}:{});
 }
 if(mode==='unauthorized')return send(401,{message:token});
 if(route==='/repos/team/sample'||route==='/repos/team/fork')return send(200,{id:route.endsWith('sample')?1:2,full_name:route.slice('/repos/'.length),fork:route.endsWith('fork'),source:{id:mode==='fork-unrelated'?9:1},archived:false,permissions:{push:true},allow_merge_commit:true,allow_squash_merge:true,allow_rebase_merge:true});
 if(route.includes('/branches/'))return send(200,{commit:{sha:route.includes('/fork/')?head:base}});
 if(req.method==='POST'&&route==='/repos/team/sample/pulls'){
   const body=JSON.parse(data);assert.equal(body.head,'team:main');assert.equal(body.head_repo,'fork');assert.equal(body.base,'main');assert.equal(body.maintainer_can_modify,false);
   if(mode==='fork-reject')return send(422,{message:token+' PRIVATE rejection'});
   if(mode==='slow-create'){res.on('close',()=>{closedAt=performance.now();});return;}
   return send(201,{...summary(),number:44,state:'open',merged:false,title:body.title,head:{ref:'main',sha:head,repo:{full_name:'team/fork'}},base:{ref:'main',sha:base,repo:{full_name:'team/sample'}}});
 }
 if(req.method==='PUT'){
   assert.equal(route,'/repos/team/sample/pulls/42/merge');const body=JSON.parse(data);assert.deepEqual(Object.keys(body).sort(),['merge_method','sha']);assert.equal(body.sha,head);assert(['merge','squash','rebase'].includes(body.merge_method));
   if(mode==='merge-reject')return send(405,{message:token+' PRIVATE protected branch'});
   if(mode==='merge-race')return send(409,{message:token+' PRIVATE head changed'});
   merged=mode!=='slow-merge-noaccept';
   if(mode.startsWith('slow-merge')){res.on('close',()=>{closedAt=performance.now();});return;}
   return send(200,{merged:true,sha:'e'.repeat(40)});
 }
 if(route==='/search/issues'){assert(url.searchParams.get('q').includes('repo:team/sample'));return send(200,{total_count:1,incomplete_results:false,items:[{...summary(),pull_request:{}}]});}
 if(route==='/repos/team/sample/issues')return send(200,[{number:72,title:'PRIVATE issue 界',state:'open',user:{login:'fixture'}},{...summary(),pull_request:{}}]);
 if(route==='/repos/team/sample/pulls'){
   if(mode==='list-pages')return send(200,[{...summary(),number:url.searchParams.get('page')==='2'?43:42}],url.searchParams.get('page')==='2'?{}:{Link:`<http://127.0.0.1:${server.address().port}${apiPrefix}/repos/team/sample/pulls?page=2>; rel="next"`});
   if(mode==='list-outside')return send(200,[summary()],{Link:'<https://outside.invalid/api/v3/repos/team/sample/pulls?page=2>; rel="next"'});
   return send(200,[summary(),{...summary(),number:43,title:'Another synthetic PR'}]);
 }
 if(req.method==='POST'){
   assert.equal(route,'/repos/team/sample/pulls/42/reviews');const body=JSON.parse(data);
   assert.equal(body.commit_id,head);assert(['COMMENT','APPROVE','REQUEST_CHANGES'].includes(body.event));assert.equal(body.body,privateBody);
   if(mode==='reject')return send(422,{message:token+' PRIVATE rejection'});
   const review={id:100+reviews.length,user:{login:'fixture'},body:body.body,state:{COMMENT:'COMMENTED',APPROVE:'APPROVED',REQUEST_CHANGES:'CHANGES_REQUESTED'}[body.event],commit_id:body.commit_id,submitted_at:'2026-10-27T00:00:00Z'};reviews.push(review);
   if(mode==='slow-submit'){res.on('close',()=>{closedAt=performance.now();});return;}
   if(mode==='invalid-submit')return send(201,{});
   return send(201,review);
 }
 if(route==='/repos/team/sample/pulls/42'||route==='/repos/team/sample/pulls/43'){
   if(mode==='slow-read'){res.on('close',()=>{closedAt=performance.now();});return;}
   return send(200,{...summary(),number:route.endsWith('43')?43:42});
 }
 if(route.endsWith('/commits'))return send(200,[{sha:'d'.repeat(40),commit:{message:'Add Unicode fixture',author:{name:'作者'}}},{sha:head,commit:{message:'Preserve rename tracking',author:{name:'Contributor'}}}]);
 if(route.endsWith('/files'))return send(200,[{filename:'資料/新.rs',previous_filename:'資料/旧.rs',status:'renamed',additions:2,deletions:1,patch:'@@ -1 +1,2 @@\n-old\n+new 界\n+// \\ path'},{filename:'assets/icon.png',status:'modified',additions:1,deletions:0}],mode==='unsafe-page'?{Link:'<http://127.0.0.1:1/stolen>; rel="next"'}:{});
 if(route.includes('/issues/')&&route.endsWith('/comments'))return send(200,[{id:7,user:{login:'contributor'},body:'PRIVATE discussion\nReady for review.',created_at:'2026-10-27T00:00:00Z'}]);
 if(route.endsWith('/comments'))return send(200,[{id:8,user:{login:'reviewer'},body:'PRIVATE inline comment',path:'資料/新.rs',line:null,in_reply_to_id:7,created_at:'2026-10-27T00:00:00Z'}]);
 if(route.endsWith('/reviews'))return send(200,reviews);
 if(route.endsWith('/check-runs'))return mode==='checks-denied'?send(403,{message:token}):send(200,{total_count:2,check_runs:[{name:'Windows build',head_sha:head,status:'completed',conclusion:'failure'},{name:'Tests',head_sha:head,status:'in_progress',conclusion:null}]});
 if(route.endsWith('/status'))return send(200,{sha:head,total_count:1,statuses:[{context:'lint',state:'pending'}]});
 throw Error('Unexpected '+req.method+' '+route);
 }catch(e){report.server_error=String(e.stack||e);res.writeHead(500);res.end('{}');}
});
const pass=s=>{report.cases.push(s);console.error('PASS '+s);};
async function until(fn,label='state'){const at=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return state;assert(performance.now()-at<45000,label+' timeout '+JSON.stringify({hosting:state.hosting,dialog:state.dialog?.kind}));await delay(60);}}
const control=k=>state.controls?.find(c=>c.id===k);
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.hosting?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2;},'stable layout');}
async function scroll(region,direction,pixels=300){await client.tool('rocket_scroll',{target:region,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(k){await stable();const visible=()=>control(k)&&control(k).rect[3]-control(k).rect[1]>18;if(visible())return;
 if(control('hosting-scroll')){for(let i=0;i<6&&!visible();i++)await scroll('hosting-scroll','up',2000);for(let i=0;i<32&&!visible();i++)await scroll('hosting-scroll','down',240);}assert(visible(),'unreachable '+k);}
async function click(k,wait=true){await reveal(k);assert(control(k)?.enabled,'disabled '+k);await client.tool('rocket_click',{target:k,expected_tab:state.active_tab});if(wait)await settled();}
async function type(k,text){await reveal(k);assert(control(k)?.enabled,'disabled '+k);await client.tool('rocket_type',{target:k,text,expected_tab:state.active_tab});await settled();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const image=r.content.find(x=>x.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(w,h){await client.tool('rocket_resize',{width:w,height:h});await until(s=>s.viewport[0]===w&&s.viewport[1]===h&&!s.input_pending);await settled();}
async function openHosting(){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);await click('repository-remotes-menu');await stable();for(let i=0;i<14&&!(control('repository-hosting')&&control('repository-hosting').rect[3]-control('repository-hosting').rect[1]>25);i++)await scroll('repository-menu-scroll','down',230);await click('repository-hosting');await until(s=>s.dialog?.kind==='hosting'&&!s.hosting.busy);await settled();}
const posts=()=>requests.filter(r=>r.method==='POST').length;
const puts=()=>requests.filter(r=>r.method==='PUT').length;
async function draft(action='comment'){
 if(!state.hosting.pull.composing)await click('hosting-review-write');
 if(state.hosting.pull.reviewing)await click('hosting-review-edit');
 await click('hosting-review-'+action);await type('hosting-review-body',privateBody);await click('hosting-review-preview');assert(state.hosting.pull.reviewing,state.hosting.error);assert.equal(state.hosting.pull.preview_head,head);
}
async function cancel(kind){await reveal('hosting-cancel');const at=performance.now();await client.tool('rocket_click',{target:'hosting-cancel',expected_tab:state.active_tab});await until(s=>s.hosting.stopping||!s.hosting.busy);const feedback=performance.now()-at;await settled();const settle=performance.now()-at;for(let i=0;i<80&&closedAt===null;i++)await delay(50);assert(closedAt!==null,'curl connection did not close');report.cancel.push({kind,feedback_ms:feedback,connection_closed_ms:closedAt-at,settled_ms:settle});}
async function closeWindow(){
 if(state?.hosting?.busy)await click('hosting-cancel');
 if(state?.dialog)await click('dialog-cancel');
 await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));
 if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}
}
async function shutdown(){
 if(service&&service.exitCode===null){await closeWindow();await client.raw('POST','/shutdown');const at=performance.now();while(service.exitCode===null){assert(performance.now()-at<15000,'service failed to shut down');await delay(60);}assert.equal(service.exitCode,0);}
 if(client)await client.finish();
 const tokenPath=path.join(profile,'access-token');if(fs.existsSync(tokenPath))fs.unlinkSync(tokenPath);report.cleanup={exit:service?.exitCode,token_removed:!fs.existsSync(tokenPath)};
}
async function enterpriseAccounts(){
 await click('hosting-tab-accounts');await click('hosting-provider-github-enterprise');await type('hosting-label','Enterprise fixture');
 assert(!control('hosting-browser'));assert(!control('hosting-username'));
 await type('hosting-server','http://enterprise.example.invalid');await type('hosting-token',token);const initial=requests.length;await click('hosting-connect');assert(state.hosting.error.includes('HTTPS'));assert.equal(requests.length,initial);
 await type('hosting-server',enterpriseOrigin+'/api/v3');await click('hosting-connect');assert(state.hosting.error.includes('without /api/v3'));assert.equal(requests.length,initial);
 await type('hosting-server',enterpriseOrigin);await type('hosting-token',token+'-wrong');await click('hosting-connect');assert(state.hosting.error.includes('hosting_unauthorized'));assert(!state.hosting.connected);assert.equal(state.hosting.accounts_count,0);await capture('enterprise-unauthorized-wide');
 for(const [failure,code] of [['auth-forbidden','hosting_forbidden'],['auth-missing','hosting_not_found'],['auth-rate','hosting_rate_limited'],['auth-redirect','hosting_redirect_refused'],['auth-service','hosting_server_unavailable'],['auth-version','hosting_api_version_unsupported'],['auth-malformed','hosting_invalid_response']]){
   mode=failure;await type('hosting-token',token);const n=requests.length;await click('hosting-connect');assert(state.hosting.error.includes(code),state.hosting.error);assert(!state.hosting.connected);assert.equal(requests.length,n+1);assert.equal(state.hosting.accounts_count,0);assert(!JSON.stringify(state).includes(token));
 }
 await resize(780,620);await reveal('hosting-connect');await capture('enterprise-error-compact');
 mode='auth-slow';closedAt=null;const n=requests.length;await click('hosting-connect',false);await until(s=>s.hosting.busy&&requests.length>n);await cancel('enterprise-connect');assert(!state.hosting.connected);assert.equal(state.hosting.accounts_count,0);mode='ok';
 pass('Enterprise auth rejects invalid URLs/tokens, permissions, hidden resources, rate limits, redirects, service errors and malformed responses; connection cancellation saves no account');
 // Two disposable profiles verify that a public GitHub token cannot become an enterprise token.
 await resize(1480,920);await click('hosting-provider-github');await type('hosting-label','Public fixture');await type('hosting-token',cloudToken);if(!state.hosting.remember)await click('hosting-remember');await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);assert.equal(state.hosting.accounts_count,1);
 await click('hosting-tab-accounts');await click('hosting-new-account');await click('hosting-provider-github-enterprise');await type('hosting-label','Enterprise fixture');await type('hosting-server',enterpriseOrigin);await type('hosting-token',token);if(!state.hosting.remember)await click('hosting-remember');await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);assert.equal(state.hosting.enterprise_version,'3.21.0');assert.equal(state.hosting.accounts_count,2);assert.equal(state.hosting.repository,'team/sample');
 const vault=path.join(root,'rocket-ui-state/hosting-accounts.dpapi');assert(fs.existsSync(vault));for(const secret of [token,cloudToken,enterpriseOrigin])assert(!fs.readFileSync(vault).includes(Buffer.from(secret)));
 await click('hosting-tab-accounts');await resize(780,620);await reveal('hosting-connect');await capture('enterprise-accounts-compact');await resize(1480,920);await capture('enterprise-accounts-wide');
 await click('hosting-load-accounts');await click('hosting-account-0');await click('hosting-connect');assert.equal(state.hosting.provider,'github');assert(state.hosting.connected,state.hosting.error);assert.equal(state.hosting.enterprise_version,null);
 await click('hosting-tab-accounts');await click('hosting-account-1');await click('hosting-connect');assert.equal(state.hosting.provider,provider);assert(state.hosting.connected,state.hosting.error);assert.equal(state.hosting.accounts_count,2);
 const beforeEdit=requests.length;await click('hosting-tab-accounts');await type('hosting-server','https://another.example.invalid');assert(!state.hosting.connected);assert.equal(requests.length,beforeEdit);await click('hosting-account-1');await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);
 pass('Public and Enterprise profiles round-trip separately through protected storage; server edits invalidate the session; credentials absent from MCP state');
 await type('hosting-repository','team/sample');mode='list-pages';await click('hosting-refresh');assert.equal(state.hosting.count,1);assert(state.hosting.more);await click('hosting-more');assert.equal(state.hosting.count,2);assert(!state.hosting.more);for(const item of state.hosting.items)assert(item.url.startsWith(enterpriseOrigin+'/team/sample/pull/'));
 mode='list-outside';const beforeOutside=requests.length;await click('hosting-refresh');assert(state.hosting.error.includes('hosting_invalid_pagination'));assert.equal(requests.length,beforeOutside+1);mode='ok';
 await type('hosting-search-text','Unicode 界');await click('hosting-search');assert.equal(state.hosting.total,1);assert.equal(state.hosting.count,1);await type('hosting-search-text','');await click('hosting-tab-issues');await click('hosting-refresh');assert.equal(state.hosting.count,1);assert.equal(state.hosting.items[0].number,72);assert.equal(state.hosting.items[0].url,enterpriseOrigin+'/team/sample/issues/72');await capture('enterprise-issues');await click('hosting-tab-pulls');
 assert(!control('hosting-tab-publish')?.enabled);pass('Enterprise API prefix, pagination, search, issue filtering and browser links stay on the configured server; public-only publishing stays disabled');
}
async function run(){
 try{
 await new Promise(r=>server.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+server.address().port);
 const exe=path.resolve(process.env.ROCKET_PR_EXE||'target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const out=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>100)throw e;await delay(100);}}
 report.version=(await client.initialize()).serverInfo.version;assert.equal(report.version,fs.readFileSync('Cargo.toml','utf8').match(/^version\s*=\s*"([^"]+)"/m)[1]);
 await client.tool('rocket_open',{path:repo});await settled();await resize(1480,920);if(control('console-close'))await click('console-close');await openHosting();
 if(enterprise)await enterpriseAccounts();else {await click('hosting-tab-accounts');await click('hosting-provider-github');await type('hosting-token',token);if(state.hosting.remember)await click('hosting-remember');await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);}
 if(!forkOnly){
 await type('hosting-repository','team/sample');await click('hosting-refresh');await click('hosting-details-0');assert.equal(state.hosting.section,'PullDetail');assert.equal(state.hosting.pull.details.head,head);assert.equal(state.hosting.pull.details.commits.count,2);await capture('overview-wide');
 for(const tab of ['commits','files','discussion','reviews','checks']){await click('hosting-pull-tab-'+tab);assert(!state.hosting.error);}
 assert.equal(state.hosting.pull.details.checks.count,2);assert.equal(state.hosting.pull.details.statuses.count,1);await capture('checks-wide');
 await click('hosting-pull-tab-files');assert.deepEqual(state.hosting.pull.details.paths.map(f=>f.patch),[true,false]);assert.equal(state.hosting.pull.details.paths[0].previous,'資料/旧.rs');await capture('files-wide');
 pass('PR tabs show Unicode renames, binary patch absence, discussion/reviews and exact-head checks');
 for(const [action,status]of[['comment','COMMENTED'],['approve','APPROVED'],['changes','CHANGES_REQUESTED']]){await draft(action);const n=posts();await click('hosting-review-submit');assert.equal(posts(),n+1);assert.equal(state.hosting.pull.submitted.state,status);assert.equal(state.hosting.pull.body_bytes,0);assert(!state.hosting.pull.reviewing);assert(!state.hosting.error);}
 assert.equal(reviews.length,3);pass('Comment, Approve and Request changes require previews and submit the pinned commit once');
 await draft();const n=posts();head='c'.repeat(40);await click('hosting-review-submit');assert(state.hosting.error.includes('hosting_pull_changed'));assert.equal(posts(),n);assert(state.hosting.pull.body_bytes>0);await capture('changed-head');await click('hosting-pull-refresh');assert.equal(state.hosting.pull.details.head,head);
 await draft();base='d'.repeat(40);await click('hosting-review-submit');assert(state.hosting.error.includes('hosting_pull_changed'));assert.equal(posts(),n);await click('hosting-pull-refresh');
 pass('Changed head/base refuse before POST, retain draft and require fresh details');
 await click('hosting-pull-back');await click('hosting-details-1');assert(state.hosting.error.includes('draft is still open'));assert.equal(state.hosting.pull.number,42);
 await resize(780,620);await draft('changes');await reveal('hosting-review-submit');await capture('review-compact');await click('hosting-review-edit');await reveal('hosting-review-body');await capture('draft-compact');await resize(1480,920);
 pass('Draft stays with its PR; compact preview, edit and submit controls remain reachable');
 mode='checks-denied';await click('hosting-pull-refresh');assert.equal(state.hosting.pull.details.checks.error,'hosting_forbidden');await click('hosting-pull-tab-checks');await capture('checks-unavailable');mode='unsafe-page';await click('hosting-pull-refresh');assert.equal(state.hosting.pull.details.files.error,'hosting_invalid_pagination');assert(!requests.some(r=>r.path.includes('stolen')));mode='ok';await click('hosting-pull-refresh');
 pass('Unavailable checks and unsafe pagination remain explicit and do not send credentials elsewhere');
 mode='slow-read';closedAt=null;const seen=requests.length;const old=state.hosting.pull.details;await click('hosting-pull-refresh',false);await until(s=>s.hosting.busy&&requests.length>seen);await cancel('read');assert.deepEqual(state.hosting.pull.details,old);assert(!state.hosting.pull.uncertain);assert(state.hosting.pull.body_bytes>0);mode='ok';
 pass('Read cancellation stops curl and preserves the previous snapshot and draft');
 await draft();mode='reject';const beforeReject=posts();await click('hosting-review-submit');assert(state.hosting.error.includes('hosting_pull_review_rejected'));assert.equal(posts(),beforeReject+1);assert(!state.hosting.pull.uncertain);assert(state.hosting.pull.body_bytes>0);await capture('review-rejected');mode='ok';
 pass('Review rejection is distinct, payload-free and retains the editable draft');
 await draft();mode='slow-submit';closedAt=null;const beforeCancel=posts();await click('hosting-review-submit',false);await until(s=>s.hosting.busy&&posts()>beforeCancel);await cancel('submission');assert(state.hosting.pull.uncertain);assert(state.hosting.error.includes('hosting_pull_review_unknown'));assert.equal(posts(),beforeCancel+1);assert(!control('hosting-review-preview')?.enabled);await capture('review-unknown');
 mode='ok';await click('hosting-pull-refresh');assert(state.hosting.pull.can_acknowledge);assert.equal(state.hosting.pull.details.reviews.count,4);await click('hosting-pull-tab-reviews');await capture('reviews-reconciled');
 await click('hosting-review-discard');await click('hosting-pull-back');await click('hosting-details-1');assert.equal(state.hosting.pull.number,43);assert(state.hosting.pull.uncertain);assert(!state.hosting.pull.can_acknowledge);await reveal('hosting-review-acknowledge');assert(!control('hosting-review-acknowledge').enabled);
 await click('hosting-pull-back');await click('hosting-details-0');assert(state.hosting.pull.can_acknowledge);await click('hosting-review-acknowledge');assert(!state.hosting.pull.uncertain);assert.equal(posts(),beforeCancel+1);
 pass('Cancelled accepted submission reports unknown; refreshed reviews and explicit acknowledgment required, no retry');
 // New workflows: use the native controls and inspect the actual HTTP payload.
 await click('hosting-pull-tab-files');
 assert.deepEqual(state.hosting.pull.details.paths[0].lines,[{old:null,new:null},{old:1,new:null},{old:null,new:1},{old:null,new:2}]);
 await click('hosting-inline-line-0-1-left');await type('hosting-inline-body','PRIVATE old-side comment');
 await type('hosting-review-body',privateBody);await click('hosting-review-preview');assert(state.hosting.error.includes('Save or cancel'));assert(!state.hosting.pull.reviewing);
 await click('hosting-inline-save');assert.equal(state.hosting.pull.comments.length,1);
 await click('hosting-inline-line-0-2-right');await type('hosting-inline-body','PRIVATE new-side comment 界');await click('hosting-inline-save');
 await click('hosting-inline-edit-0');await type('hosting-inline-body','PRIVATE edited old-side');await click('hosting-inline-save');
 await click('hosting-inline-line-0-3-right');await type('hosting-inline-body','PRIVATE discarded editor');const unfinished=state.hosting.pull.inline_body_bytes;await click('hosting-inline-edit-0');assert(state.hosting.error.includes('Save or cancel'));assert.equal(state.hosting.pull.inline_body_bytes,unfinished);await click('hosting-inline-cancel');assert.equal(state.hosting.pull.comments.length,2);
 await resize(780,620);await reveal('hosting-inline-line-0-2-right');await capture('inline-compact');
 await draft();await reveal('hosting-review-submit');await capture('inline-review-compact');await click('hosting-review-submit');assert(!state.hosting.error,state.hosting.error);
 const inlinePayload=JSON.parse(requests.filter(r=>r.method==='POST'&&r.path.endsWith('/reviews')).at(-1).body);
 assert.deepEqual(inlinePayload.comments,[{path:'資料/新.rs',line:1,side:'LEFT',body:'PRIVATE edited old-side'},{path:'資料/新.rs',line:1,side:'RIGHT',body:'PRIVATE new-side comment 界'}]);assert.equal(inlinePayload.commit_id,head);
 assert.equal(state.hosting.pull.comments.length,0);await resize(1480,920);
 pass('Inline LEFT/RIGHT comments on Unicode rename: edit, cancel draft, compact preview and one pinned review payload');
 await click('hosting-inline-line-0-2-right');await type('hosting-inline-body','PRIVATE earlier revision');await click('hosting-inline-save');
 head='f'.repeat(40);await click('hosting-pull-refresh');await type('hosting-review-body',privateBody);const stalePosts=posts();await click('hosting-review-preview');
 assert(state.hosting.error.includes('hosting_pull_review_stale'));assert.equal(posts(),stalePosts);assert.equal(state.hosting.pull.comments.length,1);await capture('inline-stale');
 await click('hosting-inline-remove-0');await click('hosting-review-discard');
 pass('Refreshed PR never silently retargets queued inline comments; stale drafts retained and removable');
 for(const method of ['merge','squash','rebase']){
   merged=false;await click('hosting-pull-refresh');await click('hosting-merge-method-'+method);await click('hosting-merge-preview');assert(state.hosting.pull.merge.reviewing,state.hosting.error);assert.equal(state.hosting.pull.merge.head,head);assert.equal(state.hosting.pull.merge.base,base);
   if(method==='squash'){await resize(780,620);await reveal('hosting-merge-submit');await capture('merge-compact');}
   const n=puts();await click('hosting-merge-submit');assert(!state.hosting.error,state.hosting.error);assert.equal(state.hosting.pull.merge.result,'e'.repeat(40));assert.equal(puts(),n+1);assert.equal(JSON.parse(requests.filter(r=>r.method==='PUT').at(-1).body).merge_method,method);await resize(1480,920);
 }
 pass('Merge, squash and rebase use reviewed head conditions; compact confirmation and confirmed results work');
 merged=false;mode='merge-blocked';await click('hosting-pull-refresh');const noWrites=puts();await click('hosting-merge-preview');assert(state.hosting.error.includes('hosting_pull_merge_blocked'));assert.equal(puts(),noWrites);await capture('merge-blocked');mode='ok';
 for(const which of ['head','base']){
   await click('hosting-pull-refresh');await click('hosting-merge-preview');assert(state.hosting.pull.merge.reviewing);
   if(which==='head')head='1'.repeat(40);else base='2'.repeat(40);
   await click('hosting-merge-submit');assert(state.hosting.error.includes('hosting_pull_changed'));assert.equal(puts(),noWrites);
 }
 pass('Blocked mergeability and changed head/base refuse without PUT');
 for(const rejection of ['merge-reject','merge-race']){
   await click('hosting-pull-refresh');await click('hosting-merge-preview');mode=rejection;const n=puts();await click('hosting-merge-submit');assert(state.hosting.error.includes('hosting_pull_merge_rejected'));assert(!state.hosting.pull.merge.uncertain);assert.equal(puts(),n+1);await capture(rejection);mode='ok';
 }
 mode='slow-read';closedAt=null;const beforePreview=requests.length;const beforePreviewWrites=puts();await click('hosting-merge-preview',false);await until(s=>s.hosting.busy&&requests.length>beforePreview);await cancel('merge-preview');assert(!state.hosting.pull.merge.reviewing);assert.equal(puts(),beforePreviewWrites);mode='ok';
 pass('Merge rejection is clear; cancelling the preview sends no mutation');
 await click('hosting-merge-preview');assert(state.hosting.pull.merge.reviewing);mode='slow-merge-noaccept';closedAt=null;const beforeMerge=puts();await click('hosting-merge-submit',false);await until(s=>s.hosting.busy&&puts()>beforeMerge);await cancel('merge-unconfirmed');assert(state.hosting.pull.merge.uncertain);assert(state.hosting.error.includes('hosting_pull_merge_unknown'));assert.equal(puts(),beforeMerge+1);mode='ok';
 await click('hosting-pull-back');await click('hosting-details-1');assert(state.hosting.pull.merge.uncertain);assert(!state.hosting.pull.merge.can_acknowledge);
 await click('hosting-pull-back');await click('hosting-details-0');assert(state.hosting.pull.merge.can_acknowledge);await capture('merge-unknown');await click('hosting-merge-acknowledge');assert(!state.hosting.pull.merge.uncertain);assert.equal(puts(),beforeMerge+1);
 await click('hosting-merge-preview');mode='slow-merge';closedAt=null;await click('hosting-merge-submit',false);await until(s=>s.hosting.busy&&puts()>beforeMerge+1);await cancel('merge-accepted');assert(state.hosting.pull.merge.uncertain);mode='ok';await click('hosting-pull-refresh');assert(state.hosting.pull.details.merged);assert(!state.hosting.pull.merge.uncertain);assert(state.hosting.pull.merge.result.includes('cannot identify'));assert.equal(puts(),beforeMerge+2);await capture('merge-reconciled');
 pass('Cancelled merges never retry; unknown outcome is scoped to its PR, explicit acknowledgment or refreshed merged state resolves it');
 }
 await click('hosting-tab-create');await type('hosting-repository','team/sample');await click('hosting-from-fork');await type('hosting-source-repository','team/fork');await type('hosting-source','main');await type('hosting-target','main');await type('hosting-title','PRIVATE cross-fork request');await type('hosting-body','PRIVATE fork description');
 mode='fork-unrelated';const forkStart=posts();await click('hosting-review');assert(state.hosting.error.includes('fork network'));assert.equal(posts(),forkStart);mode='ok';await click('hosting-review');assert(state.hosting.reviewing,state.hosting.error);assert.equal(state.hosting.review.source_repository,'team/fork');assert.equal(state.hosting.review.target_repository,'team/sample');
 head='3'.repeat(40);await click('hosting-create');assert(state.hosting.error.includes('hosting_review_stale'));assert.equal(posts(),forkStart);
 await click('hosting-review');await resize(780,620);await reveal('hosting-create');assert(control('hosting-create').rect[2]<=state.viewport[0]-18,'Create button clipped at right edge');await capture('fork-preview-compact');await click('hosting-create');assert.equal(state.hosting.created,44);assert(!state.hosting.error,state.hosting.error);assert.equal(posts(),forkStart+1);await resize(1480,920);
 pass('Cross-fork creation checks network, source and target tips; same-named branches and same-organization head_repo work');
 await click('hosting-review');mode='fork-reject';await click('hosting-create');assert(state.hosting.error.includes('hosting_rejected'));assert(!state.hosting.uncertain);mode='ok';await click('hosting-review');mode='slow-create';closedAt=null;const beforeForkCancel=posts();await click('hosting-create',false);await until(s=>s.hosting.busy&&posts()>beforeForkCancel);await cancel('fork-create');assert(state.hosting.uncertain);assert(state.hosting.error.includes('hosting_outcome_unknown'));assert.equal(posts(),beforeForkCancel+1);await capture('fork-unknown');mode='ok';
 pass('Cross-fork rejection and interrupted creation are distinct; unknown creation prevents automatic resubmission');
 assert.deepEqual(local(),before);assert(!report.server_error,report.server_error);
 await closeWindow();fs.unlinkSync(path.join(root,'rocket-hosting-api'));const beforeDenied=requests.length;
 await client.tool('rocket_open',{path:repo});await settled();await resize(1480,920);if(control('console-close'))await click('console-close');await openHosting();assert(!state.hosting.enabled);assert.equal(state.hosting.pull,undefined);
 for(const target of ['hosting-review-submit','hosting-review-preview','hosting-inline-save','hosting-merge-submit','hosting-merge-preview','hosting-from-fork','hosting-create']){const r=await client.request('tools/call',{name:'rocket_click',arguments:{target,expected_tab:state.active_tab}});assert(r.isError,'unscoped hosting control allowed');}assert.equal(requests.length,beforeDenied);
 pass('Missing fixture marker denies review controls and hides PR metadata without API access');
 await shutdown();
 const logs=[];function walk(p){for(const f of fs.readdirSync(p,{withFileTypes:true})){const name=path.join(p,f.name);if(f.isDirectory())walk(name);else if(f.name.endsWith('.jsonl'))logs.push(name);}}walk(root);
 const logText=logs.map(p=>fs.readFileSync(p,'utf8')).join('\n'),records=logText.split(/\r?\n/).filter(Boolean).map(s=>JSON.parse(s));assert(records.length>0);
 for(const secret of [token,'PRIVATE description','PRIVATE review','PRIVATE inline','PRIVATE old-side','PRIVATE fork','資料/新.rs','team/sample','team/fork','Authorization',...(enterprise?['enterprise.example.invalid','PRIVATE account response']:[])])assert(!logText.includes(secret),'private log payload '+secret);
 if(enterprise){assert(records.some(r=>r.step==='hosting_provider_github_enterprise'));for(const code of ['hosting_unauthorized','hosting_forbidden','hosting_rate_limited','hosting_redirect_refused','hosting_server_unavailable','hosting_invalid_response'])assert(logText.includes(code),'missing diagnostic '+code);assert(requests.every(r=>r.path.startsWith('/github-enterprise/api/v3/')||r.path==='/github/user'));}
 assert(records.every(r=>!r.dropped_before));for(const op of forkOnly?['hosting-preview','hosting-create']:['hosting-pull-details','hosting-review-preview','hosting-review-submit','hosting-merge-preview','hosting-merge-submit','hosting-preview','hosting-create']){const start=records.filter(r=>r.event==='request_start'&&r.operation===op),end=records.filter(r=>r.event==='request_end'&&r.operation===op);assert(start.length>0);assert.equal(start.length,end.length,op+' paired requests');}
 report.log_records=records.length;report.posts=posts();report.puts=puts();report.local_unchanged=true;report.passed=true;pass('Local HEAD/index/config/files unchanged; diagnostic spans paired, no drops or private payloads; graceful exit');
 }catch(e){report.error=String(e.stack||e);process.exitCode=1;try{if(client)await capture('failure');}catch{}try{await shutdown();}catch(cleanup){report.cleanup_error=String(cleanup);}}
 finally{server.closeAllConnections();server.close();report.after=local();fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));console.log(JSON.stringify(report,null,2));}
}
run().catch(e=>{console.error(e);process.exitCode=1});
