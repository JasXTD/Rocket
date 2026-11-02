// Focused repository manager acceptance. Only marked local repositories and an isolated profile.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/manager-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/manager';const report={passed:false,root,cases:[]};
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template'),GIT_OPTIONAL_LOCKS:'0'});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function repository(name, commits) {
  const repo = path.join(root, name), hooks = path.join(root, 'hooks-' + name);
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket Touches Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  const ids = [];
  for (let n = 1; n <= commits; n++) { fs.writeFileSync(path.join(repo, `${n}.txt`), `PRIVATE_${n}\n`); git(['add', '-A']); git(['commit', '-m', `PRIVATE_COMMIT_${n}`]); ids.push(git(['rev-parse', 'HEAD'])); }
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  return { repo, git, ids };
}
let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
const WAIT_MS = Number(process.env.ROCKET_MCP_WAIT_MS || 45000);
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < WAIT_MS, label + ' timed out; ' + JSON.stringify({ dialog: state.dialog?.kind, write: state.write, notice: state.notice }));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
function control(id) { return state.controls.find(c => c.id === id); }
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function type(target, text) { await stable(); await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeConsole() { await stable(); if (control('console-close')) await click('console-close'); }
async function closeDialog() { if(state.repository_manager?.form)await click('manager-cancel');if(state.repository_manager?.visible)await click('manager-close');for(let n=0;state?.dialog&&n<3;n++)await click('dialog-cancel');assert(!state.dialog,'Dialog still open'); }
async function closeWindow() { if (state?.dialog && !state.write) await closeDialog(); await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }

const library=()=>state.repository_manager;
async function manager(){if(!library()?.visible){await click('menu-file');await click('repository-manager');}await settle();assert(library().visible);}
async function add(repo,name){await click('manager-add');await type('manager-path',repo.repo);await type('manager-name',name);await click('manager-save');assert(!library().form,JSON.stringify(library().form));return library().repositories.find(r=>r.name===name)?.id;}
async function select(id){await click('manager-all');await click('manager-repo-'+id);}
async function startClick(id){await stable();assert(control(id)?.enabled,id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});}
async function run(){
try{
 const exe=path.resolve(process.env.ROCKET_MANAGER_EXE||'target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
 const alpha=repository('PRIVATE_alpha',1),beta=repository('PRIVATE_日本語_🚀',1),gamma=repository('PRIVATE_cancellable',1),bad=repository('PRIVATE_unavailable',1);
 // Capture user-like staged, unstaged and untracked state. The manager must not write Git.
 fs.writeFileSync(path.join(alpha.repo,'1.txt'),'PRIVATE staged\n');alpha.git(['add','1.txt']);fs.writeFileSync(path.join(alpha.repo,'1.txt'),'PRIVATE unstaged\n');fs.writeFileSync(path.join(alpha.repo,'untracked.txt'),'PRIVATE keep');
 const baseline=[alpha,beta,gamma,bad].map(r=>({repo:r.repo,head:r.git(['rev-parse','HEAD']),index:hash(path.join(r.repo,'.git/index')),file:hash(path.join(r.repo,'1.txt'))}));
 const git=cp.execFileSync('where.exe',['git.exe'],{windowsHide:true,encoding:'utf8'}).trim().split(/\r?\n/)[0];
 const serviceEnv={...env,ROCKET_MANAGER_REAL_GIT:git,PATH:path.resolve('work/repository-manager-git')+path.delimiter+env.PATH};
 cp.execFileSync(exe,['--init-http',profile,root],{env:serviceEnv,windowsHide:true});
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const fd=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:serviceEnv,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 report.server=(await client.initialize()).serverInfo;await client.tool('rocket_open',{path:alpha.repo});await settle();await resize(1480,920);await closeConsole();
 await click('view-changes');await type('commit-message','PRIVATE draft retained');const alphaTab=state.active_tab;await click('toolbar-terminal');await until(s=>s.terminal?.running,'Fixture shell ready');const shellPid=state.terminal.pid;await manager();assert.equal(state.terminal.visible,false);
 const a=await add(alpha,'Alpha saved'),b=await add(beta,'日本語 β');
 await click('manager-workspace-save');await type('manager-name','Daily / 日常');await click('manager-member-'+b);await click('manager-active-'+b);await click('manager-save');const w=library().workspaces[0].id;
 await select(b);await click('manager-edit');await type('manager-path',bad.repo);await click('manager-save');assert.equal(path.toNamespacedPath(library().repositories.find(r=>r.id===b).path).toLowerCase(),path.toNamespacedPath(bad.repo).toLowerCase());assert(library().workspaces[0].repositories.includes(b));
 fs.renameSync(path.join(bad.repo,'.git/HEAD'),path.join(bad.repo,'.git/HEAD.saved'));
 await click('manager-workspace-'+w);await click('manager-open');assert(library().error);assert.equal(state.tabs.length,1);assert(state.tabs[0].draft_present);await capture('workspace-unavailable');
 fs.renameSync(path.join(bad.repo,'.git/HEAD.saved'),path.join(bad.repo,'.git/HEAD'));
 await select(b);await click('manager-edit');await type('manager-path',beta.repo);await click('manager-save');await click('manager-workspace-'+w);await click('manager-open');assert.equal(state.tabs.length,2);assert(state.tabs.find(t=>t.id===state.active_tab).path.endsWith('PRIVATE_日本語_🚀'));assert.equal(library().error,null);
 passed('Relocated shortcuts keep workspace membership; missing repositories report errors while preserving tabs, then reopen after correction');
 await click('manager-edit');await click('manager-member-'+a);await click('manager-cancel');assert.equal(library().workspaces[0].repositories.length,2);
 await click('manager-edit');await type('manager-name','Edited workspace');await click('manager-member-'+a);await click('manager-save');assert.deepEqual(library().workspaces[0].repositories,[b]);assert.equal(library().workspaces[0].active,b);
 await click('manager-all');await type('manager-filter','日本語');assert(control('manager-repo-'+b));assert(!control('manager-repo-'+a));await type('manager-filter','NO_MATCH');assert(!state.controls.some(c=>c.id.startsWith('manager-repo-')));await capture('manager-empty-filter');await type('manager-filter','');await click('manager-repo-'+b);await capture('manager-aligned-wide');
 await resize(900,700);await capture('manager-aligned-narrow');await click('manager-workspace-'+w);await click('manager-edit');await capture('workspace-edit-narrow');assert(control('manager-save')?.enabled&&control('manager-cancel')?.enabled);await click('manager-cancel');await resize(1480,920);
 await click('manager-close');await click('tab-'+alphaTab);assert.equal(state.terminal.pid,shellPid);assert(state.terminal.running&&state.terminal.visible);assert(state.tabs.find(t=>t.id===alphaTab).draft_present);
 for(const before of baseline){const r=[alpha,beta,gamma,bad].find(r=>r.repo===before.repo);assert.equal(r.git(['rev-parse','HEAD']),before.head);assert.equal(hash(path.join(r.repo,'.git/index')),before.index);assert.equal(hash(path.join(r.repo,'1.txt')),before.file);}
 passed('Workspace edits/cancel, filtering and compact forms work; the original shell, draft and all Git state remain intact');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const pending=new Map();for(const r of records){if(!/^(request|step|git)_(start|end)$/.test(r.event))continue;const key=r.session+':'+r.event.split('_')[0]+':'+r.id;if(r.event.endsWith('_start')){assert(!pending.has(key),'Duplicate span '+key);pending.set(key,r.operation);}else{assert.equal(pending.get(key),r.operation,'Unpaired span '+key);pending.delete(key);}}assert.equal(pending.size,0,'Unclosed diagnostic spans');const payload=JSON.stringify(records);for(const secret of ['PRIVATE','CANCELLED_LABEL','Daily /','Work renamed','日本語 β','Edited workspace'])assert(!payload.includes(secret),'Diagnostic leak '+secret);
 assert(records.some(r=>r.operation==='library-check'&&r.outcome!=='ok'&&r.event==='request_end'));assert(records.some(r=>r.operation==='save_state'&&r.outcome==='ok'));report.log_records=records.length;passed('Paired library-check/save_state diagnostics include unavailable-path errors and omit names, paths and drafts');report.passed=true;
}catch(error){report.failure=error.stack;if(client)try{await capture('failure');}catch{}throw error;}
finally{fs.writeFileSync('work/repository-manager-final-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();}}
run().catch(e=>{console.error(e);process.exitCode=1});
