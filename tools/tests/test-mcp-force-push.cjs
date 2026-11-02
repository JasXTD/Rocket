// Focused force-with-lease acceptance through authenticated numeric-loopback MCP.
// All Git writes are confined to newly created, marked fixtures and bare remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('work/mcp-fixtures/force-push-'+crypto.randomUUID().slice(0,12)),profile=path.join(root,'http-profile'),branch='rocket-ui-test/force-lease';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'no-hooks'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'no-hooks')});
const run=(exe,args,extra={})=>cp.execFileSync(exe,args,{env,windowsHide:true,encoding:'utf8',stdio:['pipe','pipe','pipe'],timeout:45000,...extra}).trimEnd();
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const FILE='PRIVATE_検索 🚀 é.txt';
const mode=process.argv.includes('--cancel-feedback')?'cancel-feedback':process.argv.includes('--final')?'final':process.argv.includes('--unknown')?'unknown':process.argv.includes('--presentation')?'presentation':'functional',presentation=['presentation','final'].includes(mode),hasPush=mode!=='presentation';
const report={passed:false,version,root,mode,cases:[]},pass=s=>{report.cases.push(s);console.error('PASS '+s)};
const relay=gitRelay(root,env,'tools/helpers/productivity/git-relay.rs'),{gate,marker}=relay;
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
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading||t.search?.running));}
async function stable(){let last='',n=0;await until(s=>{const v=JSON.stringify(s.controls);n=v===last?n+1:0;last=v;return n>=2})}
async function click(target,wait=true){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});if(wait)await settle();}
async function type(target,text){await stable();await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function closeDialog(){for(let n=0;state?.dialog&&n<4;n++)await click('dialog-cancel');assert(!state?.dialog)}
async function open(f){if(state?.dialog)await closeDialog();await client.tool('rocket_open',{path:f.repo});await settle();relay.check(state);await resize(1480,920);if(control('console-close'))await click('console-close');}
async function closeWindow(){if(state?.dialog&&!state.write)await closeDialog();await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||(s.dialog?.kind==='quit'&&!s.input_pending));if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window))}}

