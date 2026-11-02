// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/tags-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/tags';
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
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function submit(){await click('dialog-submit');await settle();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}

const FILE='src/検索/結果 🚀 é.rs';
function setup(name){const f=fixture(name);f.write(FILE,'base\n');f.write('.gitignore','ignored/\n');f.base=f.commit('PRIVATE_BASE 中文');f.write(FILE,'second\n');f.tip=f.commit('PRIVATE_SECOND 🚀');return f;}
function snapshot(f){return{head:f.git(['rev-parse','HEAD']),index:f.git(['ls-files','--stage','-z']),file:fs.readFileSync(path.join(f.repo,FILE),'utf8'),config:hash(path.join(f.repo,'.git/config'))};}
function details(){return state.dialog?.tags?.details;}
async function browse(){if(state.dialog?.kind==='tags')return;if(control('view-tags')){await click('view-tags')}else{await click('menu-view');await click('view-goto');await click('goto-tags')}assert(state.dialog?.tags?.entries,JSON.stringify(state.dialog));}
async function select(name){const i=state.dialog.tags.entries.findIndex(e=>e.name===name);assert(i>=0,'Missing tag '+name);for(let n=0;!control('dialog-tag-row-'+i)&&n<30;n++)await scroll('dialog-tag-list-scroll');await click('dialog-tag-row-'+i);assert.equal(state.dialog.tags.selected,name);}
async function createPlan(name,target,message){await browse();await click('dialog-tag-new');await type('dialog-tag-name',name);await type('dialog-tag-target',target);await click(message===null?'dialog-tag-light':'dialog-tag-annotated');if(message!==null)await type('dialog-tag-message',message);assert(!control('dialog-submit').enabled);await click('dialog-preview-refresh');}
// 0.27: Delete is in the page's details card, its own scroller (page-details-scroll).
async function deletePlan(name){await browse();await select(name);for(let n=0;n<12;n++){const c=control('dialog-tag-delete');if(c&&c.rect[3]-c.rect[1]>=24)break;await scroll('page-details-scroll','down',120);}await click('dialog-tag-delete');assert(state.dialog.tags.preview_loaded);}
async function closeDialog(){for(let i=0;state.dialog&&i<3;i++)await click('dialog-cancel');assert.equal(state.dialog,null);}
function ref(f,name){const r=cp.spawnSync('git',['-C',f.repo,'rev-parse','--verify','refs/tags/'+name],{env,windowsHide:true,encoding:'utf8'});return r.status===0?r.stdout.trim():null;}

