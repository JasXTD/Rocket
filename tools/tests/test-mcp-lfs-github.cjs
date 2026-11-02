// Real network acceptance: all operations under test use authenticated localhost
// MCP widgets/controller/backend. CLI commands only prepare/check owned fixtures.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {prepare,payload,oid,name,pointer,hash,cache,clearCache,snapshot}=require('./lfs-github-fixture.cjs');
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function run(mode){
 const f=prepare(mode),root=f.root,profile=path.join(root,'lfs-http-profile');
 const report={passed:false,mode,root,bytes:payload.length,cases:[]};let service,client,state;
 const pass=s=>{report.cases.push(s);console.error('PASS '+mode+' '+s)};
 async function until(pred,label='UI state'){const t=performance.now();for(;;){state=await client.tool('rocket_state');if(pred(state))return state;assert(performance.now()-t<120000,label+' timed out');await delay(100)}}
 async function settle(){await until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running)&&!s.dialog?.lfs?.running)}
 async function stable(){let last='',n=0;await until(s=>{const current=JSON.stringify(s.controls);n=current===last?n+1:0;last=current;return n>=2},'stable widgets')}
 async function click(id){await settle();await stable();await until(s=>s.controls.some(c=>c.id===id&&c.enabled),'enabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});await settle()}
 async function show(){await click('menu-view');await click('view-goto');await click('goto-lfs')}
 async function capture(label){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,'lfs-'+label+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,'lfs-'+label+'.json'),JSON.stringify(state,null,2))}
 function unchanged(){assert.deepEqual(snapshot(f),f.r.lfs.before)}
 function activity(){return state.activities.at(-1)}
 async function fetch(){await click('dialog-lfs-fetch');assert.equal(state.dialog.lfs.preview?.operation,'lfs-fetch',state.dialog.error);await click('dialog-submit')}
 try{
  const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);
  cp.execFileSync(exe,['--init-http',profile,root],{env:f.env,windowsHide:true});
  const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
  fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);report.endpoint=`http://127.0.0.1:${port}/mcp`;
  const log=fs.openSync(path.join(root,'lfs-service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:f.env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
  for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}
  report.version=(await client.initialize()).serverInfo.version;assert.equal(report.version,'0.22.1');await client.tool('rocket_open',{path:f.client});await settle();await client.tool('rocket_resize',{width:1480,height:920});await settle();await show();
  assert(state.dialog.lfs.status,state.dialog.error);assert.equal(state.dialog.lfs.status.entries[0].path,name);assert(!state.dialog.lfs.status.entries[0].cached);unchanged();await capture('list');pass('Native Unicode pointer inspection with a missing local object');
  await click('dialog-lfs-fetch');assert.equal(state.dialog.lfs.preview?.operation,'lfs-fetch',state.dialog.error);await capture('preview');await click('dialog-cancel');unchanged();assert(!fs.existsSync(cache(f)));pass('Cancelled review makes no download or file change');
  // Exercise first-fetch metadata too, rather than relying on the upload setup's cache.
  const accessKey='lfs.https://github.com/JasXTD/rocket-auth-test.git/info/lfs.access';
  if(mode==='https'){assert.equal(f.git(['config','--local','--get',accessKey]),'basic');f.git(['config','--local','--unset',accessKey]);}
  const formats=f.git(['config','--local','--null','--list']).split('\0').filter(r=>r.startsWith('lfs.repositoryformatversion\n'));
  assert(formats.length===0||(formats.length===1&&formats[0]==='lfs.repositoryformatversion\n0'));
  if(formats.length)f.git(['config','--local','--unset','lfs.repositoryformatversion']);
  f.run('auth-off');await show();await fetch();assert.equal(activity().command_succeeded,false);assert(!fs.existsSync(cache(f)));unchanged();
  report.failure_notice=state.dialog.lfs.notice;await capture('auth-unavailable');
  assert.match(report.failure_notice,mode==='https'?/credentials_unavailable|authentication_rejected/:/ssh_authentication_rejected/);assert(!state.dialog.error,state.dialog.error);pass('Authentication unavailable is classified, refreshed and preserves local files');
  f.run('auth-on');await click('dialog-preview-refresh');await fetch();await capture('authenticated');assert(activity().command_succeeded,state.dialog.lfs.notice);assert.equal(hash(cache(f)),oid);assert.equal(fs.readFileSync(path.join(f.client,name),'utf8'),pointer);unchanged();pass('Authenticated native fetch downloads and SHA-256 verifies the object');
  clearCache(f);await click('dialog-preview-refresh');await fetch();assert(activity().command_succeeded,state.dialog.lfs.notice);assert.deepEqual(fs.readFileSync(cache(f)),payload);unchanged();pass('Repeated fetch works with Git LFS authentication metadata already present');
  await click('dialog-lfs-row-0');await click('dialog-lfs-checkout');await click('dialog-submit');assert(activity().command_succeeded,state.dialog.lfs.notice);assert.deepEqual(fs.readFileSync(path.join(f.client,name)),payload);const recovery=activity().details.find(s=>s.startsWith('Recovery data: '));assert(recovery);const dir=recovery.slice('Recovery data: '.length).split('. Read RECOVERY.md')[0];assert.equal(fs.readFileSync(path.join(dir,'original.pointer'),'utf8'),pointer);fs.writeFileSync(path.join(f.client,name),pointer);unchanged();pass('Downloaded content checks out with a retained pointer backup and unchanged staging');
  await click('dialog-lfs-fetch');f.git(['config','rocket.external-change','preserve']);await click('dialog-submit');assert.equal(activity().command_succeeded,false);assert.match(state.dialog.lfs.notice,/lfs_stale_preview/);assert.equal(f.git(['config','rocket.external-change']),'preserve');f.git(['config','--unset','rocket.external-change']);unchanged();pass('External configuration edit invalidates a reviewed network fetch and is preserved');
  await click('dialog-preview-refresh');await client.tool('rocket_resize',{width:920,height:720});await settle();await capture('compact');await click('dialog-cancel');await client.tool('rocket_close',{});await client.raw('POST','/shutdown');await delay(200);
  const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
  assert(records.length);assert(records.every(r=>r.dropped_before===0));const data=JSON.stringify(records);for(const privateValue of [name,oid,'PRIVATE_',f.remote,f.branch,'rocket@example.invalid','Synthetic content only','credential-manager','Authorization:'])assert(!data.includes(privateValue),'Diagnostic payload leak');
  for(const step of ['inspect_lfs','review_lfs','recheck_lfs','download_lfs','publish_lfs','verify_lfs','refresh_lfs'])assert(records.some(r=>r.step===step&&r.event==='step_end'),'Missing diagnostic phase '+step);
  assert(records.some(r=>r.failure_code===(mode==='https'?'credentials_unavailable':'ssh_authentication_rejected')));report.log_records=records.length;pass('Structured logs include phases and auth failures without private payloads or dropped events');
  report.passed=true;
 }catch(e){report.failure=e.message;try{if(client)await capture('failure')}catch{}throw e}
 finally{
  try{if(client&&service?.exitCode===null){if(state?.dialog&&!state.write)await click('dialog-cancel');if(state?.tabs?.length)await client.tool('rocket_close',{});await client.raw('POST','/shutdown')}}catch{}
  await delay(200);if(service?.exitCode===null)service.kill();if(client)await client.finish();
  try{f.run('cleanup');report.cleanup=f.r.cleanup;report.original_refs_unchanged=f.r.original_refs_and_default_head_unchanged}catch(e){report.cleanup_error=e.message;report.passed=false}
  fs.writeFileSync(path.join(root,'lfs-result.json'),JSON.stringify(report,null,2));fs.writeFileSync(`work/lfs-${mode}-last-result`,path.join(root,'lfs-result.json'));console.log(JSON.stringify(report,null,2));
 }
 assert(report.passed);return report;
}
(async()=>{for(const mode of process.argv.slice(2).length?process.argv.slice(2):['https','ssh'])await run(mode)})().catch(e=>{console.error(e.message);process.exitCode=1});
