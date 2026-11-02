// Focused force-with-lease acceptance through authenticated numeric-loopback MCP.
// All Git writes are confined to newly created, marked fixtures and bare remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('work/mcp-fixtures/mcp-validation-'+crypto.randomUUID().slice(0,12)),profile=path.join(root,'http-profile'),branch='rocket-ui-test/force-lease';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'no-hooks'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'no-hooks')});
const run=(exe,args,extra={})=>cp.execFileSync(exe,args,{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe'],timeout:45000,...extra}).trimEnd();
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const FILE='PRIVATE_検索 🚀 é.txt';
const mode=process.argv.includes('--cancel-feedback')?'cancel-feedback':process.argv.includes('--final')?'final':process.argv.includes('--unknown')?'unknown':process.argv.includes('--presentation')?'presentation':'functional',presentation=['presentation','final'].includes(mode),hasPush=mode!=='presentation';
const report={passed:false,version,root,mode,cases:[]},pass=s=>{report.cases.push(s);console.error('PASS '+s)};
const relay=gitRelay(root,env,'tools/helpers/file-history-preview/git-relay.rs'),{gate,marker}=relay;
function setup(name){
 const repo=path.join(root,'PRIVATE_'+name+' 文'),hooks=path.join(root,name+'-hooks'),bare=path.join(root,name+'-remote.git');fs.mkdirSync(repo);fs.mkdirSync(hooks);
 const git=a=>run('git',['-C',repo,...a]),remote=a=>run('git',['-C',bare,...a]);
 const write=(n,b)=>{fs.mkdirSync(path.dirname(path.join(repo,n)),{recursive:true});fs.writeFileSync(path.join(repo,n),b)};
 const commit=m=>{git(['add','.']);git(['commit','-m',m]);return git(['rev-parse','HEAD'])};
 git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','PRIVATE Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',k,v]);
 write(FILE,'base\n');const base=commit('PRIVATE base');write(FILE,'old remote\n');const old=commit('PRIVATE original 文');
 run('git',['clone','--bare',repo,bare]);git(['remote','add','origin',bare]);git(['config','remote.origin.fetch','+refs/heads/'+branch+':refs/remotes/origin/'+branch]);git(['fetch','origin']);
 git(['reset','--hard',base]);write(FILE,'replacement\n');const source=commit('PRIVATE replacement 🚀');
 fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
 const dirty=()=>{write(FILE,'staged\n');git(['add','--',FILE]);write(FILE,'unstaged\n');write('未跟踪.txt','PRIVATE untracked\n')};
 return{repo,hooks,bare,git,remote,write,commit,base,old,source,dirty};
}
function remoteTip(f){return f.remote(['rev-parse','refs/heads/'+branch])}
function setRemote(f,id){f.remote(['update-ref','refs/heads/'+branch,id])}
function hook(f,server,name,body){const dir=server?path.join(f.bare,'hooks'):f.hooks;fs.mkdirSync(dir,{recursive:true});if(server)f.remote(['config','core.hooksPath',dir]);const file=path.join(dir,name);fs.writeFileSync(file,'#!/bin/sh\n'+body+'\n');return file}
function touchScript(marker){return 'printf ready > "'+marker.replaceAll('\\','/')+'"\n'}
function snapshot(f){return{head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),files:Object.fromEntries(f.git(['ls-files','--cached','--others','--exclude-standard','-z']).split('\0').filter(Boolean).map(n=>[n,fs.existsSync(path.join(f.repo,n))?hash(path.join(f.repo,n)):null]))};}
let service,client,state;const delay=ms=>new Promise(r=>setTimeout(r,ms));
const control=id=>state.controls?.find(c=>c.id===id),tab=()=>state.tabs.find(t=>t.id===state.active_tab),lease=()=>state.dialog?.force_push,latest=()=>state.activities.at(-1);
async function until(test,label='state',timeout=120000){const start=performance.now();for(;;){state=await client.tool('rocket_state');if(test(state))return;assert(performance.now()-start<timeout,label+' timeout '+JSON.stringify({dialog:state.dialog,write:state.write}));await delay(90)}}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading||t.search?.running),'post-action settle',300000);}
async function stable(){let last='',n=0;await until(s=>{const v=JSON.stringify(s.controls);n=v===last?n+1:0;last=v;return n>=2})}
async function click(target,wait=true){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});if(wait)await settle();}
async function type(target,text){await stable();await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function closeDialog(){for(let n=0;state?.dialog&&n<4;n++)await click('dialog-cancel');assert(!state?.dialog)}
async function open(f){if(state?.dialog)await closeDialog();await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');}
async function closeWindow(){if(state?.dialog&&!state.write)await closeDialog();await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window))}}

