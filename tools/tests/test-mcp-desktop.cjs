// Native desktop boundary only: isolated local data, no browser/account/remote opens.
'use strict';
const fs=require('node:fs'), path=require('node:path'), cp=require('node:child_process'), crypto=require('node:crypto'), net=require('node:net'), assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {windows,shell}=require('./platform.cjs');
const mac=process.platform==='darwin', primary=mac?'Cmd':'Ctrl';
let root=path.resolve(process.env.ROCKET_MCP_RESULTS||'work/mcp-fixtures','desktop-'+crypto.randomUUID().replaceAll('-',''));
fs.mkdirSync(root,{recursive:true,mode:0o700});root=fs.realpathSync(root);
const repo=path.join(root,'資料 one'), profile=path.join(root,'http-profile');
const exe=path.resolve(process.env.ROCKET_MCP_EXE||'target/release/rocket-mcp'+(windows?'.exe':''));
const branch='rocket-ui-test/desktop', report={passed:false,root,cases:[]};
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'), delay=ms=>new Promise(r=>setTimeout(r,ms));
fs.mkdirSync(repo);fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
if(!windows){const home=path.join(root,'os-home');fs.mkdirSync(home,{mode:0o700});Object.assign(env,{HOME:home,XDG_CONFIG_HOME:path.join(home,'.config'),XDG_STATE_HOME:path.join(home,'.local/state')});}
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trimEnd();
git(['init','--initial-branch='+branch]);for(const [key,value]of [['user.name','Desktop Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false'],['core.hooksPath',path.join(root,'empty-template')]])git(['config',key,value]);
const file=path.join(repo,'資料 日本語 한국어.rs');fs.writeFileSync(file,'// PRIVATE_DESKTOP_BASE\n');git(['add','-A']);git(['commit','-m','Unicode 資料 desktop fixture']);
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);fs.appendFileSync(file,'// PRIVATE_DESKTOP_STAGED 中文 日本語 한국어 e\u0301 🦀\n');git(['add','-A']);fs.appendFileSync(file,'// PRIVATE_DESKTOP_UNSTAGED\n');fs.writeFileSync(path.join(repo,'untracked.txt'),'PRIVATE_DESKTOP_UNTRACKED');
const preserved=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(file),untracked:hash(path.join(repo,'untracked.txt'))});
const before=preserved();let state,client,service;
const control=id=>state.controls?.find(c=>c.id===id), binding=id=>state.shortcuts.bindings.find(b=>b.id===id), pass=name=>{report.cases.push(name);console.log('PASS '+name)};
// realpath retains an incoming Windows extended-path prefix. Normalize both
// spellings after resolution; do not accept a merely matching filename.
const samePath=(a,b)=>!!a&&path.toNamespacedPath(fs.realpathSync(a))===path.toNamespacedPath(fs.realpathSync(b));
async function until(fn,label='state',timeout=45000){const at=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-at<timeout,label+' timeout');await delay(45)}}
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading),'settled')}
async function stable(){let last='',n=0;await until(s=>{const now=JSON.stringify(s.controls);n=now===last?n+1:0;last=now;return !s.input_pending&&n>=2},'layout')}
async function click(id){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});await settled()}
async function type(id,text){await stable();assert(control(id)?.enabled,'Missing input '+id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settled()}
async function probe(chord,extra={}){await client.tool('rocket_shortcut',{binding:chord,expected_tab:state.active_tab,...extra});await settled()}
async function resize(width,height){
 // The hosted Mac's visible desktop cannot fit 900 content pixels vertically.
 // Preserve exact width/height checks and exercise both widths at compact height.
 if(mac)height=Math.min(height,620);
 await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending,'Requested viewport size');await stable()
}
async function capture(name){await stable();const out=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!out.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(out.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2))}
async function preferences(){await click('menu-edit');await click('preferences');await click('pref-page-shortcuts');await type('pref-shortcuts-filter','Sidebar')}
async function record(chord){await click('pref-shortcut-sidebar-edit-0');await probe(chord,{recording_id:state.shortcuts.editor.serial})}
async function done(){await click('dialog-submit');await until(s=>!s.dialog)}
async function close(){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>s.window==='closed')}}
async function open(){await client.tool('rocket_open',{path:repo});await settled();await resize(1440,900);if(control('console-close'))await click('console-close')}
async function terminalProbe(name){await client.tool('rocket_terminal_probe',{probe:name,expected_tab:state.active_tab});await settled()}
async function command(text){await type('terminal-command',text);await client.tool('rocket_terminal_enter',{expected_tab:state.active_tab})}
async function run(){try{
 report.binary=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});
 const socket=net.createServer();await new Promise(r=>socket.listen(0,'127.0.0.1',r));report.port=socket.address().port;await new Promise(r=>socket.close(r));fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${report.port}\n${root}\n`);
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i>=100)throw e;await delay(100)}}
 await client.initialize();await open();const initialSidebar=state.preferences.sidebar;
 assert.deepEqual(binding('sidebar').inputs,[primary+'+B']);assert.deepEqual(binding('next').inputs,['Ctrl+Tab']);
 await probe(primary+'+B');assert.equal(state.preferences.sidebar,!initialSidebar);await probe(primary+'+B');assert.equal(state.preferences.sidebar,initialSidebar);
 pass('Native primary shortcut dispatch and Control+Tab defaults');
 await preferences();await resize(780,660);await capture('01-compact-shortcuts');
 await click('pref-shortcut-sidebar-edit-0');const stale=state.shortcuts.editor.serial;await client.tool('rocket_escape');await settled();assert.equal(state.shortcuts.editor,null);
 await assert.rejects(probe(primary+'+B',{recording_id:stale}),/recording ended/);assert.deepEqual(binding('sidebar').inputs,[primary+'+B']);
 await record(primary+'+C');assert(state.shortcuts.editor.error);await click('pref-shortcut-cancel');
 await record(primary+'+O');assert.match(state.shortcuts.editor.error,/Open repository/);await click('pref-shortcut-cancel');
 pass('Compact capture cancellation, stale input and reserved/duplicate refusal');
 if(mac){await record('Cmd+F8');assert.equal(state.shortcuts.editor,null);await done();await probe('Cmd+F8');assert.equal(state.preferences.sidebar,!initialSidebar);await probe('Cmd+F8');await preferences();}
 await record('Ctrl+F8');assert.equal(state.shortcuts.editor,null);await done();await probe('Ctrl+F8');assert.equal(state.preferences.sidebar,!initialSidebar);await probe('Ctrl+F8');
 await preferences();await click('pref-shortcut-sidebar-add');await probe(primary+'+WheelUp',{recording_id:state.shortcuts.editor.serial});await done();await probe(primary+'+WheelUp');assert.equal(state.preferences.sidebar,!initialSidebar);await probe(primary+'+WheelUp');
 pass('Explicit Control key and modified wheel bindings are saved and dispatched');
 await resize(1440,900);await click('view-changes');await type('commit-message','PRIVATE_DESKTOP_DRAFT 中文 日本語 한국어 e\u0301 🦀');await click(state.controls.find(c=>c.id.startsWith('change-staged-')).id);await until(s=>s.tabs.find(t=>t.id===s.active_tab).patch_files?.length===1,'Unicode diff loaded');assert.equal(state.tabs.find(t=>t.id===state.active_tab).patch_files[0].path.normalize('NFC'),path.basename(file).normalize('NFC'));await capture('02-unicode-changes');
 await click('menu-file');await click('file-open');await until(s=>control('open-filter')?.enabled);await type('open-filter','資料 one');await until(s=>control('open-row-0')?.enabled);await click('open-row-0');await until(s=>control('dialog-submit')?.enabled&&samePath(s.dialog.first,repo),'selected repository ready');await resize(780,660);await capture('03-compact-repository-browser');await click('dialog-cancel');assert.equal(state.dialog,null);assert.deepEqual(preserved(),before);
 pass('Unicode repository discovery, compact folder selection and cancellation preserve local changes');
 await resize(1440,900);await close();await open();assert.deepEqual(binding('sidebar').inputs,['Ctrl+F8',primary+'+WheelUp']);await click('view-changes');assert(state.tabs.find(t=>t.id===state.active_tab).draft_present);await probe('Ctrl+F8');assert.equal(state.preferences.sidebar,!initialSidebar);await probe('Ctrl+F8');
 pass('Physical Control and mouse bindings and Unicode draft survive reopen');
 await click('toolbar-terminal');await until(s=>s.terminal?.running&&shell.started(s.terminal.tail),'terminal ready');await terminalProbe('typing');await until(s=>s.terminal.tail.includes('ROCKET_TYPING_OK')&&s.terminal.tail.trimEnd().endsWith('>'));
 const terminalLine=text=>state.terminal.tail.split(/\r?\n/).some(l=>l.trim()===text);
 await terminalProbe('interrupt-start');await until(()=>terminalLine('ROCKET_INTERRUPT_READY'),'running foreground command');const at=performance.now();await terminalProbe('cancel-line');await until(s=>s.terminal.tail.trimEnd().endsWith('>'),'same shell prompt');report.interrupt_ms=Math.round(performance.now()-at);assert(report.interrupt_ms<4000);assert(!terminalLine('ROCKET_INTERRUPT_UNEXPECTED'));
 await terminalProbe('interrupt-status');await until(()=>terminalLine('ROCKET_SHELL_True')&&terminalLine('ROCKET_STATE_42'),'original shell PID and state');
 await command(shell.after);await until(s=>s.terminal.tail.includes('AFTER_INTERRUPT')&&s.terminal.tail.trimEnd().endsWith('>'));await capture('04-terminal');
 pass('Terminal Control+C still interrupts and the same shell accepts the next command');
 await click('terminal-hide');await preferences();await click('pref-shortcut-sidebar-reset');assert.deepEqual(binding('sidebar').inputs,[primary+'+B']);await done();assert.deepEqual(preserved(),before);await close();
 const folders=[...fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).map(n=>path.join(root,n)),path.join(profile,'logs')].filter(p=>fs.existsSync(p));
 const events=folders.flatMap(p=>fs.readdirSync(p).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(p,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(events.length>0);assert(!JSON.stringify(events).includes('PRIVATE_DESKTOP_'));assert(!events.some(e=>e.dropped>0));assert(events.some(e=>JSON.stringify(e).includes('rocket_shortcut')));report.events=events.length;report.preserved=preserved();report.passed=true;
 pass('Native controls produce diagnostics without private payloads or dropped events; repository hashes unchanged');
}catch(error){report.error=error.stack;try{await capture('failure')}catch{}throw error}finally{
 try{if(service&&service.exitCode===null&&service.signalCode===null){if(state?.window==='running')await close();await client.raw('POST','/shutdown');const at=performance.now();while(service.exitCode===null&&service.signalCode===null){if(performance.now()-at>15000){service.kill();throw Error('Owned desktop test server did not stop')}await delay(50)}}}catch(error){report.passed=false;report.cleanup_error=error.stack;throw error}finally{if(client)await client.finish();fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log('RESULT '+path.join(root,'result.json'))}
}}
run().catch(error=>{console.error(error);process.exitCode=1});
