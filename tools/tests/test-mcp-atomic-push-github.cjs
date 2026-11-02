// Live atomic-push acceptance. Only explicitly owned temporary branches on the
// user-approved repository; feature actions go through native localhost MCP.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const {create}=require('./mcp-github-fixture.cjs');
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const gitExe=cp.execFileSync('where.exe',['git'],{encoding:'utf8',windowsHide:true}).trim().split(/\r?\n/)[0];
const exe=path.resolve('target/release/rocket-mcp.exe'),version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const privateFile='PRIVATE_検索 🚀 é.txt';
const continuation=process.argv.includes('--continue-cancellations')?'cancellations':process.argv.includes('--continue-after-interruption')?'interruption':null;
const priorMode=continuation==='cancellations'?'https':'ssh';
const priorFile=continuation?fs.readFileSync('work/atomic-push-'+priorMode+'-last-result','utf8').trim():null;
const prior=priorFile?JSON.parse(fs.readFileSync(priorFile,'utf8')):null;
if(prior){assert.equal(prior.mode,priorMode);assert.equal(prior.stage,continuation==='cancellations'?'cancel-before-send':'stale-review');assert.equal(prior.cases.length,continuation==='cancellations'?4:1);if(continuation==='interruption')assert.match(prior.failure,/^New user input cancelled validation; no automated input was delivered$/);assert.equal(prior.cleanup,'both-owned-branches-confirmed-absent');assert(prior.original_refs_and_default_head_unchanged);assert.equal(prior.executable_sha256,hash(exe));}
const userFiles=['.gitconfig','.ssh/config','.ssh/known_hosts','.ssh/id_ed25519_github.pub'].map(p=>path.join(process.env.USERPROFILE,p));
const userSnapshot=()=>userFiles.map(p=>({path:p,sha256:fs.existsSync(p)?hash(p):null}));
async function run(mode){
 assert(['https','ssh'].includes(mode));
 const f=create(mode),root=f.root,profile=path.join(root,'atomic-http'),branch=f.branch,feature=branch+'-文';
 const names=[branch,feature],refs=names.map(n=>'refs/heads/'+n),relay=gitRelay(root,f.env,'tools/helpers/productivity/git-relay.rs'),{gate,marker}=relay;
 const liveMarker=path.join(f.client,'.git/rocket-atomic-push-fixture');
 const report={passed:false,mode,version,root,branches:names,cases:[],timings:[],executable_sha256:hash(exe)};
 if(prior){assert.equal(mode,priorMode);report.cases=prior.cases.slice();report.previous_partial={file:priorFile,completed_groups:prior.cases.length,failure:prior.failure,cleanup:prior.cleanup};}
 const savedUser=userSnapshot();let service,client,state,baseline,source,other,windowStopped=false;
 const pass=s=>{report.cases.push(s);console.error('PASS '+mode+' '+s)};
 const save=()=>fs.writeFileSync(path.join(root,'atomic-result.json'),JSON.stringify(report,null,2));
 fs.writeFileSync('work/atomic-push-'+mode+'-last-result',path.join(root,'atomic-result.json'));save();
 const unlink=p=>{if(fs.existsSync(p))fs.unlinkSync(p)};
 const control=id=>state.controls?.find(c=>c.id===id),atomic=()=>state.dialog?.atomic_push,latest=()=>state.activities.at(-1);
 const snap=()=>({...f.snapshot(),config:hash(path.join(f.client,'.git/config')),refs:f.git(['for-each-ref','--format=%(refname) %(objectname)'])});
 const same=()=>assert.deepEqual(snap(),baseline,'Client checkout, refs, index, config or files changed');
 function remote(){const s=f.remoteState();assert.deepEqual(s.refs.filter(r=>!refs.some(ref=>r.endsWith('\t'+ref))),f.r.baseline.refs,'Other remote refs changed; preserved');assert.equal(s.head,f.r.baseline.head,'Remote default HEAD changed; preserved');return refs.map(ref=>s.refs.find(r=>r.endsWith('\t'+ref))?.split('\t')[0]||'')}
 function writeRemote(next,expected){
  assert.equal(next.length,2);assert.equal(expected.length,2);assert.deepEqual(remote(),expected,'Guarded fixture writer saw a different tip');
  for(const oid of [...next,...expected])assert(!oid||f.r.owned.includes(oid),'Fixture write references an unowned commit');
  f.r.remote_write_attempted=true;f.save();
  const updates=next.map((oid,i)=>({oid,ref:refs[i],old:expected[i]})).filter((x,i)=>x.oid!==expected[i]);
  if(updates.length)f.git(['-c','remote.origin.mirror=false','-c','push.followTags=false','-c','push.autoSetupRemote=false','push','--atomic','--porcelain','--no-follow-tags','--no-prune','--no-recurse-submodules',...updates.map(x=>'--force-with-lease='+x.ref+':'+x.old),'--','origin',...updates.map(x=>x.oid+':'+x.ref)],f.publisher);
  assert.deepEqual(remote(),next);
 }
 const reset=()=>writeRemote([f.r.base,f.r.base],remote());
 // MCP turns client hooks off, so the relay holds the real push after Rocket's last checks.
 async function holdPush(){unlink(marker);fs.writeFileSync(gate,'before:push');await click('dialog-submit',false);await until(s=>s.write&&fs.existsSync(marker),'held push')}
 async function until(pred,label='state',limit=150000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(pred(state))return state;assert(performance.now()-start<limit,label+' timed out');await delay(65)}}
 async function settle(){await until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search?.running));}
 async function stable(){let last='',count=0;await until(s=>{const v=JSON.stringify(s.controls);count=v===last?count+1:0;last=v;return count>=2},'stable controls')}
 async function click(target,wait=true){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});if(wait)await settle()}
 async function type(target,text){await stable();await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle()}
 async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,'atomic-'+name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,'atomic-'+name+'.json'),JSON.stringify(state,null,2))}
 async function closeDialog(){if(state?.dialog)await click('dialog-cancel')}
 async function openReview(){await closeDialog();await click('menu-repository');await client.tool('rocket_scroll',{target:'repository-menu-scroll',direction:'up',pixels:2000,expected_tab:state.active_tab});await settle();await click('repository-atomic-push');for(const name of names){const i=atomic().rows.findIndex(r=>r.local===name);assert(i>=0);await click('dialog-atomic-branch-'+i)}assert(!atomic().preview&&!control('dialog-submit').enabled)}
 async function review(){await click('dialog-atomic-review');const p=atomic().preview;assert(p,state.dialog.error||atomic().notice);assert.equal(p.entries.length,2);assert.deepEqual(p.entries.map(e=>e.source),source);return p}
 async function submit(){f.r.remote_write_attempted=true;f.save();await click('dialog-submit');same()}
 async function closeWindow(){if(state?.dialog&&!state.write)await closeDialog();await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window))}windowStopped=true}
 async function asyncTips(){return new Promise((resolve,reject)=>cp.execFile(gitExe,['-C',f.publisher,'ls-remote','--refs','--','origin',...refs],{env:f.env,windowsHide:true,timeout:30000,maxBuffer:1024*1024,encoding:'utf8'},(error,out)=>{if(error)return reject(Error('Independent remote observation failed; raw output omitted'));const rows=out.trim().split(/\r?\n/).filter(Boolean);if(!rows.every(r=>/^[a-f0-9]{40,64}\trefs\/heads\//.test(r)))return reject(Error('Invalid independent ref response'));resolve(refs.map(ref=>rows.find(r=>r.endsWith('\t'+ref))?.split('\t')[0]||''))}))}
 async function cancelPush(after){
  await stable();const start=performance.now();await client.tool('rocket_click',{target:'dialog-atomic-cancel',expected_tab:state.active_tab});
  await until(s=>s.write?.cancelling||s.write?.force_push_phase==='checking_remote'||!s.write,'cancel acknowledgment');
  assert(state.write,'Cancellation completed before feedback could be inspected');
  assert(state.write.cancelling||state.write.force_push_phase==='checking_remote');
  report.timings.push({action:'cancel acknowledgment',after,ms:Math.round(performance.now()-start)});
  await until(s=>s.write?.force_push_phase==='checking_remote','independent remote observation');
  assert(!state.write.cancel_enabled);assert.match(state.write.progress,/checking/i);
  report.timings.push({action:'cancel to checking remote',after,ms:Math.round(performance.now()-start)});
  if(!after)await capture('cancel-before-checking');
  unlink(gate);await settle();report.timings.push({action:'cancel to completed remote observation',after,ms:Math.round(performance.now()-start)});
  assert(!latest().command_succeeded);assert.match(JSON.stringify(latest()),/cancel/i);assert.match(latest().summary,after?/matches/:/unchanged/);
  assert.deepEqual(remote(),after?source:[f.r.base,f.r.base]);same();await capture(after?'cancel-after':'cancel-before');await closeDialog();
 }
 try{
  console.error('START '+mode+' isolated fixture and authentication preflight');report.stage='setup';save();
  f.run('setup');
  assert.deepEqual(remote(),['','']);
  const main=f.commit(f.client,privateFile,'PRIVATE first source');
  f.git(['switch','--create',feature,f.r.base]);const second=f.commit(f.client,'PRIVATE_第二.txt','PRIVATE second source');f.git(['switch',branch]);source=[main,second];
  other=f.git(['commit-tree',f.git(['rev-parse',f.r.base+'^{tree}']),'-p',f.r.base,'-m','Rocket synthetic concurrent writer']);f.r.owned.push(other);f.save();f.git(['update-ref','refs/rocket-atomic-fixture/race',other]);
  // Copy synthetic objects to the separate writer before enabling the network
  // trials. Only this one local setup subprocess permits file transport.
  cp.execFileSync(gitExe,['-C',f.publisher,'-c','protocol.file.allow=always','fetch','--no-tags','--',f.client,...refs.map((ref,i)=>ref+':refs/rocket-atomic-fixture/source-'+i),'refs/rocket-atomic-fixture/race:refs/rocket-atomic-fixture/race'],{env:{...f.env,GIT_ALLOW_PROTOCOL:'file'},windowsHide:true,stdio:'pipe',timeout:30000});
  fs.writeFileSync(liveMarker,'ROCKET-ATOMIC-PUSH-1\n'+branch+'\n'+f.remote+'\n');
  fs.writeFileSync(path.join(f.client,privateFile),'PRIVATE staged\n');f.git(['add','--',privateFile]);fs.writeFileSync(path.join(f.client,privateFile),'PRIVATE unstaged\n');fs.writeFileSync(path.join(f.client,'PRIVATE_未跟踪.txt'),'PRIVATE untracked\n');baseline=snap();
  report.owned_commits=f.r.owned;report.source=source;report.base=f.r.base;report.concurrent=other;save();
  cp.execFileSync(exe,['--init-http',profile,root],{env:f.env,windowsHide:true});
  const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
  
  const fd=fs.openSync(path.join(root,'atomic-service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
  for(let n=0;;n++){try{if((await client.raw('GET','/health')).status===200)break}catch{}assert(n<100);await delay(100)}
  assert.equal((await client.initialize()).serverInfo.version,version);await client.tool('rocket_open',{path:f.client});await settle();relay.check(state);await client.tool('rocket_resize',{width:1480,height:920});await settle();if(control('console-close'))await click('console-close');
  if(prior){writeRemote([f.r.base,f.r.base],['','']);}else{
  report.stage='policy-and-success';save();await openReview();
  const text=fs.readFileSync(liveMarker);unlink(liveMarker);await stable();await assert.rejects(()=>client.tool('rocket_click',{target:'dialog-atomic-review',expected_tab:state.active_tab}),/approved marker/);fs.writeFileSync(liveMarker,text);await settle();
  let p=await review();assert(p.entries.every(e=>e.kind==='create'));unlink(liveMarker);await stable();await assert.rejects(()=>client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab}),/approved marker/);fs.writeFileSync(liveMarker,text);await settle();
  await capture('create-preview');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.deepEqual(remote(),source);await capture('create-success');
  reset();p=await review();assert(p.entries.every(e=>e.kind==='fast_forward'));await submit();assert(latest().command_succeeded);assert.deepEqual(remote(),source);
  p=await review();assert(p.entries.every(e=>e.kind==='up_to_date'));await submit();assert(latest().command_succeeded);assert.deepEqual(remote(),source);await closeDialog();
  pass('Exact fixture/URL opt-in is required; GitHub creates, fast-forwards and confirms both selected branches without altering local edits or other refs');reset();
  }
  if(continuation!=='cancellations'){
  report.stage='stale-review';save();await openReview();await review();writeRemote([f.r.base,other],[f.r.base,f.r.base]);await submit();assert(!latest().command_succeeded);assert.match(state.dialog.error,/atomic_push_remote_changed/);assert(!atomic().preview);assert.deepEqual(remote(),[f.r.base,other]);await capture('stale-review');
  p=await review();assert(p.entries.some(e=>e.kind==='non_fast_forward'));assert(!control('dialog-submit').enabled);await capture('divergent-review');same();await closeDialog();pass('Remote changes after review invalidate the ticket; divergent history blocks the whole batch and preserves the other writer');reset();
  report.stage='server-rejection';save();await openReview();await review();f.r.remote_write_attempted=true;f.save();await holdPush();
  writeRemote([f.r.base,other],[f.r.base,f.r.base]);unlink(gate);await settle();assert(!latest().command_succeeded);assert.match(state.dialog.error,/push_remote_rejected|push_stale_lease/);assert.match(latest().summary,/differs/i);assert(!state.dialog.error.includes('PRIVATE'));assert.deepEqual(remote(),[f.r.base,other]);same();await capture('github-atomic-rejection');await closeDialog();
  pass('A concurrent update while the push is held is rejected and atomically leaves the other branch unchanged; observed differing tips and rejection remain clear');reset();
  report.stage='review-cancellation';save();await openReview();unlink(marker);fs.writeFileSync(gate,'before:--server-option=');await click('dialog-atomic-review',false);await until(()=>atomic()?.loading&&fs.existsSync(marker)&&!state.input_pending,'live review gate');await click('dialog-atomic-cancel',false);unlink(gate);await settle();assert(!atomic().preview&&!control('dialog-submit').enabled);assert.match(atomic().notice,/cancel/i);same();assert.deepEqual(remote(),[f.r.base,f.r.base]);await capture('review-cancelled');await closeDialog();
  pass('Cancelling an actual HTTPS/SSH review discards its preview and leaves both remote branches unchanged');
  }
  report.stage='cancel-before-send';save();await openReview();await review();await holdPush();unlink(marker);await cancelPush(false);
  pass('Cancel before transmission acknowledges stopping, kills the Git process tree and independently verifies that neither branch changed');
  report.stage='cancel-after-acceptance';save();await openReview();await review();unlink(marker);fs.writeFileSync(gate,'after:push');await click('dialog-submit',false);await until(s=>s.write&&fs.existsSync(marker),'real push response gate');
  const observedAt=performance.now();for(;;){const actual=await asyncTips();if(JSON.stringify(actual)===JSON.stringify(source))break;assert.deepEqual(actual,[f.r.base,f.r.base]);assert(performance.now()-observedAt<60000,'Synthetic GitHub push was not accepted');await delay(150)}
  report.both_updates_observed_before_cancel=true;await cancelPush(true);
  pass('Cancel after independently confirmed GitHub acceptance reports both tips already published without claiming rollback or retrying');
  report.stage='logs';save();await click('menu-view');await click('activity-open-menu');await until(s=>s.activity_history.open&&!s.activity_history.loading&&s.activity_history.records>0);await type('activity-search','atomic-push');await until(s=>!s.activity_history.loading&&s.activity_history.rows.some(r=>prior||r.status==='Succeeded'));const row=prior?state.activity_history.rows[0]:state.activity_history.rows.find(r=>r.status==='Succeeded');await click('activity-row-'+row.index);await capture('activity');await click('activity-close');
  await closeWindow();await client.raw('POST','/shutdown');await delay(150);
  const readLogs=at=>fs.readdirSync(at).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(at,n)).filter(p=>/^rocket(?:\.\d+)?\.jsonl$/.test(p)).flatMap(p=>fs.readFileSync(path.join(at,n,p),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
  const currentRecords=readLogs(root),records=prior?[...readLogs(prior.root),...currentRecords]:currentRecords;report.current_log_records=currentRecords.length;
  assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const value of ['PRIVATE',privateFile,branch,feature,f.remote,'rocket@example.invalid',...f.r.owned,...(prior?.owned_commits||[]),...(prior?.branches||[]),'Authorization:','credential-manager'])assert(!payload.includes(value),'Diagnostic payload leaked');
  const spans=new Map();for(const r of records.sort((a,b)=>a.session.localeCompare(b.session)||a.seq-b.seq)){if(!/^(request|step|git)_(start|end)$/.test(r.event))continue;const key=r.session+':'+r.event.split('_')[0]+':'+r.id;if(r.event.endsWith('_start')){assert(!spans.has(key));spans.set(key,r.operation)}else{assert.equal(spans.get(key),r.operation);spans.delete(key)}}assert.equal(spans.size,0);
  const results=records.filter(r=>r.event==='atomic_push_result');for(const o of ['published','unchanged','different'])assert(results.some(r=>r.remote_outcome===o),'Missing outcome '+o);assert(results.every(r=>r.branches===2));assert(records.some(r=>r.failure_code==='atomic_push_remote_changed'));assert(records.some(r=>['push_remote_rejected','push_stale_lease'].includes(r.failure_code)));
  report.timings.push(...records.filter(r=>r.event==='git_end'&&r.outcome==='cancelled').map(r=>({action:'Git cancellation to stop',microseconds:r.cancel_to_stop_us})));assert(report.timings.some(t=>t.microseconds>0));report.log_records=records.length;report.failure_codes=[...new Set(records.map(r=>r.failure_code).filter(Boolean))];report.dropped_events=0;
  pass('Native Activity and paired diagnostics show the typed failures, per-batch outcomes and cancellation timing with zero drops or tested private payloads');report.passed=true;
 }catch(e){report.failure=e.message;try{if(client)await capture('failure')}catch{}throw e}
 finally{
  // Never release a blocked push then race cleanup. Stop the owned UI first.
  if(client&&service?.exitCode===null&&!windowStopped){try{state=await client.tool('rocket_state');if(state.write||atomic()?.loading||atomic()?.applying)await click('dialog-atomic-cancel',false);unlink(gate);await settle();await closeWindow();await client.raw('POST','/shutdown')}catch(e){report.shutdown_error=e.message}await delay(200);if(service.exitCode===null)service.kill()}
  if(!service||windowStopped)unlink(gate);
  if(client)await client.finish();
  if(f.r.baseline&&f.r.remote_write_attempted){try{assert(!service||windowStopped,'Cleanup deferred: owned UI not confirmed stopped');f.assertScope();const tips=remote();writeRemote(['',''],tips);assert.deepEqual(remote(),['','']);report.cleanup='both-owned-branches-confirmed-absent';report.original_refs_and_default_head_unchanged=true}catch(e){report.cleanup_error=e.message;report.passed=false}}else report.cleanup='no-remote-write';
  try{assert.deepEqual(userSnapshot(),savedUser);report.existing_key_and_config_files_unchanged=true}catch(e){report.user_file_check_error=e.message;report.passed=false}
  save();console.log(JSON.stringify(report,null,2));
 }
 assert(report.passed);return report;
}
(async()=>{const modes=process.argv.slice(2).filter(s=>!s.startsWith('--'));assert(modes.length);if(continuation)assert.deepEqual(modes,[priorMode]);for(const mode of modes)await run(mode)})().catch(e=>{console.error(e.message);process.exitCode=1});
