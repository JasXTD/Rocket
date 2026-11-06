// Focused native Scan folder acceptance. No network, user profile or Git writes from Rocket.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/scan-'+crypto.randomUUID().replaceAll('-',''));
const evidence=path.resolve('tools/maintenance/repository-scan-redesign-20261105');
const parent=path.join(root,'PRIVATE_projects'),profile=path.join(root,'http-profile'),branch='rocket-ui-test/scan';
const report={passed:false,root,cases:[],timings:{}};
for(const p of [parent,evidence,path.join(root,'no-hooks')])fs.mkdirSync(p,{recursive:true});
fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'no-hooks'),GIT_OPTIONAL_LOCKS:'0'});
const git=(at,args)=>cp.execFileSync('git',['-C',at,...args],{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trimEnd();
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function repo(name,commit=false){const p=path.join(parent,name);fs.mkdirSync(p,{recursive:true});git(p,['init','--initial-branch='+branch]);for(const [k,v] of [['user.name','Rocket Fixture'],['user.email','rocket@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',path.join(root,'no-hooks')]])git(p,['config',k,v]);fs.writeFileSync(path.join(p,'.git/rocket-ui-fixture'),branch);if(commit){fs.writeFileSync(path.join(p,'file'),'committed');git(p,['add','.']);git(p,['commit','-m','PRIVATE fixture']);}return p;}
const normalize=p=>path.toNamespacedPath(p).toLowerCase();
let client,service,state;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const manager=()=>state.repository_manager,scan=()=>manager().scan,control=id=>state.controls.find(c=>c.id===id);
async function until(fn,label='state',ms=60000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return state;assert(performance.now()-start<ms,label+' timed out '+JSON.stringify({scan:manager()?.scan?.phase,error:manager()?.scan?.error}));await delay(50);}}
const settle=()=>until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running),'settled');
async function stable(){let last='',n=0;await until(s=>{if(s.input_pending)return false;const now=JSON.stringify([s.controls,s.viewport]);n=now===last?n+1:0;last=now;return n>=2;},'stable controls');}
async function startClick(id){await stable();assert(control(id)?.enabled,'enabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});}
// Native state can settle one frame before the corresponding controls are drawn.
// Ordinary assertions need the rendered screen; cancellation probes use startClick.
async function click(id){await startClick(id);await settle();await stable();}
async function type(id,text){await stable();await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle();await stable();}
async function scroll(direction,pixels=700){await client.tool('rocket_scroll',{target:'manager-scan-results',direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const im=result.content.find(c=>c.type==='image');assert(im);fs.writeFileSync(path.join(evidence,name+'.png'),Buffer.from(im.data,'base64'));fs.writeFileSync(path.join(evidence,name+'.json'),JSON.stringify(state,null,2));}
async function openManager(){await click('menu-file');await click('repository-manager');assert(manager().visible);}
// Scan folder opens the folder picker; choosing a folder starts the scan.
// The picker opens on a breadcrumb; Edit path shows its path field.
async function pick(folder){await until(()=>control('picker-choose')?.enabled,'folder picker ready');if(!control('picker-path'))await click('picker-edit-path');await until(()=>control('picker-path')?.enabled,'picker path field');await type('picker-path',folder);
 // A typed path applies on Go (or Enter); Choose takes the folder the picker shows.
 await click('picker-go');await until(()=>control('picker-choose')?.enabled&&!control('picker-path'),'picker opened the typed folder');}
async function newScan(folder=parent){await click('manager-scan');await pick(folder);const t=performance.now();await click('picker-choose');await until(s=>s.repository_manager.scan&&!['scanning','stopping'].includes(s.repository_manager.scan.phase),'scan finished');assert.equal(scan().phase,'complete',scan().error);return performance.now()-t;}
async function rescan(){const t=performance.now();await click('manager-scan-start');await until(s=>s.repository_manager.scan&&!['scanning','stopping'].includes(s.repository_manager.scan.phase),'rescan finished');assert.equal(scan().phase,'complete',scan().error);return performance.now()-t;}
async function closeScan(){await client.tool('rocket_escape');await settle();await stable();assert.equal(scan(),null,'Esc closes the scan dialog');}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending,'resize');await settle();await stable();}
const passed=s=>{report.cases.push(s);console.error('PASS '+s);};
async function closeWindow(){if(scan())await closeScan();if(manager()?.visible)await click('manager-close');await client.tool('rocket_close');await until(s=>s.window==='closed'||s.window==='not_started'||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.window==='closed'||s.window==='not_started');}}
async function run(){try{
 const exe=path.resolve(process.env.ROCKET_SCAN_EXE||'target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 const existing=repo('00_existing',true),unicode=repo('01_日本語_🚀'),nested=repo('nested/資料 repo');
 for(let n=2;n<30;n++)repo(String(n).padStart(2,'0')+'_project');
 const linked=path.join(parent,'worktree 日本語');git(existing,['worktree','add','-b','rocket-ui-test/scan-linked',linked]);fs.writeFileSync(linked+'.rocket-worktree-fixture','rocket-ui-test/scan-linked\n'+existing);
 const expectedCount=32;
 fs.mkdirSync(path.join(parent,'invalid/.git'),{recursive:true});
 fs.symlinkSync(existing,path.join(parent,'duplicate-link'),'junction');
 // Put a fake repository inside metadata: the scanner must not traverse .git.
 fs.mkdirSync(path.join(existing,'.git/hidden/.git'),{recursive:true});
 fs.writeFileSync(path.join(existing,'file'),'staged');git(existing,['add','file']);fs.writeFileSync(path.join(existing,'file'),'unstaged');fs.writeFileSync(path.join(existing,'untracked'),'keep');
 const baseline={head:git(existing,['rev-parse','HEAD']),index:hash(path.join(existing,'.git/index')),config:hash(path.join(existing,'.git/config')),file:hash(path.join(existing,'file')),untracked:hash(path.join(existing,'untracked'))};
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const fd=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
 for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(n===100)throw e;await delay(100);}}
 report.server=(await client.initialize()).serverInfo;
 await client.tool('rocket_open',{path:existing});await settle();await client.tool('rocket_resize',{width:1480,height:920});await settle();
 if(control('console-close'))await click('console-close');
 await openManager();await click('manager-add');await type('manager-path',existing);await type('manager-name','PRIVATE preserved label');await click('manager-save');
 const original=structuredClone(manager().repositories[0]),tabs=state.tabs.map(t=>t.path);
 await click('manager-folder-new');await type('manager-name','PRIVATE Imported');await click('manager-save');const folderId=manager().folders[0].id;await click('manager-all');
 // Scan folder opens the folder picker first; choosing the folder starts the scan.
 await click('manager-scan');await pick(parent);await capture('scan-picker');
 await startClick('picker-choose');await until(s=>s.repository_manager.scan?.phase==='scanning'&&s.repository_manager.scan.found>=1,'active scan');
 const stopAt=performance.now();await startClick('manager-scan-stop');await until(s=>['stopping','cancelled'].includes(s.repository_manager.scan?.phase),'stop feedback');report.timings.stop_feedback_ms=performance.now()-stopAt;
 await settle();report.timings.scan_stop_ms=performance.now()-stopAt;assert.equal(scan().phase,'cancelled');assert.deepEqual(manager().repositories,[original]);
 await stable();assert(control('manager-scan-start')?.enabled&&control('manager-scan-change')?.enabled,'Scan again and Choose another folder after Stop');await capture('scan-cancelled');
 passed('Scan folder starts from the folder picker; Stop answers at once and saves nothing');
 report.timings.full_scan_ms=await rescan();assert.equal(scan().rows.length,expectedCount,scan().notice);assert.equal(scan().rows.filter(r=>r.saved).length,1);assert(scan().rows.some(r=>normalize(r.path)===normalize(unicode)));assert.equal(scan().rows.filter(r=>r.kind==='Linked worktree').length,1);assert(scan().notice.includes('1 invalid'));assert(scan().notice.includes('1 skipped'));
 assert(!scan().rows.some(r=>r.path.includes('duplicate-link')||r.path.includes('hidden')));
 // Branch and last commit come from the Repository manager's summary read; a repository without commits shows neither.
 const row=p=>scan().rows.find(r=>normalize(r.path)===normalize(p));
 assert.match(row(existing).meta,/^rocket-ui-test\/scan · /);assert.match(row(linked).meta,/^rocket-ui-test\/scan-linked · /);assert.equal(row(unicode).meta,'');
 assert.equal(row(nested).group,'nested');assert.equal(row(unicode).group,'');assert(scan().match_disk,'disk folders become library folders by default');assert.deepEqual(scan().new_folders,['nested']);
 await capture('scan-results-desktop');
 passed('Results list branch and last commit per repository, grouped by disk folder');
 const total=expectedCount-1;assert.equal(scan().selection.total,total);assert(scan().selection.checked);assert.equal(scan().left_out,0);
 await click('manager-scan-group-0');assert.equal(scan().selection.total,1,'the top group box leaves out only its own rows');assert.equal(scan().left_out,total-1);assert(control('manager-scan-add')?.enabled);
 await click('manager-scan-group-0');assert.equal(scan().selection.total,total);
 const unicodeRow=scan().rows.findIndex(r=>normalize(r.path)===normalize(unicode));await click('manager-scan-row-'+unicodeRow);assert.equal(scan().selection.total,total-1);await click('manager-scan-row-'+unicodeRow);
 await type('manager-scan-filter','日本語');assert.equal(scan().visible.length,2);assert.equal(scan().selection.hidden,total-2);
 await click('manager-scan-group-0');assert.equal(scan().selection.shown,0);assert.equal(scan().selection.total,total-2);
 await type('manager-scan-filter','');assert(scan().selection.partial&&!scan().selection.checked);
 await click('manager-scan-group-0');assert.equal(scan().selection.total,total);
 await type('manager-scan-filter','NESTED');assert.equal(scan().visible.length,1);assert.equal(scan().rows[scan().visible[0]].relative_path,path.join('nested','資料 repo'));
 await type('manager-scan-filter','PRIVATE_no_match');assert.equal(scan().visible.length,0);assert.equal(scan().selection.eligible,0);assert(!scan().selection.checked);assert.equal(scan().selection.hidden,total);await capture('scan-filter-no-match');
 await click('manager-scan-clear-filter');assert.equal(scan().filter,'');
 await click('manager-scan-view-new');assert.equal(scan().visible.length,total);assert(scan().visible.every(i=>!scan().rows[i].saved));
 await click('manager-scan-view-saved');assert.equal(scan().visible.length,1);
 await click('manager-scan-view-skipped');assert(control('manager-scan-issues'),'skipped entries listed');await capture('scan-skipped');
 assert(scan().issues.some(i=>i.path.endsWith('duplicate-link')&&i.reason==='Link skipped'));assert(scan().issues.some(i=>i.path.endsWith('invalid')&&i.reason.includes('Git repository')));
 await click('manager-scan-view-all');
 passed('Group boxes and rows change only shown new repositories; search, New/Saved/Skipped views and skipped-entry explanations');
 // Cancel validation after it has started, not merely closing the dialog.
 const addAt=performance.now();await startClick('manager-scan-add');await until(s=>s.repository_manager.scan?.phase==='validating','batch validation');await startClick('manager-scan-stop');await settle();report.timings.add_stop_ms=performance.now()-addAt;assert.equal(scan().phase,'cancelled');assert.deepEqual(manager().repositories,[original]);await capture('scan-add-stopped');
 passed('Cancelling batch validation leaves the entire saved library unchanged');
 // Introduce an external invalidation after the MCP policy check has admitted the batch.
 await startClick('manager-scan-add');await until(s=>s.repository_manager.scan?.phase==='validating','second validation');
 const victim=path.join(parent,'29_project'),gitdir=path.join(victim,'.git'),moved=path.join(victim,'.git-held');fs.renameSync(gitdir,moved);
 try{await settle();assert(scan()?.error,'external repository disappearance must fail the batch');assert.deepEqual(manager().repositories,[original]);await capture('scan-external-error');}finally{fs.renameSync(moved,gitdir);}
 passed('External repository removal during validation reports an error and saves no partial batch');
 await client.tool('rocket_resize',{width:780,height:620});await settle();await stable();await capture('scan-compact');
 const fixedControls=['manager-scan-folder','manager-scan-filter','manager-scan-add'];for(const id of fixedControls)assert(control(id)?.enabled,id+' reachable at 780×620');
 const fixed=Object.fromEntries(fixedControls.map(id=>[id,structuredClone(control(id))]));
 await scroll('down',1200);await capture('scan-compact-bottom');for(const id of fixedControls)assert.deepEqual(control(id),fixed[id],id+' stays pinned while results scroll');
 await click('manager-scan-folder');await click('manager-scan-folder-'+folderId);assert.equal(scan().folder,folderId);assert(!scan().match_disk,'a chosen folder replaces Match disk folders');
 await click('manager-scan-add');assert.equal(scan(),null);assert.equal(manager().repositories.length,expectedCount);assert.deepEqual(manager().repositories.find(r=>r.id===original.id),original);assert.deepEqual(state.tabs.map(t=>t.path),tabs);
 assert(manager().repositories.filter(r=>r.id!==original.id).every(r=>r.folder===folderId));
 passed('Compact results scroll while Save in, search and Add stay put; the batch saves once into the chosen folder without opening tabs or changing duplicates');
 await resize(1480,920);
 await newScan();assert(scan().rows.every(r=>r.saved&&!r.selected));assert(!control('manager-scan-add'),'nothing to add');await capture('scan-all-saved');
 // Subfolders off: the selected folder and its direct children, not grandchildren.
 await click('manager-scan-recursive');await until(s=>s.repository_manager.scan?.phase==='complete'&&!s.repository_manager.scan.recursive,'shallow rescan');assert.equal(scan().rows.length,expectedCount-1);assert(!scan().rows.some(r=>normalize(r.path)===normalize(nested)));await closeScan();
 const empty=path.join(root,'PRIVATE_empty');fs.mkdirSync(empty);await newScan(empty);assert.equal(scan().rows.length,0);assert(!control('manager-scan-add'));assert(control('manager-scan-change')?.enabled);await capture('scan-empty');
 await click('manager-scan-change');await until(()=>control('picker-choose')?.enabled,'picker from Choose another folder');await client.tool('rocket_escape');await settle();await stable();assert(scan(),'Esc closes only the picker');await closeScan();
 // Automation can't reach outside folders or folder links: neither is scanned.
 for(const outside of [path.resolve('..'),path.join(parent,'duplicate-link')]){
  await click('manager-scan');await until(()=>control('picker-choose')?.enabled,'folder picker ready');if(!control('picker-path'))await click('picker-edit-path');await until(()=>control('picker-path')?.enabled,'picker path field');
  await client.request('tools/call',{name:'rocket_type',arguments:{target:'picker-path',text:outside,expected_tab:state.active_tab}});await settle();await stable();
  await client.request('tools/call',{name:'rocket_click',arguments:{target:'picker-go',expected_tab:state.active_tab}});await settle();await stable();
  await client.request('tools/call',{name:'rocket_click',arguments:{target:'picker-choose',expected_tab:state.active_tab}});await settle();
  await until(s=>!s.repository_manager.scan||!['scanning','stopping','validating'].includes(s.repository_manager.scan.phase),'scan settled');await stable();
  // The picker refuses the folder, or the scan refuses it: it is never what was scanned.
  assert(!scan()||scan().error||normalize(scan().root)!==normalize(outside),'scanned '+outside);
  if(control('picker-choose')){await client.tool('rocket_escape');await settle();await stable();}
  if(scan())await closeScan();
 }
 passed('Repeated scans show all-saved; shallow and empty scans; Esc closes the picker first; outside folders and links are refused');
 assert.equal(git(existing,['rev-parse','HEAD']),baseline.head);for(const [k,p] of [['index','.git/index'],['config','.git/config'],['file','file'],['untracked','untracked']])assert.equal(hash(path.join(existing,p)),baseline[k]);
 const saved=structuredClone(manager().repositories);await closeWindow();await client.tool('rocket_open',{path:existing});await settle();await openManager();assert.deepEqual(manager().repositories,saved);await capture('scan-saved-restarted');await closeWindow();await client.raw('POST','/shutdown');
 passed('Saved shortcuts persist after reopening; HEAD, index, config, staged/unstaged and untracked files unchanged');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const ends=records.filter(r=>r.event==='request_end');assert(ends.some(r=>r.operation==='library-scan'&&r.outcome==='ok'));assert(ends.some(r=>r.operation==='library-scan'&&r.outcome==='cancelled'));assert(ends.some(r=>r.operation==='library-scan-add'&&r.outcome!=='ok'));assert(ends.some(r=>r.operation==='library-scan-add'&&r.outcome==='ok'));
 assert(!JSON.stringify(records).includes('PRIVATE'));report.log_records=records.length;passed('Scan/add outcomes and cancellation logged without repository paths, names or private payloads');report.passed=true;
}catch(error){report.failure=error.stack;if(client)try{await capture('scan-failure');}catch{}throw error;}
finally{fs.writeFileSync(path.join(evidence,'mcp-result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{if(state?.repository_manager?.scan&&state.busy){await startClick('manager-scan-stop');await settle();}await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(250);if(service.exitCode===null)service.kill();}if(client)await client.finish();}}
run().catch(e=>{console.error(e);process.exitCode=1});
