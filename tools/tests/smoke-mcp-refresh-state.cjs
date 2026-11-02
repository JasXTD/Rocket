// Local-only native acceptance: actual widgets, native notifications and persisted state.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {spawnSync}=require('node:child_process');
const {RocketMcpClient}=require('../mcp/mcp-client.cjs');
const {readDraftPayload}=require('./state-fixture.cjs');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function main(){
 const workspace=path.resolve(__dirname, '../..'), name=`refresh-${crypto.randomUUID().replaceAll('-','')}`;
 const root=path.join(workspace,'work','mcp-fixtures',name),repo=path.join(root,'client'),branch=`rocket-ui-test/${name}`;
 fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'hooks'));fs.writeFileSync(path.join(root,'empty.config'),'');
 const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)/i.test(k))delete env[k];
 Object.assign(env,{GIT_CONFIG_NOSYSTEM:'1',GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'hooks')});
 const git=(...args)=>{const r=spawnSync('C:\\Program Files\\Git\\cmd\\git.exe',args,{cwd:repo,env,windowsHide:true,encoding:'utf8'});assert.equal(r.status,0,r.stderr);return r.stdout.trim();};
 git('init',`--initial-branch=${branch}`,'.');
 for(const[k,v]of [['user.name','Rocket State Test'],['user.email','test@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',path.join(root,'hooks')]])git('config',k,v);
 fs.writeFileSync(path.join(repo,'.git','rocket-ui-fixture'),branch);
 fs.writeFileSync(path.join(repo,'edit.rs'),'base\n');git('add','--all','--');git('commit','-m','Synthetic base');
 const originalHead=git('rev-parse','HEAD'),originalIndex=fs.readFileSync(path.join(repo,'.git','index'));
 const executable=path.join(workspace,'target','release','rocket-mcp.exe');
 const report={passed:false,root,cases:[],sessions:[],executable_sha256:crypto.createHash('sha256').update(fs.readFileSync(executable)).digest('hex')};
 let client,state;
 async function open(){client=new RocketMcpClient(executable,path.join(workspace,'work','mcp-fixtures'),env);await client.initialize();await client.tool('rocket_open',{path:repo});await until(s=>!s.busy);report.sessions.push({pid:state.pid});}
 async function until(predicate){const end=Date.now()+40000;do{state=await client.tool('rocket_state');assert.equal(state.startup_error??null,null);assert.equal(state.state_error??null,null);assert.equal(state.tabs?.[0]?.error??null,null);if(state.window==='running'&&state.tabs?.length&&predicate(state))return state;await pause(75);}while(Date.now()<end);throw Error(`MCP state deadline: ${JSON.stringify(state)}`);}
 const click=target=>client.tool('rocket_click',{target,expected_tab:state.active_tab});
 const type=text=>client.tool('rocket_type',{target:'commit-message',text,expected_tab:state.active_tab});
 async function logs(label){
  const out=await client.tool('rocket_logs',{limit:1000});assert(!out.more&&!out.retention_gap);assert(out.records.every(r=>!r.dropped_before));
  assert(!/PRIVATE_DRAFT|edit\.rs|rocket-ui-test|test@example/i.test(JSON.stringify(out.records)));
  if(label!=='session3')for(const kind of ['request','step','git']){
   const starts=out.records.filter(r=>r.event===`${kind}_start`),ends=out.records.filter(r=>r.event===`${kind}_end`);assert.equal(starts.length,ends.length);
   for(const start of starts)assert.equal(ends.filter(e=>e.id===start.id&&e.request===start.request).length,1);
  }
  if(label==='session1')for(const op of ['load_state','save_state','watch','auto_refresh'])assert(out.records.some(r=>r.operation===op&&r.event==='request_end'&&r.outcome==='ok'),`missing ${op} diagnostic`);
  if(label.startsWith('session'))report.sessions.at(-1).diagnostic_records=out.records.length;
  fs.writeFileSync(path.join(root,`${label}-logs.json`),JSON.stringify(out,null,2));return out;
 }
 async function close(){await client.tool('rocket_close');await until(s=>s.dialog?.kind==='quit');await click('dialog-submit');await client.finish();report.sessions.at(-1).calls=client.calls;}
 try{
  await open();assert(state.tabs[0].watching);
  const idleBefore=await logs('idle-before');await pause(2500);const idleAfter=await logs('idle-after');
  assert.equal(idleAfter.records.filter(r=>r.event==='git_start').length,idleBefore.records.filter(r=>r.event==='git_start').length);
  report.cases.push('idle: zero repeated Git commands');
  await click('view-changes');await until(s=>!s.busy);
  await type('PRIVATE_DRAFT first 文\n\nDetails');await until(s=>!s.busy);assert(state.tabs[0].draft_present);
  const baseCount=state.tabs[0].refresh_count,start=performance.now();
  for(let i=0;i<40;i++)fs.writeFileSync(path.join(repo,'edit.rs'),`burst ${i}\n`);
  await until(s=>s.tabs[0].background_refresh);
  const t=performance.now();await type('PRIVATE_DRAFT newest while refreshing 文');report.typing_during_refresh_ms=performance.now()-t;
  assert(report.typing_during_refresh_ms<1500,'typing blocked on repository refresh');
  await until(s=>!s.busy&&s.tabs[0].change_count===1);report.external_refresh_ms=performance.now()-start;
  assert(state.tabs[0].refresh_count-baseCount<=2,'save burst was not coalesced');
  report.cases.push('burst refresh, draft editing during refresh');
  await click(`change-unstaged-${Buffer.from('edit.rs').toString('hex')}`);await until(s=>!s.busy);assert.equal(state.tabs[0].patch_added,1);
  fs.writeFileSync(path.join(repo,'edit.rs'),'second\nthird\n');await until(s=>!s.busy&&s.tabs[0].patch_added===2);
  const capture=await client.tool('rocket_capture');fs.writeFileSync(path.join(root,'refresh.png'),Buffer.from(capture.content.find(c=>c.type==='image').data,'base64'));
  report.cases.push('selected diff updates for an already-modified file');
  assert.equal(git('rev-parse','HEAD'),originalHead);assert.deepEqual(fs.readFileSync(path.join(repo,'.git','index')),originalIndex);
  git('add','--','edit.rs');
  await until(s=>!s.busy&&s.controls.some(c=>c.id===`change-staged-${Buffer.from('edit.rs').toString('hex')}`));
  git('commit','-m','Synthetic external commit');const externalHead=git('rev-parse','HEAD');
  await until(s=>!s.busy&&s.tabs[0].head.oid===externalHead&&s.tabs[0].change_count===0);assert(state.tabs[0].draft_present);
  report.cases.push('external staging and commit reflected without losing the draft');
  await click('menu-edit');await click('preferences');await click('pref-page-appearance');await click('rows-compact');await click('pref-page-general');await click('pref-details');await click('pref-auto-refresh');
  const preferencesCapture=await client.tool('rocket_capture');fs.writeFileSync(path.join(root,'preferences.png'),Buffer.from(preferencesCapture.content.find(c=>c.type==='image').data,'base64'));
  await click('dialog-submit');await until(s=>!s.busy);
  assert.equal(state.preferences.auto_refresh,false);assert.equal(state.preferences.details,false);assert.equal(state.preferences.row_height,29);
  const stopped=state.tabs[0].refresh_count;fs.writeFileSync(path.join(repo,'edit.rs'),'paused\n');await pause(1400);state=await client.tool('rocket_state');assert.equal(state.tabs[0].refresh_count,stopped);
  await logs('session1');await close();
  await open();assert.equal(state.preferences.auto_refresh,false);assert.equal(state.preferences.details,false);assert.equal(state.preferences.row_height,29);assert(state.tabs[0].draft_present);
  assert(readDraftPayload(path.join(root,'rocket-ui-state')).includes(Buffer.from('PRIVATE_DRAFT newest while refreshing 文')));
  report.cases.push('draft and preferences restored after native restart; paused refresh stays paused');
  await click('menu-edit');await click('preferences');await click('pref-page-general');await click('pref-auto-refresh');await click('dialog-submit');await until(s=>!s.busy&&s.tabs[0].watching);
  await click('view-changes');await type('PRIVATE_DRAFT crash recovery');await until(s=>!s.busy);
  await logs('session2');report.sessions.at(-1).calls=client.calls;client.terminateOwnedProcess();await pause(800);
  await open();assert(state.tabs[0].draft_present);assert(state.preferences.auto_refresh);
  assert(readDraftPayload(path.join(root,'rocket-ui-state')).includes(Buffer.from('PRIVATE_DRAFT crash recovery')));
  report.cases.push('last completed autosave restored after forced termination of owned MCP process');
  await click('view-changes');await type('');await client.tool('rocket_type',{target:'commit-description',text:'',expected_tab:state.active_tab});await until(s=>!s.busy);assert.equal(state.tabs[0].draft_present,false);
  await type('PRIVATE_DRAFT flush on immediate exit'); // Do not wait for the save debounce.
  await logs('session3');await close();
  await open();assert(state.tabs[0].draft_present);assert(readDraftPayload(path.join(root,'rocket-ui-state')).includes(Buffer.from('PRIVATE_DRAFT flush on immediate exit')));
  report.cases.push('exit flushes the latest pending draft');
  await logs('session4');await close();report.passed=true;
 }catch(e){report.error=e.message;if(client){try{await client.finish();}catch{client.terminateOwnedProcess();}}throw e;
 }finally{fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));}
}
main().catch(e=>{console.error(e.message);process.exitCode=1;});
