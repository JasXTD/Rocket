// Native localhost acceptance for Preferences → Extensions: temporary repo, disposable protected
// account, first-party fixture packages and a loopback mock API. No external service or login.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto');
const http=require('node:http'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/ext-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'fixture-repo'),profile=path.join(root,'http-profile'),uiProfile=path.join(root,'rocket-ui-state'),downloads=path.join(root,'downloads'),packages=path.join(uiProfile,'integrations');
const branch='rocket-ui-test/lua',token='fixture_ext_'+crypto.randomBytes(12).toString('hex');
const exe=path.resolve('target/release/rocket-mcp.exe');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0',http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:''});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Rocket Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'a.txt'),'committed\n');git(['add','-A']);git(['commit','-m','Synthetic fixture']);fs.appendFileSync(path.join(repo,'a.txt'),'unstaged\n');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
let service,client,state;const requests=[],report={passed:false,root,cases:[]};
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const answer=url=>{const p=url.split('?')[0];
 if(p.endsWith('/issuetypes'))return{startAt:0,maxResults:50,total:2,issueTypes:[{id:'10001',name:'Task'},{id:'10002',name:'Bug'}]};
 if(p.includes('/createmeta/'))return{startAt:0,maxResults:50,total:0,fields:[]};
 if(p.endsWith('/editmeta'))return{fields:{summary:{operations:['set']},description:{operations:['set']}}};
 if(p.endsWith('/transitions'))return{transitions:[{id:'31',name:'In review',fields:{},isAvailable:true},{id:'41',name:'Done',fields:{},isAvailable:true}]};
 if(p.endsWith('/comment'))return{total:2,comments:[{author:{displayName:'Ada Park'},created:'2026-10-29T11:20:00.000+0000',body:{type:'doc',content:[{type:'paragraph',content:[{type:'text',text:'Reproduced on staging. The second redirect drops the host.'}]}]}},{author:{displayName:'Lin Chen'},created:'2026-10-28T16:42:00.000+0000',body:{type:'doc',content:[{type:'paragraph',content:[{type:'text',text:'Could this share the fix in ROCKET-3?'}]}]}}]};
 if(p.startsWith('/rest/api/3/issue/'))return{fields:{summary:'Synthetic issue 資料',updated:'2026-10-29T14:02:00.000+0000',status:{name:'In progress',statusCategory:{key:'indeterminate'}},assignee:{displayName:'Ada Park'},issuetype:{name:'Task'},description:{type:'doc',content:[{type:'paragraph',content:[{type:'text',text:'After a token refresh the redirect goes to the old host.'}]}]}}};
 return{issues:[{key:'ROCKET-7',fields:{summary:'Synthetic issue 資料',status:{name:'In progress',statusCategory:{key:'indeterminate'}}}},{key:'ROCKET-6',fields:{summary:'Keep drafts across restarts',status:{name:'In review',statusCategory:{key:'indeterminate'}}}},{key:'ROCKET-5',fields:{summary:'Show the exact request before sending',status:{name:'Done',statusCategory:{key:'done'}}}}]};};
