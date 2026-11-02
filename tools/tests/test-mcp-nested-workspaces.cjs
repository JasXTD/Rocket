// Focused native acceptance. Uses only marked local repositories, a private
// loopback service and disposable settings. No user login, hooks or network Git.
'use strict';
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('tools/maintenance/nested-workspaces-20261030/runs/native-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/nested-workspaces';
const report={passed:false,root,cases:[],captures:[]};
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));
fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0'});
const relay=gitRelay(root,env,'tools/helpers/repository-manager/scoped-git.rs'),{gate,marker}=relay;fs.writeFileSync(gate,'');
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
report.driver_sha256=hash(__filename);report.git_relay_sha256=hash(path.resolve('tools/helpers/repository-manager/scoped-git.rs'));
const gitExe=cp.execFileSync('where.exe',['git.exe'],{windowsHide:true,encoding:'utf8'}).trim().split(/\r?\n/)[0];
function repository(name){
  const repo=path.join(root,name),hooks=path.join(repo,'empty-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
  const git=args=>cp.execFileSync(gitExe,['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
  git(['init','--initial-branch='+branch]);
  for(const [k,v] of [['user.name','Rocket Workspace Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',k,v]);
  fs.writeFileSync(path.join(repo,'資料.txt'),'PRIVATE initial\n');git(['add','資料.txt']);git(['commit','-m','Synthetic workspace fixture']);
  fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);return {repo,git};
}
let service,client,state,exe;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const passed=name=>{report.cases.push(name);console.error('PASS '+name);};
const library=()=>state.repository_manager;
const control=id=>state?.controls?.find(c=>c.id===id);
const alive=pid=>{try{process.kill(pid,0);return true;}catch(e){if(e.code==='ESRCH')return false;throw e;}};
async function until(predicate,label='UI state',timeout=45000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<timeout,label+' timed out: '+JSON.stringify({busy:state.busy,dialog:state.dialog?.kind,manager:library(),state_error:state.state_error}));await delay(70);}}
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!s.repository_manager?.running&&!(s.tabs||[]).some(t=>t.loading||t.search?.running),'Settled');}
async function saved(){await settle();await until(s=>!s.state_pending,'Settings persisted');assert.equal(state.state_error,null);}
async function stable(){let previous='',count=0;await until(s=>{const next=JSON.stringify(s.controls);count=next===previous?count+1:0;previous=next;return count>=2;},'Stable controls');}
async function scroll(target,direction,pixels=400){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await stable();}
async function reveal(target){await stable();if(control(target))return;
  const viewport=/^manager-workspace-(?:toggle-)?\d+$/.test(target)?'manager-navigation':/^manager-repo-\d+$/.test(target)?'manager-repositories':library()?.form?'manager-form-scroll':'page-details-scroll';
  if(control(viewport)){await scroll(viewport,'up',2000);for(let n=0;!control(target)&&n<10;n++)await scroll(viewport,'down',180);}
}
async function click(target){await reveal(target);assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();}
async function type(target,text){await reveal(target);assert(control(target)?.enabled,target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const item=result.content.find(c=>c.type==='image');assert(item);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(item.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));report.captures.push(name);}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function manager(){if(!library()?.visible){await click('menu-file');await click('repository-manager');}await settle();assert(library().visible);}
async function add(repo,name){await click('manager-add');await type('manager-path',repo.repo);await type('manager-name',name);await click('manager-save');assert(!library().form,JSON.stringify(library().form));return library().repositories.find(r=>r.name===name).id;}
async function select(id){await click('manager-workspace-'+id);assert.equal(library().selection,'Workspace('+id+')');}
async function workspace(name,parent,members=[]){if(parent==null)await click('manager-workspace-new');else{await select(parent);await click('manager-workspace-child');}assert.equal(library().form.parent,parent);assert.deepEqual(library().form.members,[]);await type('manager-name',name);for(const id of members)await click('manager-member-'+id);await click('manager-save');assert(!library().form,JSON.stringify(library().form));const w=library().workspaces.find(w=>w.name===name&&w.parent===parent);assert(w);return w.id;}
async function parent(id){await click('manager-parent');await capture('parent-options-'+(id??'none'));await click('manager-parent-'+(id??'none'));assert.equal(library().form.parent,id);}
function visibleBounds(){for(const c of state.controls.filter(c=>/^manager-/.test(c.id))){assert(c.rect.every(Number.isFinite),c.id);assert(c.rect[0]>=-1&&c.rect[1]>=-1&&c.rect[2]<=state.viewport[0]+1&&c.rect[3]<=state.viewport[1]+1,'Clipped control '+c.id+': '+c.rect);}}
async function start(){const fd=fs.openSync(path.join(root,'service-'+Date.now()+'.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}report.server=(await client.initialize()).serverInfo;}
async function close(){await saved();if(library()?.form)await click('manager-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending),'Close window');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window),'Closed window');}await client.raw('POST','/shutdown');const deadline=Date.now()+10000;while(service.exitCode===null&&Date.now()<deadline)await delay(50);assert.notEqual(service.exitCode,null,'Owned service did not exit');await client.finish();client=null;service=null;}
async function run(){try{
  exe=path.resolve(process.env.ROCKET_WORKSPACES_EXE||'target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
  const alpha=repository('PRIVATE_alpha'),beta=repository('PRIVATE_日本語_🚀'),gamma=repository('PRIVATE_cancellable'),bad=repository('PRIVATE_unavailable');
  fs.writeFileSync(path.join(alpha.repo,'資料.txt'),'PRIVATE staged\n');alpha.git(['add','資料.txt']);fs.writeFileSync(path.join(alpha.repo,'資料.txt'),'PRIVATE unstaged\n');fs.writeFileSync(path.join(alpha.repo,'untracked.txt'),'PRIVATE keep');
  const repos=[alpha,beta,gamma,bad],baseline=repos.map(r=>({head:r.git(['rev-parse','HEAD']),index:hash(path.join(r.repo,'.git/index')),config:hash(path.join(r.repo,'.git/config')),file:hash(path.join(r.repo,'資料.txt'))}));
  cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
  const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
  await start();await client.tool('rocket_open',{path:alpha.repo});await settle();relay.check(state);await resize(1440,920);if(control('console-close'))await click('console-close');
  await click('view-changes');await type('commit-message','PRIVATE workspace draft preserved');const alphaTab=state.active_tab;await manager();
  const a=await add(alpha,'PRIVATE Alpha'),b=await add(beta,'PRIVATE 日本語'),g=await add(gamma,'PRIVATE Deferred'),u=await add(bad,'PRIVATE Unavailable');
  await click('manager-workspace-save');await type('manager-name','Everyday 日本語');await click('manager-save');assert(!library().form);const everyday=library().workspaces.find(w=>w.name==='Everyday 日本語');assert.equal(everyday.parent,null);assert.deepEqual(everyday.repositories,[a]);await saved();assert(!fs.readFileSync(path.join(root,'rocket-ui-state/state.bin')).includes(Buffer.from('ROCKET-STATE\0\x15')),'Flat save must retain a legacy format');
  passed('Existing Save open workflow retains flat workspaces, memberships and draft');
  const work=await workspace('Work / 仕事',null),backend=await workspace('Backend',work,[a,b]),tools=await workspace('Tools 🚀',backend,[b]),personal=await workspace('Personal',null),collision=await workspace('Backend',null,[a]);
  await select(work);assert.equal(library().open_count,0);assert.equal(control('manager-open')?.enabled,false);await click('manager-include-nested');assert.equal(library().open_count,2);await click('manager-open');assert.equal(state.tabs.length,2);assert(state.tabs.find(t=>t.id===alphaTab).draft_present);await capture('nested-wide');
  await select(tools);await click('manager-workspace-toggle-'+work);assert.equal(library().selection,'Workspace('+work+')');assert(!control('manager-workspace-'+backend));assert.equal(library().include_nested,false);await click('manager-workspace-toggle-'+work);assert(control('manager-workspace-'+backend));
  passed('Unicode tree, own-only default, descendant deduplication and collapse preserve existing tabs/drafts');
  await select(work);const unchanged=structuredClone(library().workspaces);await click('manager-edit');await click('manager-parent');assert(!control('manager-parent-'+work));assert(!control('manager-parent-'+backend));assert(!control('manager-parent-'+tools));await client.tool('rocket_escape');await settle();if(library().form)await click('manager-cancel');assert.deepEqual(library().workspaces,unchanged);
  await select(tools);await click('manager-edit');await type('manager-name','Backend');await parent(personal);await click('manager-save');assert.equal(library().workspaces.find(w=>w.id===tools).parent,personal);assert.deepEqual(library().workspaces.find(w=>w.id===tools).repositories,[b]);
  await select(work);await click('manager-workspace-child');await type('manager-name','Backend');const duplicateBefore=structuredClone(library().workspaces);await click('manager-save');assert.match(library().form.error,/unique|already|same parent/i);assert.deepEqual(library().workspaces,duplicateBefore);await click('manager-cancel');
  await select(work);await click('manager-remove');await click('manager-save');assert.match(library().form.error,/unique|already|same parent/i);assert.deepEqual(library().workspaces,duplicateBefore);await capture('promotion-collision');await click('manager-cancel');
  await select(collision);await click('manager-edit');await type('manager-name','Archive');await click('manager-save');await select(work);await click('manager-remove');await click('manager-save');assert(!library().form);assert(!library().workspaces.some(w=>w.id===work));assert.equal(library().workspaces.find(w=>w.id===backend).parent,null);assert.equal(library().workspaces.find(w=>w.id===tools).parent,personal);assert.equal(library().repositories.length,4);
  passed('Parent exclusions, reviewed moves, duplicate-name refusal and safe child promotion are atomic');
  await select(personal);await resize(780,620);await click('manager-close');if(!state.console_visible)await click('toolbar-console');assert(state.console_visible);await capture('compact-console-before-manager');
  // The existing full-page manager temporarily covers the repository drawer.
  await manager();assert.equal(state.console_visible,false);await select(personal);await capture('nested-compact');visibleBounds();await click('manager-edit');await type('manager-name','PRIVATE cancelled rename');await capture('nested-form-compact');visibleBounds();assert(control('manager-save')&&control('manager-cancel'));await click('manager-cancel');assert.equal(library().workspaces.find(w=>w.id===personal).name,'Personal');
  await click('manager-close');assert(state.console_visible,'Closing the manager must restore the existing Console');await click('toolbar-console');await manager();await resize(1440,920);
  passed('Compact tree/review controls remain usable; cancelled edits do not persist and the existing Console is restored');
  const unavailable=await workspace('Unavailable',null,[u]);fs.renameSync(path.join(bad.repo,'.git/HEAD'),path.join(bad.repo,'.git/HEAD.saved'));
  await click('manager-open');assert(library().error);assert.equal(state.tabs.length,2);assert(state.tabs.find(t=>t.id===alphaTab).draft_present);await capture('missing-repository');fs.renameSync(path.join(bad.repo,'.git/HEAD.saved'),path.join(bad.repo,'.git/HEAD'));
  const batch=await workspace('Batch',null,[a]);await workspace('Deferred / 取消',batch,[g,u]);await select(batch);await click('manager-include-nested');assert.equal(library().open_count,3);
  const ownership=path.join(bad.repo,'.git/rocket-ui-fixture');fs.renameSync(ownership,ownership+'.saved');await stable();await assert.rejects(()=>client.tool('rocket_click',{target:'manager-open',expected_tab:state.active_tab}),/ownership marker/i);await settle();assert.equal(state.tabs.length,2,'Entire descendant plan must be guarded before opening');fs.renameSync(ownership+'.saved',ownership);
  passed('Missing repository reports a clear error; MCP validates every descendant before opening any child');
  const deferredPath=library().repositories.find(r=>r.id===g).path;fs.writeFileSync(gate,deferredPath);await stable();await client.tool('rocket_click',{target:'manager-open',expected_tab:state.active_tab});await until(s=>s.repository_manager.running&&fs.existsSync(marker),'Gated descendant open');
  // Remaining includes the held repository check and the child still in the queue.
  const held=Number(fs.readFileSync(marker,'utf8'));assert(alive(held));assert.equal(library().remaining,2);const startStop=performance.now();
  const timing=await Promise.allSettled([
    (async()=>{await client.tool('rocket_click',{target:'manager-stop',expected_tab:state.active_tab});await until(s=>!s.repository_manager.running&&!s.input_pending,'Cancelled workspace queue');return Math.round(performance.now()-startStop);})(),
    (async()=>{while(alive(held)){assert(performance.now()-startStop<10000,'Cancelled Git child still running');await delay(10);}return Math.round(performance.now()-startStop);})()
  ]);for(const result of timing)if(result.status==='rejected')throw result.reason;
  report.cancel_feedback_ms=timing[0].value;report.cancel_process_stopped_ms=timing[1].value;report.cancel_measurement='Independent observations from MCP Stop request; process sampled every 10 ms';fs.writeFileSync(gate,'');await settle();assert.equal(state.tabs.length,2);assert(state.tabs.find(t=>t.id===alphaTab).draft_present);await capture('cancelled-open');
  await click('manager-open');assert.equal(state.tabs.length,4);assert.equal(library().error,null);await select(unavailable);await click('manager-open');assert.equal(library().error,null);assert.equal(state.tabs.length,4);
  passed('Mid-queue cancellation stops its Git child, preserves already-open tabs and supports a successful retry');
  await select(collision);await click('manager-remove');await click('manager-save');assert(!library().form);assert.equal(library().repositories.length,4);assert.equal(state.tabs.length,4);assert(state.tabs.find(t=>t.id===alphaTab).draft_present);
  await saved();const savedLibrary=structuredClone({workspaces:library().workspaces,repositories:library().repositories});assert(fs.readFileSync(path.join(root,'rocket-ui-state/state.bin')).includes(Buffer.from('ROCKET-STATE\0\x15')));await close();await start();await client.tool('rocket_open',{path:alpha.repo});await settle();await manager();assert.deepEqual({workspaces:library().workspaces,repositories:library().repositories},savedLibrary);assert(state.tabs.find(t=>t.path.includes('PRIVATE_alpha')).draft_present);assert.equal(library().include_nested,false);
  await select(personal);await capture('restarted-nested');
  for(let n=0;n<repos.length;n++){const r=repos[n],before=baseline[n];assert.equal(r.git(['rev-parse','HEAD']),before.head);assert.equal(hash(path.join(r.repo,'.git/index')),before.index);assert.equal(hash(path.join(r.repo,'.git/config')),before.config);assert.equal(hash(path.join(r.repo,'資料.txt')),before.file);}assert.equal(fs.readFileSync(path.join(alpha.repo,'untracked.txt'),'utf8'),'PRIVATE keep');
  passed('Restart preserves duplicate child names, parent IDs and drafts; HEAD/index/config and all local changes stay unchanged');
  await close();
  const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
  assert(records.length);assert(records.every(r=>r.dropped_before===0));const pending=new Map();for(const r of records){if(!/^(request|step|git)_(start|end)$/.test(r.event))continue;const key=r.session+':'+r.event.split('_')[0]+':'+r.id;if(r.event.endsWith('_start')){assert(!pending.has(key));pending.set(key,r.operation);}else{assert.equal(pending.get(key),r.operation);pending.delete(key);}}assert.equal(pending.size,0,'Unclosed diagnostic spans');const payload=JSON.stringify(records);for(const value of ['PRIVATE','Work /','Everyday 日本語','Deferred /'])assert(!payload.includes(value),'Diagnostic payload leaked '+value);assert(records.some(r=>r.operation==='library-check'&&r.event==='request_end'&&r.outcome!=='ok'));assert(records.some(r=>r.operation==='save_state'&&r.outcome==='ok'));report.log_records=records.length;
  passed('Paired library/save diagnostics record errors and cancellation without names, paths, drafts or dropped events');report.passed=true;
}catch(e){report.failure=e.stack;if(client)try{await capture('failure');}catch{}throw e;
}finally{fs.writeFileSync(gate,'');if(client)try{await close();}catch{}if(service&&service.exitCode===null)service.kill();for(const file of ['access-token','service.pid'])try{fs.unlinkSync(path.join(profile,file));}catch(e){if(e.code!=='ENOENT')console.error('Cleanup: '+e.message);}report.finished=new Date().toISOString();fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.error('Results: '+path.join(root,'result.json'));}}
run().catch(e=>{console.error(e.stack);process.exitCode=1;});
