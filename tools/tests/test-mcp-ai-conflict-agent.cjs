// Real connected-agent acceptance: only synthetic repository content is shared.
// The test exposes Rocket's reviewed MCP context in context.json, then waits for
// a reviewed AI response in proposal.json. It never invents that response itself.
// node tools/tests/test-mcp-ai-conflict-agent.cjs [result-root]
'use strict';
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve(process.argv[2]||'work/mcp-fixtures/ai-agent-'+crypto.randomUUID());
const repo=path.join(root,'repo 資料'),profile=path.join(root,'profile'),exe=path.resolve('target/release/rocket-mcp.exe');
const branch='rocket-ui-test/ai-agent',file='src/名前 🚀.js',proposalPath=path.join(root,'proposal.json');
assert(!fs.existsSync(root),'Use a new owned fixture directory');fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_TOKEN$|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'template')});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trimEnd();
const write=(name,text)=>{const dest=path.join(repo,name);fs.mkdirSync(path.dirname(dest),{recursive:true});fs.writeFileSync(dest,text);};
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'),delay=ms=>new Promise(r=>setTimeout(r,ms));
const contents=(name,retry)=>name+'\n'+Array.from({length:32},(_,i)=>'// unchanged line '+i+' 日本語').join('\n')+'\n'+retry+'\n';
git(['init','--initial-branch='+branch]);for(const[k,v]of[['user.name','Synthetic AI Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false'],['merge.conflictStyle','diff3'],['core.hooksPath',path.join(root,'template')]])git(['config',k,v]);
write(file,contents('export function normalizeName(value) { return value.trim(); }','export function retryDelay(attempt) { return 100 * attempt; }'));write('keep.txt','PRIVATE_UNRELATED_KEEP');git(['add','-A']);git(['commit','-m','Synthetic base']);
git(['checkout','-b','rocket-ui-test/theirs']);write(file,contents('export function normalizeName(value) { return value.trim().normalize("NFC"); }','export function retryDelay(attempt) { return Math.min(100 * attempt, 5000); }'));git(['add','-A']);git(['commit','-m','Unicode names and delay cap']);
git(['checkout',branch]);write(file,contents('export function normalizeName(value) { if (typeof value !== "string") throw new TypeError("name must be a string"); return value.trim(); }','export function retryDelay(attempt) { return 100 * 2 ** attempt; }'));git(['add','-A']);git(['commit','-m','Name validation and exponential retry']);assert.throws(()=>git(['merge','rocket-ui-test/theirs']));
write('untracked.txt','PRIVATE_UNRELATED_LOCAL');write('.git/rocket-ui-fixture',branch);fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const original=fs.readFileSync(path.join(repo,file),'utf8');
const fingerprint=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,file)),keep:hash(path.join(repo,'keep.txt')),untracked:hash(path.join(repo,'untracked.txt'))});
const before=fingerprint(),report={passed:false,root,model:'gpt-6-astra (connected agent)',binary:hash(exe),cases:[]};let client,service,state;
const active=()=>state.tabs.find(t=>t.id===state.active_tab),editor=()=>active()?.conflict_editor,ai=()=>editor()?.ai,control=id=>state.controls?.find(c=>c.id===id);
async function until(fn,label='state',timeout=45000){const at=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-at<timeout,'Timed out: '+label);await delay(60);}}
const settled=()=>until(s=>!s.busy&&!s.input_pending,'settled');
async function stable(){let last='',n=0;await until(s=>{const next=JSON.stringify(s.controls);n=last===next?n+1:0;last=next;return !s.input_pending&&n>=2;},'stable');}
async function click(id,wait=true){await stable();if(id.startsWith('conflict-undo-')){await client.tool('rocket_scroll',{target:'conflict-editor-scroll',direction:'up',pixels:2000,expected_tab:state.active_tab});await settled();await stable();for(let n=0;n<20&&!control(id)?.enabled;n++){await client.tool('rocket_scroll',{target:'conflict-editor-scroll',direction:'down',pixels:120,expected_tab:state.active_tab});await settled();await stable();}}assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(wait)await settled();}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await stable();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));}
const pass=s=>{report.cases.push(s);console.log('PASS '+s);};
async function main(){try{
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(n>=150||service.exitCode!==null)throw e;await delay(100);}}
 await client.initialize();await client.tool('rocket_open',{path:repo});await settled();await resize(1440,900);if(control('console-close'))await click('console-close');
 await click('view-changes');await click('change-unstaged-'+Buffer.from(file).toString('hex'));await click('conflict-open');assert.equal(editor().blocks,2);await click('conflict-ai');if(ai().source!=='agent')await click('conflict-ai-source-agent');assert.equal(ai().source,'agent');
 await client.tool('rocket_type',{target:'conflict-ai-instructions',text:'Combine both intended changes. Preserve strict string validation and NFC normalization; preserve exponential backoff and the 5000 ms cap. Return only the required resolution JSON.',expected_tab:state.active_tab});await settled();
 await click('conflict-ai-share',false);await until(()=>ai()?.shared&&!state.input_pending);const requestId=ai().request_id;
 const listed=await client.tool('rocket_ai_conflict_list',{});const context=await client.tool('rocket_ai_conflict_get',{request_id:requestId});assert(context.context);assert.equal(context.context.sections.length,2);
 assert(!JSON.stringify(context).includes('PRIVATE_UNRELATED'));fs.writeFileSync(path.join(root,'context.json'),JSON.stringify(context,null,2));fs.writeFileSync(path.join(root,'ready.json'),JSON.stringify({request_id:requestId,context:path.join(root,'context.json'),proposal:proposalPath}));
 console.log('AGENT_CONTEXT_READY '+path.join(root,'context.json'));report.request_list_count=Array.isArray(listed.requests)?listed.requests.length:null;
 const waitAt=performance.now();while(!fs.existsSync(proposalPath)){assert(performance.now()-waitAt<600000,'No reviewed agent proposal within ten minutes');await delay(250);}
 const proposal=fs.readFileSync(proposalPath,'utf8');const parsed=JSON.parse(proposal);assert.equal(parsed.resolutions.length,2);report.proposal_sha256=crypto.createHash('sha256').update(proposal).digest('hex');
 const received=await client.tool('rocket_ai_conflict_propose',{request_id:requestId,proposal});assert.equal(received.applied,false);await until(()=>ai()?.has_proposal&&!ai().busy);assert.equal(editor().draft,original);assert.deepEqual(fingerprint(),before);await capture('agent-proposal-wide');await resize(780,660);await capture('agent-proposal-compact');await resize(1440,900);
 pass('Actual Astra response crosses the authenticated MCP contract and is reviewed without touching repository files');
 await click('conflict-ai-apply');assert.equal(editor().blocks,0);const result=editor().draft;
 // Semantic acceptance runs the synthetic function bodies only, in a fresh Node
 // VM with no process, require, filesystem or network. The agent cannot supply
 // commands: Rocket already validated replacements and untouched outer content.
 const vm=require('node:vm');const box=Object.create(null);vm.createContext(box,{codeGeneration:{strings:false,wasm:false}});new vm.Script(result.replace(/^export /gm,'')).runInContext(box,{timeout:1000});
 assert.equal(box.normalizeName(' e\u0301 '),'é');assert.throws(()=>box.normalizeName(null));assert.throws(()=>box.normalizeName(42));for(const n of [0,1,4,10])assert.equal(box.retryDelay(n),Math.min(100*2**n,5000));
 assert.deepEqual(fingerprint(),before);await capture('agent-applied-draft');await click('conflict-undo-1');assert.equal(editor().blocks,1);await click('conflict-undo-0');assert.equal(editor().blocks,2);assert.equal(editor().draft,original);
 pass('Reviewed proposal preserves both behaviors; Apply only changes the draft and per-section Undo restores the original');
 await click('conflict-ai');await click('conflict-ai-share',false);await until(()=>ai()?.shared&&!state.input_pending);const staleId=ai().request_id;await click('conflict-ai-cancel');assert(!ai().busy);await assert.rejects(client.tool('rocket_ai_conflict_propose',{request_id:staleId,proposal}));assert.equal(editor().draft,original);assert.deepEqual(fingerprint(),before);
 pass('Stop invalidates the shared request; a late proposal cannot change the draft');report.passed=true;
 }catch(e){report.error=e.stack;process.exitCode=1;console.error(e.stack);if(client&&state)try{await capture('failure');}catch{}}
 finally{
  try{if(client&&state?.window==='running'){if(ai()?.busy)await click('conflict-ai-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await click('dialog-submit');await until(s=>['closed','not_started'].includes(s.window));}}if(client)await client.raw('POST','/shutdown');}catch(e){report.cleanup_error=e.message;}
  if(client)await client.finish();if(service){for(let i=0;i<100&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null){service.kill();report.cleanup_error='Owned MCP service required termination';}}
  try{assert.deepEqual(fingerprint(),before);const folders=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-'));const logs=folders.flatMap(f=>fs.readdirSync(path.join(root,f)).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n)).map(n=>fs.readFileSync(path.join(root,f,n),'utf8'))).join('\n');for(const text of ['PRIVATE_UNRELATED','name must be a string','exponential backoff'])assert(!logs.includes(text),'Diagnostic content leak');assert(logs.length>0);report.log_bytes=Buffer.byteLength(logs);}catch(e){report.error??=e.stack;report.passed=false;process.exitCode=1;}
  const token=path.join(profile,'access-token');if(fs.existsSync(token))fs.unlinkSync(token);if(report.cleanup_error){report.passed=false;process.exitCode=1;}fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log('Evidence: '+root);
 }
}main().catch(e=>{console.error(e);process.exitCode=1;});
