// Focused native localhost acceptance. Only disposable dirty repositories,
// synthetic accounts and a numeric loopback API; no real credentials/remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto');
const http=require('node:http'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/pr-collaboration-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'fixture-repo'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/pr-collaboration';
const token='fixture_collaboration_'+crypto.randomBytes(12).toString('hex'),privateBody='PRIVATE review 資料 🚀\nPreserve local edits.';
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0',http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:''});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'資料.txt'),'committed\n');git(['add','-A']);git(['commit','-m','Synthetic fixture']);
fs.writeFileSync(path.join(repo,'資料.txt'),'staged\n');git(['add','-A']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');fs.writeFileSync(path.join(repo,'untracked.txt'),'private local edits');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const local=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'untracked.txt'))}),before=local();
let service,client,state,mode='ok',head='a'.repeat(40),base='b'.repeat(40),merged=false,resolved=false,closedAt=null,provider='github',revision=7,accountName='fixture';
const requests=[],submitted=[],replies=[],report={passed:false,root,cases:[],cancel:[],interrupted_requests:[],before};
function diagnostics(){const logs=[];function walk(p){for(const f of fs.readdirSync(p,{withFileTypes:true})){const n=path.join(p,f.name);if(f.isDirectory())walk(n);else if(/^rocket(?:\.\d+)?\.jsonl$/.test(f.name))logs.push(fs.readFileSync(n,'utf8'));}}walk(root);return logs.join('\n').split(/\r?\n/).filter(Boolean).map(JSON.parse);}
const guid='11111111-2222-4333-8444-555555555555',otherGuid='66666666-7777-4888-8999-aaaaaaaaaaaa';
const providers=['github','github-enterprise','gitlab','bitbucket','bitbucket-server','azure','azure-server','gitea'];
const servers={github:'https://github.com','github-enterprise':'https://enterprise.example.invalid',gitlab:'https://gitlab.example.invalid',bitbucket:'https://bitbucket.org','bitbucket-server':'https://bitbucket.example.invalid',azure:'https://dev.azure.com/fixture','azure-server':'https://devops.example.invalid/tfs/Collection',gitea:'https://gitea.example.invalid'};
const prefix=p=>p==='github-enterprise'?'/github-enterprise/api/v3':p==='azure-server'?'/azure-server/tfs/Collection':'/'+p;
const gh=p=>p==='github'||p==='github-enterprise',az=p=>p==='azure'||p==='azure-server';
const api=p=>p==='gitlab'?'/projects/team%2Fsample/merge_requests/42':p==='bitbucket'?'/repositories/team/sample/pullrequests/42':p==='bitbucket-server'?'/projects/team/repos/sample/pull-requests/42':az(p)?'/team/_apis/git/repositories/sample/pullrequests/42':'/repos/team/sample/pulls/42';
function actor(p,own=true){const name=own?accountName:'contributor';if(p==='gitlab')return{username:name,id:own?1:2};if(p==='bitbucket')return{uuid:own?'{fixture}':'{contributor}',display_name:name};if(p==='bitbucket-server')return{name,slug:name+'-slug',displayName:name};if(az(p))return{id:own?guid:otherGuid,displayName:name,isActive:true,isContainer:false,providerDisplayName:name};return{login:name,id:own?1:2};}
function summary(p){const common={title:'PRIVATE PR 資料',description:'PRIVATE description',body:'PRIVATE description',draft:false};if(gh(p)||p==='gitea')return{...common,number:42,user:actor(p,false),state:merged?'closed':'open',merged,mergeable:true,mergeable_state:mode==='blocked'?'blocked':'clean',head:{sha:head,ref:'feature/界'},base:{sha:base,ref:'main'},commits:1,changed_files:1,additions:2,deletions:1,merge_commit_sha:'e'.repeat(40)};
 if(p==='gitlab')return{...common,iid:42,author:actor(p,false),state:merged?'merged':'opened',sha:head,source_branch:'feature/界',target_branch:'main',detailed_merge_status:mode==='blocked'?'not_approved':'mergeable',diff_refs:{start_sha:base,base_sha:'9'.repeat(40),head_sha:head},merge_commit_sha:'e'.repeat(40)};
 if(p==='bitbucket')return{...common,id:42,author:actor(p,false),state:merged?'MERGED':'OPEN',source:{branch:{name:'feature/界'},commit:{hash:head}},destination:{branch:{name:'main'},commit:{hash:base}},participants:[],merge_commit:{hash:'e'.repeat(40)}};
 if(p==='bitbucket-server')return{...common,id:42,version:revision,author:{user:actor(p,false)},state:merged?'MERGED':'OPEN',fromRef:{id:'refs/heads/feature/界',latestCommit:head},toRef:{id:'refs/heads/main',latestCommit:base},reviewers:[],properties:{mergeCommit:{id:'e'.repeat(40)}}};
 return{...common,pullRequestId:42,createdBy:actor(p,false),status:merged?'completed':'active',isDraft:false,mergeStatus:mode==='blocked'?'conflicts':'succeeded',lastMergeSourceCommit:{commitId:head},lastMergeTargetCommit:{commitId:base},lastMergeCommit:{commitId:'e'.repeat(40)},sourceRefName:'refs/heads/feature/界',targetRefName:'refs/heads/main',reviewers:[]};
}
const page=(p,items)=>az(p)?{value:items,count:items.length}:p==='bitbucket'?{values:items}:p==='bitbucket-server'?{values:items,size:items.length,start:0,isLastPage:true}:items;
function note(p,text=privateBody){return{id:81,body:text,text,content:az(p)?text:{raw:text},author:actor(p),user:actor(p),created_at:'2026-10-28',created_on:'2026-10-28',publishedDate:'2026-10-28'};}
function vote(p,a){if(p==='gitlab')return{iid:42,approved_by:[{user:actor(p)}]};if(az(p))return{...actor(p),vote:a==='approve'?10:-10};if(p==='bitbucket-server')return{user:actor(p),status:a==='approve'?'APPROVED':'NEEDS_WORK',lastReviewedCommit:head};return{user:actor(p),approved:a==='approve',state:a==='approve'?'approved':'changes_requested'};}
function review(p,b){return{id:100+submitted.length,user:actor(p),body:b.body,state:b.event==='APPROVE'||b.event==='APPROVED'?'APPROVED':b.event==='REQUEST_CHANGES'?(p==='gitea'?'REQUEST_CHANGES':'CHANGES_REQUESTED'):(p==='gitea'?'COMMENT':'COMMENTED'),commit_id:b.commit_id,submitted_at:'2026-10-28T00:00:00Z'};}
const isWrite=r=>r.method!=='GET'&&!(r.path.endsWith('/graphql')&&r.body.includes('query RocketReviewThreads'));
const writes=()=>requests.filter(isWrite).length;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const server=http.createServer(async(req,res)=>{try{
 let data='';for await(const chunk of req)data+=chunk;assert(data.length<128*1024);const url=new URL(req.url,'http://127.0.0.1');
 const p=providers.find(p=>url.pathname.startsWith(prefix(p)+'/'))||(url.pathname==='/github-enterprise/api/graphql'?'github-enterprise':null);assert(p,'unscoped route');
 const route=url.pathname==='/github-enterprise/api/graphql'?'/graphql':url.pathname.slice(prefix(p).length),pr=api(p);
 requests.push({provider:p,method:req.method,path:url.pathname,query:url.search,body:data});
 const send=(code,body,headers={})=>{res.writeHead(code,{'Content-Type':'application/json',...headers});res.end(body===undefined?'':JSON.stringify(body));};
 const auth=az(p)?'Basic '+Buffer.from(':'+token).toString('base64'):p==='gitea'?'token '+token:'Bearer '+token;assert.equal(req.headers.authorization,auth);
 const hold=()=>{res.on('close',()=>closedAt=performance.now());};
 if(route==='/user')return send(200,p==='bitbucket'?actor(p):p==='gitlab'?actor(p):actor(p));
 if(route==='/application-properties')return send(200,{}, {'X-AUSERNAME':'fixture'});
 if(route.startsWith('/_apis/connectionData'))return send(200,{authenticatedUser:actor(p),authorizedUser:{id:guid}});
 if(mode==='auth')return send(401,{message:token+' PRIVATE rejected'});
 if(route==='/graphql'){
   const b=JSON.parse(data);assert(!b.query.includes('http'));if(b.query.startsWith('query ')){
     assert.deepEqual(b.variables,{owner:'team',name:'sample',number:42});if(mode==='slow-threads')return hold();
     const t={id:'PRRT_fixture',path:'資料/新.rs',startLine:1,line:2,isResolved:resolved,isOutdated:false,viewerCanResolve:mode!=='no-resolve',viewerCanUnresolve:true,comments:{pageInfo:{hasNextPage:false},nodes:[{databaseId:8,author:{login:'contributor'},body:'PRIVATE original thread',createdAt:'2026-10-28',path:'資料/新.rs',line:2,replyTo:null},...replies.map((r,i)=>({databaseId:90+i,author:{login:'fixture'},body:r,createdAt:'2026-10-28',path:'資料/新.rs',line:2,replyTo:{databaseId:8}}))]}};
     return send(200,{data:{repository:{nameWithOwner:'team/sample',pullRequest:{number:42,headRefOid:head,baseRefOid:base,reviewThreads:{pageInfo:{hasNextPage:mode==='partial-threads'},nodes:[t]}}}}});
   }
   assert.equal(b.variables.id,'PRRT_fixture');assert(b.query.startsWith('mutation RocketThread'));if(mode==='reject')return send(403,{});resolved=b.query.includes('{resolveReviewThread');const key=resolved?'resolveReviewThread':'unresolveReviewThread';if(mode==='slow-thread-write')return hold();return send(200,{data:{[key]:{thread:{id:'PRRT_fixture',isResolved:resolved}}}});
 }
 if(route==='/repos/team/sample')return send(200,{full_name:'team/sample',archived:false,permissions:{push:true},allow_merge_commit:true,allow_merge_commits:true,allow_squash_merge:true,allow_rebase_merge:true,allow_rebase:true});
 if(route.includes('/branches/')||route.endsWith('/refs'))return send(200,az(p)?{value:[{name:'refs/heads/main',objectId:base}]}:p==='gitlab'||p==='gitea'?{commit:{id:base}}:p==='bitbucket'?{target:{hash:base}}:{commit:{sha:base}});
 if(p==='bitbucket-server'&&route.endsWith('/branches'))return send(200,page(p,[{id:'refs/heads/main',latestCommit:base}]));
 if(route==='/users')return send(200,page(p,[actor(p)]));
 if(req.method==='GET'&&route===pr+'/merge')return send(200,{canMerge:mode!=='blocked',conflicted:false,vetoes:[]});
 if(req.method==='GET'&&route===pr){if(mode==='slow-read')return hold();return send(200,summary(p));}
 if(req.method==='GET'&&route===pr.slice(0,pr.lastIndexOf('/')))return send(200,page(p,[summary(p)]));
 if(req.method==='GET'){
   if(route.endsWith('/commits'))return send(200,page(p,[{sha:head,id:head,hash:head,commitId:head,message:'PRIVATE commit',comment:'PRIVATE commit',author:{name:'Synthetic author'},commit:{message:'PRIVATE commit',author:{name:'Synthetic author'}}}]));
   if(route.endsWith('/files'))return send(200,[{filename:'資料/新.rs',previous_filename:'資料/旧.rs',status:'renamed',additions:2,deletions:1,patch:'@@ -1 +1,2 @@\n-old\n+new 界\n+another'}]);
   if(route.endsWith('/diffs'))return send(200,[{new_path:'資料/新.rs',old_path:'資料/旧.rs',renamed_file:true,diff:'@@ -1 +1,2 @@\n-old\n+new 界\n+another'}]);
   if(route.includes('/diffstat/')){assert(route.endsWith(head+'..'+base));return send(200,page(p,[{new:{path:'資料/新.rs'},old:{path:'資料/旧.rs'},status:'renamed',lines_added:2,lines_removed:1}]));}
   if(route.endsWith('/iterations'))return send(200,{value:[{id:2}]});
   if(route.endsWith('/changes'))return send(200,az(p)?{changeEntries:[{item:{path:'/資料/新.rs'},originalPath:'/資料/旧.rs',changeType:'rename'}]}:page(p,[{path:{toString:'資料/新.rs'},srcPath:{toString:'資料/旧.rs'},type:'MOVE'}]));
   if(route.endsWith('/activities'))return send(200,page(p,[{comment:note(p)}]));
   if(route.endsWith('/threads'))return send(200,{value:[{id:9,comments:[note(p)]}]});
   if(route.endsWith('/notes')||route.endsWith('/comments'))return send(200,page(p,gh(p)&&route.includes('/pulls/')?[]:[note(p)]));
   if(route.endsWith('/approvals'))return send(200,{approved_by:[]});
   if(route.endsWith('/reviews'))return send(200,submitted.filter(r=>r.provider===p).map(r=>r.result));
   if(route.endsWith('/check-runs'))return send(200,{total_count:0,check_runs:[]});
   if(route.endsWith('/status'))return send(200,{sha:head,statuses:[]});
   if(route.endsWith('/statuses')||route.endsWith('/pipelines'))return mode==='checks-denied'?send(403,{message:token}):send(200,page(p,[{state:'success',status:'success',name:'Synthetic check'}]));
 }
 const merge=(az(p)&&req.method==='PATCH'&&route===pr)||route===pr+'/merge';
 if(merge){const b=JSON.parse(data);assert(req.method===(gh(p)||p==='gitlab'?'PUT':az(p)?'PATCH':'POST'));
   if(gh(p)||p==='gitlab')assert.equal(b.sha,head);if(p==='gitlab'){assert.equal(b.auto_merge,false);assert.equal(b.should_remove_source_branch,false);}
   if(p==='bitbucket'){assert.equal(b.type,'pullrequest');assert.equal(b.close_source_branch,false);assert.equal(b.merge_strategy,'squash');}
   if(p==='bitbucket-server'){assert.equal(b.version,revision);assert.equal(b.strategyId,'squash');assert.equal(b.autoMerge,false);}
   if(az(p)){assert.equal(b.lastMergeSourceCommit.commitId,head);assert.equal(b.completionOptions.bypassPolicy,false);assert.equal(b.completionOptions.deleteSourceBranch,false);assert.equal(b.completionOptions.mergeStrategy,'squash');}
   if(p==='gitea'){assert.equal(b.head_commit_id,head);assert.equal(b.force_merge,false);assert.equal(b.delete_branch_after_merge,false);assert.equal(b.merge_when_checks_succeed,false);}
   if(mode==='merge-reject')return send(409,{message:token});if(mode==='slow-merge')return hold();merged=true;
   return p==='gitea'?send(204):send(200,gh(p)?{merged:true,sha:'e'.repeat(40)}:summary(p));
 }
 if(route===pr+'/comments/8/replies'){const b=JSON.parse(data);replies.push(b.body);if(mode==='slow-thread-write')return hold();return send(201,{id:99,user:actor(p),body:b.body,created_at:'2026-10-28',path:'資料/新.rs',line:2,in_reply_to_id:8});}
 if(req.method!=='GET'){
   const b=JSON.parse(data);if(mode==='reject')return send(422,{message:token+' PRIVATE rejected'});let result;
   if(route.endsWith('/reviews')){assert.equal(b.commit_id,head);if(p==='gitea')assert(['COMMENT','APPROVED','REQUEST_CHANGES'].includes(b.event));result=review(p,b);if(b.comments){assert.equal(b.comments[0].start_line,1);assert.equal(b.comments[0].line,2);assert.equal(b.comments[0].start_side,'RIGHT');assert.equal(b.comments[0].path,'資料/新.rs');}}
   else if(route.endsWith('/approve')){if(p==='gitlab')assert.equal(b.sha,head);result=vote(p,'approve');}
   else if(route.endsWith('/request-changes'))result=vote(p,'changes');
   else if(route.includes('/participants/')){assert(route.endsWith('/fixture-slug'));result=vote(p,b.status==='APPROVED'?'approve':'changes');}
   else if(route.includes('/reviewers/')){assert(route.endsWith('/'+guid));assert.equal(b.id,guid);result=vote(p,b.vote===10?'approve':'changes');}
   else if(route.endsWith('/threads')){assert.equal(b.comments[0].content,privateBody);result={id:9,comments:[note(p)]};}
   else if(route.endsWith('/notes')||route.endsWith('/comments')){assert.equal(p==='bitbucket'?b.content.raw:p==='bitbucket-server'?b.text:b.body,privateBody);result=note(p);}
   else throw Error('Unexpected write '+req.method+' '+route);
   submitted.push({provider:p,result});if(mode==='slow-review')return hold();if(mode==='invalid-review')return send(201,{});return send(201,result);
 }
 throw Error('Unexpected '+req.method+' '+route);
}catch(e){report.server_error=String(e.stack||e);res.writeHead(500);res.end('{}');}});
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
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const image=r.content.find(x=>x.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));}
async function resize(w,h){await client.tool('rocket_resize',{width:w,height:h});await until(s=>s.viewport[0]===w&&s.viewport[1]===h&&!s.input_pending);await settled();}
async function openHosting(){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);await click('repository-remotes-menu');await stable();for(let i=0;i<14&&!(control('repository-hosting')&&control('repository-hosting').rect[3]-control('repository-hosting').rect[1]>25);i++)await scroll('repository-menu-scroll','down',230);await click('repository-hosting');await until(s=>s.dialog?.kind==='hosting'&&!s.hosting.busy);await settled();}
async function connect(p){provider=p;await click('hosting-tab-accounts');if(control('hosting-new-account'))await click('hosting-new-account');await click('hosting-provider-'+p);await type('hosting-label','Synthetic '+p);if(!['github','bitbucket'].includes(p))await type('hosting-server',servers[p]);await type('hosting-token',token);if(state.hosting.remember)await click('hosting-remember');await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);await type('hosting-repository','team/sample');await click('hosting-refresh');await click('hosting-details-0');assert(state.hosting.pull.details,state.hosting.error);assert.equal(state.hosting.pull.details.head,head);}
async function refresh(){await click('hosting-pull-refresh');assert(!state.hosting.error,state.hosting.error);}
async function draft(action='comment',body=privateBody){if(!state.hosting.pull.composing)await click('hosting-review-write');if(state.hosting.pull.reviewing)await click('hosting-review-edit');await click('hosting-review-'+action);await type('hosting-review-body',body);await click('hosting-review-preview');assert(state.hosting.pull.reviewing,state.hosting.error);}
async function cancel(kind){await reveal('hosting-cancel');const at=performance.now();await client.tool('rocket_click',{target:'hosting-cancel',expected_tab:state.active_tab});await until(s=>s.hosting.stopping||!s.hosting.busy);const feedback=performance.now()-at;await settled();assert(closedAt!==null,'curl connection not closed');report.cancel.push({provider,kind,feedback_ms:feedback,connection_closed_ms:closedAt-at,settled_ms:performance.now()-at});}
const exe=path.resolve(process.env.ROCKET_PR_EXE||'target/release/rocket-mcp.exe');
async function launch(){const out=fs.openSync(path.join(root,'service.log'),'a');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>100)throw e;await delay(100);}}await client.initialize();await client.tool('rocket_open',{path:repo});await settled();await resize(1440,900);if(control('console-close'))await click('console-close');await openHosting();}
async function closeWindow(){if(state?.hosting?.busy)await click('hosting-cancel');if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}}
async function stop(){if(service&&service.exitCode===null&&service.signalCode===null){await closeWindow();await client.raw('POST','/shutdown');const at=performance.now();while(service.exitCode===null&&service.signalCode===null){assert(performance.now()-at<15000,'owned service did not stop');await delay(60);}assert.equal(service.exitCode,0);}if(client)await client.finish();}
async function restart(crash=false){const n=writes();if(crash){const rows=diagnostics();for(const s of rows.filter(r=>r.event==='request_start'&&r.operation==='hosting-review-submit'))if(!rows.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request))report.interrupted_requests.push(s.session+'/'+s.request);service.kill();while(service.exitCode===null&&service.signalCode===null)await delay(50);await client.finish();}else await stop();await launch();assert.equal(writes(),n,'restart replayed a mutation');}
async function githubCases(){
 await connect('github');await click('hosting-pull-tab-files');await click('hosting-inline-line-0-2-right');await type('hosting-inline-body','PRIVATE range 界');
 await type('hosting-inline-end','2');await click('hosting-inline-save');assert.equal(state.hosting.pull.comments[0].start,1);assert.equal(state.hosting.pull.comments[0].line,2);await type('hosting-review-body',privateBody);
 await delay(300);await settled();assert(state.hosting.drafts.count>0);const vault=path.join(root,'rocket-ui-state/hosting-drafts.dpapi');assert(fs.existsSync(vault));assert(!fs.readFileSync(vault).includes(Buffer.from('PRIVATE')));
 await restart();await connect('github');assert.equal(state.hosting.pull.body_bytes,Buffer.byteLength(privateBody));assert.equal(state.hosting.pull.comments.length,1);assert(!state.hosting.pull.reviewing);accountName='another';await connect('github');assert(state.hosting.error?.includes('original account'));assert.equal(state.hosting.pull.body_bytes,Buffer.byteLength(privateBody));accountName='fixture';await connect('github');assert(!state.hosting.error,state.hosting.error);await draft();await click('hosting-review-submit');assert(!state.hosting.error,state.hosting.error);assert.equal(state.hosting.pull.comments.length,0);
 pass('Protected review and multiline Unicode location survive restart; no saved preview or automatic submit; exact range payload');
 await click('hosting-pull-tab-discussion');await click('hosting-threads-refresh');assert.equal(state.hosting.pull.threads.count,1);await click('hosting-thread-reply-0');await type('hosting-thread-body','PRIVATE reply 界');
 await delay(300);await settled();await restart();await connect('github');await click('hosting-pull-tab-discussion');await click('hosting-threads-refresh');assert.equal(state.hosting.pull.threads.body_bytes,Buffer.byteLength('PRIVATE reply 界'));await click('hosting-thread-preview');assert(state.hosting.pull.threads.reviewing,state.hosting.error);await resize(780,620);await reveal('hosting-thread-submit');await capture('thread-compact');await click('hosting-thread-submit');assert(!state.hosting.error,state.hosting.error);assert.equal(replies.length,1);await resize(1440,900);
 for(const a of ['resolve','unresolve']){await click('hosting-threads-refresh');await click('hosting-thread-'+a+'-0');await click('hosting-thread-preview');assert(state.hosting.pull.threads.reviewing,state.hosting.error);await click('hosting-thread-submit');assert(!state.hosting.error,state.hosting.error);await click('hosting-threads-refresh');assert.equal(state.hosting.pull.threads.resolved[0],a==='resolve');}
 mode='no-resolve';await click('hosting-threads-refresh');await reveal('hosting-thread-resolve-0');assert(!control('hosting-thread-resolve-0').enabled);mode='ok';await click('hosting-threads-refresh');
 await click('hosting-thread-reply-0');await type('hosting-thread-body','PRIVATE cancelled reply');await click('hosting-thread-preview');head='c'.repeat(40);let n=writes();await click('hosting-thread-submit');assert(state.hosting.error.includes('hosting_pull_changed'));assert.equal(writes(),n);await refresh();await click('hosting-threads-refresh');await click('hosting-thread-preview');mode='slow-thread-write';closedAt=null;n=writes();await click('hosting-thread-submit',false);await until(s=>s.hosting.busy&&writes()>n);await cancel('thread-reply');assert(state.hosting.pull.threads.uncertain);await delay(300);await settled();mode='ok';await restart(true);await connect('github');assert(state.hosting.pull.threads.uncertain);assert(!state.hosting.pull.threads.can_acknowledge);await click('hosting-pull-tab-discussion');await click('hosting-threads-refresh');assert(state.hosting.pull.threads.can_acknowledge);await click('hosting-thread-acknowledge');await click('hosting-thread-discard');
 pass('Reply and resolve/reopen use previews, permissions and fresh revisions; interrupted reply survives process loss and needs explicit reconciliation');
 await draft();mode='slow-review';closedAt=null;n=writes();await click('hosting-review-submit',false);await until(s=>s.hosting.busy&&writes()>n);await cancel('review');assert(state.hosting.pull.uncertain);mode='ok';await delay(300);await settled();await restart(true);await connect('github');assert(state.hosting.pull.uncertain);assert(state.hosting.pull.can_acknowledge);await click('hosting-review-acknowledge');await click('hosting-review-discard');
 pass('Uncertain review remains scoped after crash; fresh review read and acknowledgment required');
 await draft();mode='slow-review';n=writes();await click('hosting-review-submit',false);await until(s=>s.hosting.busy&&writes()>n);await restart(true);mode='ok';await connect('github');assert(state.hosting.pull.uncertain,'write-ahead record was not durable before the request');assert(state.hosting.pull.can_acknowledge);await click('hosting-review-acknowledge');await click('hosting-review-discard');
 pass('Abrupt process loss during an in-flight write restores uncertainty from the pre-send recovery record');
 await click('hosting-merge-method-squash');await click('hosting-merge-preview');mode='slow-merge';closedAt=null;n=writes();await click('hosting-merge-submit',false);await until(s=>s.hosting.busy&&writes()>n);await cancel('merge');assert(state.hosting.pull.merge.uncertain);mode='ok';await delay(300);await settled();await restart();await connect('github');assert(state.hosting.pull.merge.uncertain);assert(state.hosting.pull.merge.can_acknowledge);await click('hosting-merge-acknowledge');
 pass('Uncertain merge survives normal restart without retry; refresh and explicit acknowledgment restore controls');
 await stop();const savedVault=fs.readFileSync(vault);fs.writeFileSync(vault,'external invalid data');await launch();assert(state.hosting.drafts.error);await click('hosting-tab-drafts');await click('hosting-drafts-retry');assert(state.hosting.drafts.error);await stop();assert.equal(fs.readFileSync(vault,'utf8'),'external invalid data');fs.writeFileSync(vault,savedVault);await launch();
 pass('Unreadable externally edited draft store is preserved and an idle app can still close');
}
async function providerCases(p){
 merged=false;mode='ok';head='a'.repeat(40);base='b'.repeat(40);await connect(p);assert.equal(state.hosting.pull.details.files.count,1);assert(state.hosting.pull.details.paths[0].path.includes('資料'));assert.equal(state.hosting.pull.details.commits.count,1);await capture(p+'-details');
 mode='auth';let reads=writes();await click('hosting-pull-refresh');assert(state.hosting.error.includes('hosting_unauthorized'),state.hosting.error);assert.equal(writes(),reads);assert(!state.hosting.connected);mode='ok';await connect(p);
 mode='slow-read';closedAt=null;reads=requests.length;await click('hosting-pull-refresh',false);await until(s=>s.hosting.busy&&requests.length>reads);await cancel('details-read');assert(state.hosting.pull.details);mode='ok';await refresh();
 if(p==='github-enterprise'){await click('hosting-pull-tab-discussion');await click('hosting-threads-refresh');assert(state.hosting.pull.threads.loaded);await click('hosting-thread-resolve-0');await click('hosting-thread-preview');await click('hosting-thread-submit');assert(!state.hosting.error,state.hosting.error);resolved=false;}
 for(const [a,status] of [['comment','COMMENTED'],['approve','APPROVED'],['changes','CHANGES_REQUESTED']]){
   if(p==='gitlab'&&a==='changes'){await click('hosting-review-write');await reveal('hosting-review-changes');assert(!control('hosting-review-changes').enabled);continue;}
   await draft(a,a==='comment'||gh(p)||p==='gitea'?privateBody:'');const n=writes();await click('hosting-review-submit');assert(!state.hosting.error,p+': '+state.hosting.error);assert.equal(writes(),n+1);assert.equal(state.hosting.pull.submitted.state,status);
 }
 if(state.hosting.pull.composing)await click('hosting-review-discard');
 await draft();let n=writes();head='d'.repeat(40);await click('hosting-review-submit');assert(state.hosting.error.includes('hosting_pull_changed'),state.hosting.error);assert.equal(writes(),n);assert(state.hosting.pull.body_bytes>0);await refresh();
 await draft();mode='reject';n=writes();await click('hosting-review-submit');assert(state.hosting.error.includes('hosting_pull_review_rejected'));assert.equal(writes(),n+1);mode='ok';await click('hosting-review-discard');
 await click('hosting-merge-method-squash');await click('hosting-merge-preview');assert(state.hosting.pull.merge.reviewing,p+': '+state.hosting.error);n=writes();base='f'.repeat(40);await click('hosting-merge-submit');assert(state.hosting.error.includes('hosting_pull_changed'));assert.equal(writes(),n);await refresh();await click('hosting-merge-preview');mode='merge-reject';await click('hosting-merge-submit');assert(state.hosting.error.includes('hosting_pull_merge_rejected'),state.hosting.error);assert(!state.hosting.pull.merge.uncertain);mode='ok';
 await click('hosting-merge-preview');mode='slow-merge';closedAt=null;n=writes();await click('hosting-merge-submit',false);await until(s=>s.hosting.busy&&writes()>n);await cancel('provider-merge');assert(state.hosting.pull.merge.uncertain);mode='ok';await refresh();await click('hosting-merge-acknowledge');await click('hosting-merge-preview');await click('hosting-merge-submit');assert(!state.hosting.error,p+': '+state.hosting.error);assert.equal(state.hosting.pull.merge.result,'e'.repeat(40));
 await refresh();assert(state.hosting.pull.details.merged);assert.deepEqual(local(),before);pass(p+': details, authentication error, cancelled read, supported reviews, changed tips, rejection, cancelled merge recovery and confirmed merge; local state preserved');
}
async function run(){try{
 await new Promise(r=>server.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+server.address().port);report.executable_sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');await launch();
 const only=process.argv.find(a=>a.startsWith('--provider='))?.slice(11);
 if(!only||only==='github')await githubCases();for(const p of providers.slice(1).filter(p=>!only||p===only))await providerCases(p);
 assert(!report.server_error,report.server_error);await stop();assert.deepEqual(local(),before);
 const logs=[];function walk(p){for(const f of fs.readdirSync(p,{withFileTypes:true})){const n=path.join(p,f.name);if(f.isDirectory())walk(n);else if(/^rocket(?:\.\d+)?\.jsonl$/.test(f.name))logs.push(fs.readFileSync(n,'utf8'));}}walk(root);const records=logs.join('\n').split(/\r?\n/).filter(Boolean).map(JSON.parse),text=JSON.stringify(records);assert(records.length>0);assert(records.every(r=>!r.dropped_before));for(const secret of [token,Buffer.from(':'+token).toString('base64'),'PRIVATE','Authorization','team/sample','資料/新.rs'])assert(!text.includes(secret),'private payload in logs');
 for(const op of ['hosting-pull-details','hosting-review-preview','hosting-review-submit','hosting-merge-preview','hosting-merge-submit']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length);for(const s of starts)assert(records.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request)||(op==='hosting-review-submit'&&report.interrupted_requests.includes(s.session+'/'+s.request)),'unexpected incomplete request '+op);}
 report.logs=records.length;report.requests=requests.length;report.local_unchanged=true;report.passed=true;pass('Redacted diagnostics with only deliberately crashed writes allowed incomplete; no dropped events, unchanged local state and clean owned-service shutdown');
}catch(e){report.error=String(e.stack||e);process.exitCode=1;try{await capture('failure');}catch{}try{await stop();}catch(cleanup){report.cleanup_error=String(cleanup);if(service?.exitCode===null)service.kill();}}
finally{server.closeAllConnections();server.close();if(client)await client.finish();const access=path.join(profile,'access-token');if(fs.existsSync(access))fs.unlinkSync(access);report.after=local();fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync('work/pr-collaboration-last-result',path.join(root,'result.json'));console.log(JSON.stringify(report,null,2));}}
run().catch(e=>{console.error(e);process.exitCode=1;});