// State can answer from the frame before the review is drawn, whose dialog-submit is the plain Push button.
async function review(){await click('toolbar-push');await click('push-lease-open');await stable();assert(lease());assert(!lease().preview&&!lease().running);assert(!control('dialog-submit').enabled);await click('push-lease-review');assert(lease().preview,JSON.stringify(state.dialog));return lease().preview}
async function confirm(){await type('push-lease-confirm',branch);await stable();assert(control('dialog-submit').enabled)}

const baseline=process.argv.includes('--baseline'),remaining=process.argv.includes('--final-input')||process.argv.includes('--remaining')||process.argv.includes('--transport-only')||process.argv.includes('--tags-only'),transportOnly=process.argv.includes('--transport-only')||process.argv.includes('--tags-only'),tagsOnly=process.argv.includes('--tags-only'),finalInput=process.argv.includes('--final-input');
async function waitMarker(){const start=performance.now();while(!fs.existsSync(marker)){assert(performance.now()-start<15000,'Git gate did not start');await delay(25)}}
function arm(command='check-ref-format'){if(fs.existsSync(marker))fs.unlinkSync(marker);fs.writeFileSync(gate,command)}
function releaseGate(){if(fs.existsSync(gate))fs.unlinkSync(gate)}
async function runCases(){
 if(transportOnly){await nativeCases(null,null);return}
 const f=setup('validation');f.dirty();const before=snapshot(f);await open(f);relay.check(state);await click('toolbar-push');await click('push-lease-open');
 if(!baseline){await nativeCases(f,before);return}
 arm();const clickStarted=performance.now();const input=client.tool('rocket_click',{target:'push-lease-review',expected_tab:state.active_tab});await waitMarker();
 if(baseline){
  const start=performance.now();const readState=client.tool('rocket_state');await delay(1800);releaseGate();await readState;report.state_response_ms=Math.round(performance.now()-start);await input;await settle();
  assert(report.state_response_ms>=1700,'Expected baseline UI guard to block state reads');assert.deepEqual(snapshot(f),before);assert.equal(remoteTip(f),f.old);pass('Reproduced: synchronous validation blocks state replies for the held Git read');
 }
}
function diagnosticRecords(){return fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)))}
const alive=pid=>{try{process.kill(pid,0);return true}catch{return false}};
async function pendingClick(target='push-lease-review',command='check-ref-format'){
 arm(command);const id=client.nextId+1,started=performance.now();
 const result=client.tool('rocket_click',{target,expected_tab:state.active_tab}).then(value=>({value}),error=>({error:String(error)}));
 await waitMarker();await until(s=>s.input_validation&&!s.write,'validation worker');
 const pids=fs.readFileSync(marker,'utf8').trim().split(/\s+/).map(Number);assert.equal(pids.length,2);
 return{id,started,result,pids};
}
async function finished(p,pattern){
 const result=await p.result;assert.match(result.error??'',pattern);
 await until(s=>!s.input_pending,'cancelled input cleanup');assert(p.pids.every(pid=>!alive(pid)),'Validation processes survived cleanup');
 releaseGate();return result;
}
async function responsive(label){
 const timings=[];for(let n=0;n<5;n++){const start=performance.now();state=await client.tool('rocket_state');timings.push(Math.round(performance.now()-start));assert(state.input_validation&&!state.write)}
 assert(Math.max(...timings)<1000,'UI state was blocked by the policy helper');
 report.timings??=[];report.timings.push({label,state_ms:timings});
 const start=performance.now();await client.tool('rocket_logs',{limit:20});assert(performance.now()-start<1000,'Log reads blocked during validation');
}
async function restartDirect(){
 await closeWindow();await client.raw('POST','/shutdown');const deadline=performance.now()+10000;while(service.exitCode===null){assert(performance.now()<deadline,'Previous MCP host did not exit');await delay(50)}await client.finish();
 relay.direct();const out=fs.openSync(path.join(root,'service-direct.log'),'w');service=cp.spawn(path.resolve('target/release/rocket-mcp.exe'),['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);state=undefined;
 for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(n>100)throw e;await delay(100)}}assert.equal((await client.initialize()).serverInfo.version,version);
 report.success_transport='Automatic Git directly, the bundle when there is one; interruption/expiry probes use the fixture relay';
}
async function cancelDirect(target,label){
 const result=client.tool('rocket_click',{target,expected_tab:state.active_tab}).then(value=>({value}),error=>({error:String(error)}));
 await until(s=>s.input_validation&&!s.write,label+' validation');
 const start=performance.now();const ack=await client.tool('rocket_cancel_input');assert(ack.cancel_requested);
 const ended=await result;assert.match(ended.error??'',/cancelled/);await until(s=>!s.input_pending,label+' cleanup');
 report.timings.push({label,cancel_ack_and_cleanup_ms:Math.round(performance.now()-start)});
}
async function nativeCases(f,before){
 let p;
 if(!transportOnly){
 if(!remaining){p=await pendingClick();await responsive('held destination validation');await capture('validation-pending-wide');
 const duplicate=await client.tool('rocket_click',{target:'push-lease-review',expected_tab:state.active_tab}).then(()=>'',String);assert.match(duplicate,/pending/);
 let start=performance.now();let ack=await client.tool('rocket_cancel_input');report.cancel_ack_ms=Math.round(performance.now()-start);assert(ack.cancel_requested);assert(ack.state.input_validation?.cancelling);
 await finished(p,/cancelled/);report.cancel_cleanup_ms=Math.round(performance.now()-start);assert(report.cancel_ack_ms<1000);assert(!lease().preview);assert.deepEqual(snapshot(f),before);assert.equal(remoteTip(f),f.old);
 pass('State, logs and capture stay responsive during a held Git validation; duplicate input refused; explicit cancel acknowledges immediately and reaps helper without delivering a click');

 p=await pendingClick();await client.notify('notifications/cancelled',{requestId:p.id});await finished(p,/cancelled/i);assert(!lease().preview);
 pass('MCP request cancellation stops validation and never delivers its abandoned click');

 p=await pendingClick();await client.tool('rocket_escape');await finished(p,/cancelled/);assert.equal(state.dialog.kind,'push');assert(!lease().preview);
 pass('Escape cancels the pending input without dismissing its untouched review or starting Git transport');

 p=await pendingClick();await client.tool('rocket_resize',{width:780,height:620});await finished(p,/changed/);assert(!lease().preview);
 await until(s=>s.viewport[0]===780&&s.viewport[1]===620);p=await pendingClick();await responsive('compact pending validation');await capture('validation-pending-small');await client.tool('rocket_cancel_input');await finished(p,/cancelled/);await resize(1480,920);
 pass('Resize invalidates the old coordinates instead of replaying the click; compact validation feedback remains responsive');

 // Includes must be re-read as Git sees them; a repository-config-only stamp
 // would miss this external change.
 const configBytes=fs.readFileSync(path.join(f.repo,'.git/config'));const include=path.join(root,'PRIVATE_validation_include.config');fs.writeFileSync(include,'[rocket]\nfixture = PRIVATE_before\n');f.git(['config','include.path',include]);
 p=await pendingClick();fs.writeFileSync(include,'[rocket]\nfixture = PRIVATE_after\n');releaseGate();await finished(p,/configuration changed/);assert(!lease().preview);fs.writeFileSync(path.join(f.repo,'.git/config'),configBytes);
 pass('External include-file edit during validation rejects the input and requires a fresh review');

 p=await pendingClick();fs.writeFileSync(path.join(f.repo,'.git/rocket-ui-fixture'),'rocket-ui-test/changed');releaseGate();await finished(p,/ownership changed/);fs.writeFileSync(path.join(f.repo,'.git/rocket-ui-fixture'),branch);await settle();assert(!lease().preview);
 pass('Changed fixture ownership invalidates the worker result before input delivery');

 p=await pendingClick();await responsive('deadline pending');await finished(p,/20-second deadline/);report.deadline_ms=Math.round(performance.now()-p.started);assert(report.deadline_ms>=19000&&report.deadline_ms<24000);assert(!lease().preview);assert.deepEqual(snapshot(f),before);assert.equal(remoteTip(f),f.old);
 pass('Unchanged 20-second total input deadline cancels the held helper, reports expiry, and never replays the click');

 // A denied current endpoint cannot be blessed by an old UI snapshot.
 f.git(['remote','set-url','origin','https://example.invalid/PRIVATE-never-contact']);
 const denied=await client.tool('rocket_click',{target:'push-lease-review',expected_tab:state.active_tab}).then(()=>'',String);assert.match(denied,/local paths|scope|fixture destination/);assert(!lease().preview);f.git(['remote','set-url','origin',f.bare]);
 pass('Current disallowed remote URL is rejected before any transport');

 }
 else {for(const [w,h,label] of[[1480,920,'wide'],[780,620,'small']]){await resize(w,h);p=await pendingClick();await responsive('final '+label);await delay(1100);assert(state.input_validation);await capture('validation-final-'+label);await client.tool('rocket_cancel_input');await finished(p,/cancelled/)}await resize(1480,920);pass('Final validation modal and cancellation checked at wide and compact sizes')}
 await restartDirect();await open(f);await review();assert(lease().preview);
 await confirm();await cancelDirect('dialog-submit','apply');assert(lease().preview);assert.equal(remoteTip(f),f.old);
 await click('dialog-submit');assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(remoteTip(f),f.source);assert.deepEqual(snapshot(f),before);await closeDialog();
 pass('Fresh review and explicit apply succeed; cancelling apply validation sends no push and preserves the reviewed state and dirty files');

 }
 report.timings??=[];
 // The other two former synchronous Git guards use the same worker lifecycle.
 if(!tagsOnly&&!finalInput){const pull=setup('pull-validation');pull.git(['branch','--set-upstream-to=origin/'+branch]);const pullBefore=snapshot(pull);await open(pull);await click('toolbar-pull');
 await cancelDirect('dialog-pull-fetch','pull');assert(!state.dialog.pull.fetched);assert.deepEqual(snapshot(pull),pullBefore);
 await click('dialog-pull-fetch');assert(state.dialog.pull.fetched,JSON.stringify(latest()));assert.deepEqual(snapshot(pull),pullBefore);await closeDialog();
 pass('Pull-fetch validation is cancellable and responsive; a fresh allowed fetch succeeds without integrating or changing the checkout');

 }
 if(!finalInput){const tags=setup('tag-validation'),tag='validation-tag';tags.dirty();tags.git(['tag','--no-sign',tag]);const tagsBefore=snapshot(tags);await open(tags);await click('view-tags');await click('dialog-tags-remote');
 await cancelDirect('dialog-remote-tag-refresh','remote tags');await click('dialog-remote-tag-refresh');
 const remote=()=>state.dialog.remote_tags;
 async function selectTag(){await type('dialog-remote-tag-filter',tag);let i=remote().entries.findIndex(e=>e.name===tag);assert(i>=0);await click('dialog-remote-tag-row-'+i)}
 async function reviewTag(action){await selectTag();for(let i=0;i<10;i++){const c=control('dialog-remote-tag-'+action);if(c&&c.rect[3]-c.rect[1]>22)break;await client.tool('rocket_scroll',{target:'page-details-scroll',direction:'down',pixels:200,expected_tab:state.active_tab});await settle()}await click('dialog-remote-tag-'+action);assert(remote().preview)}
 await reviewTag('publish');await click('dialog-submit');assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(tags.remote(['rev-parse','refs/tags/'+tag]),tags.source);
 await click('dialog-remote-tag-refresh');await reviewTag('delete');await type('dialog-remote-tag-confirm',tag);await click('dialog-submit');assert(latest().command_succeeded);assert.equal(tags.remote(['for-each-ref','--format=%(refname)','refs/tags/'+tag]),'');assert.deepEqual(snapshot(tags),tagsBefore);await closeDialog();
 pass('Remote-tag inspect/publish/delete use fresh asynchronous validation; cancellation is safe and only the selected tag changes');
 }
 const records=diagnosticRecords(),payload=JSON.stringify(records);assert(records.length&&records.every(r=>r.dropped_before===0));for(const value of['PRIVATE_',FILE,branch,'fixture@example.invalid'])assert(!payload.includes(value),'Diagnostic payload leak');
 for(const op of(finalInput?['force-push-policy']:tagsOnly?['remote-tag-policy']:transportOnly?['pull-policy','remote-tag-policy']:['force-push-policy','pull-policy','remote-tag-policy']))assert(records.some(r=>r.event==='request_end'&&r.operation===op));
 for(const step of['read_configuration','validate_remotes','resolve_destination','recheck_configuration','validate_fixture'])assert(records.some(r=>r.event==='step_end'&&r.operation==='mcp-input-validation'&&r.step===step));
 for(const begin of records.filter(r=>r.event==='step_start'&&r.operation==='mcp-input-validation'))assert(records.some(r=>r.event==='step_end'&&r.session===begin.session&&r.request===begin.request&&r.id===begin.id),'Unpaired validation step');
 if(!remaining)assert(records.some(r=>r.event==='request_end'&&r.operation==='force-push-policy'&&r.outcome==='timed_out'));assert(records.some(r=>r.event==='request_end'&&r.operation.endsWith('-policy')&&r.outcome==='cancelled'));
 report.log_records=records.length;pass('Validation steps and command durations are separately logged with paired spans, cancellation/timeout outcomes, no tested private payloads and zero dropped records');
}
async function main(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);run(exe,['--init-http',profile,root]);const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 if(transportOnly)relay.direct();
 const out=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:transportOnly?env:relay.env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(n>100)throw e;await delay(100)}}assert.equal((await client.initialize()).serverInfo.version,version);

 await runCases();await closeWindow();await client.raw('POST','/shutdown');report.passed=true;
 }catch(e){report.error=String(e);if(client)try{await capture('failure');fs.writeFileSync(path.join(root,'failure-state.json'),JSON.stringify(state,null,2))}catch{}throw e}finally{releaseGate();if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(250);if(service.exitCode===null)service.kill()}if(client)await client.finish();report.baseline=baseline;report.remaining=remaining;report.transport_only=transportOnly;report.tags_only=tagsOnly;report.final_input=finalInput;fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync('work/mcp-validation-'+(baseline?'baseline':finalInput?'final-input':tagsOnly?'tags':transportOnly?'transports':remaining?'remaining':'native')+'-last-result',path.join(root,'result.json'));console.log(JSON.stringify(report,null,2))}}
main().catch(e=>{console.error(e);process.exitCode=1});
