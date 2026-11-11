'use strict';
// Native Mac spelling only. Uses a fresh profile, synthetic repo and no accounts.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),cp=require('node:child_process'),net=require('node:net'),crypto=require('node:crypto');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
assert.equal(process.platform,'darwin');
assert.equal(process.env.GITHUB_ACTIONS,'true');
const exe=fs.realpathSync(process.env.ROCKET_MCP_EXE),out=process.env.ROCKET_MCP_RESULTS;
const startup=process.argv[2]||'immediate';
assert(['before-enable','immediate','ready-first'].includes(startup));
fs.mkdirSync(out,{recursive:true});
const root=fs.realpathSync(fs.mkdtempSync(path.join(out,'spelling-'))),repo=path.join(root,'資料 repo'),profile=path.join(root,'profile'),home=path.join(root,'home');
for(const p of [repo,home,path.join(root,'template')])fs.mkdirSync(p,{recursive:true});
const env={...process.env,HOME:home,XDG_CONFIG_HOME:home,GIT_CONFIG_NOSYSTEM:'1',GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_TERMINAL_PROMPT:'0',GCM_INTERACTIVE:'never',GIT_TEMPLATE_DIR:path.join(root,'template')};
for(const k of Object.keys(env))if(/^(GH_|GITHUB_TOKEN|SSH_|DYLD_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
fs.writeFileSync(env.GIT_CONFIG_GLOBAL,'');
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.writeFileSync(path.join(root,'rocket-update-api'),'http://127.0.0.1:1');
const git=args=>cp.execFileSync('/usr/bin/git',['-C',repo,...args],{env,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trimEnd();
const branch='rocket-ui-test/spelling';
git(['init','-b',branch]);git(['config','user.name','Spelling Fixture']);git(['config','user.email','fixture@example.invalid']);git(['config','commit.gpgsign','false']);
fs.writeFileSync(path.join(repo,'資料.rs'),'// unchanged synthetic source\n');git(['add','.']);git(['commit','-m','Spelling fixture']);fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const before={head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),file:hash(path.join(repo,'資料.rs')),config:hash(path.join(repo,'.git/config'))};
const report={passed:false,startup,cases:[],checks:[],events:[],binary_sha256:hash(exe)},delay=ms=>new Promise(r=>setTimeout(r,ms));
let service,client,state;
const nativeCalls=()=>fs.existsSync(path.join(root,'service.log'))?fs.readFileSync(path.join(root,'service.log'),'utf8').split(/\r?\n/).slice(0,-1).filter(l=>l.startsWith('ROCKET_SPELL_TRACE ')).map(l=>JSON.parse(l.slice(19))):[];
function event(name,extra={}){report.events.push({name,at_ms:Date.now(),spelling:state?.spelling,...extra})}
const control=id=>state.controls?.find(c=>c.id===id);
async function until(fn,label,ms=20000){const begin=performance.now();do{state=await client.tool('rocket_state');if(fn(state))return;await delay(35)}while(performance.now()-begin<ms);throw Error(label+': '+JSON.stringify(state.spelling))}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading),'settle')}
async function stable(){let prev='',n=0;await until(s=>{const next=JSON.stringify(s.controls);n=prev===next?n+1:0;prev=next;return n>=2&&!s.input_pending},'layout')}
async function click(id){await stable();assert(control(id)?.enabled,id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});await settle()}
async function menu(id){await settle();await client.tool('rocket_menu',{target:id,expected_tab:state.active_tab});await settle()}
async function type(text){await stable();event('type_before',{units:text.length});await client.tool('rocket_type',{target:'commit-message',text,expected_tab:state.active_tab});await settle();event('type_after',{units:text.length})}
async function enabled(on){await menu('preferences');await until(()=>control('pref-page-commit')?.enabled,'preferences');await click('pref-page-commit');if(state.spelling.enabled!==on)await click('pref-spell-check');await click('dialog-submit');event('toggle_saved',{on});if(!on||startup==='ready-first')await until(s=>on?s.spelling.ready&&!s.spelling.waiting:!s.spelling.ready&&!s.spelling.waiting,'checker toggle');await menu('shortcut-changes')}
async function capture(name){const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'))}
async function check(text,expected,iteration){
 const offset=nativeCalls().length;
 if(startup==='before-enable'){await type(text);await enabled(true)}else{await enabled(true);await type(text)}
 await until(s=>s.spelling.ready&&!s.spelling.waiting&&s.spelling.checked_texts>0&&(expected?s.spelling.misspellings>0:s.spelling.misspellings===0)&&nativeCalls().slice(offset).some(c=>c.text_units===text.length&&c.start===0),'spell result',12000);
 assert(!state.spelling.failed);
 const calls=nativeCalls().slice(offset).filter(c=>c.text_units===text.length);
 // The aggregate UI count alone can belong to an older text. Require a native
 // check of this input as well, without storing its contents or hashes.
 assert(calls.some(c=>c.start===0),'No native check of the actual input');
 assert.equal(calls.some(c=>!c.not_found&&c.length>0),expected,'Native result differs from input expectation');
 report.checks.push({iteration,units:text.length,expected,observed:state.spelling.misspellings,native_calls:calls.length});event('checked',{iteration});
}
async function run(){try{
 cp.execFileSync(exe,['--init-http',profile,root],{env,stdio:'pipe'});
 const port=net.createServer();await new Promise(r=>port.listen(0,'127.0.0.1',r));const number=port.address().port;await new Promise(r=>port.close(r));
 fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+number+'\n'+root+'\n');
 const runEnv={...env};if(process.env.ROCKET_SPELL_OBSERVER){runEnv.DYLD_INSERT_LIBRARIES=fs.realpathSync(process.env.ROCKET_SPELL_OBSERVER);runEnv.ROCKET_SPELL_OBSERVE='1'}
 const fd=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:runEnv,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>=100)throw e;await delay(100)}}
 await client.initialize();await client.tool('rocket_open',{path:repo});await settle();
 await client.tool('rocket_resize',{width:1180,height:620});await until(s=>s.viewport[0]===1180&&s.viewport[1]===620,'viewport');
 // Each mode starts a fresh app. Keep the original failed sentence first; vary
 // later texts so repeating a cached Cocoa answer is not our only evidence.
 for(let i=0;i<3;i++)for(const [base,expected] of [['Ths sentnce has misspelled wrds',true],['hello world',false],['資料 🚀 cafe\u0301 mispellling',true]]){
  const text=i===0?base:base+' '+(1000+i);
  await check(text,expected,i);
  if(i===0&&text.startsWith('Ths'))await capture('first-result');
  await type('');await enabled(false);
 }
 report.cases.push('Fresh process '+startup+': checker startup, native results, clean text, Unicode and stop');
 await capture('final');
 assert.deepEqual({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),file:hash(path.join(repo,'資料.rs')),config:hash(path.join(repo,'.git/config'))},before);
 report.passed=true;
}catch(e){report.error=e.stack;try{await capture('failure');fs.writeFileSync(path.join(root,'failure-state.json'),JSON.stringify({spelling:state.spelling,viewport:state.viewport,input_pending:state.input_pending},null,2))}catch{}process.exitCode=1}
finally{
 try{if(client&&service?.exitCode===null){await client.tool('rocket_close');await until(s=>s.window==='closed'||s.dialog?.kind==='quit','close');if(state.dialog?.kind==='quit'){await click('dialog-submit');await until(s=>s.window==='closed','closed')}await client.raw('POST','/shutdown');const begin=performance.now();while(service.exitCode===null&&service.signalCode===null){assert(performance.now()-begin<10000,'owned service cleanup');await delay(50)}}}catch(e){report.cleanup_error=e.message;report.passed=false;process.exitCode=1;if(service?.exitCode===null)service.kill()}
 if(client)await client.finish();
 const log=fs.existsSync(path.join(root,'service.log'))?fs.readFileSync(path.join(root,'service.log'),'utf8'):'';
 report.observer_ready=log.includes('ROCKET_SPELL_OBSERVER_READY');
 report.native=log.split(/\r?\n/).filter(l=>l.startsWith('ROCKET_SPELL_TRACE ')).map(l=>JSON.parse(l.slice(19)));
 if(process.env.ROCKET_SPELL_OBSERVER&&!report.observer_ready){report.passed=false;report.error='Native observer was not loaded';process.exitCode=1}
 fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log('RESULT '+path.join(root,'result.json'));console.log(JSON.stringify({passed:report.passed,checks:report.checks.length,native_calls:report.native.length,error:report.error}));
}}
run().catch(e=>{console.error(e.message);process.exitCode=1});
