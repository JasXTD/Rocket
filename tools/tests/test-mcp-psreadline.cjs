// Native ConPTY, widgets and captures through authenticated loopback HTTP MCP.
// All files/config are synthetic; the normal service and real repositories stay separate.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/psreadline-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'client'),profile=path.join(root,'http-profile');
const branch='rocket-ui-test/psreadline';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const basic='[Console]::Clear(); [Console]::Write("ASCII |abcd    |END`r`nCJK   |中文    |END`r`nJPN   |日本語  |END`r`nKOR   |한국어  |END`r`nEMOJI |🚀🎉    |END`r`nMARKS |éö      |END"); Start-Sleep -Seconds 30';
const emoji='[Console]::Clear(); [Console]::Write("ZWJ   |👩🏽‍💻      |END`r`nFAMILY|👨‍👩‍👧‍👦      |END`r`nFLAG  |🇲🇾      |END`r`nKEYCAP|1️⃣      |END`r`nHEART |❤️      |END`r`nTEXT  |☺︎       |END"); Start-Sleep -Seconds 30';
const margin="[Console]::Clear(); [Console]::Write((' ' * ([Console]::WindowWidth - 1)) + '中|END'); Start-Sleep -Seconds 30";
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
fs.writeFileSync(path.join(repo,'baseline.txt'),'synthetic unchanged\n');
for(const name of ['rocket-completion-aaa.txt','rocket-completion-中文.txt'])fs.writeFileSync(path.join(repo,name),'completion fixture unchanged\n');
git(['add','.']);git(['commit','-m','Synthetic terminal baseline']);fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const ordinaryHistory=path.join(process.env.APPDATA,'Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt');
const optionalHash=file=>fs.existsSync(file)?hash(file):null;
const ordinaryHistoryBefore=optionalHash(ordinaryHistory),modulePathBefore=process.env.PSModulePath;
const head=git(['rev-parse','HEAD']),before=['.git/index','.git/config','baseline.txt'].map(file=>hash(path.join(repo,file))),configHash=hash(path.join(root,'empty.config'));
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const passed=text=>{report.cases.push(text);console.error('PASS '+text);};
async function until(predicate,label='UI condition'){
  const start=performance.now();
  for(;;){state=await client.tool('rocket_state');assert(!state.terminal?.error,JSON.stringify(state.terminal));if(predicate(state))return state;assert(performance.now()-start<20000,label+' timed out: '+JSON.stringify({running:state.terminal?.running,focused:state.terminal?.screen_focused,cursor:state.terminal?.cursor,tail:state.terminal?.tail.slice(-1500)}));await delay(60);}
}
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running));}
async function click(target){await client.tool('rocket_click',{target,expected_tab:state.active_tab});return settle();}
async function command(text){await client.tool('rocket_type',{target:'terminal-command',text,expected_tab:state.active_tab});await settle();await client.tool('rocket_terminal_enter',{expected_tab:state.active_tab});}
async function capture(name){const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const image=result.content.find(c=>c.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state.terminal,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function interrupt(){await click('terminal-interrupt');await until(s=>s.terminal?.tail.trimEnd().endsWith('>'),'PowerShell prompt after interrupt');}
function printedLabel(label){const grid=state.terminal?.grid||[];return grid.some(first=>first.column===0&&label.split('').every((char,column)=>grid.some(c=>c.row===first.row&&c.column===column&&c.text===char)));}
function table(labels,clusters){
  assert.equal(state.terminal.grid_truncated,false);
  const grid=state.terminal.grid;
  for(const label of labels){
    const rowNumber=[...new Set(grid.map(c=>c.row))].find(row=>grid.filter(c=>c.row===row&&c.column<6).map(c=>c.text).join('')===label);
    assert.notEqual(rowNumber,undefined,'Missing row '+label);
    const cells=grid.filter(c=>c.row===rowNumber);
    assert.deepEqual(cells.filter(c=>c.text==='|').map(c=>c.column),[6,15],label+' separators');
    assert.equal(cells.filter(c=>c.column>=16).map(c=>c.text).join(''),'END',label+' end marker');
  }
  for(const [text,width]of clusters)assert(grid.some(c=>c.text===text&&c.width===width),`Missing intact ${text} width ${width}`);
  assert.equal(state.terminal.cursor[0],19);
}
async function closeWindow(){
  await client.tool('rocket_close');let confirmed=false;
  for(let i=0;i<200;i++){state=await client.tool('rocket_state');if(['closed','not_started'].includes(state.window))return;if(!confirmed&&!state.input_pending&&state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;}await delay(40);}throw Error('Window did not close');
}
const lastLine=()=>state.terminal.tail.trimEnd().split(/\r?\n/).at(-1);
const countOutput=marker=>state.terminal.tail.split(/\r?\n/).filter(line=>line.trim()===marker).length;
async function probe(probe){await client.tool('rocket_terminal_probe',{probe,expected_tab:state.active_tab});await settle();}
async function output(marker){await until(()=>countOutput(marker)>0&&lastLine().endsWith('>'),'Printed '+marker);}
async function checkReadline(pid){
  assert(!state.terminal.tail.includes('Cannot load PSReadline'));
  await capture('startup-clean');
  await assert.rejects(client.tool('rocket_terminal_probe',{probe:'typing; exit',expected_tab:state.active_tab}));
  await assert.rejects(client.tool('rocket_terminal_probe',{probe:'typing',expected_tab:state.active_tab+500}),/Active repository changed/);
  await probe('status');await output('ROCKET_CORE_True');assert(countOutput('ROCKET_PSREADLINE_True'));
  assert(state.terminal.tail.replaceAll('\n','').replaceAll('\r','').includes(path.join(root,'rocket-ui-state/terminal-history.log')));
  passed('clean startup, PSReadLine and standard modules loaded, isolated history, invalid/stale probes refused');

  await probe('typing');await output('ROCKET_TYPING_OK');await capture('typing-corrected');
  await probe('history-seed');await output('ROCKET_HISTORY_SECOND');
  await probe('history-previous');await until(()=>lastLine().endsWith("Write-Output 'ROCKET_HISTORY_FIRST'; Write-Output 'ROCKET_HISTORY_SECOND'"),'Up recalls latest command');
  await probe('history-previous');await until(()=>lastLine().endsWith("Write-Output 'ROCKET_TYPING_OK'"),'Up recalls older command');
  await probe('history-next');await until(()=>lastLine().endsWith("Write-Output 'ROCKET_HISTORY_FIRST'; Write-Output 'ROCKET_HISTORY_SECOND'"),'Down recalls newer command');
  await capture('history-recalled');let n=countOutput('ROCKET_HISTORY_SECOND');await probe('submit');await until(()=>countOutput('ROCKET_HISTORY_SECOND')===n+1&&lastLine().endsWith('>'),'Execute recalled command');
  passed('native screen typing, Left/Backspace editing, Up/Down history and Enter execute the expected command');

  await probe('resize-draft');await until(()=>lastLine().endsWith("ROCKET_RESIZE_中文_🚀_é_OX'"));
  for(const [width,height] of [[780,620],[1480,920],[900,680],[780,620],[1480,920]]) {
    await resize(width,height);
    await until(()=>lastLine().endsWith("ROCKET_RESIZE_中文_🚀_é_OX'"));
    assert.equal(state.terminal.tail.split(/\r?\n/).filter(l=>l.includes('ROCKET_RESIZE_')).length,1,'Resize duplicated or overwrote pending input');
  }
  await capture('resize-pending-wide');await resize(780,620);await capture('resize-pending-narrow');
  await probe('resize-edit');await output('ROCKET_RESIZE_中文_🚀_é_OK');
  await probe('history-previous');await until(()=>lastLine().endsWith("ROCKET_RESIZE_中文_🚀_é_OK'"));
  await resize(1480,920);await until(()=>lastLine().endsWith("ROCKET_RESIZE_中文_🚀_é_OK'"));
  await probe('history-next');await until(()=>lastLine()==='>');
  await capture('resize-history');
  passed('Repeated narrow/wide resize preserves pending Unicode input, its middle cursor and history without manual redraw');

  await probe('completion-command');await until(()=>lastLine().trimEnd().endsWith('Write-Output'),'Command Tab completion');await capture('command-completion');
  await probe('completion-argument');await output('ROCKET_COMPLETION_OK');
  await probe('completion-parameter');await until(()=>lastLine().trimEnd().endsWith('Write-Output -InputObject'),'Parameter Tab completion');
  n=countOutput('ROCKET_COMPLETION_OK');await probe('completion-argument');await until(()=>countOutput('ROCKET_COMPLETION_OK')===n+1&&lastLine().endsWith('>'));
  await probe('completion-path');await until(()=>lastLine().includes('rocket-completion-aaa.txt'),'Path completion');
  await probe('completion-backward');await until(()=>lastLine().includes('rocket-completion-中文.txt'),'Shift+Tab reverse completion');await capture('unicode-path-completion');
  await probe('submit');await until(()=>lastLine().endsWith('>'));assert(!state.terminal.tail.includes('Cannot find path'));
  passed('Tab completes command, parameter and filename; Shift+Tab reverses to a Unicode path');

  await probe('unicode');await output('ROCKET_UNICODE_中文 🚀 é');
  await probe('completion-command');await until(()=>lastLine().trimEnd().endsWith('Write-Output'));
  await probe('cancel-line');await until(()=>lastLine().endsWith('>'));
  await command('Start-Sleep -Seconds 30');await until(s=>s.terminal.tail.includes('Start-Sleep -Seconds 30'));
  const start=performance.now();await interrupt();report.timings_ms.interrupt=Math.round(performance.now()-start);
  await command("Write-Output ('AFTER_' + 'INTERRUPT')");await output('AFTER_INTERRUPT');
  passed('Unicode typing and output, cancelling unfinished input and interrupting a running command preserve the session');

  await resize(780,620);n=countOutput('ROCKET_TYPING_OK');await probe('typing');await until(()=>countOutput('ROCKET_TYPING_OK')>n&&lastLine().endsWith('>'));
  await capture('typing-narrow');
  assert.equal(lastLine(),'>');
  assert(state.terminal.tail.split(/\r?\n/).filter(l=>l.includes("Write-Output 'ROCKET_TYPING_OK'")).every(l=>l.startsWith('> Write-Output')),'Typing overlaps path prompt');
  await click('terminal-hide');await click('menu-terminal');await click('terminal-new');assert.equal(state.terminal.pid,pid);
  await resize(1480,920);await command("Write-Output ('ROCKET_' + 'TERMINAL_OK')");await output('ROCKET_TERMINAL_OK');
  await click('terminal-end');await until(s=>s.terminal.closed);
  await assert.rejects(client.tool('rocket_terminal_probe',{probe:'typing',expected_tab:state.active_tab}),/Open an owned terminal/);
  await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal.running&&lastLine().endsWith('>'));
  assert.notEqual(state.terminal.pid,pid);assert(!state.terminal.tail.includes('Cannot load PSReadline'));
  await probe('history-previous');await until(()=>lastLine().endsWith("Write-Output ('ROCKET_' + 'TERMINAL_OK')"),'Saved history after restart');await capture('history-after-restart');
  await probe('submit');await output('ROCKET_TERMINAL_OK');await click('terminal-end');await until(s=>s.terminal.closed);
  assert(fs.readFileSync(path.join(root,'rocket-ui-state/terminal-history.log'),'utf8').includes('ROCKET_TYPING_OK'));
  assert.equal(optionalHash(ordinaryHistory),ordinaryHistoryBefore,'Ordinary user history changed');assert.equal(process.env.PSModulePath,modulePathBefore);
  for(const name of ['rocket-completion-aaa.txt','rocket-completion-中文.txt'])assert.equal(fs.readFileSync(path.join(repo,name),'utf8'),'completion fixture unchanged\n');
  passed('780x620 typing, hide/reveal, command field, shutdown/reopen and saved history work; user history and environment unchanged');
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
    await checkReadline(pid);
    const logs=await client.tool('rocket_logs',{limit:1000});assert.equal(state.dropped_events,0);
    assert(logs.records.some(r=>r.operation==='terminal_start'));assert(logs.records.some(r=>r.operation==='terminal_exit'));
    assert.equal(git(['rev-parse','HEAD']),head);assert.deepEqual(['.git/index','.git/config','baseline.txt'].map(file=>hash(path.join(repo,file))),before);assert.equal(hash(path.join(root,'empty.config')),configHash);
    await closeWindow();await client.raw('POST','/shutdown');
    const records=fs.readdirSync(root).filter(name=>name.startsWith('mcp-logs-')).flatMap(name=>fs.readFileSync(path.join(root,name,'rocket.jsonl'),'utf8').trim().split(/\r?\n/).filter(Boolean).map(line=>JSON.parse(line)));
    const serialized=JSON.stringify(records);for(const payload of ['[Console]::','中文','👩🏽‍💻','ROCKET_TERMINAL_OK','ROCKET_TYPING','ROCKET_HISTORY','ROCKET_PSREADLINE','Write-Output','rocket-completion-'])assert(!serialized.includes(payload));
    for(const operation of ['terminal_start','terminal_ready','terminal_input_queued','terminal_interrupt','terminal_stop','terminal_exit']){
      const starts=records.filter(r=>r.event==='request_start'&&r.operation===operation);assert(starts.length,operation);
      for(const start of starts)assert(records.some(r=>r.event==='request_end'&&r.request===start.request&&r.outcome==='ok'),operation+' missing successful end');
    }
    assert(records.every(r=>r.dropped_before===0));report.log_records=records.length;
    passed('valid lifecycle diagnostics exclude terminal payloads; zero dropped events; HEAD/index/files/config unchanged');report.passed=true;
  }catch(error){if(client)try{await capture('failure');}catch{}throw error;
  }finally{
    fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
    if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}await delay(150);if(service.exitCode===null)service.kill();}
    if(client)await client.finish();
  }
}
run().catch(error=>{console.error(error);process.exitCode=1;});

