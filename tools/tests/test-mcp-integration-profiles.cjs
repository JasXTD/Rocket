// Native localhost acceptance: temporary dirty repo, disposable protected account,
// first-party Lua fixtures and a loopback mock API. No external service or login.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto');
const http=require('node:http'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/lua-profiles-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'fixture-repo'),profile=path.join(root,'http-profile'),uiProfile=path.join(root,'rocket-ui-state');
const branch='rocket-ui-test/lua',token='fixture_lua_'+crypto.randomBytes(12).toString('hex'),privateText='PRIVATE issue 資料 🚀';
const tokenB='fixture_lua_b_'+crypto.randomBytes(12).toString('hex'),privateB='PRIVATE personal 資料';
const exe=path.resolve('target/release/rocket-mcp.exe'),worker=path.resolve('target/release/rocket-lua-host.exe');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0',http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:''});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'資料.txt'),'committed\n');git(['add','-A']);git(['commit','-m','Synthetic fixture']);
fs.writeFileSync(path.join(repo,'資料.txt'),'staged\n');git(['add','-A']);fs.appendFileSync(path.join(repo,'資料.txt'),'unstaged\n');fs.writeFileSync(path.join(repo,'untracked.txt'),'private local edits');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const local=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt')),untracked:hash(path.join(repo,'untracked.txt'))}),before=local();
let service,client,state,mode='ok',closedAt=null;const requests=[],report={passed:false,root,cases:[],cancel:[],before};
const delay=ms=>new Promise(r=>setTimeout(r,ms)),writes=()=>requests.filter(r=>r.method!=='GET').length;
const server=http.createServer(async(req,res)=>{try{
 let data='';for await(const c of req)data+=c;requests.push({method:req.method,path:req.url,body:data});
 assert(['Bearer '+token,'Bearer '+tokenB].includes(req.headers.authorization)); const expected=req.headers.authorization==='Bearer '+tokenB?privateB:privateText;
 const send=(status,body,extra={})=>{res.writeHead(status,{'Content-Type':'application/json',...extra});res.end(JSON.stringify(body));};
 if(mode==='slow-read'&&req.method==='GET'||mode==='slow-write'&&req.method!=='GET'){res.on('close',()=>closedAt=performance.now());return;}
 if(mode==='auth')return send(401,{message:token+' '+privateText});
 if(mode==='reject')return send(403,{message:privateText});
 if(mode==='conflict')return send(409,{});
 if(mode==='rate')return send(429,{});
 if(mode==='redirect')return send(302,{}, {Location:'http://127.0.0.1:'+server.address().port+'/redirected'});
 if(mode==='invalid'){res.writeHead(200);res.end('invalid '+privateText);return;}
 if(req.method==='GET'){if(req.url.includes('/createmeta/'))return send(200,req.url.split('?')[0].endsWith('/issuetypes')?{issueTypes:[{id:'10001',name:'Task'}]}:{isLast:true,fields:[]});return send(200,{issues:[{key:'ROCKET-7',fields:{summary:privateText,status:{name:'In progress'},assignee:{displayName:'Synthetic user'}}}]});}
 const body=JSON.parse(data);assert(body.fields?.summary===expected||body.body?.content[0]?.content[0]?.text===expected);
 return send(201,{id:'1007',key:'ROCKET-7'});
}catch(e){report.server_error=String(e.stack||e);res.writeHead(500);res.end('{}');}});
const pass=s=>{report.cases.push(s);console.error('PASS '+s);};
async function until(fn,label='state'){const at=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return state;assert(performance.now()-at<30000,label+' timeout '+JSON.stringify({viewport:state.viewport,window:state.window,input_pending:state.input_pending,integrations:state.integrations,dialog:state.dialog?.kind}));await delay(60);}}
const control=k=>state.controls?.find(c=>c.id===k);
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.integrations?.busy&&(!s.integrations?.draft_pending||s.integrations?.draft_error)&&(!s.integrations?.draft_pending||s.integrations?.draft_error)&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2;},'stable');}
async function scroll(region,direction,pixels=300){await client.tool('rocket_scroll',{target:region,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(k){await stable();const visible=()=>control(k)&&control(k).rect[3]-control(k).rect[1]>18;if(visible())return;
 const region=control('prefs-scroll')?'prefs-scroll':control('repository-menu-scroll')?'repository-menu-scroll':control('int-panel-scroll')?'int-panel-scroll':control('page-details-scroll')?'page-details-scroll':null;
 if(region){for(let i=0;i<3&&!visible();i++)await scroll(region,'up',2000);for(let i=0;i<25&&!visible();i++)await scroll(region,'down',220);}assert(visible(),'unreachable '+k+' controls='+state.controls.map(c=>c.id).join(','));}
async function click(k,wait=true){await reveal(k);assert(control(k)?.enabled,'disabled '+k+' '+JSON.stringify(state.integrations));await client.tool('rocket_click',{target:k,expected_tab:state.active_tab});if(wait)await settled();}
async function type(k,text){await reveal(k);await client.tool('rocket_type',{target:k,text,expected_tab:state.active_tab});await settled();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const image=r.content.find(x=>x.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));}
async function resize(w,h){await stable();await client.tool('rocket_resize',{width:w,height:h});await until(s=>s.viewport[0]===w&&s.viewport[1]===h&&!s.input_pending);await settled();}
async function openPanel(){await click('menu-view');await click('view-goto');await click('goto-integrations');await settled();}
async function config(){if(control('int-configure'))await click('int-configure');await settled();}
async function clearChangedDraft(){if(state.integrations.draft_blocked){assert(state.integrations.draft_saved);await click('int-clear-draft');await click('int-clear-confirm');assert(!state.integrations.draft_blocked);assert(!state.integrations.draft_saved);}}
async function panel(){const id=state.integrations.selected;await click('dialog-close');await click('view-ext-'+id);await settled();}
// The list, from anywhere in Preferences → Extensions.
async function list(){await stable();if(state.integrations.details)await click('int-back');}
async function reload(){await list();await click('int-reload');}
// An extension's details in Preferences.
async function pick(id){await list();await click('int-package-'+id);}
// An enabled extension's page, from Preferences or another page.
async function show(id){await stable();if(control('dialog-close'))await click('dialog-close');await click('view-ext-'+id);await settled();}
// Saves the form, then reviews or turns on whatever isn't running yet.
async function approve(){await stable();if(control('int-save')){await click('int-save');await stable();if(control('int-save'))return;}const id=state.integrations.selected;if(state.integrations.needs_review){await click(control('int-review')?'int-review':'int-review-'+id);await click('int-review-approve');}else if(!state.integrations.extension_enabled){await click(control('int-enable')?'int-enable':'int-enable-'+id);if(state.integrations.prompt)await click('int-review-approve');}}
async function installJira(){await list();await click('int-enable-jira');await click('int-review-approve');}
async function launch(){const out=fs.openSync(path.join(root,'service.log'),'a');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>100)throw e;await delay(100);}}await client.initialize();await client.tool('rocket_open',{path:repo});await settled();await resize(1440,900);if(control('console-close'))await click('console-close');await openPanel();}
async function stop(){if(service&&service.exitCode===null&&service.signalCode===null){if(state?.integrations?.busy)await click('int-cancel');if(control('dialog-cancel'))await click('dialog-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}await client.raw('POST','/shutdown');const at=performance.now();while(service.exitCode===null&&service.signalCode===null){assert(performance.now()-at<15000,'owned service stop');await delay(60);}assert.equal(service.exitCode,0);}if(client)await client.finish();}
async function restart(crash=false){const n=writes();if(crash){service.kill();while(service.exitCode===null&&service.signalCode===null)await delay(50);await client.finish();}else await stop();await launch();assert.equal(writes(),n,'restart replayed a write');}
async function cancel(kind){await reveal('int-cancel');const at=performance.now();await client.tool('rocket_click',{target:'int-cancel',expected_tab:state.active_tab});await until(s=>s.integrations.stopping||!s.integrations.busy);const feedback=performance.now()-at;await settled();assert(closedAt!==null,'connection not closed');report.cancel.push({kind,feedback_ms:feedback,connection_closed_ms:closedAt-at,settled_ms:performance.now()-at});}
async function choose(id){await click('int-connection');await click('int-connection-'+id);}
function packageFile(dir,manifest,source){fs.mkdirSync(dir,{recursive:true});fs.writeFileSync(path.join(dir,'integration.json'),JSON.stringify(manifest));fs.writeFileSync(path.join(dir,'main.lua'),source);}
const counterSource=`return function(ctx)
 local n=ctx.state.count or 0
 if ctx.event=='increment' then n=n+1 end
 if ctx.event=='verify-close' then assert(n==2 and ctx.values.note=='PRIVATE close pending draft','close lost draft') end
 if ctx.event=='verify' then assert(n==2,'wrong restored state'); assert(ctx.values.note==ctx.settings.expected,'wrong restored draft'); assert(ctx.values.flag==true,'wrong checkbox'); assert(ctx.values.kind=='Second','wrong choice') end
 return {state={count=n},view=ui.page{title='Saved drafts · 資料',children={
 ui.input{id='note',label='Draft text',value=''},ui.checkbox{id='flag',label='Include details',value=false},
 ui.select{id='kind',label='Choice',options={'First','Second'},value='First'},
 ui.text('Count '..n),ui.button{id='increment',label='Increment'},ui.button{id='verify',label='Verify restored draft'},ui.button{id='verify-close',label='Verify close draft'}
 }}} end`;
async function fillCounter(text){await click('int-run');await type('int-ui-note',text);await click('int-ui-flag');await click('int-ui-kind');await click('int-ui-kind-1');await click('int-ui-increment');await click('int-ui-increment');await click('int-ui-verify');assert(!state.integrations.error,state.integrations.error);}
async function run(){try{
 fs.cpSync(path.resolve('crates/rocket-integrations/examples/playground'),path.join(uiProfile,'integrations/playground'),{recursive:true,errorOnExist:true,force:false});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+server.address().port);report.executable_sha256=hash(exe);report.worker_sha256=hash(worker);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');await launch();
 const packageRoot=path.join(uiProfile,'integrations'),counter=path.join(packageRoot,'計數器');
 packageFile(counter,{api:1,id:'counter',name:'Draft counter 資料',version:'1.0',description:'Synthetic draft fixture',permissions:{network:true,write:false},settings:[{id:'expected',label:'Expected draft',default:'PRIVATE draft A 資料 🚀'}]},counterSource);
 fs.mkdirSync(path.join(packageRoot,'broken'));fs.writeFileSync(path.join(packageRoot,'broken/integration.json'),'invalid fixture');
 await config();await reload();assert(state.integrations.packages.some(p=>p.id==='counter'));assert.equal(state.integrations.package_errors.length,1);await pick('counter');await type('int-server','https://counter.example.invalid');await approve();await panel();await fillCounter('PRIVATE draft A 資料 🚀');
 await capture('draft-wide');await resize(780,660);await capture('draft-compact');await resize(1440,900);
 pass('Invalid package is reported independently while Unicode native controls and a healthy Lua callback remain usable');

 await config();await type('int-connection-name','Work 資料');await approve();await reload();await pick('playground');await approve();await panel();await click('int-run');await click('int-package-counter');await click('int-ui-verify');assert(!state.integrations.error,state.integrations.error);
 await config();await click('int-new-connection');const counterB=state.integrations.connection;assert.notEqual(counterB,'default');assert(!state.integrations.enabled);assert(!state.integrations.has_view);await type('int-connection-name','Personal');await type('int-server','https://counter.example.invalid');await type('int-setting-expected','PRIVATE draft B 한국어');await approve();await panel();await fillCounter('PRIVATE draft B 한국어');await choose('default');await click('int-ui-verify');assert(!state.integrations.error,state.integrations.error);await choose(counterB);await click('int-ui-verify');assert(!state.integrations.error);
 await config();await capture('profiles-wide');await resize(780,660);await capture('profiles-compact');await resize(1440,900);await panel();
 pass('Package switches and named profile switches preserve independent Unicode forms, checkbox/select values and script state');

 await restart();await show('counter');assert.equal(state.integrations.connection,counterB);assert(state.integrations.has_view);await click('int-ui-verify');assert(!state.integrations.error);await choose('default');await click('int-ui-verify');assert(!state.integrations.error);await restart(true);await show('counter');assert(state.integrations.has_view);await click('int-ui-verify');assert(!state.integrations.error);
 pass('Normal and abrupt restarts restore the last saved panel without auto-running scripts or replaying requests');

 await config();fs.appendFileSync(path.join(counter,'main.lua'),'\n-- external edit');await reload();await pick('counter');assert(state.integrations.draft_blocked);assert(state.integrations.draft_saved);await approve();assert(state.integrations.draft_blocked);await panel();assert(!control('int-run').enabled);await capture('blocked-draft');await click('int-clear-draft');await click('int-clear-back');assert(state.integrations.draft_saved);
 fs.writeFileSync(path.join(counter,'main.lua'),counterSource);await config();await reload();await pick('counter');await approve();await panel();await click('int-ui-verify');assert(!state.integrations.error);
 await choose(counterB);await click('int-clear-draft');await click('int-clear-confirm');assert(!state.integrations.has_view);assert(!state.integrations.draft_saved);await choose('default');await click('int-ui-verify');assert(!state.integrations.error);
 pass('Changed source preserves and blocks the old draft; repair restores it; cancelled clearing preserves it and confirmed clearing affects only one profile');

 await config();await installJira();await type('int-connection-name','Jira Work');await type('int-server','https://work.example.invalid');await click('int-auth-bearer');await type('int-token',token);await click('int-remember');await click('int-allow-write');await approve();await panel();await click('int-run');await click('int-ui-create_open');await type('int-ui-summary',privateText);await type('int-ui-description','PRIVATE work description');
 await config();await click('int-new-connection');const jiraB=state.integrations.connection;await type('int-connection-name','Jira Personal');await type('int-server','https://personal.example.invalid');await click('int-auth-bearer');await type('int-token',tokenB);await click('int-allow-write');await approve();await panel();await click('int-run');await click('int-ui-create_open');await type('int-ui-summary',privateB);await type('int-ui-description','PRIVATE personal description');let n=writes();await click('int-ui-create');assert(state.integrations.review);assert(!control('int-connection').enabled);await click('int-preview-back');assert.equal(writes(),n);await choose('default');await config();let reads=requests.length;await type('int-server','https://changed.example.invalid');await approve();assert(state.integrations.draft_blocked);await type('int-server','https://work.example.invalid');await approve();assert(!state.integrations.draft_blocked);assert(state.integrations.has_view);assert.equal(requests.length,reads);await panel();await click('int-ui-create');await click('int-preview-submit');assert(!state.integrations.error,state.integrations.error);assert.equal(writes(),n+1);await choose(jiraB);await click('int-ui-create');await click('int-preview-submit');assert(!state.integrations.error);assert.equal(writes(),n+2);
 pass('Separate account origins and credentials submit only their own restored draft; profile switching is disabled during review');

 await restart();await show('jira');await choose(jiraB);assert(state.integrations.has_view);await config();await type('int-token',tokenB);await approve();assert(!state.integrations.draft_blocked);await panel();await click('int-ui-create');await click('int-preview-submit');assert(!state.integrations.error);
 const vault=path.join(uiProfile,'integration-accounts.dpapi');
 const decode="Add-Type -AssemblyName System.Security; $b=[IO.File]::ReadAllBytes('"+vault.replaceAll("'","''")+"'); $p=[Security.Cryptography.ProtectedData]::Unprotect($b,$null,[Security.Cryptography.DataProtectionScope]::CurrentUser); [Console]::Write([Text.Encoding]::UTF8.GetString($p))";
 const decoded=JSON.parse(cp.execFileSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',decode],{windowsHide:true,encoding:'utf8'}));assert.equal(decoded.version,2);assert.equal(decoded.profiles.jira.connections[jiraB].token,'');assert(!JSON.stringify(decoded).includes(tokenB));assert.equal(decoded.profiles.jira.connections.default.token,token);
 pass('Remembered credentials survive restart; session-only credentials are absent from storage and the same re-entered token keeps its own draft binding');

 await choose('default');mode='auth';await click('int-run');assert.equal(state.integrations.error,'integration_unauthorized');assert(state.integrations.has_view);mode='slow-read';closedAt=null;n=requests.length;await click('int-run',false);await until(s=>s.integrations.busy&&requests.length>n);assert(!control('int-connection').enabled);await cancel('read');mode='ok';assert(state.integrations.has_view);await click('int-run');await click('int-ui-create_open');await type('int-ui-summary',privateText);await click('int-ui-create');mode='slow-write';closedAt=null;n=writes();await click('int-preview-submit',false);await until(s=>s.integrations.busy&&writes()>n);await cancel('write');assert(state.integrations.uncertain);mode='ok';await choose(jiraB);assert(!state.integrations.uncertain);await choose('default');assert(state.integrations.uncertain);await restart();await show('jira');assert(state.integrations.uncertain);await config();await reveal('int-remove-connection');assert(!control('int-remove-connection').enabled);await panel();await choose(jiraB);assert(!state.integrations.uncertain);await choose('default');await click('int-inspected');await click('int-acknowledge');assert(!state.integrations.uncertain);
 pass('Authentication errors and read/write cancellation preserve drafts; uncertain results survive restart and belong only to the affected connection');

 const saved=fs.readFileSync(vault);fs.appendFileSync(vault,'EXTERNAL');await type('int-ui-description','PRIVATE unsaved external-edit draft');assert.equal(state.integrations.draft_error,'integration_store_changed');assert(state.integrations.draft_pending);const bad=fs.readFileSync(vault);await click('int-package-counter');assert.equal(state.integrations.selected,'jira');assert.equal(state.integrations.connection,'default');assert.deepEqual(fs.readFileSync(vault),bad);await client.tool('rocket_close');await until(s=>s.dialog?.kind==='quit'&&!s.input_pending);await click('dialog-submit');assert(state.integrations.draft_pending);assert.equal(state.integrations.draft_error,'integration_store_changed');assert.notEqual(state.window,'closed');assert.deepEqual(fs.readFileSync(vault),bad);fs.writeFileSync(vault,saved);await click('int-draft-retry');assert(!state.integrations.draft_pending);assert(!state.integrations.draft_error);await choose(jiraB);await choose('default');await click('int-ui-create');await click('int-preview-back');
 pass('External protected-file edits block autosave/navigation without replacing disk bytes or discarding the in-memory draft; explicit retry recovers');

 await config();await choose(jiraB);await click('int-remove-connection');await click('int-remove-back');assert(state.integrations.connections.includes(jiraB));await click('int-remove-connection');await click('int-remove-confirm');assert(!state.integrations.connections.includes(jiraB));assert.equal(state.integrations.connection,'default');await panel();await click('int-ui-create');await click('int-preview-back');
 await config();await click('int-remove-connection');await click('int-remove-confirm');assert(!state.integrations.enabled);await type('int-server','https://replacement.example.invalid');await click('int-auth-bearer');await approve();assert.equal(state.integrations.error,'integration_auth');await pick('counter');await panel();await choose('default');await reveal('int-ui-note');await client.tool('rocket_type',{target:'int-ui-note',text:'PRIVATE close pending draft',expected_tab:state.active_tab});await until(s=>!s.input_pending);await stop();await launch();await show('counter');await click('int-ui-verify-close');assert(!state.integrations.error);await stop();
 pass('Cancelled removal keeps the connection; confirmed removal preserves other drafts; close drains pending local saves before exiting');

 assert(!report.server_error,report.server_error);assert.deepEqual(local(),before);const logs=[];function walk(p){for(const f of fs.readdirSync(p,{withFileTypes:true})){const n=path.join(p,f.name);if(f.isDirectory())walk(n);else if(/^rocket(?:\.\d+)?\.jsonl$/.test(f.name))logs.push(fs.readFileSync(n,'utf8'));}}walk(root);const records=logs.join('\n').split(/\r?\n/).filter(Boolean).map(JSON.parse),text=JSON.stringify(records);assert(records.length);assert(records.every(r=>!r.dropped_before));for(const secret of[token,tokenB,privateText,privateB,'PRIVATE draft','PRIVATE work description','PRIVATE personal description','PRIVATE unsaved','work.example.invalid','personal.example.invalid'])assert(!text.includes(secret),'private data in diagnostics');
 for(const op of['integration-package','integration-profile-save','integration-draft-save','integration-run','integration-submit'])assert(records.some(r=>r.operation===op),'missing operation '+op);for(const code of['integration_files','integration_store_changed','integration_outcome_unknown'])assert(records.some(r=>r.failure_code===code),'missing failure '+code);
 report.logs=records.length;report.requests=requests.length;report.local_unchanged=true;report.passed=true;pass('Profile/draft/package diagnostics are redacted with no dropped events; HEAD/index/config and staged/unstaged/untracked bytes are unchanged');
}catch(e){report.error=String(e.stack||e);process.exitCode=1;try{await capture('failure');}catch{}try{await stop();}catch(cleanup){report.cleanup_error=String(cleanup);if(service?.exitCode===null)service.kill();}}
finally{server.closeAllConnections();server.close();if(client)await client.finish();const access=path.join(profile,'access-token');if(fs.existsSync(access))fs.unlinkSync(access);report.after=local();fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync('work/integration-profiles-last-result',path.join(root,'result.json'));console.log(JSON.stringify(report,null,2));}}
run().catch(e=>{console.error(e);process.exitCode=1});



