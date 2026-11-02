const fs = require('node:fs');
const path = require('node:path');
const { randomUUID, createHash } = require('node:crypto');
const { spawnSync } = require('node:child_process');
const assert = require('node:assert/strict');
const { RocketMcpClient } = require('../mcp/mcp-client.cjs');
const workspace = path.resolve(__dirname, '../..');
const id = randomUUID().replaceAll('-','');
const root = path.resolve('work/mcp-fixtures','run-'+id);
const client = path.join(root,'ROCKET_PRIVATE_client');
const publisher = path.join(root,'ROCKET_PRIVATE_publisher');
const remote = path.join(root,'upstream.git');
const hooks = path.join(root,'hooks');
const branch = 'rocket-ui-test/mcp-'+id;
const reference = 'refs/heads/'+branch;
const tracking = 'refs/remotes/origin/'+branch;
for (const dir of [client,publisher,hooks,path.join(root,'empty-home')]) fs.mkdirSync(dir,{recursive:true});
fs.writeFileSync(path.join(root,'empty.config'),'');
const env = {...process.env};
for (const key of Object.keys(env)) if (/^(GIT_|GCM_|ROCKET_|SSH_|CURL_|SSL_CERT_)|_PROXY$|^NETRC$/i.test(key)) delete env[key];
Object.assign(env,{HOME:path.join(root,'empty-home'),GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),
  GIT_CONFIG_NOSYSTEM:'1',GIT_TEMPLATE_DIR:hooks,GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GCM_INTERACTIVE:'0',LC_ALL:'C'});
