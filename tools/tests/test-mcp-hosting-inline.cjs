// GitLab/Gitea native inline reviews through localhost MCP. Disposable repo/profile;
// existing credentials, SSH config and saved accounts are not read or changed.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const provider=process.argv[2];assert(['gitlab','gitea','bitbucket','bitbucket-server','azure','azure-server'].includes(provider),'Choose an inline-review provider');
const {create}=require(['gitlab','gitea'].includes(provider)?'./fixtures/hosting-inline.cjs':'./fixtures/hosting-other-inline.cjs');
const root=path.resolve('work/mcp-fixtures/inline-'+provider+'-'+crypto.randomUUID()),repo=path.join(root,'repo 資料'),profile=path.join(root,'profile'),exe=path.resolve('target/release/rocket-mcp.exe'),branch='rocket-ui-test/inline-'+provider;
const token='fixture-inline-'+crypto.randomBytes(12).toString('hex'),body='PRIVATE inline review 界 🚀',env={...process.env};
for(const k of Object.keys(env))if(/^(GIT_|GCM_|SSH_|CODEX_|OPENAI_|ANTHROPIC_)|_TOKEN$|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_TERMINAL_PROMPT:'0',GIT_ALLOW_PROTOCOL:'file',GIT_TEMPLATE_DIR:path.join(root,'template'),http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:''});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8'}).trim(),hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
git(['init','--initial-branch='+branch]);git(['config','user.name','Hosting Fixture']);git(['config','user.email','fixture@example.invalid']);git(['config','commit.gpgsign','false']);
fs.writeFileSync(path.join(repo,'資料.txt'),'committed\n');git(['add','--all']);git(['commit','-m','Synthetic']);fs.appendFileSync(path.join(repo,'資料.txt'),'staged\n');git(['add','--all']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const local=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt'))}),before=local();
const fixture=create(provider,token),api=fixture.state,report={passed:false,root,provider,binary:hash(exe),cases:[],cancel:[]};
let client,service,state;const delay=ms=>new Promise(r=>setTimeout(r,ms)),control=id=>state.controls?.find(c=>c.id===id),posts=()=>api.requests.filter(r=>r.method==='POST').length,pass=s=>{report.cases.push(s);console.log('PASS '+s);};
async function until(fn,label='state'){const at=performance.now();for(;;){assert(!api.error,api.error);state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-at<60000,label+' timeout '+JSON.stringify({hosting:state.hosting,dialog:state.dialog?.kind}));await delay(65);}}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!s.hosting?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',n=0;await until(s=>{const now=JSON.stringify(s.controls);n=now===last?n+1:0;last=now;return n>=2;},'stable');}
async function scroll(target,direction,pixels=300){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(id){await stable();const visible=()=>control(id)&&control(id).rect[3]-control(id).rect[1]>18;if(visible())return;
 const region=id.startsWith('repository-')?'repository-menu-scroll':'hosting-scroll';if(control(region)){await scroll(region,'up',2000);for(let i=0;i<35&&!visible();i++)await scroll(region,'down',230);}assert(visible(),'Missing '+id);}
async function click(id,wait=true){await reveal(id);assert(control(id).enabled,'Disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(wait)await settle();}
async function type(id,text){await reveal(id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle();}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function select(){await click('hosting-pull-tab-files');await click('hosting-pull-file-0');await click('hosting-inline-line-0-3-right');
 // Focus the newly mounted editor explicitly, as a user does. A delivered MCP
 // input receipt is not proof that the field accepted text. Never proceed to a
 // preview unless the observed field contains the complete Unicode message.
 await click('hosting-inline-body');await type('hosting-inline-body',body);assert.equal(state.hosting.pull.inline_body_bytes,Buffer.byteLength(body),'Inline editor did not accept the complete message');}
async function draft(){if(state.hosting.pull.reviewing)await click('hosting-review-edit');await select();await click('hosting-inline-save');await click('hosting-review-comment');await click('hosting-review-preview');assert(state.hosting.pull.reviewing,state.hosting.error);}
async function cancel(kind){await reveal('hosting-cancel');const at=performance.now();await client.tool('rocket_click',{target:'hosting-cancel',expected_tab:state.active_tab});await until(s=>s.hosting.stopping||!s.hosting.busy);const feedback=performance.now()-at;await settle();for(let i=0;i<100&&api.closedAt===null;i++)await delay(40);assert(api.closedAt!==null);report.cancel.push({kind,feedback_ms:Math.round(feedback),disconnect_ms:Math.round(api.closedAt-at)});}
async function main(){try{
 const port=await fixture.listen();fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+port);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const mcpPort=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+mcpPort+'\n'+root+'\n');
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>100)throw e;await delay(100);}}
 await client.initialize();await client.tool('rocket_open',{path:repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');
 await click('menu-repository');await click('repository-remotes-menu');await click('repository-hosting');await until(s=>s.dialog?.kind==='hosting');
 await click('hosting-tab-accounts');await click('hosting-provider-'+provider);if(provider==='gitea'||fixture.server)await type('hosting-server',fixture.server||'https://gitea.example.invalid');await type('hosting-token',token);if(state.hosting.remember)await click('hosting-remember');await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);
 await type('hosting-repository','team/sample');await click('hosting-refresh');await click('hosting-details-0');assert(state.hosting.pull.details.inline_supported);assert(!state.hosting.pull.details.inline_ranges);assert.equal(state.hosting.pull.details.inline_limit,provider==='gitea'?32:1);
 await select();await capture('line-draft-wide');const n=posts();await click('hosting-inline-cancel');assert.equal(state.hosting.pull.comments.length,0);assert.equal(state.hosting.pull.inline_body_bytes,0);assert.equal(posts(),n);
 pass('Unicode rename diff and cancel line draft work without remote writes');
 await draft();await resize(780,620);await reveal('hosting-review-submit');const rect=control('hosting-review-submit').rect;assert(rect[0]>=0&&rect[2]<=780&&rect[3]<=620);await capture('review-compact');await click('hosting-review-edit');await reveal('hosting-inline-edit-0');await capture('draft-compact');await click('hosting-review-preview');await click('hosting-review-submit');assert.equal(posts(),n+1);assert(!state.hosting.error,state.hosting.error);assert.equal(state.hosting.pull.submitted.state,'COMMENTED');assert.equal(state.hosting.pull.comments.length,0);
 await resize(1480,920);await click('hosting-pull-refresh');assert.equal(state.hosting.pull.details.inline.count,1);await click('hosting-pull-tab-discussion');await capture('inline-published');
 pass('Compact preview/edit/submit stays reachable; one pinned write is confirmed and read back');
 await draft();const writes=posts();api.head='d'.repeat(40);await click('hosting-review-submit');assert(state.hosting.error.includes('hosting_pull_changed'));assert.equal(posts(),writes);assert.equal(state.hosting.pull.comments.length,1);await capture('stale-review');await click('hosting-review-discard');await click('hosting-pull-refresh');
 pass('Concurrent head change refuses before writing and keeps the original comment');
 if(provider==='bitbucket-server'||provider.startsWith('azure')){
  await draft();const count=posts(),field=provider==='bitbucket-server'?'version':'iteration';api[field]++;await click('hosting-review-submit');assert(state.hosting.error.includes('hosting_pull_changed'));assert.equal(posts(),count);assert.equal(state.hosting.pull.comments.length,1);await capture('changed-diff-version');await click('hosting-review-discard');await click('hosting-pull-refresh');
  pass('A new PR version or Azure iteration with the same head refuses before writing');
 }
 api.mode='truncated';await click('hosting-pull-refresh');assert.equal(state.hosting.pull.details.paths[0].lines.length,0);await click('hosting-pull-tab-files');await capture('incomplete-diff');api.mode='ok';await click('hosting-pull-refresh');
 pass('Incomplete diff never exposes invented comment positions');
 if(provider==='gitlab'){
  api.mode='no-refs';await click('hosting-pull-refresh');assert(!state.hosting.pull.details.inline_supported);await reveal('hosting-inline-line-0-3-right');assert(!control('hosting-inline-line-0-3-right').enabled);await capture('missing-diff-revision');api.mode='ok';await click('hosting-pull-refresh');
  pass('Missing GitLab diff revision disables inline targets with actionable guidance');
 }
 if(provider==='gitea'){
  await draft();const prior=posts();api.pending={id:900,user:{login:'fixture'},state:'PENDING'};await click('hosting-review-submit');assert(state.hosting.error.includes('hosting_pending_review'));assert.equal(posts(),prior);assert.equal(state.hosting.pull.comments.length,1);await capture('pending-review');api.pending=null;await click('hosting-review-discard');
  pass('A pending review created after preview is protected by the submit-time check');
 }
 await draft();api.mode='reject';const rejected=posts();await click('hosting-review-submit');assert.equal(posts(),rejected+1);assert(state.hosting.error.includes(provider==='gitea'?'hosting_pull_review_unknown':'hosting_pull_review_rejected'));assert.equal(state.hosting.pull.uncertain,provider==='gitea');assert.equal(state.hosting.pull.comments.length,1);await capture('rejected-review');api.mode='ok';
 if(provider==='gitea'){
  await click('hosting-pull-refresh');assert(state.hosting.pull.can_acknowledge);assert.equal(state.hosting.pull.details.inline.count,2);await capture('partial-pending-review');
  // Explicit provider-side recovery, not a Rocket API write.
  api.pending=null;api.notes=api.notes.filter(n=>n.review_id!==900);await click('hosting-pull-refresh');await click('hosting-review-acknowledge');
 }
 pass('Provider rejection retains the draft; a possible partial save requires explicit recovery');
 if(state.hosting.pull.reviewing)await click('hosting-review-edit');await click('hosting-review-preview');api.mode='slow-submit';api.closedAt=null;const submitting=posts();await click('hosting-review-submit',false);await until(s=>s.hosting.busy&&posts()>submitting);await cancel('write');assert(state.hosting.pull.uncertain);assert(state.hosting.error.includes('hosting_pull_review_unknown'));assert.equal(state.hosting.pull.comments.length,1);assert.equal(posts(),submitting+1);await capture('unknown-review');api.mode='ok';
 await click('hosting-pull-refresh');assert(state.hosting.pull.can_acknowledge);assert.equal(state.hosting.pull.details.inline.count,2);await click('hosting-review-acknowledge');await click('hosting-review-discard');
 pass('Cancellation after POST is unknown, never retried; refresh verifies and explicit acknowledgement unlocks recovery');
 const prior=state.hosting.pull.details;api.mode='slow-read';api.closedAt=null;const reads=api.requests.length;await click('hosting-pull-refresh',false);await until(s=>s.hosting.busy&&api.requests.length>reads);await cancel('read');assert.deepEqual(state.hosting.pull.details,prior);assert(!state.hosting.pull.uncertain);api.mode='ok';
 pass('Read cancellation preserves the last complete snapshot and closes the helper request');
 api.mode='401';await click('hosting-pull-refresh');assert(state.hosting.error.includes('hosting_unauthorized'));assert(!state.hosting.connected);assert(!JSON.stringify(state).includes(token));await capture('expired-auth');api.mode='ok';
 pass('Expired authentication disconnects without leaking credentials or changing the checkout');
 assert.deepEqual(local(),before);report.passed=true;
 }catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client&&state)try{await capture('failure');}catch{}}
 finally{
  api.mode='ok';api.error=null;if(client&&state)try{if(state.hosting?.busy)await click('hosting-cancel');if(state.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await click('dialog-submit');await until(s=>['closed','not_started'].includes(s.window));}await client.raw('POST','/shutdown');}catch(e){report.cleanup_error=e.message;}
  if(client)await client.finish();if(service){for(let i=0;i<100&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null){report.cleanup_error='Owned MCP host did not exit';cp.spawnSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});}}
  fixture.close();const access=path.join(profile,'access-token');if(fs.existsSync(access))fs.unlinkSync(access);
  try{const rows=[];for(const folder of fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')))for(const file of fs.readdirSync(path.join(root,folder)).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n)))rows.push(...fs.readFileSync(path.join(root,folder,file),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse));assert(rows.length);assert(rows.every(r=>r.dropped_before===0));for(const value of[token,body,'PRIVATE','資料/新'])assert(!JSON.stringify(rows).includes(value),'Diagnostic leak');for(const start of rows.filter(r=>r.event==='request_start'))assert(rows.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request));for(const code of ['hosting_unauthorized','hosting_pull_changed',provider==='gitea'?'hosting_pending_review':'hosting_pull_review_rejected','hosting_pull_review_unknown'])assert(rows.some(r=>r.failure_code===code),'Missing diagnostic '+code);report.log_events=rows.length;assert.deepEqual(local(),before);}catch(e){report.log_error=e.stack;report.passed=false;process.exitCode=1;}
  if(report.cleanup_error||service?.exitCode!==0){report.passed=false;process.exitCode=1;}
  report.cleanup={exit:service?.exitCode,access_token_removed:!fs.existsSync(access)};fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log('Evidence: '+root);
 }}main().catch(e=>{console.error(e);process.exitCode=1;});
