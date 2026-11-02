// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/merge-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/merge';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const focused=process.argv.includes('--preview-only');
const report={passed:false,version,root,focused,cases:[]},fixtures=[];
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function fixture(name){
  const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
  const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8'}).trimEnd();
  const write=(name,body)=>{const p=path.join(repo,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,body);};
  const commit=message=>{git(['add','.']);git(['commit','-m',message]);return git(['rev-parse','HEAD']);};
  git(['init','--initial-branch='+branch]);
  for(const [key,value]of [['user.name','Rocket Commit Fixture'],['user.email','rocket@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',key,value]);
  write('base.txt','base\n');commit('PRIVATE_SOURCE baseline 中文');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
  const f={repo,hooks,git,write,commit,config:hash(path.join(repo,'.git/config'))};fixtures.push(f);return f;
}
let client,service,state;
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const passed=text=>{report.cases.push(text);console.error('PASS '+text);};
async function until(predicate,label='UI state'){
  const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<45000,label+' timed out; '+JSON.stringify({dialog:state.dialog,write:state.write,notice:state.notice}));await delay(80);}
}
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running));}
function active(){return state.tabs.find(t=>t.id===state.active_tab);}
function control(id){return state.controls.find(c=>c.id===id);}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=3;},'Stable widget geometry');}
async function click(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();}
async function type(target,text){await stable();assert(control(target)?.enabled,'Missing/disabled input '+target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const image=result.content.find(c=>c.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function scroll(target,direction='down',pixels=600){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settle();await stable();}
async function menuItem(id){await click('menu-repository');await scroll('repository-menu-scroll','up',2000);for(let i=0;i<8;i++){const c=control(id);if(c&&c.rect[3]-c.rect[1]>=28)break;await scroll('repository-menu-scroll','down',250);}await click(id);}
async function open(f){console.error("OPEN fixture "+path.basename(f.repo));if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function submit(){await click('dialog-submit');await settle();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}

// The harness above matches the other isolated native MCP acceptance runners.
const FILE='src/検索/結果 🚀 é.rs',topic='feature/合并';
function setup(name,kind='ff'){
 const f=fixture(name);f.write(FILE,'base\n');f.write('.gitignore','ignored/\n');f.base=f.commit('PRIVATE_BASE 中文');
 f.git(['switch','-c',topic]);f.write(kind==='conflict'?FILE:'incoming 文.rs','incoming\n');f.incoming=f.commit('PRIVATE_INCOMING 🚀');f.git(['switch',branch]);
 if(kind!=='ff'){f.write(kind==='conflict'?FILE:'local.rs','ours\n');f.commit('PRIVATE_LOCAL');}
 f.original=f.git(['rev-parse','HEAD']);return f;
}
function snapshot(f){return {head:f.git(['rev-parse','HEAD']),index:f.git(['ls-files','--stage']),file:fs.readFileSync(path.join(f.repo,FILE),'utf8'),config:hash(path.join(f.repo,'.git/config'))};}
async function selected(){await click('view-branches');const b=active().branches.find(b=>b.reference==='refs/heads/'+topic);assert(b);await click('branch-row-'+b.index);await click('branch-merge');assert.equal(state.dialog.kind,'merge');assert.equal(state.dialog.merge.revision,'refs/heads/'+topic);}
async function review(mode){if(mode){await click('dialog-merge-'+mode);assert.equal(state.dialog.merge.preview,null);}await click('dialog-preview-refresh');}
async function finishDialog(){if(state.dialog)await click('dialog-cancel');}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 const ff=setup('PRIVATE_UNICODE 中文');ff.write('untracked 文.txt','preserved');ff.write('ignored/local.txt','ignored preserved');const original=snapshot(ff);
 await open(ff);await selected();assert.equal(state.dialog.merge.preview.target,ff.incoming);assert.equal(state.dialog.merge.preview.relation,'FastForward');assert.deepEqual(snapshot(ff),original);
 await capture('merge-preview-wide');await resize(780,620);await stable();
 for(const id of ['dialog-submit','dialog-preview-refresh','dialog-cancel']){const r=control(id).rect;assert(r[0]>=0&&r[1]>=0&&r[2]<=780&&r[3]<=620,id+' clipped');}
 await capture('merge-preview-small');await resize(1480,920);await submit();assert(latest().command_succeeded);assert.equal(ff.git(['rev-parse','HEAD']),ff.incoming);
 assert.equal(fs.readFileSync(path.join(ff.repo,'untracked 文.txt'),'utf8'),'preserved');assert.equal(fs.readFileSync(path.join(ff.repo,'ignored/local.txt'),'utf8'),'ignored preserved');
 passed('Selected Unicode branch, read-only preview, wide/small layout and fast-forward merge preserve unrelated local files');
 const d=setup('PRIVATE_DIVERGED','diverged');await open(d);await selected();assert.equal(state.dialog.merge.preview.relation,'Diverged');assert(!control('dialog-submit').enabled);await capture('merge-ff-refused');
 await review('allow');await submit();assert(latest().command_succeeded);assert.deepEqual(d.git(['rev-list','--parents','-1','HEAD']).split(' ').slice(1),[d.original,d.incoming]);
 passed('Diverged history refuses ff-only; mode changes invalidate the review and a reviewed merge preserves both parents');
 const stale=setup('PRIVATE_STALE');await open(stale);await selected();stale.git(['update-ref','refs/heads/'+topic,stale.original]);await submit();
 assert(state.dialog.error.includes('merge_stale_preview'));assert(!latest().command_succeeded);assert.equal(stale.git(['rev-parse','HEAD']),stale.original);await capture('merge-stale-target');
 stale.git(['update-ref','refs/heads/'+topic,stale.incoming]);await review();stale.write(FILE,'external PRIVATE edit\n');await submit();assert(state.dialog.error.includes('merge_dirty_worktree'));assert.equal(fs.readFileSync(path.join(stale.repo,FILE),'utf8'),'external PRIVATE edit\n');await finishDialog();
 passed('Moved source branch and external working edits reject application without overwriting changes');
 const invalid=setup('PRIVATE_INVALID');await open(invalid);await selected();await type('dialog-merge-target','missing-PRIVATE-ref');assert.equal(state.dialog.merge.preview,null);assert(!control('dialog-submit').enabled);await review();assert(state.dialog.error.includes('merge_target_unavailable'));await capture('merge-missing-target');
 await type('dialog-merge-target','refs/heads/'+topic);await review();fs.writeFileSync(path.join(invalid.repo,'.git/index.lock'),'existing lock');await submit();assert(state.dialog.error.includes('merge_index_locked'));assert.equal(fs.readFileSync(path.join(invalid.repo,'.git/index.lock'),'utf8'),'existing lock');fs.unlinkSync(path.join(invalid.repo,'.git/index.lock'));await finishDialog();
 passed('Target edits invalidate reviews; missing references and index locks have distinct errors and preserve locks');
 for(const resolution of ['abort','continue']){
  const f=setup('PRIVATE_CONFLICT_'+resolution,'conflict');await open(f);await selected();await review('allow');await submit();assert(latest().recovery_pending);assert(latest().summary.includes('conflicts'));assert.equal(active().view,'Changes');assert(fs.existsSync(path.join(f.repo,'.git/MERGE_HEAD')));await capture('merge-conflicts-'+resolution);
  if(resolution==='abort'){
   await click('operation-abort');assert.equal(state.dialog.kind,'merge-abort');await click('dialog-cancel');assert(fs.existsSync(path.join(f.repo,'.git/MERGE_HEAD')));await click('operation-abort');await submit();assert(latest().command_succeeded);assert.equal(f.git(['rev-parse','HEAD']),f.original);assert.equal(fs.readFileSync(path.join(f.repo,FILE),'utf8'),'ours\n');
  }else{
   await click('change-unstaged-'+Buffer.from(FILE).toString('hex'));await click('conflict-open');await click('conflict-ours');await click('conflict-stage');await submit();await click('view-changes');if(control('console-close'))await click('console-close');await type('commit-message','Resolved merge through Rocket');await click('commit');assert(latest().command_succeeded);assert.deepEqual(f.git(['rev-list','--parents','-1','HEAD']).split(' ').slice(1),[f.original,f.incoming]);
  }
  assert(!fs.existsSync(path.join(f.repo,'.git/MERGE_HEAD')));
 }
 passed('Conflicts route to Changes; reviewed Abort and native editor, staging and Continue complete recovery');
 for(const phase of ['before-commit','after-commit']){
  const f=setup('PRIVATE_CANCEL_'+phase),marker=path.join(f.repo,'.git/merge-hook-reached'),hook=phase==='before-commit'?'pre-merge-commit':'post-merge';
  fs.writeFileSync(path.join(f.hooks,hook),'#!/bin/sh\nprintf reached > .git/merge-hook-reached\nsleep 30\n');
  await open(f);await selected();await review('no-ff');await stable();const start=performance.now();
  await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>!!s.write&&fs.existsSync(marker),'Merge hook reached');
  await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report[phase+'_cancel_ms']=performance.now()-start;assert(!latest().command_succeeded);assert(report[phase+'_cancel_ms']<15000);assert(latest().details.some(d=>/cancel/i.test(d)));await capture('merge-cancel-'+phase);assert.equal(f.git(['rev-parse','refs/heads/'+topic]),f.incoming);
  if(phase==='after-commit'){
   assert.deepEqual(f.git(['rev-list','--parents','-1','HEAD']).split(' ').slice(1),[f.original,f.incoming]);
   assert(active().operation_state.merge_committed);assert(latest().details.some(d=>d.includes('merge_already_committed')));
   assert(control('operation-finish-merge')?.enabled);assert(!control('operation-abort'));assert(!control('commit')?.enabled);
   f.write(FILE,'local after committed merge\n');f.git(['add','--',FILE]);f.write(FILE,'unstaged after committed merge\n');f.write('local-untracked','preserve');
   const preserved=snapshot(f);await click('operation-finish-merge');assert.equal(state.dialog.kind,'merge-finish');await capture('merge-cleanup-review');await click('dialog-cancel');assert(fs.existsSync(path.join(f.repo,'.git/MERGE_HEAD')));
   await click('operation-finish-merge');await submit();assert(latest().command_succeeded);assert.deepEqual(snapshot(f),preserved);assert.equal(fs.readFileSync(path.join(f.repo,'local-untracked'),'utf8'),'preserve');assert(!fs.existsSync(path.join(f.repo,'.git/MERGE_HEAD')));await capture('merge-completed-state-cleared');
  }
  else{assert.equal(f.git(['rev-parse','HEAD']),f.original);if(fs.existsSync(path.join(f.repo,'.git/MERGE_HEAD'))){await finishDialog();await click('operation-abort');await submit();assert(latest().command_succeeded);}}
  fs.unlinkSync(path.join(f.hooks,hook));await finishDialog();
 }
 passed('Owned Git/hook cancellation before and after commit reconciles actual state without retry or rollback');
 for(const f of fixtures)assert.equal(hash(path.join(f.repo,'.git/config')),f.config);
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const secret of ['PRIVATE_',topic,FILE,'検索'])assert(!payload.includes(secret),'Diagnostic leak');
 for(const op of ['merge-preview','merge','merge-abort','merge-continue','merge-finish']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const s of starts)assert(records.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request),'Unpaired request');}
 for(const step of ['review_merge','revalidate_merge','inspect_merge_result'])assert(records.some(r=>r.step===step),'Missing step '+step);
 for(const code of ['merge_stale_preview','merge_dirty_worktree','merge_target_unavailable','merge_index_locked','merge_conflicts','merge_already_committed'])assert(records.some(r=>r.failure_code===code),'Missing '+code);
 assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;passed('Private paired diagnostics, step outcomes, typed errors and cancellation; zero dropped records and unchanged configs');report.passed=true;
}catch(e){report.error=e.message;if(client)try{await capture('failure');}catch{}throw e;}
finally{
 fs.writeFileSync('work/merge-last-result',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(200);if(service.exitCode===null)service.kill();}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});