async function review(){await click('toolbar-push');await click('push-lease-open');await stable();assert(lease());assert(!lease().preview&&!lease().running);assert(!control('dialog-submit').enabled);await click('push-lease-review');assert(lease().preview,JSON.stringify(state.dialog));return lease().preview}
async function confirm(){await type('push-lease-confirm',branch);await stable();assert(control('dialog-submit').enabled)}
// MCP turns client hooks off and refuses a receiver that has one, so the relay holds the real push
// after Rocket's last checks, and a receiver hook goes in then.
async function holdPush(point='before:push'){unlink(marker);unlink(marker+'.child');fs.writeFileSync(gate,point);await click('dialog-submit',false);await until(()=>state.write&&fs.existsSync(marker),'held push');return Number(fs.readFileSync(marker,'utf8'))}
function releasePush(){unlink(gate);unlink(marker)}
async function atServer(f,name,body){await holdPush();hook(f,true,name,body);releasePush()}
async function cases(){
 const f=setup('replacement');f.dirty();const before=snapshot(f),refs=f.git(['for-each-ref','--format=%(refname) %(objectname)']);await open(f);
 // Regression: ordinary push remains non-forcing, with no implicit review/retry.
 if(!presentation){await click('toolbar-push');await click('dialog-submit');assert(!latest().command_succeeded);assert(JSON.stringify(latest()).includes('push_non_fast_forward'));assert.equal(remoteTip(f),f.old);await closeDialog();}
 const p=await review();assert.equal(p.expected,f.old);assert.equal(p.source,f.source);assert.deepEqual([p.removed_count,p.added_count],[1,1]);assert(p.added[0].subject.includes('🚀'));assert.deepEqual(snapshot(f),before);
 await type('push-lease-confirm','wrong');await stable();assert(!control('dialog-submit').enabled);await confirm();await capture('lease-review-wide');await resize(780,620);await stable();for(const id of['dialog-submit','push-lease-review','dialog-cancel']){const c=control(id);assert(c&&c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=780&&c.rect[3]<=620,id+' clipped')}await capture('lease-review-small');await resize(1480,920);
 if(presentation){assert.deepEqual(snapshot(f),before);assert.equal(remoteTip(f),f.old);await closeDialog();pass('Final-build wide/compact force-with-lease preview, exact confirmation and fixed footer; no push performed');return;}
 await click('dialog-submit');assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(remoteTip(f),f.source);assert(!lease().preview);assert.deepEqual(snapshot(f),before);assert.equal(f.git(['for-each-ref','--format=%(refname) %(objectname)']),refs);await capture('lease-success');await click('push-lease-review');assert(!lease().preview.ready);assert(!control('dialog-submit').enabled);await closeDialog();
 pass('Ordinary push rejects replacement; exact preview/confirmation succeeds with Unicode paths/messages and preserves HEAD, refs, index, config and all dirty files; wide/compact layouts');
 const g=setup('stale');g.dirty();await open(g);await review();setRemote(g,g.base);await confirm();await click('dialog-submit');assert(state.dialog.error.includes('force_push_remote_changed'));assert.equal(remoteTip(g),g.base);assert(!lease().preview);await capture('lease-stale-remote');await closeDialog();setRemote(g,g.old);
 await review();g.git(['update-ref','refs/heads/'+branch,g.base]);await confirm();await click('dialog-submit');assert(state.dialog.error.includes('force_push_stale_preview'));assert.equal(remoteTip(g),g.old);await closeDialog();g.git(['update-ref','refs/heads/'+branch,g.source]);await click('refresh');
 await review();g.git(['config','push.default','matching']);await confirm();await click('dialog-submit');assert(state.dialog.error.includes('force_push_stale_preview'));assert.equal(remoteTip(g),g.old);await closeDialog();g.git(['config','--unset','push.default']);
 pass('Remote, source and configuration edits invalidate old reviews without starting a push or retrying automatically');
 const r=setup('race');const peer=path.join(root,'race-peer');run('git',['clone',r.bare,peer]);const pg=a=>run('git',['-C',peer,...a]);pg(['config','user.name','Fixture']);pg(['config','user.email','fixture@example.invalid']);pg(['commit','--allow-empty','-m','PRIVATE concurrent']);const concurrent=pg(['rev-parse','HEAD']);pg(['push','origin','HEAD:refs/heads/held']);
 const ready=path.join(root,'race-ready'),release=path.join(root,'race-release');r.dirty();const raceBefore=snapshot(r);await open(r);await review();await confirm();await atServer(r,'pre-receive',touchScript(ready)+'while ! test -f "'+release.replaceAll('\\','/')+'"; do sleep 0.1; done');await until(()=>state.write&&fs.existsSync(ready),'pre-receive race gate');setRemote(r,concurrent);fs.writeFileSync(release,'go');await settle();assert(!latest().command_succeeded);assert.equal(remoteTip(r),concurrent);assert(JSON.stringify(latest()).includes(concurrent));assert.deepEqual(snapshot(r),raceBefore);await capture('lease-race');await closeDialog();
 pass('A concurrent remote change after Git advertises refs is rejected by the server compare-and-swap; the other commit survives and the UI reports the observed tip');
 const h=setup('reject');h.dirty();const rejectedBefore=snapshot(h);await open(h);await review();await confirm();const refusedRule=hook(h,true,'pre-receive','echo ran > refused-rule-ran');await click('dialog-submit');assert(!latest().command_succeeded);assert.match(JSON.stringify(latest()),/without active hooks/);assert(!fs.existsSync(path.join(h.bare,'refused-rule-ran')));assert.equal(remoteTip(h),h.old);fs.unlinkSync(refusedRule);await closeDialog();await review();await confirm();await atServer(h,'pre-receive','echo ran > rule-ran\necho "PRIVATE server policy denied" >&2\nexit 1');await settle();assert(fs.existsSync(path.join(h.bare,'rule-ran')),JSON.stringify(latest()));assert(!latest().command_succeeded);assert(state.dialog.error.includes('push_remote_rejected'));assert(!state.dialog.error.includes('PRIVATE server'));assert.equal(remoteTip(h),h.old);assert.deepEqual(snapshot(h),rejectedBefore);await capture('lease-rejected');await closeDialog();
 pass('A receiver rule already in place is refused before Git starts and never runs; server rejection stays visible with specific guidance, independent unchanged-tip observation and no raw hook payload in errors');
 // Pause a real Git read in an inherited fixture-only relay; no fake successful responses.
 await open(g);await click('toolbar-push');await click('push-lease-open');unlink(marker);fs.writeFileSync(gate,'before:cat-file');await click('push-lease-review',false);await until(()=>lease()?.loading&&fs.existsSync(marker)&&!state.input_pending,'live review gate');let start=performance.now();await click('push-lease-cancel');report.review_cancel_ms=Math.round(performance.now()-start);assert(!lease().preview&&!lease().running);assert(lease().notice.includes('cancelled'));fs.unlinkSync(gate);await capture('lease-review-cancelled');await closeDialog();
 for(const after of[false,true]){const c=setup(after?'cancel-after':'cancel-before'),ready=path.join(root,after?'after-ready':'before-ready');c.dirty();const original=snapshot(c);await open(c);await review();await confirm();await atServer(c,after?'post-receive':'pre-receive',touchScript(ready)+'sleep 30');await until(()=>state.write&&fs.existsSync(ready),'live push cancellation');start=performance.now();await click('push-lease-cancel');report[after?'after_cancel_ms':'before_cancel_ms']=Math.round(performance.now()-start);assert(performance.now()-start<20000);assert(!latest().command_succeeded);assert(JSON.stringify(latest()).includes('cancel'));assert.equal(remoteTip(c),after?c.source:c.old);assert(latest().summary.includes(after?'already matches':'still matches'));assert.deepEqual(snapshot(c),original);await capture(after?'lease-cancel-after':'lease-cancel-before');await closeDialog();}
 pass('Live review cancellation discards its result; push cancellation before and after server update reports unchanged versus already-applied state and protects local data');
 await unknownCase();
}
async function guardCase(){
 const f=setup('policy');await open(f);await click('toolbar-push');await click('push-lease-open');
 // Expire one actual policy read, then release it: a late click must never start work.
 unlink(marker);fs.writeFileSync(gate,'before:config');await stable();const started=performance.now();const expired=client.tool('rocket_click',{target:'push-lease-review',expected_tab:state.active_tab}).then(()=>undefined,e=>String(e));for(let n=0;!fs.existsSync(marker);n++){assert(n<300,'Policy relay did not start');await delay(50)}const timedOut=await expired;assert(timedOut&&/20-second deadline/.test(timedOut),timedOut??'Expired click was accepted');fs.unlinkSync(gate);await settle();report.expired_validation_ms=Math.round(performance.now()-started);assert(!lease().preview&&!lease().running);assert.equal(remoteTip(f),f.old);
 async function denied(target){await stable();let error;try{await client.tool('rocket_click',{target,expected_tab:state.active_tab})}catch(e){error=String(e)}assert(error&&/local|fixture|approved/i.test(error),error??'Unsafe destination accepted');await settle();assert(!lease().running);assert.equal(remoteTip(f),f.old)}
 f.git(['config','remote.origin.pushurl','https://example.invalid/PRIVATE_not-contacted']);await denied('push-lease-review');assert(!lease().preview);f.git(['config','--unset','remote.origin.pushurl']);
 await click('push-lease-review');assert(lease().preview);await confirm();f.git(['config','remote.origin.pushurl','https://example.invalid/PRIVATE_not-contacted']);await denied('dialog-submit');assert(lease().preview);f.git(['config','--unset','remote.origin.pushurl']);await closeDialog();
 pass('MCP rejects an expired policy read without late input; re-reads changed configuration at review/apply and refuses out-of-scope destinations without transport');
}
async function unknownCase(){
 const u=setup('unknown');const replacement=path.join(root,'unknown-other.git');run('git',['clone','--bare',u.repo,replacement]);await open(u);await review();await confirm();await holdPush('after:push');u.git(['config','remote.origin.pushurl',replacement]);releasePush();await settle();assert.equal(remoteTip(u),u.source);assert(!latest().command_succeeded);assert(latest().summary.includes('unknown'));assert(state.dialog.error.includes('force_push_unconfirmed'));assert(state.dialog.error.includes('force_push_observation_changed'));assert(!state.dialog.error.includes('No push started'));await capture('lease-unknown');await closeDialog();
 pass('External configuration change during push makes reconciliation explicitly unknown even when the original destination received the update');
}

