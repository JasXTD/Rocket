// Insights acceptance through the native UI + localhost MCP. Synthetic local
// repositories and loopback API only; no personal login, hosted writes or Git pushes.
'use strict';
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto');
const http=require('node:http'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/insights-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'profile'),exe=path.resolve(process.env.ROCKET_INSIGHTS_EXE||'target/release/rocket-mcp.exe');
const token='fixture_insights_'+crypto.randomBytes(12).toString('hex'),branch='rocket-ui-test/insights';
for(const d of ['home','local','roaming','template','hooks','shots'])fs.mkdirSync(path.join(root,d),{recursive:true});
fs.writeFileSync(path.join(root,'empty.config'),'');fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.writeFileSync(path.join(root,'rocket-update-api'),'http://127.0.0.1:1');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_|ROCKET_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'template'),USERPROFILE:path.join(root,'home'),HOME:path.join(root,'home'),LOCALAPPDATA:path.join(root,'local'),APPDATA:path.join(root,'roaming'),http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:''});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function fixture(name,remote=true){const repo=path.join(root,name);fs.mkdirSync(repo);const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8'}).trim();git(['init','--quiet','--initial-branch='+branch]);for(const[k,v]of[['user.name','PRIVATE 測試'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',path.join(root,'hooks')]])git(['config',k,v]);fs.writeFileSync(path.join(repo,'資料.md'),'PRIVATE synthetic content\n');git(['add','.']);git(['commit','--quiet','-m','PRIVATE 初期']);fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);if(remote)git(['remote','add','origin','https://github.com/team/insights.git']);fs.appendFileSync(path.join(repo,'資料.md'),'local edits stay safe\n');return{repo,git,snapshot:()=>[git(['rev-parse','HEAD']),hash(path.join(repo,'.git/index')),hash(path.join(repo,'.git/config')),hash(path.join(repo,'資料.md'))]};}
const sample=fixture('PRIVATE_資料'),other=fixture('PRIVATE_other',false),before=sample.snapshot();
let mode='normal',detailMode='normal',reviewMode='normal',held=[],detailHeld=[],reviewHeld=[],requests=[],server,detailReads=0;
const head='a'.repeat(40),base='b'.repeat(40);
const now=Date.now(),ago=d=>new Date(now-d*86400000).toISOString(),after=(iso,s)=>new Date(Date.parse(iso)+s*1000).toISOString();
const row=(number,state,created,merged)=>({number,title:'PRIVATE Unicode 改善 '+number,state,created_at:created,merged_at:merged,updated_at:merged??created,user:{login:'fixture'},head:{ref:'feature/資料'},base:{ref:'main'},requested_reviewers:[],requested_teams:[]});
const rows=[row(1,'open',ago(10),null),row(2,'closed',ago(8),ago(6)),row(3,'closed',ago(70),ago(60)),row(4,'closed','invalid',ago(2))];
rows[0].user.login='Alice';rows[1].user.login='林 Example';rows[2].user.login='林 Example';rows[3].user.login='@formula';
// #1 still waits on a person and a team. #2's first review by a person comes an hour after it
// opened; a bot commented sooner and must not count.
rows[0].requested_reviewers=[{login:'Reviewer'}];rows[0].requested_teams=[{slug:'core',name:'Core'}];
const reviewsOf=n=>n===2?[{id:21,user:{login:'copilot[bot]',type:'Bot'},body:'PRIVATE reviewed',state:'COMMENTED',commit_id:head,submitted_at:after(rows[1].created_at,600)},
 {id:22,user:{login:'Reviewer',type:'User'},body:'PRIVATE reviewed',state:'COMMENTED',commit_id:head,submitted_at:after(rows[1].created_at,3600)}]
 :n===3?[{id:31,user:{login:'Reviewer',type:'User'},body:'PRIVATE reviewed',state:'APPROVED',commit_id:head,submitted_at:ago(1)}]:[];
