// Native ConPTY, widgets and captures through authenticated loopback HTTP MCP.
// All files/config are synthetic; the normal service and real repositories stay separate.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/changes-layout-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'client'),profile=path.join(root,'http-profile');
const branch='rocket-ui-test/changes-layout';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report={passed:false,version,root,cases:[],timings_ms:{}};
let client,service,state;
fs.mkdirSync(repo,{recursive:true});
// Explicit local consent: this runner creates only synthetic shell/editor data.
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.mkdirSync(path.join(root,'hooks'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'hooks')});
function git(args){return cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8'});}
git(['init','--initial-branch='+branch]);for(const [key,value]of [['user.name','Rocket Terminal Fixture'],['user.email','rocket@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',path.join(root,'hooks')]])git(['config',key,value]);
fs.writeFileSync(path.join(repo,'baseline.txt'),'synthetic unchanged\n');git(['add','.']);git(['commit','-m','Synthetic terminal baseline']);fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const head=git(['rev-parse','HEAD']),before=['.git/index','.git/config','baseline.txt'].map(file=>hash(path.join(repo,file))),configHash=hash(path.join(root,'empty.config'));
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const passed=text=>{report.cases.push(text);console.error('PASS '+text);};
async function until(predicate,label='UI condition'){
  const start=performance.now();
  for(;;){state=await client.tool('rocket_state');assert(!state.terminal?.error,JSON.stringify(state.terminal));if(predicate(state))return state;assert(performance.now()-start<20000,label+' timed out: '+JSON.stringify(state.terminal));await delay(60);}
}
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running));}
async function click(target){await client.tool('rocket_click',{target,expected_tab:state.active_tab});return settle();}
async function command(text){await client.tool('rocket_type',{target:'terminal-command',text,expected_tab:state.active_tab});await settle();await client.tool('rocket_terminal_enter',{expected_tab:state.active_tab});}
async function capture(name){const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const image=result.content.find(c=>c.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function interrupt(){await click('terminal-interrupt');await until(s=>s.terminal?.tail.trimEnd().endsWith('>'),'PowerShell prompt after interrupt');}
function control(id){return state.controls?.find(c=>c.id===id);}
async function stableLayout(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=3;},'Stable layout');}
async function scroll(direction,pixels=800,target='changes-scroll'){
  await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settle();await stableLayout();
}
async function reveal(id,direction='down'){
  for(let i=0;i<12;i++){
    const c=control(id);if(c&&c.rect[3]-c.rect[1]>=25)return c;
    await scroll(direction,600);
  }
  throw Error('Could not reveal '+id);
}
function layout(){
  const terminalTop=control('terminal-hide').rect[1]-6;
  const candidates=state.controls.filter(c=>/^(change-|changes-scroll$|stage-all$|unstage-all$|commit$|commit-message)/.test(c.id));
  assert(candidates.length>0);
  for(const c of candidates){assert(c.rect[2]>c.rect[0]&&c.rect[3]>c.rect[1],c.id+' invalid visible bounds');assert(c.rect[3]<=terminalTop,c.id+' overlaps terminal: '+JSON.stringify(c.rect));}
  const message=control('commit-message'),button=control('commit');
  if(message&&button)assert(message.rect[3]<button.rect[1],'Message editor overlaps Commit');
}
async function reject(name,args,needle){const r=await client.request('tools/call',{name,arguments:args});assert(r.isError);assert(JSON.stringify(r).includes(needle),JSON.stringify(r));}
async function closeWindow(){
  await client.tool('rocket_close');let confirmed=false;
  for(let i=0;i<200;i++){state=await client.tool('rocket_state');if(['closed','not_started'].includes(state.window))return;if(!confirmed&&!state.input_pending&&state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;}await delay(40);}throw Error('Window did not close');
}
async function run(){
  try{
    const exe=path.resolve('target/release/rocket-mcp.exe');cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
    const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
    fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);report.endpoint=`http://127.0.0.1:${port}/mcp`;
    const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);
    client=new RocketHttpClient(profile);
    for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(error){if(i===100)throw error;await delay(100);}}
    assert.equal((await client.initialize()).serverInfo.version,version);
    await client.tool('rocket_open',{path:repo});await settle();await click('view-changes');await click('menu-terminal');await click('terminal-new');
    await until(s=>s.terminal?.running&&s.terminal.tail.trimEnd().endsWith('>'),'Initial shell');const pid=state.terminal.pid;
    await resize(780,620);await stableLayout();layout();await capture('clean-narrow-top');
    assert(!control('commit'),'Commit button should be below the short initial viewport');
    await reject('rocket_type',{target:'commit',text:'MUST_NOT_APPEAR',expected_tab:state.active_tab},'Control is not present');
    await reject('rocket_scroll',{target:'changes-scroll',direction:'down',pixels:2001,expected_tab:state.active_tab},'invalid type');
    await reject('rocket_scroll',{target:'terminal-screen',direction:'down',pixels:100,expected_tab:state.active_tab},'Scroll target');
    await reject('rocket_scroll',{target:'changes-scroll',direction:'side',pixels:100,expected_tab:state.active_tab},'direction');
    await reject('rocket_scroll',{target:'changes-scroll',direction:'down',pixels:100,expected_tab:state.active_tab+999},'Active repository changed');
    passed('clean narrow layout and hidden-control / bounded-scroll guards');
    await reveal('commit');layout();assert.equal(control('commit').enabled,false);await capture('clean-narrow-form');
    const draft='Rocket layout fixture message 中文 é\n\n'+Array.from({length:40},(_,i)=>'Detail '+i+' preserves the complete draft.').join('\n');
    await client.tool('rocket_type',{target:'commit-message',text:draft,expected_tab:state.active_tab});await settle();await stableLayout();layout();assert(state.tabs[0].draft_present);await capture('draft-narrow');
    await scroll('down',1600,'commit-message-scroll');layout();await capture('draft-narrow-scrolled');
    await scroll('up',1600,'commit-message-scroll');layout();
    for(const [width,height,name] of [[900,700,'draft-medium'],[1480,920,'draft-wide'],[780,620,'draft-narrow-return']]){
      await resize(width,height);await stableLayout();await reveal('commit');layout();assert(state.tabs[0].draft_present);await capture(name);
    }
    await click('terminal-hide');await stableLayout();assert(state.tabs[0].draft_present);await capture('draft-terminal-hidden');
    await click('menu-terminal');await click('terminal-new');await stableLayout();assert.equal(state.terminal.pid,pid);layout();
    passed('bounded long Unicode draft survives repeated resize and terminal hide/reveal');
    const files=Array.from({length:12},(_,i)=>'change-'+String(i).padStart(2,'0')+'-日本語.txt');
    for(const file of files)fs.writeFileSync(path.join(repo,file),'synthetic '+file+'\n');
    const contents=files.map(file=>hash(path.join(repo,file)));git(['add','--',files[0]]);
    await until(s=>s.tabs[0].change_count===files.length&&!s.busy,'External fixture changes');await settle();
    await scroll('up',2000);await reveal('stage-all','up');layout();await capture('mixed-narrow-top');
    await click('stage-all');assert.equal(git(['diff','--cached','--name-only','-z']).split('\0').filter(Boolean).length,files.length);
    await reveal('unstage-all');await click('unstage-all');assert.equal(git(['diff','--cached','--name-only']).trim(),'');
    await reveal('stage-all','up');await click('stage-all');await reveal('commit');layout();assert(control('commit').enabled);await capture('staged-narrow-form');
    passed('mixed and long file lists remain scrollable; stage/unstage actions work with terminal open');
    assert.equal(git(['rev-parse','HEAD']),head);assert.equal(hash(path.join(repo,'baseline.txt')),before[2]);
    await click('commit');await until(s=>s.tabs[0].head.oid!==head.trim(),'Commit result');await settle();
    assert.equal(git(['log','-1','--format=%B']).trimEnd(),draft.trimEnd());assert.equal(git(['status','--porcelain']).trim(),'');
    assert.deepEqual(files.map(file=>hash(path.join(repo,file))),contents);assert.equal(hash(path.join(repo,'.git/config')),before[1]);assert.equal(hash(path.join(root,'empty.config')),configHash);
    passed('Commit stays reachable and saves the exact long draft; working bytes and config preserved');
    await command("Write-Output ('ROCKET_' + 'TERMINAL_OK')");await until(s=>s.terminal.tail.includes('ROCKET_TERMINAL_OK'));
    await click('terminal-end');await until(s=>s.terminal.closed);assert.equal(state.terminal.running,false);
    const logs=await client.tool('rocket_logs',{limit:1000});assert.equal(state.dropped_events,0);assert(logs.records.every(r=>!r.dropped_before));
    assert(!JSON.stringify(logs).includes('Rocket layout fixture message'));assert(!JSON.stringify(logs).includes('ROCKET_TERMINAL_OK'));
    passed('terminal input/shutdown and diagnostic privacy still pass with zero dropped events');
    await closeWindow();await client.raw('POST','/shutdown');
    const records=fs.readdirSync(root).filter(name=>name.startsWith('mcp-logs-')).flatMap(name=>fs.readFileSync(path.join(root,name,'rocket.jsonl'),'utf8').trim().split(/\r?\n/).filter(Boolean).map(line=>JSON.parse(line)));
    assert(records.every(r=>r.dropped_before===0));report.log_records=records.length;report.passed=true;
  }catch(error){if(client)try{await capture('failure');}catch{}throw error;
  }finally{
    fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
    if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}
    if(client)await client.finish();
  }
}
run().catch(error=>{console.error(error);process.exitCode=1;});