function diagnosticRecords(){return fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(p=>/^rocket(?:\.\d+)?\.jsonl$/.test(p)).flatMap(p=>fs.readFileSync(path.join(root,n,p),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)))}
function unlink(p){if(fs.existsSync(p))fs.unlinkSync(p)}
function alive(pid){try{process.kill(pid,0);return true}catch(e){if(e.code==='ESRCH')return false;throw e}}
async function cancelNow(name){
 await stable();assert(control('push-lease-cancel')?.enabled);
 const started=performance.now();const reply=await client.tool('rocket_click',{target:'push-lease-cancel',expected_tab:state.active_tab});
 assert(reply.input_applied);state=reply.state;
 assert.equal(state.write.force_push_phase,'stopping');assert.equal(state.write.progress,'Stopping…');assert.equal(state.write.cancel_enabled,false);
 const timing={name,input_ack_ms:Math.round(performance.now()-started)};
 assert(timing.input_ack_ms<1000,'Native cancel acknowledgement took over one second');
 fs.writeFileSync(path.join(root,name+'-stop-ack.json'),JSON.stringify(state,null,2));
 return {timing,started};
}
async function cancellationFeedbackCases(){
 report.timings=[];report.remaining_only=process.argv.includes('--remaining');
 // A receiver hook pauses the real push. Only the *later* remote observation is
 // gated by the relay, so stopping and checking can be measured apart.
 for(const variant of (report.remaining_only?['unknown']:['before','after','unknown'])){
  const after=variant==='after', f=setup('feedback-'+variant), ready=path.join(root,variant+'-ready');
  f.dirty();const original=snapshot(f),refs=f.git(['for-each-ref','--format=%(refname) %(objectname)']);
  await open(f);await review();await confirm();const held=await holdPush();hook(f,true,after?'post-receive':'pre-receive',touchScript(ready)+'sleep 30');releasePush();
  await until(()=>state.write?.force_push_phase==='pushing'&&fs.existsSync(ready)&&fs.existsSync(marker+'.child'),'active push hook');
  const pids=[held,Number(fs.readFileSync(marker+'.child','utf8'))];assert(pids.every(Number.isInteger));
  assert(pids.every(alive),'Push relay/Git must be live before cancellation');assert.equal(remoteTip(f),after?f.source:f.old);
  unlink(marker);fs.writeFileSync(gate,'before:--server-option=');
  const {timing,started}=await cancelNow(variant);
  await until(()=>state.write?.force_push_phase==='checking_remote'&&fs.existsSync(marker),'checking after stopped push');
  timing.checking_visible_ms=Math.round(performance.now()-started);
  assert(state.write.cancelling);assert.equal(state.write.progress,'Push stopped—checking the remote result…');assert.equal(state.write.cancel_enabled,false);
  assert(pids.every(pid=>!alive(pid)),'Owned push processes survived the checking phase');
  for(const key of ['push-lease-cancel','status-cancel'])if(control(key))assert(!control(key).enabled,key+' should be disabled');
  let repeated;try{await client.tool('rocket_click',{target:'push-lease-cancel',expected_tab:state.active_tab})}catch(e){repeated=String(e)}assert(repeated&&/disabled/i.test(repeated),'Repeated cancellation must be refused');
  if(variant==='before'){await capture('cancel-checking-wide');await client.tool('rocket_resize',{width:780,height:620});await until(s=>s.viewport[0]===780&&s.viewport[1]===620&&!s.input_pending);await stable();const stop=control('push-lease-cancel');assert(stop&&stop.rect[0]>=0&&stop.rect[1]>=0&&stop.rect[2]<=780&&stop.rect[3]<=620,'push-lease-cancel clipped');for(const key of['dialog-submit','push-lease-back'])assert(!control(key),key+' offered while the push runs');await capture('cancel-checking-small');}
  // Do not use resize() here: it waits for the operation to settle. The verification
  // gate is released below, except for the explicit timeout/unknown-outcome case.
  if(variant!=='unknown')unlink(gate);
  await settle();timing.total_settle_ms=Math.round(performance.now()-started);unlink(gate);
  assert(!latest().command_succeeded);assert.match(JSON.stringify(latest()),/cancel/);
  assert.match(latest().summary,variant==='unknown'?/unknown/:after?/already matches/:/still matches/);
  assert(!lease().preview);assert.equal(remoteTip(f),after?f.source:f.old);assert.deepEqual(snapshot(f),original);assert.equal(f.git(['for-each-ref','--format=%(refname) %(objectname)']),refs);
  const records=diagnosticRecords(),end=records.filter(r=>r.event==='git_end'&&r.operation==='push'&&r.outcome==='cancelled').at(-1);
  assert(Number.isSafeInteger(end?.cancel_to_stop_us));timing.request=end.request;timing.git_stop_cleanup_ms=end.cancel_to_stop_us/1000;assert(timing.git_stop_cleanup_ms<1000,'Git stop/cleanup exceeded one second');
  const observation=records.find(r=>r.request===end.request&&r.event==='step_end'&&r.step==='observe_force_push_remote');assert(observation);timing.remote_check_ms=observation.elapsed_ms;
  if(variant==='unknown'){assert(observation.outcome==='timed_out');assert(timing.remote_check_ms>=10000);assert(timing.remote_check_ms<16000);}
  timing.remote_outcome=variant==='unknown'?'unknown':after?'replaced':'unchanged';report.timings.push(timing);await capture('cancel-'+variant+'-result');await closeDialog();
  pass('Immediate native stop feedback and separate Git stop/remote-check timing: '+variant+'; no repeated cancel, retry, rollback claim or local data changes');
 }
 // Cancellation before the push starts must not pretend to have a remote result.
 const early=setup('feedback-revalidation');early.dirty();const earlyBefore=snapshot(early);await open(early);await review();await confirm();unlink(marker);fs.writeFileSync(gate,'before:--server-option=');await click('dialog-submit',false);
 await until(()=>state.write?.force_push_phase==='checking_lease'&&fs.existsSync(marker),'revalidation read');const earlyTiming=await cancelNow('revalidation');await settle();unlink(gate);
 assert(!latest().command_succeeded);assert.match(JSON.stringify(latest()),/cancel/);assert.equal(remoteTip(early),early.old);assert.deepEqual(snapshot(early),earlyBefore);
 const earlyRecords=diagnosticRecords(),request=earlyRecords.filter(r=>r.event==='request_start'&&r.operation==='force-push').at(-1).request;assert(!earlyRecords.some(r=>r.request===request&&r.event==='git_start'&&r.operation==='push'));
 await closeDialog();pass('Cancel during lease revalidation acknowledges immediately and starts no push or unnecessary remote verification');
 // Preview cancellation keeps its existing immediate no-push notice.
 await click('toolbar-push');await click('push-lease-open');unlink(marker);fs.writeFileSync(gate,'before:--server-option=');await click('push-lease-review',false);await until(()=>lease()?.loading&&fs.existsSync(marker),'preview read');await click('push-lease-cancel');unlink(gate);assert(!lease().running&&!lease().preview);assert.match(lease().notice,/No push started/);await closeDialog();pass('Read-only review cancellation still discards its preview and acknowledges no push started');
 for(const rejected of [false,true]){
  const f=setup(rejected?'feedback-rejected':'feedback-success'),ready=path.join(root,'complete-'+rejected),release=path.join(root,'release-'+rejected);
  f.dirty();const before=snapshot(f);await open(f);await review();await confirm();await atServer(f,'pre-receive',touchScript(ready)+'while ! test -f "'+release.replaceAll('\\','/')+'"; do sleep 0.1; done'+(rejected?'\necho PRIVATE_policy_rejected >&2\nexit 1':''));await until(()=>state.write?.force_push_phase==='pushing'&&fs.existsSync(ready),'noncancelled push');
  unlink(marker);fs.writeFileSync(gate,'before:--server-option=');fs.writeFileSync(release,'go');await until(()=>state.write?.force_push_phase==='checking_remote'&&fs.existsSync(marker),'noncancelled checking');assert.equal(state.write.progress,'Checking the remote result…');assert(!state.write.cancelling);assert(!state.write.cancel_enabled);unlink(gate);await settle();assert.equal(latest().command_succeeded,!rejected);assert.equal(remoteTip(f),rejected?f.old:f.source);assert.deepEqual(snapshot(f),before);if(rejected)assert.match(JSON.stringify(latest()),/push_remote_rejected/);await closeDialog();
  pass('Non-cancelled '+(rejected?'server rejection':'successful replacement')+' shows checking phase without falsely saying the push was cancelled');
 }
 await click('menu-view');await click('activity-open-menu');await until(s=>s.activity_history?.open&&!s.activity_history.loading,'diagnostic viewer');await type('activity-search','force-push');await click('activity-cancelled');
 const row=state.activity_history.rows.find(r=>r.request===report.timings[0].request);assert(row,'Cancelled push missing from diagnostic viewer');await click('activity-row-'+row.index);
 const stop=state.activity_history.selected.records.find(r=>r.operation==='push'&&r.outcome==='cancelled');assert.equal(stop.cancel_to_stop_us/1000,report.timings[0].git_stop_cleanup_ms);assert(!JSON.stringify(state.activity_history.selected).includes('PRIVATE_'));await capture('cancel-activity-timing');
 pass('Activity diagnostics retain the measured stop time separately from command duration and exclude private fixture payloads');
}

