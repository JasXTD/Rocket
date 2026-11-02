// Native localhost acceptance: temporary dirty repo, disposable protected account,
// first-party Lua fixtures and a loopback mock API. No external service or login.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto');
const http=require('node:http'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/lua-workflows-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'fixture-repo'),profile=path.join(root,'http-profile'),uiProfile=path.join(root,'rocket-ui-state');
const branch='rocket-ui-test/lua',token='fixture_lua_'+crypto.randomBytes(12).toString('hex'),privateText='PRIVATE issue 資料 🚀';
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
let version=1,issueSummary='Original issue 資料',trimTransitions=false,extraRequired=false;
let richDescription={type:'doc',version:1,content:[{type:'paragraph',content:[{type:'text',text:'Keep this rich description',marks:[{type:'strong'}]},{type:'mention',attrs:{id:'synthetic-user'}}]}]};
const originalDescription=JSON.stringify(richDescription),nextToken='cursor/第二?x=1';
const server=http.createServer(async(req,res)=>{try{
 let data='';for await(const c of req)data+=c;requests.push({method:req.method,path:req.url,body:data});
 assert.equal(req.headers.authorization,'Bearer '+token);
 const url=new URL(req.url,'http://127.0.0.1'),pathname=url.pathname;
 const send=(status,body)=>{res.writeHead(status,{'Content-Type':'application/json'});res.end(status===204?'':JSON.stringify(body));};
 if(mode==='slow-read'&&req.method==='GET'||mode==='slow-write'&&req.method!=='GET'||mode==='slow-check'&&url.searchParams.get('fields')==='updated'){res.on('close',()=>closedAt=performance.now());return;}
 if(mode==='auth')return send(401,{message:token+' '+privateText});
 if(mode==='reject')return send(403,{message:privateText});
 if(mode==='rate')return send(429,{message:privateText});
 if(req.method==='GET'){
  if(pathname==='/rest/api/3/search/jql'){
   const next=url.searchParams.get('nextPageToken');if(next)assert.equal(next,nextToken);
   return send(200,{issues:[{key:next?'ROCKET-8':'ROCKET-7',fields:{summary:issueSummary,status:{name:'Open'},assignee:{displayName:'Fixture'}}}],isLast:!!next,...(!next?{nextPageToken:nextToken}:{})});
  }
  if(pathname.endsWith('/issuetypes'))return send(200,{issueTypes:[{id:'10001',name:'Task'}]});
  if(pathname.endsWith('/comment'))return send(200,{total:1,comments:[{author:{displayName:'Fixture'},created:'2026-10-29T11:20:00.000+0000',body:{type:'doc',content:[{type:'paragraph',content:[{type:'text',text:'Synthetic comment'}]}]}}]});
  if(pathname.includes('/createmeta/'))return send(200,{isLast:true,fields:[{fieldId:'summary',name:'Summary',required:true,hasDefaultValue:false},...(extraRequired?[{fieldId:'customfield_123',name:'Required custom field',required:true,hasDefaultValue:false}]:[])]});
  if(pathname.endsWith('/editmeta'))return send(200,{fields:{summary:{operations:['set']},description:{operations:['set']}}});
  if(pathname.endsWith('/transitions'))return send(200,{transitions:[...(!trimTransitions?[{id:'31',name:'Done',fields:{},isAvailable:true}]:[]),{id:'11',name:'Open',fields:{},isAvailable:true},{id:'99',name:'Needs custom field',fields:{custom:{required:true}},isAvailable:true},{id:'91',name:'Unavailable',isAvailable:false}]});
  if(/^\/rest\/api\/3\/issue\/ROCKET-\d+$/.test(pathname))return send(200,{key:pathname.split('/').at(-1),fields:{summary:issueSummary,description:richDescription,status:{name:'Open'},...(mode==='missing-marker'?{}:{updated:'v'+version})}});
  throw Error('Unexpected fixture read '+pathname);
 }
 const body=JSON.parse(data);
 if(req.method==='PUT'){
  assert.equal(pathname,'/rest/api/3/issue/ROCKET-7');assert(body.fields);assert(Object.keys(body.fields).every(k=>['summary','description'].includes(k)));
  if(body.fields.summary!==undefined)issueSummary=body.fields.summary;
  if(body.fields.description!==undefined)richDescription=body.fields.description;
  version++;return send(204,{});
 }
 if(pathname.endsWith('/transitions')){assert(['31','11'].includes(body.transition.id));assert.deepEqual(Object.keys(body),['transition']);version++;return send(204,{});}
 if(pathname.endsWith('/comment')){assert(body.body.content);return send(201,{id:'44'});}
 assert.equal(pathname,'/rest/api/3/issue');assert(body.fields.summary);return send(201,{id:'1007',key:'ROCKET-7'});
}catch(e){report.server_error=String(e.stack||e);res.writeHead(500);res.end('{}');}});