async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);

 const f=setup('PRIVATE_TAGS');f.git(['tag','--no-sign','-a','-m','PRIVATE_EXISTING 日本語\nrelease notes','v0',f.base]);f.git(['tag','same-commit',f.base]);f.git(['tag','tree','HEAD^{tree}']);
 f.write(FILE,'PRIVATE_STAGED\n');f.git(['add','--',FILE]);f.write(FILE,'PRIVATE_WORKING\n');f.write('untracked','PRIVATE_UNTRACKED\n');f.write('ignored/cache','PRIVATE_IGNORED\n');const original=snapshot(f);
 await open(f);await browse();assert.equal(state.dialog.tags.entries.length,3);assert(!control('dialog-tag-delete')?.enabled);await select('v0');assert(details().annotation.includes('PRIVATE_EXISTING 日本語'));assert.equal(details().target,f.base);assert.equal(details().signature_verified,false);await capture('tags-wide');
 await select('same-commit');assert.equal(details().object_type,'commit');assert.equal(details().annotation,null);await resize(780,620);await capture('tags-narrow');await closeDialog();await click('menu-terminal');await click('terminal-new');await deletePlan('v0');await capture('delete-narrow-terminal');assert(control('dialog-submit').rect[3]>0);await closeDialog();assert.deepEqual(snapshot(f),original);assert(ref(f,'v0'));await resize(1480,920);
 await createPlan('release/検索-🚀','HEAD~','PRIVATE_ANNOTATION\n# literal line\nUnicode é 🚀');assert(state.dialog.tags.preview_loaded);assert.equal(state.dialog.tags.preview.details.target,f.base);await capture('create-wide');await resize(780,620);await capture('create-narrow-terminal');await closeDialog();assert.equal(ref(f,'release/検索-🚀'),null);assert.deepEqual(snapshot(f),original);await resize(1480,920);
 passed('Native annotated/lightweight inspection, one selection for duplicate targets, wide/compact layouts with terminal, cancelled create/delete previews preserve files and refs');

 await createPlan('release/検索-🚀','HEAD~','PRIVATE_ANNOTATION\n# literal line\nUnicode é 🚀');const reviewed=state.dialog.tags.preview.details;await type('dialog-tag-message','PRIVATE_EDITED\n# literal\n中文 🚀');assert(!control('dialog-submit').enabled);await click('dialog-preview-refresh');const finalReview=state.dialog.tags.preview.details;assert.notEqual(finalReview.id,reviewed.id);await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(state.dialog.tags.action,null);assert.equal(ref(f,'release/検索-🚀'),finalReview.id);assert.equal(f.git(['cat-file','tag',finalReview.id]),finalReview.annotation.trimEnd());await select('release/検索-🚀');assert.equal(details().id,finalReview.id);await capture('created-annotation');
 await createPlan('local/轻量','HEAD',null);assert.equal(state.dialog.tags.preview.details.object_type,'commit');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(ref(f,'local/轻量'),f.tip);assert.deepEqual(snapshot(f),original);
 await deletePlan('release/検索-🚀');const backup=state.dialog.tags.preview.recovery_reference;await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(ref(f,'release/検索-🚀'),null);assert.equal(f.git(['rev-parse',backup]),finalReview.id);assert(latest().details.some(s=>s.includes(backup)));assert.deepEqual(snapshot(f),original);assert.equal(fs.readFileSync(path.join(f.repo,'untracked'),'utf8'),'PRIVATE_UNTRACKED\n');assert.equal(fs.readFileSync(path.join(f.repo,'ignored/cache'),'utf8'),'PRIVATE_IGNORED\n');
 await deletePlan('tree');assert.equal(state.dialog.tags.preview.details.target_type,'tree');await submit();assert(latest().command_succeeded);assert.equal(ref(f,'tree'),null);
 passed('Actual Unicode annotated/lightweight creation, edited draft requires review, exact annotation inspection, checked deletion and original-object recovery; local files/index/config preserved');

 for(const[name,target,message,code]of [['v0','HEAD',null,'tag_exists'],['bad name','HEAD',null,'tag_invalid_name'],['blank','HEAD',' ','tag_invalid_message'],['missing','not-a-commit',null,'tag_target_unavailable']]){
  await createPlan(name,target,message);assert(state.dialog.error.includes('['+code+']'),state.dialog.error);assert(!control('dialog-submit').enabled);await capture(code);await click('dialog-tag-back');
 }
 await deletePlan('v0');const staleId=state.dialog.tags.preview.details.id;f.git(['tag','--no-sign','-f','-a','-m','PRIVATE_EXTERNAL replacement','v0',f.base]);const replacement=ref(f,'v0');assert.notEqual(replacement,staleId);await submit();assert(!latest().command_succeeded);assert(state.dialog.error.includes('[tag_stale_preview]'),state.dialog.error);assert.equal(ref(f,'v0'),replacement);await capture('stale-deletion');await click('dialog-tag-back');await select('v0');assert(details().annotation.includes('PRIVATE_EXTERNAL'));await closeDialog();
 const locked=setup('PRIVATE_LOCK');await open(locked);await createPlan('locked','HEAD',null);const lock=path.join(locked.repo,'.git/refs/tags/locked.lock');fs.mkdirSync(path.dirname(lock),{recursive:true});fs.writeFileSync(lock,'PRIVATE_OTHER_LOCK');await submit();assert(!latest().command_succeeded);assert(latest().details.some(s=>s.includes('[tag_ref_locked]')),JSON.stringify(latest()));assert.equal(ref(locked,'locked'),null);assert.equal(fs.readFileSync(lock,'utf8'),'PRIVATE_OTHER_LOCK');await capture('tag-lock-error');await closeDialog();
 passed('Clear duplicate/name/message/target errors, same-target external annotation replacement refuses stale deletion, ref lock retained and observed outcome shown');

 const pinned=setup('PRIVATE_PINNED');await open(pinned);await createPlan('pinned','HEAD','PRIVATE_PINNED annotation');const pinnedReview=state.dialog.tags.preview.details;pinned.write('third','third\n');pinned.commit('PRIVATE_EXTERNAL commit');await submit();assert(latest().command_succeeded,JSON.stringify(latest()));assert.equal(pinned.git(['rev-parse','pinned^{}']),pinnedReview.target);await closeDialog();
 const readCancel=setup('PRIVATE_READ_CANCEL');await open(readCancel);await client.tool('rocket_click',{target:'view-tags',expected_tab:state.active_tab});await until(s=>s.dialog?.tags?.loading);const readStart=performance.now();await client.tool('rocket_click',{target:'dialog-cancel',expected_tab:state.active_tab});await settle();report.read_cancel_ms=performance.now()-readStart;assert.equal(state.dialog,null);assert(report.read_cancel_ms<10000);
 for(const phase of ['prepared','committed']){
  const c=setup('PRIVATE_CANCEL_'+phase.toUpperCase());c.git(['tag','--no-sign','-a','-m','PRIVATE_CANCEL annotation','cancel-tag']);const before=snapshot(c),old=ref(c,'cancel-tag');await open(c);await deletePlan('cancel-tag');const recovery=state.dialog.tags.preview.recovery_reference;
  fs.writeFileSync(path.join(c.hooks,'reference-transaction'),'#!/bin/sh\nlines=$(cat)\nif [ "$1" = "'+phase+'" ] && printf "%s" "$lines" | grep -q "refs/tags/cancel-tag$"; then echo ready > cancel-ready; sleep 30; fi\n');
  await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.write&&fs.existsSync(path.join(c.repo,'cancel-ready')),'Hook pause '+phase);const start=performance.now();await client.tool('rocket_click',{target:'cancel-operation',expected_tab:state.active_tab});await settle();report[phase+'_cancel_ms']=performance.now()-start;assert(report[phase+'_cancel_ms']<10000);assert(!latest().command_succeeded);assert.equal(ref(c,'cancel-tag'),phase==='prepared'?old:null);assert.deepEqual(snapshot(c),before);if(phase==='committed'){assert.equal(c.git(['rev-parse',recovery]),old);assert(latest().details.some(s=>s.includes('already matches')),JSON.stringify(latest()));}await capture('cancel-'+phase);await closeDialog();
 }
 passed('Pinned creation survives a moving branch; cancelled reads and deletion before/after transaction commit reconcile actual refs without automatic retry');

 const paging=setup('PRIVATE_PAGING');for(let i=0;i<205;i++)paging.git(['tag','page-'+String(i).padStart(3,'0')]);await open(paging);await browse();assert.equal(state.dialog.tags.entries.length,200);assert(state.dialog.tags.more);await select('page-001');await click('dialog-tags-more');assert.equal(state.dialog.tags.entries.length,205);assert.equal(state.dialog.tags.selected,'page-001');assert.equal(details().name,'page-001');await capture('tag-pages');await closeDialog();
 const large=setup('PRIVATE_LARGE');large.write('annotation','PRIVATE_LARGE_MESSAGE\n'+('日本語 release notes 🚀\n'.repeat(1000)));large.git(['tag','--no-sign','-a','-F','annotation','large']);await open(large);await browse();await select('large');assert(details().annotation_truncated);assert.equal([...details().annotation].length,16000);await resize(780,620);for(let i=0;i<5;i++)await scroll('dialog-tags-scroll','down',2000);await capture('bounded-annotation-narrow');assert(control('dialog-cancel').rect[3]<620);await closeDialog();
 passed('Large Unicode annotations are explicitly truncated in UI/MCP; compact scrolling keeps footer controls reachable');
 for(const fixture of fixtures)assert.equal(hash(path.join(fixture.repo,'.git/config')),fixture.config,'Repository config changed');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const secret of ['PRIVATE_','検索',FILE,'release/','cancel-tag'])assert(!payload.includes(secret),'Diagnostic leak '+secret);
 for(const op of ['tags','tag-inspect','tag-preview','tag-create','tag-delete']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired '+op);}
 for(const step of ['read_tags','inspect_tag','review_create_tag','review_delete_tag','revalidate_tag','write_tag_object','commit_tag_transaction','observe_tag','verify_tag'])assert(payload.includes(step),'Missing step '+step);
 for(const code of ['tag_exists','tag_invalid_name','tag_invalid_message','tag_target_unavailable','tag_stale_preview','tag_ref_locked'])assert(records.some(r=>r.failure_code===code),'Missing error '+code);
 assert(records.some(r=>r.outcome==='cancelled'));report.log_records=records.length;passed('Growing tag window preserves name selection; paired diagnostic spans, named phases, fixed failure codes, cancellations, zero dropped events and no private payloads');report.passed=true;
}catch(error){if(client)try{await capture('failure');}catch{}throw error;}
finally{
 fs.writeFileSync('work/tags-last-result.txt',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}if(client)await client.finish();
}}
run().catch(e=>{console.error(e);process.exitCode=1});