async function main(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);run(exe,['--init-http',profile,root]);const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const out=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',out]});fs.closeSync(out);client=new RocketHttpClient(profile);for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(n>100)throw e;await delay(100)}}assert.equal((await client.initialize()).serverInfo.version,version);
 if(mode==='cancel-feedback')await cancellationFeedbackCases();else if(mode==='unknown')await unknownCase();else{await cases();if(mode==='final'){await unknownCase();await guardCase();}}await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const value of['PRIVATE_',FILE,'fixture@example.invalid',branch])assert(!payload.includes(value),'Diagnostic leak');
 for(const op of(hasPush?['force-push-preview','force-push','force-push-policy']:['force-push-preview','force-push-policy'])){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired '+op)}
 if(hasPush){
 for(const step of['review_force_push','inspect_force_push_remote','compare_force_push_history','revalidate_force_push','push_with_explicit_lease','observe_force_push_remote','verify_force_push_remote'])assert(records.some(r=>r.step===step&&r.event==='step_end'),'Missing step '+step);
 for(const code of(mode==='cancel-feedback'?['force_push_unconfirmed','push_remote_rejected']:mode==='functional'?['force_push_remote_changed','force_push_stale_preview','push_remote_rejected','force_push_unconfirmed','force_push_observation_changed']:['force_push_unconfirmed','force_push_observation_changed']))assert(records.some(r=>r.failure_code===code),'Missing error '+code);
 if(mode==='functional')assert(records.some(r=>r.outcome==='cancelled'));for(const outcome of (mode==='cancel-feedback'?['replaced','unchanged','unknown']:mode==='functional'?['replaced','unchanged','different','unknown']:['unknown']))assert(records.some(r=>r.event==='force_push_result'&&r.remote_outcome===outcome),'Missing observed outcome '+outcome);assert(records.some(r=>r.event==='force_push_review'&&r.removed_commits===1&&r.added_commits===1));}
 for(const begin of records.filter(r=>r.event==='step_start'&&['force-push-preview','force-push'].includes(r.operation)))assert(records.some(r=>r.event==='step_end'&&r.session===begin.session&&r.request===begin.request&&r.id===begin.id),'Unpaired force-push step');
 const policyReads=records.filter(r=>r.event==='request_end'&&r.operation==='force-push-policy');assert(policyReads.length);report.policy_max_ms=Math.max(...policyReads.map(r=>r.elapsed_ms));for(const r of policyReads)assert(!records.some(c=>c.request===r.request&&c.event==='git_start'&&c.operation==='--version'),'Repeated Git discovery in policy guard');
 report.log_records=records.length;pass(mode==='functional'?'Paired request/step diagnostics, typed rejection/stale/unknown/cancellation outcomes, zero drops and no tested private payloads':'Scoped request/step diagnostics and required outcome codes, zero drops and no tested private payloads');report.passed=true;
 }catch(e){report.error=String(e);if(client)try{await capture('failure')}catch{}throw e}finally{if(fs.existsSync(gate))fs.unlinkSync(gate);if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(200);if(service.exitCode===null)service.kill()}if(client)await client.finish();fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(mode==='functional'?'work/force-push-last-result':'work/force-push-'+mode+'-last-result',path.join(root,'result.json'));console.log(JSON.stringify(report,null,2))}}
main().catch(e=>{console.error(e);process.exitCode=1});
