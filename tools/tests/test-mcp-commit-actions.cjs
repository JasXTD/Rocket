// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {SUBMENU}=require('./menu-paths.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('work/mcp-fixtures/commit-actions-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/commit-actions';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report={passed:false,version,root,cases:[]},fixtures=[];
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const relay=gitRelay(root,env,'tools/helpers/productivity/git-relay.rs'),{gate,marker}=relay;
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
async function revealMessage(){for(let i=0;i<8;i++){const c=control('dialog-amend-message');if(c&&c.rect[3]-c.rect[1]>25)return;await scroll('dialog-commit-scroll');}throw Error('Amend message not reachable');}
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();relay.check(state);await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function select(id){await click('view-commits');await click('history-scope');await click('history-all');assert(active().history_ids.includes(id));await click('commit-'+id);}
async function preview(operation){await click('menu-repository');await click(SUBMENU['commit-'+operation]);await stable();await click('commit-'+operation);await until(s=>s.dialog&&!s.dialog.commit_preview_loading&&!s.busy);await stable();}
async function submit(){await click('dialog-submit');await settle();}
async function recovery(kind){await click('operation-'+kind);await settle();await stable();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.dialog.commit_applying)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}

async function run(){try{
  const exe=path.resolve('target/release/rocket-mcp.exe');cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
  const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
  fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);report.endpoint=`http://127.0.0.1:${port}/mcp`;
  const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
  for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(error){if(i===100)throw error;await delay(100);}}
  assert.equal((await client.initialize()).serverInfo.version,version);

  const amend=fixture('amend');amend.write('base.txt','staged 中文\n');amend.git(['add','base.txt']);amend.write('base.txt','unstaged untouched\n');amend.write('PRIVATE_FILE.txt','keep\n');
  await open(amend);await click('view-changes');await type('commit-message','PRIVATE_REGULAR_DRAFT keep this draft');await click('menu-terminal');await click('terminal-new');await settle();
  await preview('amend');assert(state.dialog.commit_preview_loaded);const old=amend.git(['rev-parse','HEAD']),indexBefore=hash(path.join(amend.repo,'.git/index'));
  await capture('amend-preview-wide');await resize(780,620);await revealMessage();await capture('amend-preview-narrow-terminal');
  if(process.argv.includes('--preview-only')){passed('early native preview render with terminal at wide and minimum sizes');report.passed=true;await closeWindow();await client.raw('POST','/shutdown');return;}
  await type('dialog-amend-message','PRIVATE_AMEND_MESSAGE 中文\n\n# literal body');await click('dialog-cancel');assert.equal(amend.git(['rev-parse','HEAD']),old);assert.equal(hash(path.join(amend.repo,'.git/index')),indexBefore);assert(active().draft_present);
  await resize(1480,920);await preview('amend');await revealMessage();await type('dialog-amend-message','PRIVATE_AMEND_MESSAGE 中文\n\n# literal body');
  amend.write('added-after-preview.txt','new index content\n');amend.git(['add','added-after-preview.txt']);await settle();await submit();
  assert.equal(latest().command_succeeded,false);assert(state.dialog.error.includes('stale_preview'));assert(state.dialog.first.includes('PRIVATE_AMEND_MESSAGE'));assert.equal(amend.git(['rev-parse','HEAD']),old);await capture('amend-stale-preview');
  await click('dialog-preview-refresh');await submit();assert.equal(latest().command_succeeded,true);assert.equal(amend.git(['rev-parse','refs/rocket/recovery/amend/'+old]),old);assert.equal(amend.git(['log','-1','--format=%B']),'PRIVATE_AMEND_MESSAGE 中文\n\n# literal body');assert.equal(fs.readFileSync(path.join(amend.repo,'base.txt'),'utf8'),'unstaged untouched\n');assert(active().draft_present);
  passed('amend preview/cancel, stale index refusal, exact Unicode message, recovery ref and separate draft preservation');

  // MCP runs Git with hooks off (SECURITY.md), so a value only git commit reads makes real Git fail with that value in its
  // error, which must stay out of Activity. Then the relay holds the real amend for Stop, until Stop has settled.
  amend.git(['config','commit.status','PRIVATE_GIT_PAYLOAD']);const beforeFailure=amend.git(['rev-parse','HEAD']);
  await preview('amend');await revealMessage();await type('dialog-amend-message','PRIVATE_FAILED_AMEND preserved');await submit();assert.equal(latest().command_succeeded,false);assert(state.dialog.error.includes('command_failed'));assert.equal(state.dialog.first,'PRIVATE_FAILED_AMEND preserved');assert(!JSON.stringify(latest()).includes('PRIVATE_GIT_PAYLOAD'));assert.equal(amend.git(['rev-parse','HEAD']),beforeFailure);await capture('amend-git-failure');
  amend.git(['config','--unset','commit.status']);fs.rmSync(marker,{force:true});fs.writeFileSync(gate,'before:--amend');
  await click('dialog-preview-refresh');await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.write?.operation==='amend'&&fs.existsSync(marker),'Amend held by the relay');
  await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();fs.rmSync(gate,{force:true});assert.equal(latest().command_succeeded,false);assert(latest().details.some(d=>d.includes('cancelled')));assert.equal(state.dialog.first,'PRIVATE_FAILED_AMEND preserved');assert.equal(amend.git(['rev-parse','HEAD']),beforeFailure);await click('dialog-cancel');
  passed('Git failure and actual cancellation retain the amendment message and reconcile history without retry');

  const clean=fixture('clean');clean.git(['switch','-c','source']);clean.write('folder/picked-日本語.txt','picked\n');const source=clean.commit('PRIVATE_SOURCE picked 中文');clean.git(['switch',branch]);clean.write('main.txt','main\n');clean.commit('main');clean.write('folder/unrelated.txt','keep unrelated\n');const baseTree=clean.git(['rev-parse','HEAD^{tree}']);
  await open(clean);await select(source);await preview('cherry-pick');assert(state.dialog.commit_preview_loaded);await capture('cherry-pick-preview');await submit();assert.equal(latest().command_succeeded,true);const picked=clean.git(['rev-parse','HEAD']);assert.equal(clean.git(['show','HEAD:folder/picked-日本語.txt']),'picked');
  await select(picked);await preview('revert');await capture('revert-preview');await submit();assert.equal(latest().command_succeeded,true);assert.equal(clean.git(['rev-parse','HEAD^{tree}']),baseTree);assert.equal(fs.readFileSync(path.join(clean.repo,'folder/unrelated.txt'),'utf8'),'keep unrelated\n');
  passed('selected cherry-pick and revert produce correct commits and preserve unrelated untracked files');

  for(const operation of ['cherry-pick','revert']){
    const f=fixture('conflict-'+operation);let target;
    if(operation==='cherry-pick'){f.git(['switch','-c','source']);f.write('base.txt','source\n');target=f.commit('PRIVATE_SOURCE conflict');f.git(['switch',branch]);}
    else {f.write('base.txt','source\n');target=f.commit('PRIVATE_SOURCE conflict');}
    f.write('base.txt','main\n');f.commit('main');const previous=f.git(['rev-parse','HEAD']);await open(f);
    for(const kind of ['abort','continue']){
      await select(target);await preview(operation);await submit();assert.equal(latest().command_succeeded,false);assert(latest().details.some(d=>d.includes('conflicts_remain')));assert(!active().operation_state.idle);assert(!control('operation-skip'),'one commit has nothing to skip to');
      if(kind==='abort'){
        await resize(780,620);await capture(operation+'-conflict-narrow');await recovery('continue');assert(state.dialog.error.includes('conflicts_remain'));assert.equal(control('dialog-submit').enabled,false);await click('dialog-cancel');
        await recovery('abort');assert(state.dialog.commit_preview_loaded);await capture(operation+'-abort-preview');f.write('base.txt','resolution changed after preview\n');await submit();assert(state.dialog.error.includes('stale_preview'));await click('dialog-preview-refresh');await submit();
      }else if(kind==='continue'){
        await resize(1480,920);f.write('base.txt','resolved 中文\n');await refresh();await click('change-unstaged-'+Buffer.from('base.txt').toString('hex'));await click('conflict-resolved');await recovery('continue');assert(state.dialog.commit_preview_loaded);await capture(operation+'-continue-preview');await submit();
      }
      assert.equal(latest().command_succeeded,true);assert(active().operation_state.idle);
      if(kind!=='continue'){assert.equal(f.git(['rev-parse','HEAD']),previous);assert.equal(fs.readFileSync(path.join(f.repo,'base.txt'),'utf8'),'main\n');}
      else assert.equal(fs.readFileSync(path.join(f.repo,'base.txt'),'utf8'),'resolved 中文\n');
    }
    passed(operation+' conflict preview, blocked Continue, stale resolution protection, Abort, no Skip on one commit, UI Mark resolved and Continue');
  }

  const guards=fixture('guards');guards.git(['switch','-c','source']);guards.write('guarded/PRIVATE_PAYLOAD.txt','incoming\n');const guarded=guards.commit('PRIVATE_SOURCE guarded');guards.git(['switch',branch]);
  fs.mkdirSync(path.join(guards.repo,'.git/info'),{recursive:true});fs.writeFileSync(path.join(guards.repo,'.git/info/exclude'),'guarded/\n');guards.write('guarded/PRIVATE_PAYLOAD.txt','keep ignored\n');await open(guards);await select(guarded);await preview('cherry-pick');assert(state.dialog.error.includes('local_file_collision'));assert.equal(control('dialog-submit').enabled,false);await capture('ignored-file-refusal');await click('dialog-cancel');
  assert.equal(fs.readFileSync(path.join(guards.repo,'guarded/PRIVATE_PAYLOAD.txt'),'utf8'),'keep ignored\n');fs.unlinkSync(path.join(guards.repo,'guarded/PRIVATE_PAYLOAD.txt'));
  guards.write('base.txt','dirty\n');await preview('cherry-pick');assert(state.dialog.error.includes('dirty_worktree'));await click('dialog-cancel');guards.write('base.txt','base\n');await preview('cherry-pick');assert(state.dialog.commit_preview_loaded);
  const lock=path.join(guards.repo,'.git/index.lock');fs.writeFileSync(lock,'existing fixture lock');await submit();assert.equal(latest().command_succeeded,false);assert(state.dialog.error.includes('index_locked'));assert.equal(fs.readFileSync(lock,'utf8'),'existing fixture lock');await capture('index-lock-refusal');fs.unlinkSync(lock);await click('dialog-preview-refresh');await submit();assert.equal(latest().command_succeeded,true);
  passed('ignored-file and dirty-worktree refusals preserve bytes; index-lock failure is clear and leaves the existing lock intact');

  const empty=fixture('empty');empty.write('added','already applied');const emptyTarget=empty.commit('PRIVATE_SOURCE empty replay');await open(empty);await select(emptyTarget);await preview('cherry-pick');await submit();assert(latest().details.some(d=>d.includes('empty_change')));await capture('empty-cherry-pick');await recovery('abort');await submit();assert(active().operation_state.idle);assert.equal(empty.git(['rev-parse','HEAD']),emptyTarget);
  passed('empty cherry-pick is reported clearly and dropped only on explicit reviewed action');

  const merge=fixture('mainline');merge.git(['switch','-c','source']);merge.write('side','side');merge.commit('side');merge.git(['switch',branch]);merge.write('main','main');merge.commit('main');merge.git(['merge','--no-edit','source']);const merged=merge.git(['rev-parse','HEAD']);await open(merge);await select(merged);await preview('revert');assert.equal(state.dialog.error??null,null);assert(state.dialog.commit_preview_loaded);assert.equal(control('dialog-submit').enabled,true);await click('dialog-mainline-2');await until(s=>s.dialog&&s.dialog.commit_preview_loaded&&!s.busy);await stable();await click('dialog-mainline-1');await until(s=>s.dialog&&s.dialog.commit_preview_loaded&&!s.busy);await stable();await capture('merge-mainline-preview');await submit();assert.equal(latest().command_succeeded,true);assert(!fs.existsSync(path.join(merge.repo,'side')));assert(fs.existsSync(path.join(merge.repo,'main')));
  passed('merge revert opens reviewed on the side it merged into, can change side, and applies the displayed change');

  for(const f of fixtures)assert.equal(hash(path.join(f.repo,'.git/config')),f.config,'Repository config changed');assert.equal(fs.readFileSync(path.join(root,'empty.config'),'utf8'),'');
  await closeWindow();await client.raw('POST','/shutdown');
  const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(line=>JSON.parse(line))));
  assert(records.length>0);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const secret of ['PRIVATE_SOURCE','PRIVATE_AMEND_MESSAGE','PRIVATE_REGULAR_DRAFT','PRIVATE_GIT_PAYLOAD','PRIVATE_FAILED_AMEND','PRIVATE_FILE','PRIVATE_PAYLOAD'])assert(!payload.includes(secret),'Diagnostic payload leak');
  for(const op of ['amend','cherry-pick','revert','cherry-pick-continue','cherry-pick-abort','revert-continue','revert-abort'])assert(records.some(r=>r.operation===op&&r.event==='request_end'),'Missing request span '+op);
  for(const code of ['stale_preview','conflicts_remain','empty_change','command_failed','local_file_collision','dirty_worktree','index_locked'])assert(records.some(r=>r.failure_code===code),'Missing failure code '+code);
  passed('all operation spans and failure codes present; zero dropped events, private payloads absent and config preserved');report.log_records=records.length;report.passed=true;
}catch(error){if(client)try{await capture('failure');}catch{}throw error;}
finally{
  fs.rmSync(gate,{force:true});fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
  if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}
  if(client)await client.finish();
}}
run().catch(error=>{console.error(error);process.exitCode=1;});
