// Focused native overview acceptance. No network, personal config or real repos.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const {RocketHttpClient} = require('../mcp/mcp-http-client.cjs');
const {gitRelay} = require('./git-relay.cjs');
const root = path.resolve('work/mcp-fixtures/diff-overview-' + crypto.randomUUID());
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/overview';
const exe = path.resolve(process.env.ROCKET_TEST_MCP_EXE || 'target/release/rocket-mcp.exe');
const report = {passed:false, root, cases:[], timings:{}};
fs.mkdirSync(root, {recursive:true});fs.mkdirSync(path.join(root, 'empty-template'));
fs.writeFileSync(path.join(root, 'empty.config'), '');
fs.writeFileSync(path.join(root, 'rocket-private-state'), 'synthetic-content-only-v1\n');
const env = {...process.env};
for (const k of Object.keys(env)) if (/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k)) delete env[k];
Object.assign(env, {GIT_CONFIG_GLOBAL:path.join(root,'empty.config'), GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),
 GIT_CONFIG_NOSYSTEM:'1', GIT_ALLOW_PROTOCOL:'file', GIT_TERMINAL_PROMPT:'0', GIT_OPTIONAL_LOCKS:'0', GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const realGit = cp.execFileSync('where.exe', ['git.exe'], {encoding:'utf8',windowsHide:true}).trim().split(/\r?\n/)[0];
const relay = gitRelay(root, env, 'tools/helpers/patch-series/git-relay.rs'), {gate, marker} = relay;
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const delay = ms => new Promise(r => setTimeout(r,ms));
let client, service, state;
const tab = () => state.tabs?.find(t => t.id === state.active_tab);
const control = id => state.controls?.find(c => c.id === id);
const editor = () => tab().conflict_editor;
const overview = () => tab().diff_tools.overview;
function fixture(name) {
 const repo = path.join(root,name);fs.mkdirSync(repo);
 const git = args => cp.execFileSync(realGit,['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:'pipe',maxBuffer:32*1024*1024}).trimEnd();
 const write = (name,text) => {const dest=path.join(repo,name);fs.mkdirSync(path.dirname(dest),{recursive:true});fs.writeFileSync(dest,text)};
 const commit = () => {git(['add','-A']);git(['commit','-m','Synthetic overview fixture'])};
 git(['init','-b',branch]);
 for(const [k,v] of [['user.name','Synthetic Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false'],['core.hooksPath',path.join(root,'empty-template')]]) git(['config',k,v]);
 write('.git/rocket-ui-fixture',branch);return {repo,git,write,commit};
}
function preserved(f,names) {return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),files:Object.fromEntries(names.map(n=>[n,hash(path.join(f.repo,n))]))}}
function pass(message) {report.cases.push(message);console.log('PASS '+message);fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2))}
async function until(test,label='state',timeout=60000) {
 const at=performance.now();for(;;){state=await client.tool('rocket_state');if(test(state))return;assert(performance.now()-at<timeout,label+' timed out '+JSON.stringify({busy:state.busy,notice:state.notice,dialog:state.dialog?.kind}));await delay(60)}
}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading||t.search?.running),'settled')}
async function stable(){let previous='',same=0;await until(s=>{const current=JSON.stringify(s.controls);same=current===previous?same+1:0;previous=current;return !s.input_pending&&same>=2},'geometry')}
async function click(id,wait=true){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(wait)await settle();else await until(s=>!s.input_pending)}
async function type(id,text){await stable();assert(control(id)?.enabled,'Missing '+id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle()}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle()}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(result.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2))}
async function open(f){await client.tool('rocket_open',{path:f.repo});await settle();relay.check(state);await resize(1480,920);if(control('console-close'))await click('console-close')}
async function change(name){await click('view-changes');await click('change-unstaged-'+Buffer.from(name).toString('hex'));await stable();assert.equal(tab().patch_error,null)}
async function close(){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>s.window==='closed'||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await click('dialog-submit',false);await until(s=>s.window==='closed')}}
async function diffCases(){
 const f=fixture('PRIVATE_DIFF_資料'),file='src/資料 🚀 é.rs',small='short.rs';
 const base=Array.from({length:1000},(_,i)=>'// base line '+i+' 文\n').join('');
 const edited=Array.from({length:1000},(_,i)=>i%90<28?'// PRIVATE_CHANGED '+i+' 🚀\n':'// base line '+i+' 文\n').join('');
 f.write(file,base);f.write(small,'old\n');f.write('keep.txt','base\n');f.write('binary.bin',Buffer.from([0,1,2]));f.commit();
 f.write(file,edited);f.write(small,'new\n');f.write('keep.txt','PRIVATE_STAGED\n');f.git(['add','keep.txt']);f.write('keep.txt','PRIVATE_UNSTAGED\n');f.write('untracked.txt','PRIVATE_UNTRACKED\n');f.write('binary.bin',Buffer.from([0,3,4]));
 const names=[file,small,'keep.txt','untracked.txt','binary.bin'],before=preserved(f,names);
 await open(f);await change(file);assert(overview().visible);assert(overview().marked_rows>100);const count=overview().rows;
 const selected=tab().selected_lines;await click('diff-overview');await until(()=>overview().offset>100);assert.equal(tab().selected_lines,selected);await capture('01-unified');
 await click('diff-split');await stable();assert(overview().visible);assert(overview().rows<count);await click('diff-overview');await until(()=>overview().offset>100);await capture('02-split');
 await resize(780,660);await click('toolbar-console');await stable();assert(overview().visible);const r=control('diff-overview').rect;assert(r[0]>=0&&r[2]<=780&&r[1]>=0&&r[3]<=660);await click('diff-overview');await capture('03-compact-console');await click('console-close');await resize(1480,920);
 assert.deepEqual(preserved(f,names),before);pass('Unified/split overview, Unicode path, compact window with console and navigation preserve HEAD/index/files and patch selection');
 await click('diff-unified');await type('diff-search','PRIVATE_CHANGED 900');await until(()=>!tab().diff_tools.searching&&tab().diff_tools.matches===1);assert(overview().offset>100);await type('diff-search','');assert.deepEqual(preserved(f,names),before);pass('Search navigation and overview share the same displayed row coordinates');
 const prior=overview().rows;f.write(file,base.replace('// base line 50 文','// PRIVATE_EXTERNAL changed'));
 await until(()=>!state.busy&&overview().rows!==prior,'automatic external refresh',45000);assert(overview().rows<30);assert.equal(overview().visible,false);assert(!control('diff-overview'));assert.equal(hash(path.join(f.repo,'.git/index')),before.index);
 await change(small);assert(!overview().visible);assert(!control('diff-overview'));await change('binary.bin');assert(!overview().visible);assert(!control('diff-overview'));assert.equal(overview().marked_rows,0);pass('Automatic external refresh rebuilds markers; short/binary files remove stale overview controls');
 fs.writeFileSync(gate,'before:diff');await click('change-unstaged-'+Buffer.from(file).toString('hex'),false);await until(()=>fs.existsSync(marker)&&!state.input_pending,'read gate');
 const pids=fs.readFileSync(marker,'utf8').trim().split(/\s+/).map(Number);await stable();assert(control('diff-cancel')?.enabled);const at=performance.now();await client.tool('rocket_click',{target:'diff-cancel',expected_tab:state.active_tab});
 await until(()=>pids.every(pid=>{try{process.kill(pid,0);return false}catch{return true}}),'owned Git stopped');report.timings.diff_stop_ms=Math.round(performance.now()-at);assert(report.timings.diff_stop_ms<5000);
 fs.rmSync(gate,{force:true});fs.rmSync(marker,{force:true});await settle();report.timings.diff_settled_ms=Math.round(performance.now()-at);assert.match(tab().patch_error,/cancel/i);assert(!control('diff-overview'));
 await change(small);await change(file);assert.equal(tab().patch_error,null);assert.equal(overview().visible,false);assert.equal(hash(path.join(f.repo,'.git/index')),before.index);pass('Cancelled diff read stops its Git process, clears the overview and retries with fresh contents');
}
async function conflictCases(){
 const f=fixture('PRIVATE_CONFLICT_資料'),file='src/解決 🚀.rs';
 const content=(a,b)=>Array.from({length:300},(_,i)=>i===12?a:i===230?b:'// common '+i+' 文').join('\n')+'\n';
 f.write(file,content('base one','base two'));f.write('keep.txt','PRIVATE_KEEP\n');f.commit();
 f.git(['switch','-c','rocket-ui-test/theirs']);f.write(file,content('PRIVATE_THEIRS one','PRIVATE_THEIRS two'));f.commit();f.git(['switch',branch]);f.write(file,content('PRIVATE_OURS one','PRIVATE_OURS two'));f.commit();assert.throws(()=>f.git(['merge','rocket-ui-test/theirs']));
 f.write('untracked.txt','PRIVATE_UNTRACKED\n');const names=[file,'keep.txt','untracked.txt'],before=preserved(f,names),original=fs.readFileSync(path.join(f.repo,file),'utf8');
 await open(f);await change(file);await click('conflict-open');await until(()=>control('conflict-overview')?.enabled&&editor().overview.visible,'conflict overview rendered');assert(editor().three_columns);const draft=editor().draft;
 await click('conflict-overview');await until(()=>editor().scroll_y.every(y=>y>100));assert.equal(editor().draft,draft);assert.deepEqual(preserved(f,names),before);
 await click('conflict-marker-1');await until(()=>editor().selected===1&&editor().scroll_y.every(y=>y>2000));await click('conflict-marker-0');await until(()=>editor().selected===0&&editor().scroll_y.every(y=>y<500));await capture('04-conflict-linked');
 await click('conflict-linked');const y=editor().scroll_y.slice();await click('conflict-overview');await until(()=>editor().scroll_y[1]>1000);assert(Math.abs(editor().scroll_y[0]-y[0])<2&&Math.abs(editor().scroll_y[2]-y[2])<2);assert.equal(editor().draft,draft);await click('conflict-linked');pass('Conflict overview and exact marker clicks preserve the draft and respect linked/unlinked panes');
 await resize(780,660);await click('toolbar-console');await capture('05-conflict-compact');assert(editor().overview.visible);await click('conflict-overview');assert.equal(editor().draft,draft);await click('console-close');await resize(1480,920);
 await click('conflict-marker-0');await click('conflict-ours');await click('conflict-theirs');assert.equal(editor().blocks,0);const resolved=editor().draft;assert(editor().overview.marked_rows>0);await click('conflict-save');await click('dialog-cancel');assert.equal(editor().draft,resolved);assert.deepEqual(preserved(f,names),before);pass('Compact conflict view, resolved markers and cancelled save preserve local changes');
 f.write(file,'PRIVATE_EXTERNAL conflict edit\n');await click('conflict-save');await click('dialog-submit');assert.match(editor().error,/changed outside/);assert.equal(editor().draft,resolved);assert.equal(fs.readFileSync(path.join(f.repo,file),'utf8'),'PRIVATE_EXTERNAL conflict edit\n');assert.equal(hash(path.join(f.repo,'.git/index')),before.index);await capture('06-external-edit-refused');pass('External conflict edits are refused and the resolution draft remains available');
 // Restore only this synthetic fixture, explicitly reload and verify the unchanged save/backup path.
 f.write(file,original);await click('conflict-reload');await click('dialog-submit');await until(()=>!state.busy&&editor().blocks===2);await click('conflict-marker-0');await click('conflict-ours');await click('conflict-theirs');const saved=editor().draft;
 await click('conflict-save');await click('dialog-submit');assert.equal(fs.readFileSync(path.join(f.repo,file),'utf8'),saved);assert.equal(hash(path.join(f.repo,'.git/index')),before.index);
 const latest=state.activities.at(-1);assert(latest.command_succeeded,JSON.stringify(latest));
 const backup=latest.details.find(s=>s.startsWith('Recovery copies retained at '))?.match(/^Recovery copies retained at (.+)\. If publication/);assert(backup,'missing recovery copies '+JSON.stringify(latest.details));for(const name of ['observed-original','replaced-original'])assert.equal(fs.readFileSync(path.join(backup[1],name),'utf8'),original);pass('Reviewed save keeps the original recovery copies and leaves the index unchanged');
}
(async()=>{try{
 report.executable_sha256=hash(exe);
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));report.port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${report.port}\n${root}\n`);
 const fd=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>100)throw e;await delay(100)}}await client.initialize();await until(()=>true);
 await diffCases();await conflictCases();await close();
 const events=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(x=>/^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x=>fs.readFileSync(path.join(root,n,x),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(events.length>0);assert(events.every(e=>!e.dropped_before));assert(!JSON.stringify(events).includes('PRIVATE_'));assert(events.some(e=>e.outcome==='cancelled'));for(const start of events.filter(e=>e.event==='request_start'))assert(events.some(e=>e.event==='request_end'&&e.session===start.session&&e.request===start.request));report.log_records=events.length;pass('Diagnostics retain cancellation/errors and paired requests without fixture contents or dropped events');report.passed=true;
}catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client)try{await capture('failure')}catch{}}
finally{
 fs.rmSync(gate,{force:true});if(client)try{if(state?.window!=='closed')await close();await client.raw('POST','/shutdown');await client.finish()}catch(e){report.cleanup_error=String(e)}
 if(service){for(let i=0;i<100&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null){report.forced_cleanup=true;cp.execFileSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true})}}
 const token=path.join(profile,'access-token');fs.rmSync(token,{force:true});report.cleanup={exit:service?.exitCode,token_removed:!fs.existsSync(token)};if(report.cleanup_error||report.forced_cleanup||service?.exitCode!==0){report.passed=false;process.exitCode=1}
 fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.mkdirSync('work/results/diff-overview',{recursive:true});fs.writeFileSync('work/results/diff-overview/native-mcp.json',JSON.stringify(report,null,2));console.log('Evidence: '+root);
}})();