const pass=s=>{report.cases.push(s);console.error('PASS '+s);};
async function until(fn,label='state'){const at=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return state;assert(performance.now()-at<30000,label+' timeout '+JSON.stringify({integrations:state.integrations,dialog:state.dialog?.kind}));await delay(60);}}
const control=k=>state.controls?.find(c=>c.id===k);
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.integrations?.busy&&(!s.integrations?.draft_pending||s.integrations?.draft_error)&&!(s.tabs||[]).some(t=>t.loading),'settled');}
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

async function run(){try{
 fs.cpSync(path.resolve('crates/rocket-integrations/examples/playground'),path.join(uiProfile,'integrations/playground'),{recursive:true,errorOnExist:true,force:false});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+server.address().port);report.executable_sha256=hash(exe);report.worker_sha256=hash(worker);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');await launch();
 await config();await reload();await pick('playground');await click('int-developer');
 const playground=path.join(uiProfile,'integrations/playground/main.lua'),playgroundSource=fs.readFileSync(playground,'utf8');
 fs.writeFileSync(playground,"while true do end -- compile only; must never execute");await click('int-validate');assert.equal(state.integrations.validation?.valid,true);assert(!state.integrations.enabled);assert.equal(requests.length,0);
 fs.writeFileSync(playground,"local secret='PRIVATE diagnostic payload'\nreturn function(\n");await click('int-validate');assert.equal(state.integrations.validation?.valid,false);assert(state.integrations.validation.line>=2);assert(!JSON.stringify(state.integrations).includes('PRIVATE diagnostic payload'));assert.equal(requests.length,0);await capture('developer-error-wide');await resize(780,660);await reveal('int-validate');await capture('developer-error-compact');await resize(1440,900);
 fs.writeFileSync(playground,playgroundSource);await click('int-validate');assert(state.integrations.validation.valid);await reload();await approve();await panel();await click('int-run');assert(!state.integrations.error);
 pass('Native developer validation compiles disabled and changed source without running module initialization, callback, Git or network; error location is redacted');

 await config();await list();if(!control('int-install-repository'))await click('int-developer');await click('int-install-repository');await pick('repository');assert(!state.integrations.repository_allowed);
 const repositoryLua=path.join(uiProfile,'integrations/repository/main.lua');
 const repositoryProbe=`return function(ctx)
 local r=ctx.repository
 if not r then return ui.page{children={ui.button{id='context_off',label='Context is off',disabled=true}}} end
 assert(r.path==nil and r.remote==nil and r.author==nil and r.files==nil and r.message==nil)
 assert(r.head=='${before.head}' and r.branch=='${branch}' and r.changes.staged==1 and r.changes.unstaged==1)
 if ctx.event=='external' then assert(r.changes.untracked==2) else assert(r.changes.untracked==1) end
 return ui.page{title='Repository snapshot',children={ui.text(r.branch),ui.button{id='context_on',label='Context verified',disabled=true},ui.button{id='external',label='Verify external change'}}}
 end`;
 fs.writeFileSync(repositoryLua,repositoryProbe);await reload();await pick('repository');await approve();await panel();await click('int-run');assert(state.integrations.widget_ids.includes('context_off'));assert.equal(requests.length,0);
 await config();await click('int-allow-repository');await approve();assert(state.integrations.repository_allowed);await panel();await clearChangedDraft();await click('int-run');assert(!state.integrations.error);assert(state.integrations.widget_ids.includes('context_on'));await capture('repository-context');
 fs.writeFileSync(path.join(repo,'external-fixture.txt'),'external PRIVATE content');await click('int-ui-external');assert(!state.integrations.error);fs.unlinkSync(path.join(repo,'external-fixture.txt'));await click('int-run');assert(!state.integrations.error);
 await reveal('int-run');await client.tool('rocket_click',{target:'int-run',expected_tab:state.active_tab});await until(s=>s.integrations.busy&&!s.input_pending);await client.tool('rocket_click',{target:'int-cancel',expected_tab:state.active_tab});await settled();assert(!state.integrations.busy);assert.deepEqual(local(),before);
 await config();await click('int-allow-repository');await approve();assert(!state.integrations.repository_allowed);await panel();await clearChangedDraft();await click('int-run');assert(state.integrations.widget_ids.includes('context_off'));
 pass('Repository context is off by default, explicitly granted/revoked, metadata-only, refreshed after external changes and cancellable without repository mutations');

 await config();await installJira();await type('int-server','https://jira.example.invalid');await click('int-auth-bearer');await type('int-token',token);await click('int-remember');await click('int-allow-write');await approve();await panel();await click('int-run');assert(!state.integrations.error);
 const query='project = ROCKET AND text ~ "資料"';await type('int-ui-search_0',query);await click('int-ui-search');assert.equal(new URL(requests.at(-1).path,'http://fixture').searchParams.get('jql'),query);
 await click('int-ui-next_page');assert.equal(new URL(requests.at(-1).path,'http://fixture').searchParams.get('nextPageToken'),nextToken);await reveal('int-ui-next_page');assert(!control('int-ui-next_page').enabled);await click('int-ui-previous_page');assert.equal(new URL(requests.at(-1).path,'http://fixture').searchParams.get('nextPageToken'),null);
 pass('Jira JQL search and bounded token pagination encode Unicode and reset page cursors correctly');

 await click('int-ui-issues-ROCKET-7');assert(!state.integrations.error);assert(state.integrations.widget_ids.includes('comment_body_ROCKET-7'));await click('int-ui-edit_open');assert(state.integrations.widget_ids.includes('edit_summary_ROCKET-7'));
 await type('int-ui-edit_summary_ROCKET-7',privateText);let n=writes();await click('int-ui-edit_issue');assert(state.integrations.review);assert.equal(writes(),n);await capture('checked-review');await click('int-preview-back');assert.equal(writes(),n);await click('int-ui-edit_issue');await click('int-preview-submit');assert(!state.integrations.error);assert.equal(writes(),n+1);assert.equal(issueSummary,privateText);assert.equal(JSON.stringify(richDescription),originalDescription);
 await click('int-ui-pane_cancel');await click('int-run');await capture('jira-details-wide');await resize(780,660);await capture('jira-details-compact');await resize(1440,900);await click('int-ui-edit_open');
 pass('Issue details and edit metadata feed native forms; reviewed summary edits preserve rich descriptions and Back sends nothing');

 await type('int-ui-edit_summary_ROCKET-7','PRIVATE retained edit 資料');n=writes();version++;await click('int-ui-edit_issue');assert.equal(state.integrations.error,'integration_stale');assert.equal(writes(),n);assert(!state.integrations.review);
 await click('int-ui-edit_reload');await click('int-ui-edit_issue');assert(state.integrations.review);version++;await click('int-preview-submit');assert.equal(state.integrations.error,'integration_stale');assert.equal(writes(),n);assert(!state.integrations.uncertain);
 await click('int-ui-edit_reload');await click('int-ui-edit_issue');await click('int-preview-submit');assert(!state.integrations.error);assert.equal(issueSummary,'PRIVATE retained edit 資料');
 pass('External issue changes before review or before send reject the write while preserving edit fields; reload and review recover safely');

 await click('int-ui-edit_reload');await type('int-ui-edit_summary_ROCKET-7','PRIVATE review source guard');await click('int-ui-edit_issue');const jira=path.join(uiProfile,'integrations/jira/main.lua'),jiraSource=fs.readFileSync(jira,'utf8');n=writes();fs.appendFileSync(jira,'\n-- changed during review');await click('int-preview-submit');assert.equal(state.integrations.error,'integration_changed');assert.equal(writes(),n);fs.writeFileSync(jira,jiraSource);await click('int-preview-back');
 await click('int-ui-edit_issue');mode='slow-check';closedAt=null;const reads=requests.length;n=writes();await click('int-preview-submit',false);await until(s=>s.integrations.busy&&requests.length>reads);await cancel('preflight');assert.equal(writes(),n);assert(!state.integrations.uncertain);mode='ok';
 pass('Edited package source blocks reviewed submission; cancelling the preflight read sends no write and clears its local recovery guard');

 await click('int-ui-edit_issue');mode='slow-write';closedAt=null;n=writes();await click('int-preview-submit',false);await until(s=>s.integrations.busy&&writes()>n);await cancel('write');assert(state.integrations.uncertain);assert.equal(state.integrations.error,'integration_outcome_unknown');mode='ok';await restart();await show('jira');assert(state.integrations.uncertain);await click('int-run');await click('int-inspected');await click('int-acknowledge');assert(!state.integrations.uncertain);
 pass('Cancelling a sent edit retains durable uncertainty and the draft across restart; no replay occurs and explicit inspection restores write access');

 await click('int-run');await click('int-ui-pane_cancel');await click('int-ui-transition_ROCKET-7');await click('int-ui-transition_ROCKET-7-0');trimTransitions=true;await click('int-run');assert(!state.integrations.draft_error);await click('int-ui-transition_issue');assert(state.integrations.review);n=writes();await click('int-preview-submit');assert(!state.integrations.error);assert.equal(writes(),n+1);assert.equal(JSON.parse(requests.filter(r=>r.method!=='GET').at(-1).body).transition.id,'11');
 pass('Transitions respect available operations and required-field limits; changed choices discard stale selections before a reviewed transition');

 await click('int-ui-edit_open');await click('int-ui-replace_description_ROCKET-7');await type('int-ui-edit_description_ROCKET-7','Replacement 資料 é');await click('int-ui-edit_issue');await click('int-preview-submit');assert(!state.integrations.error);assert.equal(richDescription.content[0].content[0].text,'Replacement 資料 é');
 await click('int-ui-pane_cancel');await click('int-ui-create_open');await type('int-ui-summary','New issue 資料');extraRequired=true;n=writes();await click('int-ui-create');assert(!state.integrations.review);assert.equal(writes(),n);extraRequired=false;await click('int-ui-create');await click('int-preview-submit');assert(!state.integrations.error);
 await click('int-run');await type('int-ui-'+state.integrations.widget_ids.find(id=>id.startsWith('comment_body_')),privateText);await click('int-ui-comment');await click('int-preview-submit');assert(!state.integrations.error);
 pass('Explicit description replacement, create-field requirements, reviewed creation and Unicode comments use supported Jira payloads');

 mode='missing-marker';await click('int-run');await click('int-ui-edit_open');await reveal('int-ui-edit_issue');assert(!control('int-ui-edit_issue').enabled);mode='ok';await click('int-run');
 for(const [m,code] of [['auth','integration_unauthorized'],['reject','integration_forbidden'],['rate','integration_rate_limited']]){mode=m;await click('int-run');assert.equal(state.integrations.error,code);assert(state.integrations.has_view);}mode='ok';
 mode='slow-read';closedAt=null;n=requests.length;await click('int-run',false);await until(s=>s.integrations.busy&&requests.length>n);await cancel('search');mode='ok';await click('int-run');assert(!state.integrations.error);
 await config();if(!state.integrations.developer_open)await click('int-developer');assert(state.integrations.recent_actions.some(r=>r.code==='cancelled'));await capture('developer-history');await click('int-history-clear');assert.equal(state.integrations.recent_actions.length,0);await panel();await stop();
 pass('Missing update markers disable edits; authentication, permission and rate errors plus read cancellation preserve the panel; diagnostics are inspectable and clearable');

 assert(!report.server_error,report.server_error);assert.deepEqual(local(),before);const logs=[];
 function walk(p){for(const f of fs.readdirSync(p,{withFileTypes:true})){const file=path.join(p,f.name);if(f.isDirectory())walk(file);else if(/^rocket(?:\.\d+)?\.jsonl$/.test(f.name))logs.push(fs.readFileSync(file,'utf8'));}}walk(root);
 const records=logs.join('\n').split(/\r?\n/).filter(Boolean).map(JSON.parse),text=JSON.stringify(records);assert(records.length);assert(records.every(r=>!r.dropped_before));
 for(const secret of[token,privateText,'PRIVATE retained edit','PRIVATE diagnostic payload','Keep this rich description',nextToken])assert(!text.includes(secret),'private data in diagnostics');
 for(const op of['integration-validate','integration-validation-result','integration-run','integration-submit'])assert(records.some(r=>r.operation===op),'missing '+op);
 for(const code of['integration_lua_syntax','integration_stale','integration_outcome_unknown'])assert(records.some(r=>r.failure_code===code),'missing '+code);
 report.logs=records.length;report.requests=requests.length;report.local_unchanged=true;report.passed=true;
 pass('Redacted diagnostic codes and request steps, zero dropped events and exact preservation of HEAD/index/config/staged/unstaged/untracked data');
}catch(e){report.error=String(e.stack||e);process.exitCode=1;try{await capture('failure');}catch{}try{await stop();}catch(cleanup){report.cleanup_error=String(cleanup);if(service?.exitCode===null)service.kill();}}
finally{server.closeAllConnections();server.close();if(client)await client.finish();const access=path.join(profile,'access-token');if(fs.existsSync(access))fs.unlinkSync(access);report.after=local();fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync('work/integration-workflows-last-result',path.join(root,'result.json'));console.log(JSON.stringify(report,null,2));}}
run().catch(e=>{console.error(e);process.exitCode=1;});
