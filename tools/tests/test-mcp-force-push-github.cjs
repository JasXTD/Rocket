// Live force-with-lease acceptance: only the exact approved GitHub repository,
// one random owned branch per transport, and synthetic history. Feature actions
// use native UI events over authenticated localhost MCP, never backend shortcuts.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const {create,Fixture}=require('./mcp-github-fixture.cjs');
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const cancellationOnly=process.argv.includes('--cancellation-only');
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const gitExe='C:\\Program Files\\Git\\cmd\\git.exe';
const FILE='PRIVATE_検索 🚀 é.txt';
async function run(mode){
 assert(['https','ssh'].includes(mode),'Explicit https or ssh mode required');
 const f=create(mode),root=f.root,profile=path.join(root,'force-http'),branch=f.branch,ref=f.ref;
 const report={passed:false,mode,version,root,branch,cancellation_only:cancellationOnly,cases:[],owned_commits:[]};
 let service,client,state,old,source,concurrent,baseline,windowStopped=false;
 const relay=gitRelay(root,f.env,'tools/helpers/productivity/git-relay.rs'),{gate,marker}=relay;
 const liveMarker=path.join(f.client,'.git/rocket-force-push-fixture');
 const pass=s=>{report.cases.push(s);console.error('PASS '+mode+' '+s)};
 const control=id=>state.controls?.find(c=>c.id===id),lease=()=>state.dialog?.force_push,latest=()=>state.activities.at(-1);
 const snapshot=()=>({...f.snapshot(),config:hash(path.join(f.client,'.git/config')),refs:f.git(['for-each-ref','--format=%(refname) %(objectname)'])});
 const same=()=>assert.deepEqual(snapshot(),baseline,'Client HEAD, refs, index, config or file bytes changed');
 const tip=()=>f.tip(f.remoteState());
 const resetRemote=expected=>{f.assertRemote(expected);f.leasedWrite(old,expected);f.r.expected_tip=old;f.save();f.assertRemote(old)};
 const unlink=p=>{if(fs.existsSync(p))fs.unlinkSync(p)};
 async function until(pred,label='state',timeout=150000){const started=performance.now();for(;;){state=await client.tool('rocket_state');if(pred(state))return;assert(performance.now()-started<timeout,label+' timed out');await delay(100)}}
 async function settle(){await until(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading||t.search?.running));}
 async function stable(){let last='',count=0;await until(s=>{const v=JSON.stringify(s.controls);count=v===last?count+1:0;last=v;return count>=2},'stable controls')}
 async function click(target,wait=true){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});if(wait)await settle()}
 async function type(target,text){await stable();await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle()}
 async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,'force-'+name+'.png'),Buffer.from(r.content.find(x=>x.type==='image').data,'base64'));fs.writeFileSync(path.join(root,'force-'+name+'.json'),JSON.stringify(state,null,2))}
 async function closeDialog(){for(let n=0;state?.dialog&&n<4;n++)await click('dialog-cancel');assert(!state?.dialog)}
 async function openReview(){await click('toolbar-push');await click('push-lease-open');await stable();assert(!lease().preview&&!lease().running);assert(!control('dialog-submit').enabled)}
 async function review(){await openReview();await click('push-lease-review');assert(lease().preview,state.dialog.error||lease().notice);assert.equal(lease().preview.source,source);assert.equal(lease().preview.expected,old);assert.equal(lease().preview.removed_count,1);assert.equal(lease().preview.added_count,1)}
 async function confirm(){await type('push-lease-confirm',branch);await stable();assert(control('dialog-submit').enabled)}
 // MCP turns client hooks off, so the relay holds the real push after Rocket's last checks.
 async function holdPush(){unlink(marker);fs.writeFileSync(gate,'before:push');await click('dialog-submit',false);await until(s=>s.write&&fs.existsSync(marker),'held push')}
 async function closeWindow(){if(state?.dialog&&!state.write)await closeDialog();await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window))}}
 // Async independent observer: waiting for the real transport must not freeze
 // MCP polling, cancellation, or let a delayed hook continue without supervision.
 async function asyncTip(){return new Promise((resolve,reject)=>{
  cp.execFile(gitExe,['-C',f.publisher,'ls-remote','--refs','--','origin',ref],{env:f.env,windowsHide:true,timeout:30000,maxBuffer:1024*1024,encoding:'utf8'},(error,out)=>{
   if(error)return reject(Error('Independent remote observation failed; raw output omitted'));
   const rows=out.trim().split(/\r?\n/).filter(Boolean);
   if(rows.length!==1||!rows[0].endsWith('\t'+ref))return reject(Error('Unexpected remote observation'));
   resolve(rows[0].split('\t')[0]);
  });
 })}
 try{
  console.error('START '+mode+' isolated fixture setup');
  f.run('setup');
  fs.writeFileSync(liveMarker,'ROCKET-FORCE-PUSH-1\n'+branch+'\n'+f.remote+'\n');
  // Expected absence protects an existing branch even in the unlikely UUID collision.
  f.r.remote_write_attempted=true;f.save();
  f.git(['push','--porcelain','--no-follow-tags','--no-prune','--no-recurse-submodules','--force-with-lease='+ref+':','--','origin',f.r.base+':'+ref]);
  f.r.expected_tip=f.r.base;f.save();f.assertRemote(f.r.base);
  f.syncPublisher(true);old=f.commit(f.publisher,FILE,'PRIVATE original remote');
  f.leasedWrite(old,f.r.base);f.r.expected_tip=old;f.save();
  f.git(['fetch','--no-tags','--no-prune','--no-recurse-submodules','--','origin',ref+':'+f.tracking]);
  f.config(f.client,'branch.'+branch+'.remote','origin');f.config(f.client,'branch.'+branch+'.merge',ref);
  source=f.commit(f.client,FILE,'PRIVATE replacement source');
  concurrent=f.commit(f.publisher,'PRIVATE_concurrent.txt','PRIVATE other writer');
  report.owned_commits=[...f.r.owned];report.expected_tip=old;report.source=source;report.concurrent=concurrent;
  fs.writeFileSync(path.join(f.client,FILE),'PRIVATE staged\n');f.git(['add','--',FILE]);fs.writeFileSync(path.join(f.client,FILE),'PRIVATE unstaged\n');fs.writeFileSync(path.join(f.client,'未跟踪.txt'),'PRIVATE untracked sentinel\n');
  baseline=snapshot();f.assertRemote(old);
  const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
  cp.execFileSync(exe,['--init-http',profile,root],{env:f.env,windowsHide:true});
  const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
  const fd=fs.openSync(path.join(root,'force-service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);
  client=new RocketHttpClient(profile);for(let n=0;;n++){try{if((await client.raw('GET','/health')).status===200)break}catch{}assert(n<100);await delay(100)}
  assert.equal((await client.initialize()).serverInfo.version,version);await client.tool('rocket_open',{path:f.client});await settle();relay.check(state);await client.tool('rocket_resize',{width:1480,height:920});await settle();if(control('console-close'))await click('console-close');
  if(!cancellationOnly){
  await openReview();
  const markerText=fs.readFileSync(liveMarker);fs.unlinkSync(liveMarker);
  await stable();await assert.rejects(()=>client.tool('rocket_click',{target:'push-lease-review',expected_tab:state.active_tab}),/approved marker/);
  assert(!lease().preview&&!lease().running);fs.writeFileSync(liveMarker,markerText);
  await click('push-lease-review');assert.equal(lease().preview.expected,old);assert.equal(lease().preview.source,source);
  await type('push-lease-confirm','main');await stable();assert(!control('dialog-submit').enabled);await confirm();
  fs.unlinkSync(liveMarker);await stable();await assert.rejects(()=>client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab}),/approved marker/);fs.writeFileSync(liveMarker,markerText);
  await capture('preview');same();f.assertRemote(old);await closeDialog();
  pass('Live review/apply require the exact branch/URL marker; wrong confirmation is disabled and review preserves local and remote state');
  await review();await confirm();await click('dialog-submit');
  assert(latest().command_succeeded,JSON.stringify(latest()));assert(!lease().preview);same();f.assertRemote(source);await capture('success');await closeDialog();
  pass('Reviewed HTTPS/SSH force-with-lease replaces only the owned remote branch and independently verifies its exact tip');
  resetRemote(source);
  await review();f.leasedWrite(concurrent,old);await confirm();await click('dialog-submit');
  assert(!latest().command_succeeded);assert.match(state.dialog.error,/force_push_remote_changed/);assert(!lease().preview);same();f.assertRemote(concurrent);await capture('stale-review');await closeDialog();
  pass('Concurrent remote advance after review invalidates the preview before push and preserves the other writer commit');
  resetRemote(concurrent);
  await review();await confirm();await holdPush();
  f.leasedWrite(concurrent,old);f.assertRemote(concurrent);unlink(gate);await settle();
  assert(!latest().command_succeeded);assert.match(state.dialog.error,/push_remote_rejected|push_stale_lease/);assert.match(latest().summary,/different/i);assert(JSON.stringify(latest()).includes(concurrent));assert(!state.dialog.error.includes('PRIVATE_'));same();f.assertRemote(concurrent);await capture('server-race-rejection');await closeDialog();
  pass('A concurrent update while the push is held is rejected; distinct rejection and observed remote tip remain visible with no automatic retry');
  resetRemote(concurrent);
  }
  // The URL-only --get-url probe also uses ls-remote. Gate only the actual
  // protocol-v2 advertisement, after the synchronous input guard has finished.
  report.stage='review-cancellation';
  await openReview();unlink(marker);fs.writeFileSync(gate,'before:--server-option=');await click('push-lease-review',false);
  await until(()=>lease()?.loading&&fs.existsSync(marker)&&!state.input_pending,'live review response gate');
  let started=performance.now();await click('push-lease-cancel');report.review_cancel_ms=Math.round(performance.now()-started);assert(!lease().preview&&!lease().running);assert.match(lease().notice,/cancel/i);unlink(gate);same();f.assertRemote(old);await capture('review-cancelled');await closeDialog();
  pass('Cancellation during a real remote review discards the preview and preserves local and remote state');
  await review();await confirm();await holdPush();
  started=performance.now();await click('push-lease-cancel');report.before_cancel_ms=Math.round(performance.now()-started);assert(report.before_cancel_ms<25000);assert(!latest().command_succeeded);assert.match(JSON.stringify(latest()),/cancel/i);assert.match(latest().summary,/still matches/i);same();f.assertRemote(old);await capture('cancel-before');await closeDialog();unlink(gate);
  pass('Cancel before sending the update terminates the push and freshly observes the unchanged remote tip');
  await review();await confirm();unlink(marker);fs.writeFileSync(gate,'after:push');await click('dialog-submit',false);await until(s=>s.write&&fs.existsSync(marker),'real push response gate');
  const observedAt=performance.now();for(;;){const actual=await asyncTip();if(actual===source)break;assert.equal(actual,old);assert(performance.now()-observedAt<60000,'GitHub did not accept synthetic replacement');await delay(200)}
  report.update_observed_before_cancel=true;assert(state.write);started=performance.now();await click('push-lease-cancel');report.after_cancel_ms=Math.round(performance.now()-started);unlink(gate);assert(report.after_cancel_ms<25000);assert(!latest().command_succeeded);assert.match(JSON.stringify(latest()),/cancel/i);assert.match(latest().summary,/already matches/i);assert(!lease().preview);same();f.assertRemote(source);await capture('cancel-after');await closeDialog();
  pass('Cancel after independently confirmed GitHub acceptance reports already-applied remote state without claiming rollback or retrying');
  await closeWindow();windowStopped=true;await client.raw('POST','/shutdown');await delay(150);
  const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(p=>/^rocket(?:\.\d+)?\.jsonl$/.test(p)).flatMap(p=>fs.readFileSync(path.join(root,n,p),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
  assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);
  for(const value of['PRIVATE_',FILE,branch,f.remote,'rocket@example.invalid',...f.r.owned,'Authorization:','credential-manager'])assert(!payload.includes(value),'Diagnostic payload leaked');
  for(const operation of['force-push-policy','force-push-preview','force-push']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===operation);assert(starts.length);for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired request')}
  for(const begin of records.filter(r=>r.event==='step_start'&&['force-push-preview','force-push'].includes(r.operation)))assert(records.some(r=>r.event==='step_end'&&r.session===begin.session&&r.request===begin.request&&r.id===begin.id),'Unpaired step');
  for(const step of['review_force_push','inspect_force_push_remote','compare_force_push_history','revalidate_force_push','push_with_explicit_lease','observe_force_push_remote','verify_force_push_remote'])assert(records.some(r=>r.event==='step_end'&&r.step===step),'Missing step '+step);
  for(const outcome of(cancellationOnly?['replaced','unchanged']:['replaced','unchanged','different']))assert(records.some(r=>r.event==='force_push_result'&&r.remote_outcome===outcome),'Missing observation '+outcome);
  if(!cancellationOnly){assert(records.some(r=>r.failure_code==='force_push_remote_changed'));assert(records.some(r=>['push_remote_rejected','push_stale_lease'].includes(r.failure_code)));}assert(records.some(r=>r.outcome==='cancelled'));
  report.log_records=records.length;report.failure_codes=[...new Set(records.map(r=>r.failure_code).filter(Boolean))];report.dropped_events=0;pass('Paired diagnostic phases include the exercised typed outcomes, zero drops and no tested paths, URLs, commit IDs or credential payloads');
  report.passed=true;
 }catch(e){report.failure=e.message;try{if(client)await capture('failure')}catch{}throw e}
 finally{
  // Stop the owned UI/worker before permitting a fixture gate to continue.
  if(client&&service?.exitCode===null){try{state=await client.tool('rocket_state');if(state.write||lease()?.running)await click('push-lease-cancel');await closeWindow();windowStopped=true;await client.raw('POST','/shutdown')}catch{}await delay(250);if(service.exitCode===null)service.kill()}
  unlink(gate);
  if(client)await client.finish();
  if(f.r.baseline&&f.r.remote_write_attempted){try{assert(!service||windowStopped,'Cleanup deferred: could not confirm the owned UI worker stopped');f.run('cleanup');report.cleanup=f.r.cleanup;report.original_refs_unchanged=f.r.original_refs_and_default_head_unchanged}catch(e){report.cleanup_error=e.message;report.passed=false}}else report.cleanup='no-remote-write';
  fs.writeFileSync(path.join(root,'force-result.json'),JSON.stringify(report,null,2));fs.writeFileSync('work/force-push-'+mode+'-last-result',path.join(root,'force-result.json'));
  console.log(JSON.stringify(report,null,2));
 }
 assert(report.passed);return report;
}
(async()=>{const modes=process.argv.slice(2).filter(x=>!x.startsWith('--'));assert(modes.length,'Pass https or ssh explicitly');for(const mode of modes)await run(mode)})().catch(e=>{console.error(e.message);process.exitCode=1});
