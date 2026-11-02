// Focused native acceptance over authenticated loopback MCP. All data is synthetic.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('work/mcp-fixtures/fine-diff-'+crypto.randomUUID()),profile=path.join(root,'http-profile');
const exe=path.resolve('target/release/rocket-mcp.exe'),branch='rocket-ui-test/fine-diff';
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const realGit=cp.execFileSync('where.exe',['git.exe'],{encoding:'utf8',windowsHide:true}).trim().split(/\r?\n/)[0];
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'),delay=ms=>new Promise(r=>setTimeout(r,ms));
const relay=gitRelay(root,env,'tools/helpers/patch-series/git-relay.rs'),{gate,marker}=relay;
const report={passed:false,root,cases:[],timings:{},executable_sha256:hash(exe)};
let client,service,state;
const tab=()=>state.tabs.find(t=>t.id===state.active_tab),control=id=>state.controls.find(c=>c.id===id),wf=()=>state.dialog?.workflow,latest=()=>state.activities.at(-1);
function fixture(name){
  const repo=path.join(root,name);fs.mkdirSync(repo);
  const git=a=>cp.execFileSync(realGit,['-C',repo,...a],{env,windowsHide:true,encoding:'utf8',stdio:'pipe'}).trimEnd();
  const write=(name,bytes)=>{fs.mkdirSync(path.dirname(path.join(repo,name)),{recursive:true});fs.writeFileSync(path.join(repo,name),bytes);};
  const commit=m=>{git(['add','-A']);git(['commit','-m',m]);return git(['rev-parse','HEAD']);};
  git(['init','-b',branch]);for(const[k,v]of[['user.name','Synthetic Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false'],['core.hooksPath',path.join(root,'empty-template')]])git(['config',k,v]);
  write('.git/rocket-ui-fixture',branch);write('keep.txt','base\n');commit('PRIVATE_BASE');return{repo,git,write,commit};
}
function pass(s){report.cases.push(s);console.log('PASS '+s);fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));}
async function until(test,label='state',timeout=90000){const end=performance.now()+timeout;for(;;){state=await client.tool('rocket_state');if(test(state))return;assert(performance.now()<end,label+' timeout '+JSON.stringify({busy:state.busy,dialog:state.dialog,notice:state.notice}));await delay(70);}}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading||t.search?.running),'settled');}
async function stable(){let last='',same=0;await until(s=>{const next=JSON.stringify(s.controls);same=next===last?same+1:0;last=next;return !s.input_pending&&same>=2;},'geometry');}
async function reveal(id){await stable();const visible=()=>control(id)&&control(id).rect[3]-control(id).rect[1]>8;if(visible())return;const region=state.dialog?'dialog-stash-apply-scroll':'changes-list-scroll';if(control(region)){for(let i=0;i<16&&!visible();i++){await client.tool('rocket_scroll',{target:region,direction:'down',pixels:220,expected_tab:state.active_tab});await stable();}}assert(visible(),'Missing '+id+' '+JSON.stringify(state.controls.map(c=>c.id)));}
async function click(id,wait=true){await reveal(id);assert(control(id)?.enabled,'Disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(wait)await settle();}
async function type(id,text){await reveal(id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle();}
async function resize(width,height){await stable();await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending,'native resize',10000);await settle();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function open(f){if(state?.dialog)await click('dialog-cancel');await settle();await client.tool('rocket_open',{path:f.repo});await settle();relay.check(state);await resize(1480,920);if(control('console-close'))await click('console-close');}
async function refresh(){await click('refresh');}
async function change(name,staged=false){await click('view-changes');await click('change-'+(staged?'staged-':'unstaged-')+Buffer.from(name).toString('hex'));await stable();assert.equal(tab().patch_error,null);}
const lineControls=()=>state.controls.filter(c=>/^diff-line-\d+$/.test(c.id)).sort((a,b)=>Number(a.id.split('-').at(-1))-Number(b.id.split('-').at(-1)));
async function showStash(f){await open(f);await click('view-stashes');await click('stash-select-0');await click('stash-apply-selected');assert(wf()?.saved_selection?.files,state.dialog?.error);}
function saved(name){const i=wf().saved_selection.files.findIndex(f=>f.path===name);assert(i>=0);return{i,ids:wf().saved_selection.files[i].hunk_lines[0]};}
async function choosePair(name){const{i,ids}=saved(name);assert.equal(ids.length,3);if(!control(`dialog-stash-line-${i}-${ids[0]}`))await click(`dialog-stash-show-${i}`);await click(`dialog-stash-line-${i}-${ids[0]}`);await click(`dialog-stash-line-${i}-${ids[1]}`);assert.equal(wf().saved_selection.selected[0].lines.length,2);return{i,ids};}
// Each change of the selection is checked by itself; wait for that check.
async function review(){await until(s=>!s.busy&&!s.input_pending&&(s.dialog?.workflow?.preview||s.dialog?.error),'automatic stash check');assert(wf().preview,state.dialog?.error);assert.equal(wf().preview.kind,'stash-apply-selection');}
async function cancelAt(start,command,stop,label){fs.rmSync(marker,{force:true});fs.writeFileSync(gate,command);await click(start,false);await until(s=>fs.existsSync(marker)&&!s.input_pending,'gate');const pids=fs.readFileSync(marker,'utf8').trim().split(/\s+/).map(Number);await reveal(stop);const began=performance.now();await client.tool('rocket_click',{target:stop,expected_tab:state.active_tab});await until(s=>!s.input_pending);report.timings[label+'_feedback_ms']=Math.round(performance.now()-began);await until(()=>pids.every(p=>{try{process.kill(p,0);return false}catch{return true}}),'Git stopped');report.timings[label+'_stop_ms']=Math.round(performance.now()-began);fs.rmSync(gate,{force:true});await settle();}
async function stashCases(){
  const f=fixture('PRIVATE_stash'),name='解析 🚀.rs';f.write(name,'before\nold\nafter\n');f.commit('PRIVATE_before');f.write(name,'before\nnew\nextra\nafter\n');f.git(['stash','push','-m','PRIVATE_stash']);f.write('keep.txt','staged unrelated\n');f.git(['add','keep.txt']);f.write('local.txt','PRIVATE_UNTRACKED\n');
  const index=hash(path.join(f.repo,'.git/index')),stash=f.git(['rev-parse','refs/stash']),head=f.git(['rev-parse','HEAD']);
  const unchanged=()=>{assert.equal(hash(path.join(f.repo,'.git/index')),index);assert.equal(f.git(['rev-parse','refs/stash']),stash);assert.equal(f.git(['rev-parse','HEAD']),head);assert.equal(fs.readFileSync(path.join(f.repo,'local.txt'),'utf8'),'PRIVATE_UNTRACKED\n');};
  await showStash(f);const{i,ids}=await choosePair(name);await review();await capture('stash-lines-wide');await resize(780,620);await capture('stash-lines-compact');assert(control('dialog-submit').enabled);
  await click(`dialog-stash-line-${i}-${ids[2]}`);assert.equal(wf().saved_selection.selected[0].lines.length,3);await review();await click(`dialog-stash-line-${i}-${ids[2]}`);assert.equal(wf().saved_selection.selected[0].lines.length,2);await review();await click('dialog-cancel');unchanged();assert.equal(fs.readFileSync(path.join(f.repo,name),'utf8'),'before\nold\nafter\n');pass('Each line choice is checked at once; compact layout and Close preserve all data');
  await showStash(f);await choosePair(name);await review();f.write(name,'PRIVATE_EXTERNAL\n');await click('dialog-submit');assert(!latest().command_succeeded);assert.equal(fs.readFileSync(path.join(f.repo,name),'utf8'),'PRIVATE_EXTERNAL\n');unchanged();await click('dialog-preview-refresh');assert(!wf().preview);assert.match(state.dialog.error,/patch_conflict|changed|stale/i);await capture('stash-stale-error');await click('dialog-cancel');f.write(name,'before\nold\nafter\n');pass('External edits reject stale stash saves and show a clear conflict');
  // A change of the selection starts the check that is stopped; choosing the line again checks the pair.
  await showStash(f);await choosePair(name);await cancelAt(`dialog-stash-line-${i}-${ids[1]}`,'before:apply','dialog-stash-stop','preview');assert(!wf().preview);assert.match(wf().notice,/cancel/i);unchanged();await click(`dialog-stash-line-${i}-${ids[1]}`);await review();await cancelAt('dialog-submit','before:check-attr','cancel-operation','apply');assert(!latest().command_succeeded);unchanged();assert.equal(fs.readFileSync(path.join(f.repo,name),'utf8'),'before\nold\nafter\n');pass('Cancel during stash review and save preparation stops owned Git processes without writes');
  await click('dialog-preview-refresh');await review();await click('dialog-submit');assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(fs.readFileSync(path.join(f.repo,name),'utf8'),'before\nnew\nafter\n');unchanged();
  const recovery=latest().details.find(d=>d.startsWith('Working-file recovery: '));assert(recovery);const backup=recovery.slice('Working-file recovery: '.length);
  assert.equal(fs.readFileSync(path.join(backup,'replaced-original'),'utf8'),'before\nold\nafter\n');
  assert.equal(fs.readFileSync(path.join(backup,'observed-original'),'utf8'),'before\nold\nafter\n');
  await capture('stash-applied');await click('dialog-cancel');pass('Applying selected replacement lines keeps the extra line unapplied, preserves stash/index and verifies exact recovery bytes');
}
async function stageCases(){
  const f=fixture('PRIVATE_stage'),newName='新規 🚀.dart',gone='消えた.txt',old='旧名.rs',renamed='新名.rs';const base=Array.from({length:30},(_,i)=>`line ${i}\n`).join('');f.write(gone,'one\ntwo\nthree\n');f.write(old,base);f.commit('PRIVATE_stage_base');f.git(['mv',old,renamed]);f.write(renamed,base.replace('line 5\n','changed five\n'));f.git(['add',renamed]);f.write(newName,'first\nsecond 世界\nthird\n');fs.unlinkSync(path.join(f.repo,gone));await open(f);
  await change(newName);let lines=lineControls();assert.equal(lines.length,3);await click(lines[1].id);await capture('new-selected-lines');await click('diff-apply-selection');assert(latest().command_succeeded);assert.equal(f.git(['show',':'+newName]),'second 世界');assert.equal(fs.readFileSync(path.join(f.repo,newName),'utf8'),'first\nsecond 世界\nthird\n');
  await change(newName,true);await click(lineControls()[0].id);await click('diff-apply-selection');assert.equal(f.git(['ls-files','--',newName]),'');await change(newName);await click(state.controls.find(c=>c.id.startsWith('diff-hunk-')).id);await click('diff-apply-selection');await change(newName,true);await click(lineControls()[1].id);await click('diff-apply-selection');assert.equal(f.git(['show',':'+newName]),'first\nthird');pass('New UTF-8 files stage selected lines and unstage selected additions with working bytes intact');
  await change(gone);lines=lineControls();assert.equal(lines.length,3);await click(lines[1].id);await click('diff-apply-selection');assert.equal(f.git(['show',':'+gone]),'one\nthree');await change(gone);await click(state.controls.find(c=>c.id.startsWith('diff-hunk-')).id);await click('diff-apply-selection');assert.equal(f.git(['ls-files','--',gone]),'');await change(gone,true);await click(lineControls()[1].id);await click('diff-apply-selection');assert.equal(f.git(['show',':'+gone]),'two');assert(!fs.existsSync(path.join(f.repo,gone)));pass('Deleted-file partial stage and unstage preserve the deleted working file');
  await change(renamed,true);lines=lineControls();assert.equal(lines.length,2);for(const l of lines)await click(l.id);await capture('rename-selected-lines');await click('diff-apply-selection');assert.equal(f.git(['show',':'+renamed]),base.trimEnd());assert.equal(f.git(['ls-files','--',old]),'');assert.equal(fs.readFileSync(path.join(f.repo,renamed),'utf8'),base.replace('line 5\n','changed five\n'));pass('Unstaging rename text preserves the new index path and working edits');
}
async function syntaxCases(){
  const f=fixture('PRIVATE_syntax');const samples={
    '文/main.dart':['Dart',"/* outer /* inner */\n   end */\nfinal s = r'''世界 🚀\n// literal''';\n"],
    '文/main.ex':['Elixir','defmodule Rocket do\n  @doc """\n  # 世界\n  """\n  def ready, do: ~r{[a-z]+#literal}i\nend\n'],
    '文/main.erl':['Erlang','start() ->\n  [$%, $", $\\\'],\n  {\'quoted atom\', "世界"}. % comment\n'],
    '文/main.m':['MATLAB',"%{\ncomment\n%}\ny = matrix';\ns = 'it''s 世界';\n"],
    '文/main.F90':['Fortran',"PROGRAM rocket\n  PRINT *, '世界' ! comment\nEND PROGRAM rocket\n"],
    '文/raw.cpp':['C++','auto s = R"tag(世界 "quotes"\n/* literal */)tag";\n'],
  };
  for(const name of Object.keys(samples))f.write(name,'before\n');f.commit('PRIVATE_syntax_base');for(const[name,[,body]]of Object.entries(samples))f.write(name,body);const index=hash(path.join(f.repo,'.git/index'));await open(f);
  for(const[name,[language]]of Object.entries(samples)){await change(name);assert.equal(tab().patch_files[0].syntax_language,language);assert(tab().patch_files[0].syntax_spans>0);assert(!tab().patch_files[0].syntax_limited);await capture('syntax-'+language.replaceAll('+','p'));}
  await change('文/main.dart');await click('diff-split');await type('diff-search','世界');await until(()=>!tab().diff_tools.searching&&tab().diff_tools.matches===1);await capture('syntax-split-search');await click('diff-syntax');assert(!tab().diff_tools.syntax);await click('diff-syntax');await click('diff-unified');await type('diff-search','');assert.equal(hash(path.join(f.repo,'.git/index')),index);pass('Dart, Elixir, Erlang, MATLAB, Fortran and C++ render; Unicode search, split view and syntax toggle preserve data');
  f.write('文/main.dart',"final external = '世界 changed';\n");await refresh();await change('文/main.dart');await type('diff-search','external');await until(()=>!tab().diff_tools.searching&&tab().diff_tools.matches===1);await type('diff-search','');assert.equal(tab().patch_files[0].syntax_language,'Dart');assert.equal(hash(path.join(f.repo,'.git/index')),index);pass('External file edits refresh highlighted content and search results');
  // Changing the file invalidates its cached diff. Cancel a real Git read before
  // highlighting; retry must display current data rather than a late result.
  await change('文/main.ex');f.write('文/main.dart',"final retried = '世界 retry';\n");await refresh();
  await cancelAt('change-unstaged-'+Buffer.from('文/main.dart').toString('hex'),'before:diff','diff-cancel','diff');assert.match(tab().patch_error,/cancel/i);await change('文/main.ex');await change('文/main.dart');await type('diff-search','retried');await until(()=>!tab().diff_tools.searching&&tab().diff_tools.matches===1);assert.equal(hash(path.join(f.repo,'.git/index')),index);pass('Cancelled highlighted diff stops Git; retry shows fresh content without stale completion');
}
async function closeWindow(){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>s.window==='closed'||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await click('dialog-submit',false);await until(s=>s.window==='closed');}}
(async()=>{try{
  
  cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
  const fd=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
  for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>100)throw e;await delay(100);}}report.version=(await client.initialize()).serverInfo.version;await until(()=>true);
  const only=process.argv.find(a=>a.startsWith('--only='))?.slice(7);report.only=only??null;
  if(!only||only==='stash')await stashCases();if(!only||only==='stage')await stageCases();if(!only||only==='syntax')await syntaxCases();await closeWindow();
  const rows=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(x=>/^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x=>fs.readFileSync(path.join(root,n,x),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
  assert(rows.length);assert(rows.every(r=>!r.dropped_before));assert(!JSON.stringify(rows).includes('PRIVATE_'));if(!only||only==='stash')assert(rows.some(r=>r.operation==='stash-apply-selection'&&r.event==='step_end'));for(const s of rows.filter(r=>r.event==='request_start'))assert(rows.some(e=>e.event==='request_end'&&e.request===s.request&&e.session===s.session));report.log_records=rows.length;pass('Diagnostics have paired requests, operation steps and zero dropped events, without fixture payloads');report.passed=true;
}catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client)try{await capture('failure');}catch{}}
finally{
  fs.rmSync(gate,{force:true});if(client)try{if(state?.window!=='closed')await closeWindow();await client.raw('POST','/shutdown');await client.finish();}catch(e){report.cleanup_error=String(e);}
  if(service){for(let i=0;i<100&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null){report.forced_cleanup=true;cp.execFileSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});}}
  const token=path.join(profile,'access-token');fs.rmSync(token,{force:true});report.cleanup={exit:service?.exitCode,token_removed:!fs.existsSync(token)};if(report.cleanup_error||report.forced_cleanup||service?.exitCode!==0){report.passed=false;process.exitCode=1;}
  fs.mkdirSync('work/results/fine-diff-0.85',{recursive:true});fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync('work/results/fine-diff-0.85/native-mcp.json',JSON.stringify(report,null,2));console.log('Evidence: '+root);
}})();
