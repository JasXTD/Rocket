require('node:fs').mkdirSync(require('node:path').resolve('work/results/bitbucket-hosting-0.79'), {recursive:true});
// File-menu navigation only: isolated profile/repository/SSH folder, no sign-in or keys.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict'),http=require('node:http'),net=require('node:net');
const {RocketHttpClient}=require('../../../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/bitbucket-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'repo 文'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/bitbucket';
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
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const img=r.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settled();}
function menuFits(){for(const id of ['file-ssh-keys','file-accounts']){const c=control(id);assert(c?.enabled);assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=state.viewport[0]&&c.rect[3]<=state.viewport[1]);}}
function actionsDoNotOverlap(){const rows=['hosting-forget','hosting-disconnect','hosting-browser','hosting-connect'].map(id=>{const c=control(id);assert(c&&c.rect[3]-c.rect[1]>=27,'Account action clipped: '+id);return c;});for(let i=0;i<rows.length;i++)for(let j=i+1;j<rows.length;j++){const a=rows[i].rect,b=rows[j].rect;assert(!(Math.max(a[0],b[0])<Math.min(a[2],b[2])&&Math.max(a[1],b[1])<Math.min(a[3],b[3])),rows[i].id+' overlaps '+rows[j].id);}assert(control('hosting-connect').rect[1]<=control('hosting-remember').rect[3]+20,'Account actions separated from the form');}
async function scroll(region,direction,pixels=300){await client.tool('rocket_scroll',{target:region,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(k){await stable();if(control(k)&&control(k).rect[3]-control(k).rect[1]>18)return;
 if(control('hosting-scroll')){const visible=()=>control(k)&&control(k).rect[3]-control(k).rect[1]>18;for(let i=0;i<6&&!visible();i++){const previous=JSON.stringify(state.controls);await scroll('hosting-scroll','up',2000);if(previous===JSON.stringify(state.controls))break;}for(let i=0;i<40&&!visible();i++){const previous=JSON.stringify(state.controls);await scroll('hosting-scroll','down',240);if(previous===JSON.stringify(state.controls))break;}}
 assert(control(k)&&control(k).rect[3]-control(k).rect[1]>18,'unreachable '+k);}
async function click(k,wait=true){await reveal(k);assert(control(k)?.enabled,'disabled '+k);await client.tool('rocket_click',{target:k,expected_tab:state.active_tab});if(wait)await settled();}
async function type(k,text){await reveal(k);assert(control(k)?.enabled,'disabled '+k);await client.tool('rocket_type',{target:k,text,expected_tab:state.active_tab});await settled();}
const apiToken='BITBUCKET_PRIVATE_TOKEN_'+crypto.randomBytes(12).toString('hex');
const requests=[];let mode='ok',changed=false,stoppedAt=null;
const serverItem=(id,state='OPEN')=>({id,title:'PRIVATE_BITBUCKET 改善 🚀',state,author:{user:{displayName:'Fixture 界'}},fromRef:{id:'refs/heads/feature/界'},toRef:{id:'refs/heads/main'}});
async function apiRequest(req,res){
 apiRequests++;let body='';for await(const c of req)body+=c;
 const u=new URL(req.url,'http://127.0.0.1'),p=u.pathname.split('/')[1],route=u.pathname.slice(p.length+1);
 const entry={provider:p,method:req.method,path:u.pathname,query:u.search};requests.push(entry);
 const send=(status,data,headers={})=>{res.writeHead(status,{'Content-Type':'application/json',...headers});res.end(JSON.stringify(data));};
 const browser=route==='/rocket/browser-sign-in';
 assert.equal(req.headers.authorization,'Bearer '+(browser?'rocket-disposable-fixture':apiToken));
 if(mode==='slow-browser'&&browser||mode==='slow-list'&&route.includes('/pull-requests')||mode==='slow-create'&&req.method==='POST'&&!browser){res.on('close',()=>stoppedAt=performance.now());return;}
 if(mode==='401')return send(401,{message:apiToken});
 if(browser)return send(200,{access_token:apiToken});
 if(p==='bitbucket'&&route==='/user')return send(200,{display_name:'Fixture Bitbucket'});
 if(route==='/application-properties')return send(200,{version:'fixture'},mode==='anonymous'?{}:{'X-AUSERNAME':'fixture%40work'});
 if(route.endsWith('/branches')){
   const ref=u.searchParams.get('filterText'),id=(ref==='main'?'b':changed?'c':'a').repeat(40);
   return send(200,{values:[{id:'refs/heads/'+ref,latestCommit:id}],start:0,isLastPage:true});
 }
 if(mode==='redirect')return send(302,{}, {Location:'http://127.0.0.1:1/never'});
 if(/^\d+$/.test(mode))return send(Number(mode),{message:apiToken+' PRIVATE_BITBUCKET_SERVER_BODY'});
 assert.equal(p,'bitbucket-server');
 assert.equal(route,'/projects/TEAM/repos/%E8%B3%87%E6%96%99/pull-requests');
 if(req.method==='POST'){
   const data=JSON.parse(body);assert.equal(data.title,'PRIVATE_BITBUCKET 改善 🚀');assert.equal(data.fromRef.id,'refs/heads/feature/界');assert.equal(data.toRef.id,'refs/heads/main');
   assert.deepEqual(data.fromRef.repository,{slug:'資料',project:{key:'TEAM'}});assert.deepEqual(data.toRef.repository,data.fromRef.repository);
   return send(201,serverItem(43));
 }
 const start=Number(u.searchParams.get('start'));assert([0,17].includes(start));
 return send(200,{values:[serverItem(start?44:41),serverItem(start?45:42,'MERGED')],start,isLastPage:start===17,nextPageStart:mode==='bad-page'?0:17});
}
async function preview(){
 await click('hosting-tab-create');await type('hosting-repository','TEAM/資料');await type('hosting-source','feature/界');await type('hosting-target','main');
 await type('hosting-title','PRIVATE_BITBUCKET 改善 🚀');await type('hosting-body','Synthetic line\nUnicode 界');await click('hosting-review');
 assert(state.hosting.reviewing,state.hosting.error);assert.equal(state.hosting.review.source_id,'a'.repeat(40));
}
function compactActionsFit(browser){
 for(const id of ['hosting-connect','hosting-disconnect','hosting-forget',...(browser?['hosting-browser']:[])]){
  const r=control(id)?.rect;assert(r&&r[2]-r[0]>25&&r[3]-r[1]>=25,'Clipped action '+id);
  assert(r[0]>=0&&r[2]<=state.viewport[0]&&r[1]>=0&&r[3]<=state.viewport[1],'Offscreen '+id);
 }
}
async function reachCompactActions(browser){
 const ids=['hosting-connect','hosting-disconnect','hosting-forget',...(browser?['hosting-browser']:[])];
 // Scroll remains a real user action. A resized form can need another wheel
 // event after its wrapped rows establish their height; never accept a clipped row.
 for(let attempt=0;attempt<6;attempt++){
  const fits=ids.every(id=>{const r=control(id)?.rect;return r&&r[2]-r[0]>25&&r[3]-r[1]>=25&&r[0]>=0&&r[2]<=state.viewport[0]&&r[1]>=0&&r[3]<=state.viewport[1]});
  if(fits){compactActionsFit(browser);(report.compact_scroll??=[]).push({provider:browser?'bitbucket':'bitbucket-server',extra_scrolls:attempt});return;}
  await scroll('hosting-scroll','down',400);
 }
 compactActionsFit(browser);
}
async function cancelOperation(action,modeName,expectedError){
 mode=modeName;stoppedAt=null;const beforeRequests=requests.length;await click(action,false);await until(s=>s.hosting.busy&&!s.input_pending);
 // Wait for the actual API request; do not confuse preflight cancellation with a sent mutation.
 const deadline=performance.now()+15000;while(!requests.slice(beforeRequests).some(r=>action==='hosting-browser'?r.path.endsWith('browser-sign-in'):action==='hosting-create'?r.method==='POST':r.path.endsWith('pull-requests'))){assert(performance.now()<deadline);await delay(50);}
 const began=performance.now();await click('hosting-cancel');
 assert(!state.hosting.busy);if(expectedError)assert(state.hosting.error?.includes(expectedError),JSON.stringify(state.hosting));
 else assert(state.hosting.notice?.includes('Cancelled')||state.hosting.notice?.includes('stopped'),JSON.stringify(state.hosting));
 const end=performance.now();report.cancel??=[];report.cancel.push({mode:modeName,ui_ms:Math.round(end-began),socket_closed_ms:stoppedAt===null?null:Math.max(0,Math.round(stoppedAt-began))});
 mode='ok';
}
async function run(){try{
 api=http.createServer((req,res)=>apiRequest(req,res).catch(e=>{report.api_error=e.message;res.destroy();}));
 await new Promise(r=>api.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+api.address().port);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>120)throw e;await delay(100)}}
 report.version=(await client.initialize()).serverInfo.version;assert.equal(report.version,'0.79.0');await client.tool('rocket_open',{path:repo});await settled();await resize(1440,960);
 await click('menu-file');await click('file-accounts');await click('hosting-provider-bitbucket');await type('hosting-label','Cloud fixture');
 if(!state.hosting.remember)await click('hosting-remember');await capture('cloud-accounts');
 await click('hosting-browser');assert(state.hosting.connected,state.hosting.error);assert(state.hosting.notice.includes('Temporary Bitbucket'));assert.equal(state.hosting.accounts_count,0);
 assert(!fs.existsSync(path.join(root,'rocket-ui-state/hosting-accounts.dpapi')));pass('Cloud browser exchange verifies bearer identity without saving the short-lived token');
 await click('hosting-tab-accounts');await click('hosting-disconnect');mode='401';await click('hosting-browser');assert(state.hosting.error.includes('hosting_unauthorized'));assert(!state.hosting.connected);assert(!state.hosting.error.includes(apiToken));mode='ok';
 await cancelOperation('hosting-browser','slow-browser');assert(!state.hosting.connected);pass('Browser rejection and cancellation leave login/profile unchanged');
 await click('hosting-provider-bitbucket-server');await type('hosting-label','Server fixture');await type('hosting-server','https://git.example.com:8443/bitbucket');await type('hosting-username','fixture@work');await type('hosting-token',apiToken);
 await capture('server-accounts');mode='anonymous';await click('hosting-connect');assert(!state.hosting.connected);assert(state.hosting.error.includes('hosting_identity_unverified'));mode='ok';await type('hosting-token',apiToken);await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);assert.equal(state.hosting.accounts_count,1);pass('Server rejects anonymous success and verifies authenticated identity with a separate saved profile');
 await type('hosting-repository','TEAM/資料');await click('hosting-refresh');assert.equal(state.hosting.count,2);assert(state.hosting.more);await click('hosting-more');assert.equal(state.hosting.count,4);assert(!state.hosting.more);
 assert.equal(state.hosting.items[0].url,'https://git.example.com:8443/bitbucket/projects/TEAM/repos/%E8%B3%87%E6%96%99/pull-requests/41');
 await click('hosting-state-closed');await click('hosting-refresh');assert.equal(state.hosting.count,1);assert.equal(state.hosting.items[0].state,'MERGED');await capture('server-pulls');pass('Server paging uses returned offsets; closed filter and Unicode/context-path links are correct');
 await click('hosting-tab-issues');assert(!control('hosting-refresh'));pass('Server issues capability is explicitly unavailable');
 await preview();await capture('server-review');await click('hosting-create');assert.equal(state.hosting.created,43,state.hosting.error);pass('Reviewed Server PR creation has the expected branch and repository payload');
 await preview();changed=true;const posts=requests.filter(r=>r.method==='POST').length;await click('hosting-create');assert(state.hosting.error.includes('hosting_review_stale'));assert.equal(requests.filter(r=>r.method==='POST').length,posts);changed=false;pass('Concurrent branch change blocks creation before POST');
 await click('hosting-tab-pulls');for(const [m,error]of [['403','forbidden'],['404','not_found'],['429','rate_limited'],['redirect','invalid_response'],['bad-page','invalid_pagination']]){mode=m;await click('hosting-refresh');assert(state.hosting.error.includes('hosting_'+error),JSON.stringify(state.hosting));assert(!state.hosting.error.includes(apiToken));}mode='ok';pass('Permission, missing-resource, rate-limit, redirect and pagination errors are clear and redacted');
 await cancelOperation('hosting-refresh','slow-list');
 await preview();mode='409';await click('hosting-create');assert(state.hosting.error.includes('hosting_rejected'));mode='ok';
 await preview();await cancelOperation('hosting-create','slow-create','hosting_outcome_unknown');assert(state.hosting.uncertain);pass('List cancellation, rejected creation and unknown post-cancellation outcome are distinct');
 await click('hosting-tab-accounts');await click('hosting-disconnect');await click('hosting-load-accounts');assert.equal(state.hosting.accounts_count,1);
 await click('hosting-account-0');assert.equal(state.hosting.provider,'bitbucket-server');await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);
 await click('hosting-tab-accounts');await click('hosting-disconnect');await capture('server-saved');
 await resize(780,620);await scroll('hosting-scroll','down',1000);await reachCompactActions(false);await capture('server-compact');
 await resize(1440,960);await scroll('hosting-scroll','up',2000);await click('hosting-provider-bitbucket');await resize(780,620);await scroll('hosting-scroll','down',1000);await reachCompactActions(true);await capture('cloud-compact');pass('Both providers render in compact accounts view; protected Server profile reloads');
 assert.deepEqual(fingerprint(),before);assert(!report.api_error,report.api_error);assert(!JSON.stringify(state).includes(apiToken));pass('Local staged/unstaged/untracked files, Git/SSH config and MCP secret redaction preserved');
 report.requests=requests;report.passed=true;
}catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client)try{await capture('failure')}catch{}}
finally{
 if(client){try{if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}await client.raw('POST','/shutdown');}catch(e){report.cleanup_error=e.message;}await client.finish();}
 if(service){for(let i=0;i<80&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null)cp.spawnSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});}
 if(api){api.closeAllConnections();await new Promise(r=>api.close(r));}const access=path.join(profile,'access-token');if(fs.existsSync(access))fs.unlinkSync(access);
 report.cleanup={service_exit:service?.exitCode,access_token_removed:!fs.existsSync(access),error:report.cleanup_error??null};report.hosting_requests=apiRequests;
 if(report.cleanup.error||report.cleanup.service_exit!==0){report.passed=false;process.exitCode=1;}
 fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.resolve('work/results/bitbucket-hosting-0.79/native-latest.json'),JSON.stringify(report,null,2));console.log('Evidence: '+root);
}}
run();