const server=http.createServer(async(req,res)=>{let data='';for await(const c of req)data+=c;requests.push({method:req.method,path:req.url,body:data});assert.equal(req.headers.authorization,'Bearer '+token);res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify(answer(req.url)));});
const pass=s=>{report.cases.push(s);console.error('PASS '+s);};
async function until(fn,label='state'){const at=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return state;assert(performance.now()-at<30000,label+' timeout '+JSON.stringify({input_pending:state.input_pending,integrations:state.integrations,dialog:state.dialog?.kind}));await delay(60);}}
const control=k=>state.controls?.find(c=>c.id===k),ext=()=>state.integrations;
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.integrations?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2;},'stable');}
async function scroll(region,direction,pixels=300){await client.tool('rocket_scroll',{target:region,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(k){await stable();const visible=()=>control(k)&&control(k).rect[3]-control(k).rect[1]>18;if(visible())return;
 const region=control('prefs-scroll')?'prefs-scroll':control('int-panel-scroll')?'int-panel-scroll':null;
 if(region){for(let i=0;i<3&&!visible();i++)await scroll(region,'up',2000);for(let i=0;i<25&&!visible();i++)await scroll(region,'down',220);}assert(visible(),'unreachable '+k+' controls='+state.controls.map(c=>c.id).join(','));}
async function click(k){await reveal(k);assert(control(k)?.enabled,'disabled '+k+' '+JSON.stringify(ext()));await client.tool('rocket_click',{target:k,expected_tab:state.active_tab});await settled();await stable();}
async function type(k,text){await reveal(k);await client.tool('rocket_type',{target:k,text,expected_tab:state.active_tab});await settled();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const image=r.content.find(x=>x.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));}
async function resize(w,h){await stable();await client.tool('rocket_resize',{width:w,height:h});await until(s=>s.viewport[0]===w&&s.viewport[1]===h&&!s.input_pending);await settled();}
async function list(){await stable();if(ext().details)await click('int-back');}
async function extensions(){await stable();if(control('dialog-close'))await click('dialog-close');await click('menu-view');await click('view-goto');await click('goto-integrations');await settled();}
async function page(id){await stable();if(control('dialog-close'))await click('dialog-close');await click('view-ext-'+id);await settled();}
async function pickFile(file){await until(s=>s.controls.some(c=>c.id==='picker-choose'),'picker');for(let i=0;i<10&&!control('picker-path');i++){await delay(120);await click('picker-edit-path');}await client.tool('rocket_type',{target:'picker-path',text:path.dirname(file),expected_tab:state.active_tab});await settled();await click('picker-go');await stable();await client.tool('rocket_type',{target:'picker-filter',text:path.basename(file),expected_tab:state.active_tab});await settled();await until(s=>s.controls.some(c=>c.id==='picker-row-0'),'file listed');await click('picker-row-0');await click('picker-choose');await until(s=>!s.controls.some(c=>c.id==='picker-choose'),'picker closed');await settled();}
async function install(file){await list();await click('int-install-file');await pickFile(file);}
function folder(name,manifest,source){const dir=path.join(downloads,name);fs.mkdirSync(dir,{recursive:true});fs.writeFileSync(path.join(dir,'integration.json'),typeof manifest==='string'?manifest:JSON.stringify(manifest));fs.writeFileSync(path.join(dir,'main.lua'),source);return path.join(dir,'integration.json');}
const source=note=>`return function(ctx)\n  return ui.page { title = "Team dashboard", children = {\n    ui.text("${note}"),\n  } }\nend\n`;
const team=(version,permissions)=>({api:1,id:'team-dashboard',name:'Team dashboard',version,description:'Builds and notes for your team.',permissions});
async function launch(){const out=fs.openSync(path.join(root,'service.log'),'a');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>100)throw e;await delay(100);}}await client.initialize();await client.tool('rocket_open',{path:repo});await settled();await resize(1440,900);if(control('console-close'))await click('console-close');}
async function stop(){if(service&&service.exitCode===null&&service.signalCode===null){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}await client.raw('POST','/shutdown');const at=performance.now();while(service.exitCode===null&&service.signalCode===null){assert(performance.now()-at<15000,'owned service stop');await delay(60);}assert.equal(service.exitCode,0);}if(client)await client.finish();}
async function run(){try{
 await new Promise(r=>server.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+server.address().port);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');await launch();

 await settled();assert.deepEqual(ext().entries,[]);assert(!control('collapse-extensions'),'no sidebar section with nothing enabled');await capture('sidebar-none');
 await extensions();assert(control('int-package-jira'));assert(!control('pref-ext-jira'));assert(!fs.existsSync(path.join(packages,'jira')));await capture('first-run');
 assert(control('int-package-playground'),'PlayGround ships with Rocket, off');await click('int-developer');assert(control('int-open-folder')&&control('int-open-playground')&&control('int-install-repository'));await capture('developer-tools');await click('int-developer');
 pass('First run lists bundled Jira off, nothing on disk, no sidebar section, and Developer tools keeps the folder, playground and sample');

 await click('int-enable-jira');assert.equal(ext().prompt.kind,'enable');assert(!fs.existsSync(path.join(packages,'jira')),'the review comes first');await capture('review-enable');
 await click('int-review-code');assert(control('int-package-code-scroll'));await capture('review-enable-code');await click('int-review-code');
 assert.deepEqual(ext().prompt.new_commands,[],'a first review lists commands with nothing to compare');await click('int-review-approve');assert(fs.existsSync(path.join(packages,'jira/main.lua')));assert.equal(ext().details,true);assert.equal(ext().needs_connection,true);assert.equal(requests.length,0);assert(control('pref-ext-jira'));
 await capture('details-no-connection');await page('jira');assert(control('int-add-connection'));await capture('page-no-connection');
 await click('int-add-connection');assert.equal(ext().details,true);await type('int-server','https://jira.example.invalid');await click('int-auth-bearer');await type('int-token',token);await click('int-remember');assert(control('int-save'));assert(control('int-discard'));await capture('details-edited');
 await click('int-save');assert(!control('int-save'));assert.equal(ext().enabled,true);await page('jira');assert(control('int-run-first'));await capture('page-not-loaded');assert.equal(requests.length,0,'opening a page runs nothing');
 await click('int-run-first');assert(ext().has_view,ext().error);await capture('jira-panel');assert(requests.length>0);
 pass('Enable reviews Jira, then copies it; connection fields save inside the extension; the sidebar page runs nothing until Refresh');

 await click('int-ui-issues-ROCKET-7');assert(!ext().error,ext().error);assert(ext().widget_ids.includes('comment_body_ROCKET-7'));await capture('jira-issue');
 await click('int-ui-scope-1');assert(new URL(requests.at(-1).path,'http://x').searchParams.get('jql').startsWith('project = "ROCKET"'));
 await click('int-ui-create_open');assert(ext().widget_ids.includes('summary'));await capture('jira-create');await click('int-ui-pane_cancel');
 await click('int-ui-edit_open');await capture('jira-edit');await click('int-ui-pane_cancel');
 await resize(780,620);await capture('jira-narrow-issue');await click('int-ui-pane-0-0');await capture('jira-narrow-list');await resize(1440,900);
 pass('Jira shows issues and the chosen issue side by side; a row opens it, forms take the right pane, and a narrow window shows two tabs');

 const before=requests.length;await click('menu-repository');assert(control('repository-extensions'));await capture('menu-repository-extensions-row');await click('repository-extensions');assert(control('ext-menu-jira-create-issue'));await capture('menu-repository-extensions');assert.equal(requests.length,before,'opening a menu runs nothing');await click('ext-menu-jira-create-issue');assert.equal(ext().selected,'jira');assert(ext().widget_ids.includes('summary'),ext().error);assert(!ext().review);await click('int-ui-pane_cancel');
 const commit=()=>state.controls.find(c=>/^commit-[0-9a-f]{40}$/.test(c.id)).id,sha=()=>commit().slice('commit-'.length);
 await click('view-commits');await client.tool('rocket_click',{target:commit(),button:'right',expected_tab:state.active_tab});await stable();assert(control('ext-context-jira'));await capture('right-click-share');
 await click('ext-context-jira');assert.equal(ext().details,true,'the row opens the connection, where the switch is');
 await click('int-allow-repository');await click('int-save');assert(ext().repository_allowed);await page('jira');await click('view-commits');const picked=sha();
 await client.tool('rocket_click',{target:commit(),button:'right',expected_tab:state.active_tab});await stable();await capture('right-click-commit');await client.tool('rocket_click',{target:'ext-context-jira',expected_tab:state.active_tab});await stable();assert(control('ext-context-jira-find-commit'));await capture('right-click-commit-submenu');
 const reads=requests.length;await click('ext-context-jira-find-commit');assert(!ext().error,ext().error);assert.equal(ext().selected,'jira');assert(requests.length>reads);assert(new URL(requests.at(-1).path,'http://x').searchParams.get('jql').includes(picked.slice(0,7)));assert(state.controls.some(c=>c.id==='view-ext-jira'));await capture('command-commit');
 pass('Commands show in Repository → Extensions and on right-click, a menu runs nothing, sharing stays the user\'s step, and a command runs on the commit that was clicked');

 fs.appendFileSync(path.join(packages,'jira/main.lua'),'-- edited\n');await extensions();await click('int-reload');await click('dialog-close');await click('view-commits');assert.equal(ext().entries.find(e=>e.id==='jira').block,'review');
 await client.tool('rocket_click',{target:commit(),button:'right',expected_tab:state.active_tab});await stable();await capture('right-click-review');await click('ext-context-jira');assert.equal(ext().prompt.kind,'again');await click('int-review-approve');assert.equal(ext().entries.find(e=>e.id==='jira').block,'draft','the draft belongs to the old files');
 await client.tool('rocket_click',{target:commit(),button:'right',expected_tab:state.active_tab});await stable();await capture('right-click-draft');await click('ext-context-jira');assert.equal(ext().selected,'jira');assert(ext().draft_blocked);assert(!ext().error||ext().error==='integration_draft_changed',ext().error);await capture('page-old-draft');await click('int-clear-draft');await click('int-clear-confirm');assert.equal(ext().entries.find(e=>e.id==='jira').block,null);
 await click('view-commits');await client.tool('rocket_shortcut',{binding:'Ctrl+K',expected_tab:state.active_tab});await until(s=>s.controls.some(c=>c.id==='palette-input'),'palette');await type('palette-input','issue');await capture('palette-commands');await click('palette-row-0');assert(ext().widget_ids.includes('summary'),ext().error);await click('int-ui-pane_cancel');
 pass('Changed files leave one row that opens the review, then one that leads to the draft they outdated; the palette finds every command under its extension');

 const v1=folder('team-dashboard',team('2.3.0',{network:true,repository:true}),source('one'));
 await extensions();await install(v1);assert.equal(ext().prompt.kind,'install');assert(!fs.existsSync(path.join(packages,'team-dashboard')),'nothing lands before the review');await capture('review-install');
 await click('int-review-approve');assert(fs.existsSync(path.join(packages,'team-dashboard/main.lua')));assert.equal(ext().selected,'team-dashboard');assert.equal(ext().details,true);await capture('details-installed');
 const offline=folder('release-checklist',{api:1,id:'release-checklist',name:'Release checklist',version:'1.0.1',description:'Checks a release branch before you tag it.',permissions:{network:false,write:false,repository:true},settings:[{id:'branch',label:'Release branch',default:'main'}]},source('offline'));
 await install(offline);await click('int-review-approve');assert(!control('int-new-connection'),'no Connection card without network');assert(control('int-setting-branch'));assert(control('int-allow-repository'));await capture('details-offline');
 await click('int-developer');assert(control('int-validate'));await click('int-validate');assert(ext().validation.valid);await capture('details-developer');await click('int-developer');
 await list();await capture('list');await page('team-dashboard');await capture('sidebar-entries');
 pass('Install from file reviews the chosen files before copying them; offline extensions have Settings, not a connection');

 const v2=folder('team-dashboard-2',team('2.4.0',{network:true,repository:true,write:true}),source('two')+'-- changed\n');
 await extensions();await install(v2);assert.equal(ext().prompt.kind,'update');assert.equal(ext().prompt.version,'2.3.0 → 2.4.0');assert.deepEqual(ext().prompt.new_permissions,['Reviewed writes']);assert.equal(ext().prompt.added,2);assert.equal(ext().prompt.removed,1);await capture('review-update');
 await click('int-review-code');await capture('review-update-changes');await click('int-review-approve');assert.equal(JSON.parse(fs.readFileSync(path.join(packages,'team-dashboard/integration.json'),'utf8')).version,'2.4.0');assert.equal(ext().needs_review,false);
 const v3=folder('team-dashboard-3',team('2.5.0',{network:true,repository:true,write:true}),source('two')+'-- changed again\n');
 await click('int-enable');assert.equal(ext().extension_enabled,false);await install(v3);await click('int-review-approve');assert.equal(JSON.parse(fs.readFileSync(path.join(packages,'team-dashboard/integration.json'),'utf8')).version,'2.5.0');assert.equal(ext().extension_enabled,false,'an update leaves an extension that was off, off');await click('int-enable');assert.equal(ext().prompt,null,'the update was its review');assert.equal(ext().extension_enabled,true);
 pass('Update from file names the new permission, counts and shows changed lines, replaces the same folder, and leaves an extension that is off, off');

 fs.appendFileSync(path.join(packages,'release-checklist/main.lua'),'-- edited\n');
 await list();await click('int-reload');assert(control('int-review-release-checklist'));assert(ext().entries.find(e=>e.id==='release-checklist').attention);await capture('list-files-changed');
 await page('release-checklist');assert(ext().needs_review);assert(control('int-review'));assert(!control('int-run').enabled);await capture('page-files-changed');await click('int-review');assert.equal(ext().prompt.kind,'again');await capture('review-again');await click('int-review-approve');assert.equal(ext().needs_review,false);assert(!ext().entries.find(e=>e.id==='release-checklist').attention);
 pass('Edited files ask again, mark the extension in the sidebar, block its page, and one review clears them');

 for(const[name,file,code]of[['invalid',folder('broken-sample','{"api":1}',source('x')),'integration_manifest'],['newer-api',folder('board-view',{api:2,id:'board',name:'Board view',version:'1'},source('x')),'integration_api'],['bundled-id',folder('jira-tools',{api:1,id:'jira',name:'Jira tools',version:'9',permissions:{network:true}},source('x')),'integration_bundled']]){await extensions();await install(file);assert.deepEqual(ext().prompt,{kind:'failed',code});assert(control('int-review-choose'));await capture('error-'+name);await click('dialog-close');}
 await extensions();await install(v3);assert.equal(ext().prompt.kind,'installed');await capture('error-same-version');await click('int-review-show');assert.equal(ext().prompt,null);assert.equal(ext().selected,'team-dashboard');
 assert(!fs.existsSync(path.join(packages,'board')));assert(!fs.existsSync(path.join(packages,'jira-tools')));
 pass('Invalid, newer-API, bundled-ID and already-installed files each say why, and nothing is installed');

 fs.cpSync(path.join(packages,'team-dashboard'),path.join(packages,'team-copy'),{recursive:true});fs.mkdirSync(path.join(packages,'broken-sample'));fs.writeFileSync(path.join(packages,'broken-sample/integration.json'),'{"api":1}');
 await list();await click('int-reload');assert.deepEqual(ext().package_errors.sort(),['integration_duplicate','integration_duplicate','integration_files']);assert(control('int-package-jira'));await capture('list-problems');
 await click('int-issue-remove-1');assert.equal(ext().prompt.kind,'remove');await capture('review-remove-copy');await click('int-review-approve');assert(!fs.existsSync(path.join(packages,'team-copy'))&&fs.existsSync(path.join(packages,'team-dashboard')));assert.equal(ext().package_errors.length,1);
 fs.rmSync(path.join(packages,'broken-sample'),{recursive:true});await click('int-reload');assert.equal(ext().package_errors.length,0);
 pass('Problem folders are rows, healthy extensions stay, and one copy of a duplicate ID can be removed');

 await click('int-package-jira');await click('int-remove');assert.equal(ext().prompt.kind,'remove');await capture('review-remove');await click('dialog-close');assert.equal(ext().prompt,null);assert(fs.existsSync(path.join(packages,'jira')));
 await list();await click('int-package-release-checklist');await click('int-remove');await click('int-review-approve');assert(!fs.existsSync(path.join(packages,'release-checklist')));assert.equal(ext().details,false);assert(!ext().entries.some(e=>e.id==='release-checklist'));
 await click('int-package-jira');await click('int-enable');assert(!ext().entries.some(e=>e.id==='jira'));assert.equal(ext().prompt,null);await click('int-enable');assert.equal(ext().prompt,null,'approved files need no second review');assert(ext().entries.some(e=>e.id==='jira'));
 pass('Remove closes without acting until confirmed, takes the extension and its connections, and Enable keeps its approval');

 await click('dialog-close');await click('menu-view');await click('view-goto');assert(control('goto-ext-jira')&&control('goto-integrations'));await capture('view-go-to');await click('goto-ext-jira');assert.equal(ext().selected,'jira');
 await client.tool('rocket_shortcut',{binding:'Ctrl+K',expected_tab:state.active_tab});await until(s=>s.controls.some(c=>c.id==='palette-input'),'palette');await type('palette-input','ext');await capture('palette');await click('palette-row-0');assert(control('pref-page-integrations'),'the palette opens Extensions');
 await page('jira');await click('int-configure');await list();await click('int-developer');await click('int-open-playground');await until(s=>s.integrations.playground.loaded&&!s.integrations.playground.busy,'playground');assert(control('int-play-run'));await capture('playground');await click('int-play-back');assert(control('pref-page-integrations'));
 pass('View → Go to and the palette reach extensions and the list; the playground opens in the content area and returns to Extensions');

 await list();await click('int-enable-playground');assert.equal(ext().prompt.kind,'enable');await capture('review-playground');await click('int-review-approve');await page('playground');await click('int-run-first');assert(ext().has_view,ext().error);await capture('playground-controls');
 for(const[i,name]of[[1,'text'],[2,'data'],[3,'layout'],[4,'commands']]){await click('int-ui-section-'+i);assert(!ext().error,ext().error);await capture('playground-'+name);}
 await click('int-ui-section-2');await click('int-ui-rows-marks');assert(!ext().error,ext().error);await capture('playground-data-row');
 await click('int-ui-section-0');await resize(780,620);await capture('playground-narrow');await resize(1440,900);
 pass('PlayGround shows every section beside its Lua, and a table row answers a click');

 if(control('dialog-close'))await click('dialog-close');await resize(1440,620);assert(!control('view-ext-jira'),'below the fold at this height');await click('menu-view');await click('view-goto');await click('goto-ext-jira');await until(()=>{const r=control('view-ext-jira')?.rect;return r&&r[3]-r[1]>=27;},'the whole sidebar row');await capture('sidebar-revealed');await resize(1440,900);await extensions();
 pass('An extension opened from View → Go to brings its sidebar row into view');

 await resize(780,620);await list();await capture('narrow-list');await click('int-package-jira');await capture('narrow-details');await click('dialog-close');await capture('narrow-sidebar');await click('menu-view');await click('view-goto');await click('goto-ext-jira');await capture('narrow-page');await resize(1440,900);
 pass('The list, details and page fit 780×620');

 await stop();await launch();assert(state.integrations.entries.some(e=>e.id==='jira'),'the sidebar lists extensions without opening a page');await settled();await until(s=>s.controls.some(c=>c.id==='view-ext-jira'),'sidebar entry after restart');await click('view-ext-jira');assert.equal(ext().enabled,true);await stop();
 pass('After a restart the sidebar lists enabled extensions at once, with their connections');
 assert(!JSON.stringify(state).includes(token));
 report.requests=requests.length;report.passed=true;
}catch(e){report.error=String(e.stack||e);process.exitCode=1;try{await capture('failure');}catch{}try{await stop();}catch(cleanup){report.cleanup_error=String(cleanup);if(service?.exitCode===null)service.kill();}}
finally{server.closeAllConnections();server.close();if(client)await client.finish();const access=path.join(profile,'access-token');if(fs.existsSync(access))fs.unlinkSync(access);fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync('work/extensions-last-result',path.join(root,'result.json'));console.log(JSON.stringify(report,null,2));}}
run().catch(e=>{console.error(e);process.exitCode=1;});