function git(args,at=client) {
  const r=spawnSync('git.exe',['-C',at,'-c','gc.auto=0','-c','maintenance.auto=false',...args],{env,encoding:'utf8',windowsHide:true,timeout:15000,maxBuffer:1024*1024});
  if(r.status!==0) throw Error('Fixture Git failed: '+args[0]+' '+r.stderr); return r.stdout.trimEnd();
}
function hash(file) {return createHash('sha256').update(fs.readFileSync(file)).digest('hex');}
function local() { return JSON.stringify({head:git(['rev-parse','HEAD']),index:hash(path.join(client,'.git','index')),files:fs.readdirSync(client,{withFileTypes:true}).filter(f=>f.isFile()).map(f=>[f.name,hash(path.join(client,f.name))])}); }
function stagedState() {const value=JSON.parse(local());delete value.index;value.entries=git(['ls-files','--stage','-z']);return JSON.stringify(value);}
const owned = new Set();
function commit(at,file,text) { fs.writeFileSync(path.join(at,file),text+'\n');git(['add','--',file],at);git(['commit','-m','Rocket MCP synthetic fixture'],at);const oid=git(['rev-parse','HEAD'],at);owned.add(oid);return oid; }
function tip() {const text=git(['ls-remote','--refs','origin',reference]);return text ? text.split('\t')[0] : '';}
function publish(expected) {const oid=git(['rev-parse','HEAD'],publisher);assert(owned.has(oid)&&owned.has(expected));git(['push','--no-follow-tags',`--force-with-lease=${reference}:${expected}`,'origin',`HEAD:${reference}`],publisher);assert.equal(tip(),oid);}
function syncPublisher() {git(['fetch','--no-tags','origin',`${reference}:${tracking}`],publisher);git(['merge','--ff-only',tracking],publisher);}
for(const at of [client,publisher]) {
  git(['init','--initial-branch='+branch],at);
  for(const [k,v] of [['user.name','Rocket MCP Test'],['user.email','rocket@example.invalid'],['core.hooksPath',hooks],['core.autocrlf','false'],['commit.gpgsign','false'],['credential.helper','']]) git(['config',k,v],at);
  git(['remote','add','origin',remote],at);git(['config','remote.origin.fetch',`+${reference}:${tracking}`],at);
}
git(['init','--bare','--initial-branch=main',remote]);
fs.writeFileSync(path.join(client,'.git','rocket-ui-fixture'),branch);
const base=commit(client,'ROCKET_PRIVATE_file','base');
const report={passed:false,root,branch,started_utc:new Date().toISOString(),cases:[],cleanup:'not-needed'};
const mcp=new RocketMcpClient(path.join(workspace,'target','release','rocket-mcp.exe'),root,env);
let state; let lastActivities=0; let wroteRemote=false;
async function settled() {
  const until=Date.now()+60000;
  do {state=await mcp.tool('rocket_wait',{timeout_ms:10000}); if(!state.busy&&!state.input_pending) return state;} while(Date.now()<until);
  throw Error('Rocket workers did not settle');
}
async function click(target,expected_tab=state.active_tab) {await mcp.tool('rocket_click',{target,expected_tab});state=await mcp.tool('rocket_state');}
async function type(target,text) {await mcp.tool('rocket_type',{target,text,expected_tab:state.active_tab});state=await mcp.tool('rocket_state');}
async function capture(name) {const r=await mcp.tool('rocket_capture');const content=r.content.find(c=>c.type==='image');assert.equal(content.mimeType,'image/png');const bytes=Buffer.from(content.data,'base64');assert(bytes.subarray(0,8).equals(Buffer.from('89504e470d0a1a0a','hex')));fs.writeFileSync(path.join(root,name+'.png'),bytes);}
function passed(name) {report.cases.push(name); console.log('PASS '+name);}
async function outcome(operation,code='') {
  await settled(); assert.equal(state.activities.length,lastActivities+1);lastActivities=state.activities.length;
  const a=state.activities.at(-1);assert.equal(a.operation,operation);
  if(code) {assert.equal(a.command_succeeded,false);assert(a.details.join('\n').includes('['+code+']'));assert(a.details.join('\n').includes('Next: '));}
  else {assert.equal(a.command_succeeded,true);assert.deepEqual(a.details,[]);}
  assert.equal(state.dropped_events,0);return a;
}
async function operation(name,code='',upstream=false) {
  await click('toolbar-'+name);assert.equal(state.dialog.kind,name);
  if(name==='push') {await type('push-destination',branch);if(upstream)await click('push-upstream');assert.equal(state.dialog.second,branch);assert.equal(state.dialog.flag,upstream);}
  await click('dialog-submit');await outcome(name,code);
}
(async()=>{
  try {
    await mcp.initialize(); const list=await mcp.request('tools/list');assert.equal(list.tools.length,13);
    const cold=await mcp.tool('rocket_state');assert.equal(cold.window_count,0);
    await assert.rejects(mcp.tool('rocket_open',{path:workspace}),/outside/);
    await mcp.tool('rocket_open',{path:client});await settled();assert.equal(state.window_count,1);assert.equal(state.pid,mcp.child.pid);
    await assert.rejects(mcp.tool('rocket_click',{target:'toolbar-fetch',expected_tab:99999}),/changed/);
    await assert.rejects(mcp.tool('rocket_click',{target:'does-not-exist',expected_tab:state.active_tab}),/not present/);
    passed('MCP handshake, lazy window launch, fixture scope and stale-target rejection');
    await click('view-changes');
    await assert.rejects(mcp.tool('rocket_click',{target:'commit',expected_tab:state.active_tab}),/disabled/);
    fs.writeFileSync(path.join(client,'ROCKET_PRIVATE_file'),'edited through fixture\n');
    await click('refresh');await settled();
    await click('stage-all');await outcome('stage');
    await click('unstage-all');await outcome('unstage');
    await click('stage-all');await outcome('stage');
    await type('commit-message','Rocket MCP UI commit');await click('commit');await outcome('commit');
    assert.notEqual(git(['rev-parse','HEAD']),base);assert.equal(state.tabs[0].draft_present,false);
    owned.add(git(['rev-parse','HEAD']));passed('Actual controls stage, unstage and commit in the same window');
    await click('view-commits');await settled();
    await click('toolbar-push');await type('push-destination','main');
    await assert.rejects(mcp.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab}),/outside/);
    assert.equal(tip(),'');await mcp.tool('rocket_escape');state=await mcp.tool('rocket_state');assert.equal(state.dialog,null);
    passed('Push destination guard prevents writing another branch');
    wroteRemote=true;await operation('push','',true);assert.equal(tip(),git(['rev-parse','HEAD']));
    await operation('push');passed('New branch and up-to-date pushes through actual dialogs');
    fs.writeFileSync(path.join(client,'ROCKET_PRIVATE_untracked'),'preserve this untracked fixture\n');
    git(['fetch','--no-tags','origin',`${reference}:${tracking}`],publisher);git(['switch','--create',branch,'--no-track',tracking],publisher);
    const beforeAdvance=git(['rev-parse','HEAD']);const advance=commit(publisher,'ROCKET_PRIVATE_file','remote advance');publish(beforeAdvance);
    const before=local();await operation('fetch');assert.equal(local(),before);assert.equal(git(['rev-parse',tracking]),advance);
    await operation('pull');assert.equal(git(['rev-parse','HEAD']),advance);assert.equal(fs.readFileSync(path.join(client,'ROCKET_PRIVATE_file'),'utf8'),'remote advance\n');
    await capture('pull-confirmed');await operation('pull');passed('Fetch preserves local state and pull fast-forwards with updated bytes');
    const clientAdvance=commit(client,'ROCKET_PRIVATE_local','local advance');await click('refresh');await settled();await operation('push');assert.equal(tip(),clientAdvance);
    syncPublisher();const diverged=commit(publisher,'ROCKET_PRIVATE_remote','divergence');publish(clientAdvance);
    const localAhead=commit(client,'ROCKET_PRIVATE_local','local competing commit');await click('refresh');await settled();const divergedLocal=local();
    await operation('push','push_fetch_first');assert.equal(local(),divergedLocal);assert.equal(tip(),diverged);
    await operation('fetch');await operation('push','push_non_fast_forward');await operation('pull','pull_diverged');
    assert.equal(local(),divergedLocal);assert.equal(git(['rev-parse','HEAD']),localAhead);await capture('divergence-guidance');
    fs.writeFileSync(path.join(client,'ROCKET_PRIVATE_file'),'staged bytes\n');git(['add','--','ROCKET_PRIVATE_file']);fs.writeFileSync(path.join(client,'ROCKET_PRIVATE_file'),'unstaged bytes\n');
    await click('refresh');await settled();const dirty=local();await operation('pull','pull_dirty_worktree');assert.equal(local(),dirty);await capture('dirty-guidance');
    passed('Fetch-first, non-fast-forward, divergent and dirty pull errors preserve data');
    await click('view-changes');await type('commit-message','MCP cancellation draft');
    const entered=path.join(root,'hook-entered');const release=path.join(root,'hook-release');
    const quote=s=>"'"+s.replaceAll('\\','/').replaceAll("'","'\"'\"'")+"'";
    fs.writeFileSync(path.join(hooks,'pre-commit'),`#!/bin/sh\nprintf x > ${quote(entered)}\nwhile test ! -f ${quote(release)}; do sleep 0.1; done\n`);
    // Git commit can refresh its index TREE cache before the hook runs. Verify
    // staged paths/modes/object IDs plus HEAD and file bytes, not cache bytes.
    const beforeCancel=stagedState();const beforeIndex=hash(path.join(client,'.git','index'));await click('commit');
    const deadline=Date.now()+10000;while(!fs.existsSync(entered)&&Date.now()<deadline) await new Promise(r=>setTimeout(r,100));assert(fs.existsSync(entered));
    state=await mcp.tool('rocket_state');assert(state.busy);assert(state.controls.some(c=>c.id==='cancel-operation'&&c.enabled));
    await assert.rejects(mcp.tool('rocket_click',{target:'toolbar-push',expected_tab:state.active_tab}),/disabled|Wait/);
    await click('cancel-operation');await settled();lastActivities++;assert.equal(state.activities.length,lastActivities);assert.equal(state.activities.at(-1).command_succeeded,false);assert.equal(state.tabs[0].draft_present,true);assert.equal(stagedState(),beforeCancel);report.cancelled_commit_index_cache_refreshed=beforeIndex!==hash(path.join(client,'.git','index'));
    fs.unlinkSync(path.join(hooks,'pre-commit'));passed('UI cancellation stops a blocked commit, preserves draft and rejects reentry');
    const logs=[];let seq=0;
    for(;;) {const batch=await mcp.tool('rocket_logs',{after_seq:seq,limit:1000});logs.push(...batch.records);seq=batch.next_seq;if(!batch.more)break;}
    assert(logs.length>0);const serialized=JSON.stringify(logs);assert(!/ROCKET_PRIVATE_|rocket-ui-test|github_pat_|gh[pousr]_[A-Za-z0-9]{16,}|PRIVATE KEY/.test(serialized));
    assert(logs.every(r=>r.dropped_before===0));
    for(const start of logs.filter(r=>['request_start','step_start','git_start'].includes(r.event))) assert.equal(logs.filter(r=>r.event===start.event.replace('_start','_end')&&r.id===start.id&&r.session===start.session).length,1);
    assert(logs.some(r=>r.failure_code==='pull_dirty_worktree'));assert(logs.some(r=>r.failure_code==='push_non_fast_forward'));
    fs.writeFileSync(path.join(root,'mcp-diagnostics.json'),JSON.stringify(logs,null,2));
    assert.equal(state.pid,mcp.child.pid);assert.equal(state.window_count,1);assert(!JSON.stringify(state).includes('raw_output'));
    await capture('cancelled-commit');
    await mcp.tool('rocket_close');state=await mcp.tool('rocket_state');assert.equal(state.dialog.kind,'quit');
    await click('dialog-cancel');assert.equal(state.tabs[0].draft_present,true);
    await type('commit-message','');assert.equal(state.tabs[0].draft_present,false,'Empty MCP replacement must clear the draft');
    await type('commit-message','');assert.equal(state.tabs[0].draft_present,false,'Clearing an empty draft must remain empty');
    passed('Empty text replacement clears the actual editor after cancelling close');
    await mcp.tool('rocket_close');await mcp.finish();
    report.pid=mcp.child.pid;report.window_count=1;report.tool_calls=mcp.calls;report.operation_activities=lastActivities;report.diagnostic_records=logs.length;report.dropped_events=0;
    passed('MCP screenshots, paired private-data-free diagnostics and graceful window close');report.passed=true;
  } catch(error) {report.error=error.message;console.error(error.stack);}
  finally {
    try {await mcp.finish();} catch(e) {report.shutdown_error=e.message;mcp.terminateOwnedProcess();report.passed=false;}
    if(wroteRemote) {try {const current=tip();if(current){assert(owned.has(current),'Unowned remote tip; preserved');git(['push',`--force-with-lease=${reference}:${current}`,'origin',`:${reference}`],publisher);}assert.equal(tip(),'');assert.equal(git(['ls-remote','--refs','origin']),'');report.cleanup='confirmed-absent';}catch(e){report.cleanup='needs-verification';report.cleanup_error=e.message;report.passed=false;}}
    report.finished_utc=new Date().toISOString();fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));
    if(mcp.stderr)fs.writeFileSync(path.join(root,'mcp.stderr.log'),mcp.stderr);
    console.log('MCP native UI results: '+path.join(root,'result.json'));if(!report.passed)process.exitCode=1;
  }
})();