rows[1].title='=PRIVATE("資料", "quoted")';
const fifth=row(5,'closed',ago(12),ago(8));fifth.user.login='Alice';
const report={passed:false,root,cases:[]};const pass=s=>{report.cases.push(s);console.error('PASS '+s)};
server=http.createServer(async(req,res)=>{try{
 let body='';for await(const chunk of req)body+=chunk;
 const u=new URL(req.url,'http://127.0.0.1'),route=u.pathname.slice('/github'.length);
 assert(u.pathname.startsWith('/github/'));requests.push({method:req.method,route,query:u.search});
 const send=(status,data,headers={})=>{res.writeHead(status,{'Content-Type':'application/json',...headers});res.end(JSON.stringify(data))};
 if(req.headers.authorization!=='Bearer '+token)return send(401,{message:'invalid credentials'});
 if(route==='/user')return send(200,{login:'fixture'});
 if(route==='/graphql'){const v=JSON.parse(body).variables,r={nameWithOwner:'team/'+v.name};for(const[k,id]of Object.entries(v))if(/^c\d+$/.test(k))r[k]={oid:id,statusCheckRollup:null};return send(200,{data:{repository:r}})}
 assert.equal(req.method,'GET');
 if(route==='/repos/team/insights/pulls'){
  assert.equal(u.searchParams.get('state'),'all');const page=u.searchParams.get('page');
  if(mode==='hold'){held.push(()=>send(200,rows));return}
  if(mode==='unauthorized')return send(401,{message:'invalid credentials'});
  if(mode==='empty')return send(200,[]);
  if(mode==='bad-link')return send(200,rows,{Link:'<https://untrusted.invalid/steal>; rel="next"'});
  if(page==='1')return send(200,rows,{Link:`<http://127.0.0.1:${server.address().port}/github/repos/team/insights/pulls?state=all&sort=updated&direction=desc&per_page=50&page=2>; rel="next"`});
  assert.equal(page,'2');return send(200,[rows[0],fifth]);
 }
 const pull=route.match(/^\/repos\/team\/insights\/pulls\/(\d+)(?:\/(commits|files|comments|reviews))?$/);
 if(pull){
  const number=Number(pull[1]),item=[...rows,fifth].find(r=>r.number===number);assert(item);
  if(!pull[2]){
   detailReads++;
   const summary={...item,body:'PRIVATE description\n\nPlain data: <script>do not run</script>',draft:false,merged:!!item.merged_at,mergeable:true,mergeable_state:'clean',head:{sha:detailMode==='changed'&&detailReads%2===0?'c'.repeat(40):head,ref:item.head.ref},base:{sha:base,ref:'main'},commits:1,changed_files:1,additions:2,deletions:1};
   if(detailMode==='hold'){detailHeld.push(()=>send(200,summary));return}
   if(detailMode==='unauthorized')return send(401,{message:'invalid credentials'});
   return send(200,summary);
  }
  if(pull[2]==='commits')return send(200,[{sha:head,author:{login:item.user.login},commit:{message:'PRIVATE commit 資料'}}]);
  if(pull[2]==='files')return detailMode==='partial'?send(403,{message:'files unavailable'}):send(200,[{filename:'資料/新しい.rs',previous_filename:'資料/古い.rs',status:'renamed',additions:2,deletions:1,patch:'@@ -1 +1 @@\n-old\n+new'}]);
  if(pull[2]==='comments')return send(200,[]);
  if(pull[2]==='reviews'){
   // Read reviews asks for 100 a page; the details read asks for 50.
   if(u.searchParams.get('per_page')==='100'){
    if(number===5)return send(404,{message:'Not Found'});
    if(reviewMode==='hold'&&number===1){reviewHeld.push(()=>send(200,reviewsOf(number)));return}
   }
   return send(200,reviewsOf(number));
  }
 }
 if(/^\/repos\/team\/insights\/issues\/\d+\/comments$/.test(route))return send(200,[{id:2,user:{login:'Reviewer'},body:'PRIVATE comment',created_at:ago(1)}]);
 if(route==='/repos/team/insights')return send(200,{full_name:'team/insights',permissions:{pull:true,push:false}});
 if(route.endsWith('/check-runs'))return send(200,{total_count:0,check_runs:[]});
 if(route.endsWith('/status'))return send(200,{sha:route.split('/')[5],state:'pending',total_count:0,statuses:[]});
 throw Error('Unexpected mock request '+route);
}catch(e){report.server_error=String(e.stack);res.writeHead(500);res.end('{}')}});
let client,service,state;const delay=ms=>new Promise(r=>setTimeout(r,ms));const control=id=>state?.controls?.find(c=>c.id===id),stats=()=>state?.dialog?.statistics,pulls=()=>stats()?.pulls;
async function until(predicate,label='state',ms=45000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return;assert(performance.now()-start<ms,label+' timeout '+JSON.stringify({dialog:state.dialog?.kind,pulls:pulls(),hosting:state.hosting?.error}));await delay(70)}}
const settle=()=>until(s=>!s.input_pending&&!s.busy&&!s.hosting?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');
async function stable(){let last='',count=0;await until(s=>{const current=JSON.stringify(s.controls);count=current===last?count+1:0;last=current;return !s.input_pending&&count>=2},'stable')}
async function reveal(id){await stable();const visible=()=>control(id)?.rect?.[3]-control(id)?.rect?.[1]>18;if(visible())return;const regions=pulls()?.export?['insights-export-body']:['insights-scroll','insights-list-scroll','insights-detail-scroll','stats-scroll','hosting-scroll'];for(const region of regions)if(control(region)){for(const direction of ['down','up'])for(let i=0;i<16&&!visible();i++){await client.tool('rocket_scroll',{target:region,direction,pixels:160,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable()}}assert(visible(),'unreachable '+id)}
async function click(id){if(id==='view-statistics'){for(const target of ['menu-view','view-goto','goto-statistics'])await click(target);return}await reveal(id);assert(control(id)?.enabled,'disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});await settle()}
async function type(id,text){await reveal(id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle()}
async function resize(width,height){await until(s=>s.first_presentation&&!s.state_pending,'window initialized');await stable();await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending,`resize ${width}x${height}`);await settle()}
async function top(){await client.tool('rocket_scroll',{target:'insights-scroll',direction:'up',pixels:2000,expected_tab:state.active_tab});await settle();await stable()}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,'shots',name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'))}
async function close(){if(!client)return;for(let i=0;i<3&&state?.dialog;i++){await client.tool('rocket_escape');await settle()}await client.tool('rocket_close');await until(s=>s.window==='closed'||s.window==='not_started'||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.window==='closed')}await client.raw('POST','/shutdown')}
const loaded=()=>until(()=>pulls()?.loaded&&!pulls().running,'PR sample');
const detailLoaded=()=>until(()=>pulls()?.detail.loaded&&!pulls().detail.running,'PR details');
async function author(name){await click('insights-author');const index=pulls().authors.indexOf(name);assert(name===null||index>=0,'author exists');await click(name===null?'insights-author-all':'insights-author-'+index)}
function csvRecords(text){const out=[];let record=[],field='',quoted=false;for(let i=0;i<text.length;i++){const c=text[i];if(c==='"'){if(quoted&&text[i+1]==='"'){field+='"';i++}else quoted=!quoted}else if(c===','&&!quoted){record.push(field);field=''}else if(c==='\n'&&!quoted){record.push(field.replace(/\r$/,''));out.push(record);record=[];field=''}else field+=c}assert(!quoted);return out}
async function reviews(){
 // Waiting requests come with the list; nothing is read per pull request until asked.
 const reads=()=>requests.filter(r=>/\/pulls\/\d+\/reviews$/.test(r.route)&&r.query.includes('per_page=100'));
 assert.equal(reads().length,0,'reviews are read only when asked');
 await click('insights-filter-0');assert.equal(pulls().provider,'github');
 assert.deepEqual([pulls().waiting,pulls().requests,pulls().requests_known,pulls().requests_unknown],[1,2,1,0]);
 assert.deepEqual(pulls().reviewers.map(r=>[r.name,r.team,r.waiting,r.reviewed]),[['Reviewer',false,1,0],['team/core',true,1,0]]);
 let r=pulls().reviews;assert.deepEqual([r.opened,r.read,r.not_read,r.median_first_review_seconds],[3,0,3,null]);
 assert(control('insights-reviews-read')?.enabled,'Read reviews offered');await capture('reviews-unread');
 // Stop keeps what was read: #2 is the newest, #1 is held.
 reviewMode='hold';await click('insights-reviews-read');
 await until(()=>reviewHeld.length===1&&pulls().reviews.running&&pulls().reviews.read_ids.length===1,'review read held');
 assert.deepEqual(pulls().reviews.read_ids,[2]);await capture('reviews-reading');
 const started=performance.now();await click('stats-cancel');await until(()=>!pulls().reviews.running,'review stop',10000);report.review_cancel_ms=Math.round(performance.now()-started);
 assert.match(pulls().reviews.error,/Stopped/);assert.deepEqual(pulls().reviews.read_ids,[2]);
 for(const send of reviewHeld.splice(0))send();await stable();assert.deepEqual(pulls().reviews.read_ids,[2],'a late reply changes nothing');
 reviewMode='normal';await click('insights-reviews-read');
 await until(()=>!pulls().reviews.running&&pulls().reviews.read_ids.length===2,'reviews read');
 assert.deepEqual([pulls().reviews.read_ids,pulls().reviews.failed_ids],[[1,2],[5]]);
 r=pulls().reviews;assert.deepEqual([r.opened,r.read,r.reviewed,r.median_first_review_seconds,r.no_review,r.unavailable,r.not_read],[3,2,1,3600,1,1,0]);
 assert.deepEqual(pulls().reviewers.map(r=>[r.name,r.waiting,r.reviewed]),[['copilot[bot]',0,1],['Reviewer',1,1],['team/core',1,0]]);
 assert(reads().every(q=>q.query.includes('per_page=100&page=1')),'one page of 100 each');
 assert.deepEqual([...new Set(reads().map(q=>Number(q.route.split('/')[5])))].sort(),[1,2,5],'only the period\'s pull requests');
 await capture('reviews-read-wide');
 // A reviewer and the Waiting filter narrow the numbers and the rows.
 await click('insights-reviewer');await click('insights-reviewer-1');assert.deepEqual(pulls().reviewer,{name:'Reviewer',team:false});
 assert.deepEqual(pulls().visible_ids,[1,2]);assert.deepEqual([pulls().waiting,pulls().requests],[1,1]);
 await click('insights-filter-3');assert.deepEqual(pulls().visible_ids,[1]);await capture('reviews-reviewer-waiting');
 await resize(820,640);await top();await capture('reviews-compact');await resize(1440,900);
 await click('insights-filter-0');await click('insights-reviewer');await click('insights-reviewer-all');assert.equal(pulls().reviewer,null);
 pass('Waiting requests from the list, Read reviews on request with Stop, reviewer and Waiting filters, first-review median without bots, failures named');
}
async function explore(){
 assert.equal(detailReads,0,'list must not crawl PR details');await click('insights-filter-0');await author('林 Example');
 assert.deepEqual(pulls().visible_ids,[2,3]);assert.deepEqual([pulls().author_count,pulls().opened,pulls().merged,pulls().open_now],[2,1,1,0]);
 await click('insights-filter-1');assert.deepEqual(pulls().visible_ids,[]);assert.equal(pulls().author_count,2);await capture('author-empty');
 await click('insights-filter-2');await click('insights-pr-2');await detailLoaded();assert.equal(pulls().detail.snapshot_number,2);assert.deepEqual(pulls().detail.files,['資料/新しい.rs']);await capture('detail-summary-wide');
 await click('insights-detail-tab-1');await capture('detail-files-wide');
 await click('insights-detail-tab-2');await capture('detail-discussion');await click('insights-detail-tab-3');await capture('detail-checks');
 await resize(820,640);await reveal('insights-detail-tab-1');await click('insights-detail-tab-1');await capture('detail-compact');await resize(1440,900);
 pass('Author-scoped metrics, state-only row filtering, selected PR details, Unicode rename files and compact tabs');
 detailMode='hold';await click('insights-detail-refresh');await until(()=>detailHeld.length===1&&pulls().detail.running);await capture('detail-loading');
 const started=performance.now();await click('insights-detail-stop');await until(()=>!pulls().detail.running,'detail cancellation',10000);report.detail_cancel_ms=Math.round(performance.now()-started);
 assert.match(pulls().detail.error,/Stopped/);assert.deepEqual(pulls().visible_ids,[2,3]);for(const send of detailHeld.splice(0))send();await stable();assert(!pulls().detail.loaded);
 await click('insights-pr-2');await until(()=>detailHeld.length===1&&pulls().detail.running);detailMode='normal';await click('insights-pr-3');await detailLoaded();
 for(const send of detailHeld.splice(0))send();await stable();assert.equal(pulls().detail.snapshot_number,3);
 detailMode='hold';await click('insights-detail-refresh');await until(()=>detailHeld.length===1);await author('Alice');assert.equal(pulls().detail.number,null);for(const send of detailHeld.splice(0))send();await stable();assert(!pulls().detail.loaded);
 await author('林 Example');detailMode='changed';detailReads=0;await click('insights-pr-2');await until(()=>!pulls().detail.running&&pulls().detail.error);assert.match(pulls().detail.error,/chang|refresh|stale/i);assert(!pulls().detail.loaded);
 detailMode='unauthorized';await click('insights-detail-refresh');await until(()=>!pulls().detail.running&&pulls().detail.error);assert.match(pulls().detail.error,/auth|sign|credential|account/i);await capture('detail-auth-error');
 detailMode='partial';await click('insights-detail-refresh');await detailLoaded();assert.notEqual(pulls().detail.files_error,'None');await click('insights-detail-tab-1');await capture('detail-partial');
 detailMode='normal';await click('insights-detail-refresh');await detailLoaded();
 pass('Detail Stop, selection/filter cancellation, stale response isolation, changed remote revision, auth and partial errors');
 const exports=path.join(root,'exports 資料');fs.mkdirSync(exports);const expectedRows=pulls().visible_ids.slice();const apiBefore=requests.filter(r=>/\/pulls(?:\/|$)/.test(r.route)).length;
 await click('insights-export');assert.equal(pulls().export.rows,2);await type('insights-export-folder',exports);await type('insights-export-name','cancelled.csv');await resize(780,620);await capture('export-compact');
 await click('insights-export-close');assert(!pulls().export);assert.deepEqual(fs.readdirSync(exports),[]);await resize(1440,900);
 await click('insights-export');await click('insights-export-json');await type('insights-export-folder',exports);await type('insights-export-name','結果.json');await capture('export-json-review');
 await click('insights-export-save');await until(()=>pulls().export.notice&&!pulls().export.running,'JSON saved');assert.equal(pulls().export.error,null);
 const file=path.join(exports,'結果.json'),json=JSON.parse(fs.readFileSync(file,'utf8')),original=hash(file);
 assert.equal(json.schema_version,1);assert.equal(json.filters.author,'林 Example');assert.equal(json.filters.state,'merged');assert.equal(json.filters.rows_period,'all_loaded_dates');assert.equal(json.filters.summary_period,'30');
 assert.deepEqual(json.items.map(i=>i.number),expectedRows);assert.equal(json.coverage.loaded_unique_prs,5);assert.equal(json.coverage.partial,true);assert.equal(json.metrics.loaded,2);assert.equal(json.metrics.merge_samples,1);assert.equal(json.items[0].title,rows[1].title);
 const item=n=>json.items.find(i=>i.number===n);assert.equal(json.filters.reviewer,null);
 assert.deepEqual([item(2).reviews,item(2).first_reviewer,item(2).first_review_unix_seconds],['read','Reviewer',Math.floor(Date.parse(rows[1].created_at)/1000)+3600]);
 assert.deepEqual([item(3).reviews,item(3).first_review_unix_seconds],['not_read',null]);assert.deepEqual(item(2).review_requests,[]);
 const exported=fs.readFileSync(file,'utf8');for(const excluded of [token,'PRIVATE description','PRIVATE comment','PRIVATE reviewed'])assert(!exported.includes(excluded));
 await click('insights-export-save');await until(()=>pulls().export.error&&!pulls().export.running);assert.match(pulls().export.error,/already exists/);assert.equal(hash(file),original);await capture('export-existing-file');
 await click('insights-export-csv');await type('insights-export-name','結果.csv');await click('insights-export-save');await until(()=>pulls().export.notice&&!pulls().export.running);assert.equal(pulls().export.error,null);
 const csv=fs.readFileSync(path.join(exports,'結果.csv'),'utf8');assert(csv.startsWith('\ufeff'));const records=csvRecords(csv.slice(1));assert.equal(records.length,3);const column=n=>records[0].indexOf(n);assert.equal(records[1][column('title')],"'"+rows[1].title);assert.equal(records[1][column('author')],'林 Example');assert.equal(records[1][column('partial')],'true');assert.equal(records[1][column('loaded_unique_prs')],'5');
 await type('insights-export-folder',path.dirname(root));await type('insights-export-name',path.basename(root)+'-outside.csv');await click('insights-export-save');await until(()=>pulls().export.error&&!pulls().export.running);assert(!fs.existsSync(path.join(path.dirname(root),path.basename(root)+'-outside.csv')));
 assert.deepEqual(fs.readdirSync(exports).sort(),['結果.csv','結果.json']);assert.equal(requests.filter(r=>/\/pulls(?:\/|$)/.test(r.route)).length,apiBefore,'exports do not fetch additional PR data');
 await click('insights-export-close');await click('insights-detail-close');await author(null);await click('insights-filter-0');
 pass('Reviewed CSV/JSON exports match visible sample and filters; Unicode/formulas safe; cancelled preview, existing files and scope protected');
}
async function main(){try{
await new Promise(r=>server.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+server.address().port);
report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);const fd=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>150)throw e;await delay(100)}}await client.initialize();await client.tool('rocket_open',{path:sample.repo});await settle();await resize(1440,900);if(control('console-close'))await click('console-close');
await client.tool('rocket_shortcut',{binding:'Ctrl+K',expected_tab:state.active_tab});await until(()=>control('palette-input'));await type('palette-input','Insights');await click('palette-row-0');await until(()=>stats()?.activity&&!stats().activity_running);assert.equal(stats().activity.commits,1);await capture('activity');await click('stats-section-pulls');await until(()=>pulls()?.error);assert.equal(requests.length,0);await capture('no-account');pass('Activity reads local history; PRs require a matched account without sending a request');
await click('insights-hosting');await until(()=>control('hosting-accounts')||control('hosting-token'));if(!control('hosting-token'))await click('hosting-accounts');await type('hosting-token',token);await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);await click('dialog-cancel');if(state.dialog)await click('dialog-cancel');await click('view-statistics');await click('stats-section-pulls');await loaded();assert.equal(pulls().error,null);assert.deepEqual([pulls().loaded_count,pulls().opened,pulls().merged,pulls().open_now,pulls().merge_samples,pulls().median_merge_seconds],[4,2,2,1,1,172800]);assert.equal(pulls().incomplete_dates,1);assert(pulls().more);await capture('pulls-wide');pass('Dated cohorts, missing dates, merge median denominator, open age and partial sample');
await click('insights-view-trends');assert(pulls().trends);await reveal('insights-trend-chart');const sum=i=>pulls().trend.buckets.reduce((n,b)=>n+b[i],0);assert.deepEqual([pulls().trend.unit,sum(1),sum(2)],['Week',pulls().opened,pulls().merged]);assert.equal(pulls().comparison,'ready');assert.deepEqual([pulls().previous.opened,pulls().previous.merged],[0,0]);assert.equal(typeof pulls().coverage.after,'number');assert(control('insights-view-list').enabled);await capture('trends-wide');
await click('insights-period-90');assert.equal(pulls().comparison,'not_loaded');await reveal('insights-compare-more');await capture('trends-not-loaded');await resize(820,640);await top();await capture('trends-compact');await resize(1440,900);await click('insights-period-30');await click('insights-view-list');assert(!pulls().trends);
pass('Trends: weekly bars add up to the tiles; periods compared only when the sample reaches back over the previous one');
await click('insights-more');await loaded();assert.deepEqual([pulls().loaded_count,pulls().pages,pulls().merge_samples,pulls().median_merge_seconds],[5,2,2,259200]);assert(pulls().limited&&!pulls().more);assert.equal(pulls().comparison,'changed');await click('insights-period-all');assert.equal(pulls().merged,4);assert.equal(pulls().merge_samples,3);await click('insights-period-30');await resize(820,640);await top();await capture('pulls-compact');await click('insights-filter-1');assert.equal(pulls().loaded_count,5);await resize(1440,900);pass('Pagination deduplicates concurrent rows, period changes recompute, compact controls work');
await reviews();
await explore();
{const reads=()=>requests.filter(r=>/\/pulls(?:\/|$)/.test(r.route)).length,before=reads(),readAt=pulls().read_at;await click('dialog-cancel');assert.equal(state.dialog,null);await click('view-statistics');assert.equal(stats().section,'Pulls');assert.deepEqual([pulls().loaded_count,pulls().read_at,pulls().show],[5,readAt,0]);await delay(1000);state=await client.tool('rocket_state');assert.equal(reads(),before,'opening Insights again must not read the list');assert.equal(pulls().running,false);await capture('pulls-reopened');
pass('Insights again: the tab, filters and sample of this repository at once, without a hosting read')}
mode='hold';await click('stats-refresh');await until(()=>held.length===1&&pulls().running);await capture('loading');const start=performance.now();await click('stats-cancel');await until(()=>!pulls().running,'cancel completion',10000);report.cancel_ms=Math.round(performance.now()-start);assert.match(pulls().error,/Stopped/);assert.equal(pulls().loaded_count,5);for(const send of held.splice(0))send();await stable();assert.equal(pulls().loaded_count,5);pass('Stop cancels a held API read; late response cannot replace prior metrics');
mode='unauthorized';await click('stats-refresh');await until(()=>!pulls().running&&pulls().error);assert.equal(pulls().loaded_count,5);assert.match(pulls().error,/auth|sign|credential|account/i);await capture('auth-error');mode='bad-link';await click('stats-refresh');await until(()=>!pulls().running&&pulls().error);assert.equal(pulls().loaded_count,5);assert.match(pulls().error,/page|link|pagination|refus/i);pass('Authentication and hostile pagination failures are clear and retain the previous sample');
mode='empty';await click('stats-refresh');await loaded();assert.equal(pulls().loaded_count,0);assert.equal(pulls().median_merge_seconds,null);await capture('empty');pass('Empty results show unavailable timing instead of a fabricated zero');
mode='hold';await click('stats-refresh');await until(()=>held.length===1&&pulls().running);await client.tool('rocket_open',{path:other.repo});await settle();for(const send of held.splice(0))send();await click('view-statistics');await click('stats-section-pulls');await until(()=>pulls()?.error&&!pulls().running);assert.equal(pulls().loaded_count,0);pass('Switching repositories discards in-flight results and cannot leak another repository sample');
assert.deepEqual(sample.snapshot(),before);await close();
const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
const logs=JSON.stringify(records);assert(records.length&&records.every(r=>r.dropped_before===0));
for(const secret of [token,'PRIVATE','example.invalid','資料','林 Example','@formula'])assert(!logs.includes(secret),'diagnostic payload leak');
for(const operation of ['insights-pulls','insights-detail','insights-export']){
 const starts=records.filter(r=>r.event==='request_start'&&r.operation===operation);assert(starts.length>=3,'missing diagnostic spans '+operation);
 for(const e of starts)assert(records.some(r=>r.event==='request_end'&&r.session===e.session&&r.request===e.request));
}
assert(records.some(r=>r.operation==='insights-pulls'&&r.outcome==='cancelled'));
assert(records.some(r=>r.operation==='insights-detail'&&r.outcome==='cancelled'));
assert(records.some(r=>r.operation==='insights-export'&&r.outcome==='io_error'));
const reviewSpans=records.filter(r=>r.event==='request_start'&&r.operation==='insights-reviews');assert(reviewSpans.length>=2,'missing diagnostic spans insights-reviews');
for(const e of reviewSpans)assert(records.some(r=>r.event==='request_end'&&r.session===e.session&&r.request===e.request));
assert(records.some(r=>r.operation==='insights-reviews'&&r.outcome==='cancelled'));
assert(!report.server_error,report.server_error);assert(requests.every(r=>r.method==='GET'||r.route==='/graphql'));
pass('Repository/index/config/edits preserved; read-only API traffic, paired list/detail/export logs, no credentials or PR content');report.passed=true;
}catch(e){report.failure=e.stack;fs.writeFileSync(path.join(root,'failure-state.json'),JSON.stringify(state,null,2));if(client)try{await capture('failure')}catch{}throw e}finally{
for(const send of [...held.splice(0),...detailHeld.splice(0)])send();
if(service&&service.exitCode===null){try{await close()}catch{}await delay(200);if(service.exitCode===null)service.kill()}
server.closeAllConnections();await new Promise(r=>server.close(r));if(client)await client.finish();
fs.rmSync(path.join(profile,'access-token'),{force:true});
fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state,null,2));fs.writeFileSync('work/insights-last-result.json',JSON.stringify({result:path.join(root,'result.json')}));console.log(JSON.stringify(report,null,2));}}
main().catch(()=>{process.exitCode=1});
