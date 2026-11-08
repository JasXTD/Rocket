// Native ConPTY, widgets and captures through authenticated loopback HTTP MCP.
// All files/config are synthetic; the normal service and real repositories stay separate.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const platform=require('./platform.cjs');
const root=path.resolve(process.env.ROCKET_MCP_RESULTS || 'work/mcp-fixtures', 'terminal-unicode-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'client'),profile=path.join(root,'http-profile');
const branch='rocket-ui-test/terminal-unicode';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const basicPs='[Console]::Clear(); [Console]::Write("ASCII |abcd    |END`r`nCJK   |中文    |END`r`nJPN   |日本語  |END`r`nKOR   |한국어  |END`r`nEMOJI |🚀🎉    |END`r`nMARKS |éö      |END"); Start-Sleep -Seconds 30';
const emojiPs='[Console]::Clear(); [Console]::Write("ZWJ   |👩🏽‍💻      |END`r`nFAMILY|👨‍👩‍👧‍👦      |END`r`nFLAG  |🇲🇾      |END`r`nKEYCAP|1️⃣      |END`r`nHEART |❤️      |END`r`nTEXT  |☺︎       |END"); Start-Sleep -Seconds 30';
const marginPs="[Console]::Clear(); [Console]::Write((' ' * ([Console]::WindowWidth - 1)) + '中|END'); Start-Sleep -Seconds 30";
// Linux: the same rows through bash's printf, exactly as in crates/rocket-mcp/src/terminal_probes.rs.
const forBash=ps=>"printf '\\033[H\\033[2J\\033[3J"+ps.slice(ps.indexOf('Write("')+7,ps.lastIndexOf('"); Start-Sleep')).replaceAll('`r`n','\\r\\n')+"'; sleep 30";
const [basic,emoji,margin]=platform.windows?[basicPs,emojiPs,marginPs]:[forBash(basicPs),forBash(emojiPs),"printf '\\033[H\\033[2J\\033[3J%*s中|END' \"$(($(stty size | cut -d' ' -f2) - 1))\" ''; sleep 30"];
const report={passed:false,version,root,cases:[],timings_ms:{}};
let client,service,state;
fs.mkdirSync(repo,{recursive:true});
// Explicit local consent: this runner creates only synthetic shell/editor data.
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.mkdirSync(path.join(root,'hooks'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'hooks')});
// Isolated Unix home: readline may inspect inputrc even with --norc.
if (!platform.windows) {
 fs.mkdirSync(path.join(root,'home'), {mode:0o700});
 Object.assign(env,{HOME:path.join(root,'home'),XDG_CONFIG_HOME:path.join(root,'home','.config'),XDG_STATE_HOME:path.join(root,'home','.local','state'),INPUTRC:'/dev/null'});
 delete env.BASH_ENV; delete env.ENV; delete env.PROMPT_COMMAND;
}
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
async function run(){
  try{
    const exe=path.resolve(process.env.ROCKET_MCP_EXE || platform.exe('target/release/rocket-mcp'));cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
    const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
    fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);report.endpoint=`http://127.0.0.1:${port}/mcp`;
    const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);
    client=new RocketHttpClient(profile);
    for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(error){if(i===100)throw error;await delay(100);}}
    assert.equal((await client.initialize()).serverInfo.version,version);
    await client.tool('rocket_open',{path:fs.realpathSync(repo)});await settle();await click('view-changes');await click('menu-terminal');await click('terminal-new');
    await until(s=>s.terminal?.running&&s.terminal.tail.trimEnd().endsWith('>'),'Initial shell');const pid=state.terminal.pid;
    const denied=await client.request('tools/call',{name:'rocket_type',arguments:{target:'terminal-command',text:basic+'; Get-ChildItem',expected_tab:state.active_tab}});assert(denied.isError);assert(JSON.stringify(denied).includes('fixed synthetic probes'));
    passed('HTTP MCP refuses modified probes; native terminal starts without errors');
    let start=performance.now();await command(basic);await until(()=>printedLabel('MARKS'),'Basic Unicode output');
    table(['ASCII','CJK','JPN','KOR','EMOJI','MARKS'],[['中',2],['日',2],['한',2],['🚀',2],['é',1],['ö',1]]);
    report.timings_ms.basic_output=Math.round(performance.now()-start);await click('terminal-screen');await capture('basic-wide');
    passed('real '+(platform.windows?'ConPTY':'PTY')+' CJK, emoji and combining accents retain text, fixed columns and cursor alignment');await interrupt();
    await command(emoji);await until(()=>printedLabel('TEXT'),'Complex emoji output');
    table(['ZWJ','FAMILY','FLAG','KEYCAP','HEART','TEXT'],[['👩🏽‍💻',2],['👨‍👩‍👧‍👦',2],['🇲🇾',2],['1️⃣',2],['❤️',2],['☺︎',1]]);
    await capture('emoji-wide');passed('ZWJ, skin tone, family, flag, keycap and variation-selector clusters stay intact');await interrupt();
    await resize(780,620);await command(basic);await until(()=>printedLabel('MARKS'));
    table(['ASCII','CJK','JPN','KOR','EMOJI','MARKS'],[['中',2],['🚀',2],['é',1]]);
    await click('terminal-screen');await capture('basic-narrow');passed('Unicode table remains aligned after narrow-window '+(platform.windows?'ConPTY':'PTY')+' resize');await interrupt();
    for(const [width,height,name]of [[780,620,'margin-narrow'],[1480,920,'margin-wide']]){
      await resize(width,height);await command(margin);await until(s=>s.terminal.tail.includes('中|END')&&s.terminal.cursor[0]===6,'Wide glyph at right margin');
      const lead=state.terminal.grid.find(c=>c.text==='中');assert(lead);assert.equal(lead.column,0);assert.equal(lead.width,2);assert.equal(state.terminal.grid.find(c=>c.text==='|').column,2);
      await capture(name);await interrupt();
    }
    passed('wide characters wrap as complete cells at both narrow and wide right margins');
    await click('terminal-hide');await click('menu-terminal');await click('terminal-new');assert.equal(state.terminal.pid,pid);
    await command(platform.shell.ok);await until(s=>s.terminal.tail.includes('ROCKET_TERMINAL_OK'));
    await click('terminal-end');await until(s=>s.terminal.closed);assert.equal(state.terminal.running,false);
    passed('native Enter input, interrupt, hide/reveal and owned terminal shutdown still work');
    const logs=await client.tool('rocket_logs',{limit:1000});assert.equal(state.dropped_events,0);
    assert(logs.records.some(r=>r.operation==='terminal_start'));assert(logs.records.some(r=>r.operation==='terminal_exit'));
    assert.equal(git(['rev-parse','HEAD']),head);assert.deepEqual(['.git/index','.git/config','baseline.txt'].map(file=>hash(path.join(repo,file))),before);assert.equal(hash(path.join(root,'empty.config')),configHash);
    await closeWindow();await client.raw('POST','/shutdown');
    const records=fs.readdirSync(root).filter(name=>name.startsWith('mcp-logs-')).flatMap(name=>fs.readFileSync(path.join(root,name,'rocket.jsonl'),'utf8').trim().split(/\r?\n/).filter(Boolean).map(line=>JSON.parse(line)));
    const serialized=JSON.stringify(records);for(const payload of ['[Console]::','中文','👩🏽‍💻','ROCKET_TERMINAL_OK'])assert(!serialized.includes(payload));
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
