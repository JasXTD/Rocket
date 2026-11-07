const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {RocketMcpClient}=require('../mcp/mcp-client.cjs');
const {windows,exe,gone,shell}=require('./platform.cjs');
const root=path.resolve('work/mcp-fixtures/terminal-'+crypto.randomUUID().replaceAll('-','')),repo=path.join(root,'client'),branch='rocket-ui-test/terminal';
fs.mkdirSync(repo,{recursive:true});
// Explicit local consent: this runner creates only synthetic shell/editor data.
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.mkdirSync(path.join(root,'hooks'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'hooks')});
function git(args){const r=cp.spawnSync('git',args,{cwd:repo,env,windowsHide:true,encoding:'utf8'});assert.equal(r.status,0,r.stderr);return r.stdout;}
git(['init','--initial-branch='+branch]);for(const [k,v]of [['user.name','Rocket Terminal Fixture'],['user.email','rocket@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',path.join(root,'hooks')]])git(['config',k,v]);
fs.writeFileSync(path.join(repo,'baseline.txt'),'synthetic\n');git(['add','.']);git(['commit','-m','Synthetic terminal baseline']);fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
// Rocket shows and checks the resolved path on Linux, where work/ may be a link.
const shown=windows?repo:fs.realpathSync(repo);
const client=new RocketMcpClient(path.resolve(exe('target/release/rocket-mcp')),root,env);
const report={root,cases:[]};let state;
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const settle=async()=>{for(let n=0;n<8;n++){state=await client.tool('rocket_wait',{timeout_ms:15000});if(!state.busy&&!state.input_pending)return state;}throw Error('UI did not settle');};
const click=async target=>{await client.tool('rocket_click',{target,expected_tab:state.active_tab});return settle();};
async function until(predicate){const start=Date.now();for(;;){state=await client.tool('rocket_state');assert(!state.terminal?.error,JSON.stringify(state.terminal));if(predicate(state))return state;assert(Date.now()-start<15000,'Terminal condition timed out: '+JSON.stringify(state.terminal));await pause(100);}}
async function command(text){await client.tool('rocket_type',{target:'terminal-command',text,expected_tab:state.active_tab});await settle();await click('terminal-send');}
async function capture(name){const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError,JSON.stringify(r));const image=r.content.find(x=>x.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));}
async function main(){try{
 await client.initialize();await client.tool('rocket_open',{path:repo});await settle();await click('view-changes');await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal?.running&&shell.started(s.terminal.tail));const pid=state.terminal.pid;
 await client.tool('rocket_type',{target:'terminal-command',text:shell.ok,expected_tab:state.active_tab});await settle();await client.tool('rocket_terminal_enter',{expected_tab:state.active_tab});await until(s=>s.terminal.tail.includes('ROCKET_TERMINAL_OK'));report.cases.push('native Enter key submits the command field');
 await command(shell.set);await command(shell.get);await until(s=>s.terminal.tail.includes('ROCKET_VALUE_42'));assert.equal(state.terminal.pid,pid);
 await command(shell.location);await until(s=>s.terminal.tail.includes(shown));await command(shell.accent);await until(s=>s.terminal.tail.includes('é'));report.cases.push('persistent '+shell.name+', correct working directory, Unicode, one process');
 await assert.rejects(()=>client.tool('rocket_type',{target:'terminal-command',text:shell.denied,expected_tab:state.active_tab}),/fixed synthetic probes/);report.cases.push('MCP arbitrary shell input rejected');
 const refresh=state.tabs[0].refresh_count;await command(shell.edit(path.join(shown,'rocket-terminal-fixture.txt')));await until(s=>s.tabs[0].refresh_count>refresh&&s.tabs[0].change_count===1);await settle();assert.equal(fs.readFileSync(path.join(repo,'rocket-terminal-fixture.txt'),'utf8'),'ROCKET_TERMINAL_EDIT');report.cases.push('terminal file edit detected by automatic repository refresh');
 await command(shell.sleep);await until(s=>s.terminal.tail.trimEnd().endsWith(shell.sleep));
 const start=performance.now();await client.tool('rocket_type',{target:'commit-message',text:'Draft while terminal is busy',expected_tab:state.active_tab});report.input_while_busy_ms=performance.now()-start;assert(report.input_while_busy_ms<1000);await settle();assert(state.tabs[0].draft_present);await click('terminal-interrupt');await until(s=>s.terminal.tail.trimEnd().endsWith(shown+'\n>'));await command(shell.after);await until(s=>s.terminal.tail.includes('AFTER_INTERRUPT'));assert.equal(state.terminal.pid,pid);report.cases.push('UI typing during blocked command, Ctrl+C resumes the same shell');
 await command(shell.error);await until(s=>s.terminal.tail.includes(shell.errorShown));await capture('terminal-error');report.cases.push(shell.name+' errors remain visible in terminal');
 await command(shell.flood);await until(s=>s.terminal.scrollback_rows===1000);await click('terminal-clear-scrollback');assert(state.terminal.scrollback_rows<1000);report.cases.push('output flood bounded, clear scrollback');
 await click('terminal-hide');assert.equal(state.terminal.visible,false);await click('menu-terminal');await click('terminal-new');assert.equal(state.terminal.pid,pid);assert.equal(state.terminal.visible,true);
 await click('terminal-screen');await capture('terminal');report.cases.push('hide/show preserves session, direct-input focus and native capture');
 await client.tool('rocket_resize',{width:780,height:620});await until(s=>s.viewport[0]===780&&s.viewport[1]===620&&s.terminal.columns<100);await capture('terminal-narrow');assert(state.controls.some(c=>c.id==='terminal-command'&&c.enabled));
 await client.tool('rocket_resize',{width:1480,height:920});await until(s=>s.viewport[0]===1480&&s.viewport[1]===920);report.cases.push('native narrow and wide layout with '+(windows?'ConPTY':'PTY')+' resize');
 await click('terminal-end');await until(s=>s.terminal.closed);assert.equal(state.terminal.running,false);
 await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal?.running&&shell.started(s.terminal.tail));assert.notEqual(state.terminal.pid,pid);report.cases.push('end session and restart');
 await command('exit');await until(s=>s.terminal.closed);assert.equal(state.terminal.exit_code,0);report.cases.push('normal shell exit');
 await client.tool('rocket_type',{target:'commit-message',text:'',expected_tab:state.active_tab});await settle();await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal.running);const tabPid=state.terminal.pid;
 const beforeClose=await client.tool('rocket_logs',{limit:1000});const exits=beforeClose.records.filter(r=>r.operation==='terminal_exit'&&r.event==='request_start').length;
 await click('close-tab-'+state.active_tab);assert.equal(state.tabs.length,0);
 for(let n=0;;n++){const log=await client.tool('rocket_logs',{limit:1000});if(log.records.filter(r=>r.operation==='terminal_exit'&&r.event==='request_start').length>exits)break;assert(n<100,'missing terminal exit after tab close');await pause(50);}
 assert(gone(tabPid),'shell still running after tab close');report.cases.push('repository tab closure logs terminal exit and reaps the owned process');
 await client.tool('rocket_open',{path:repo});await settle();await click('view-changes');
 await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal.running);report.shutdown_pid=state.terminal.pid;
 const logs=await client.tool('rocket_logs',{limit:1000});assert.equal(state.dropped_events,0);assert(logs.records.some(r=>r.operation==='terminal_start'));assert(logs.records.some(r=>r.operation==='terminal_exit'));for(const payload of ['ROCKET_TERMINAL_EDIT','ROCKET_EXPECTED_ERROR',shell.variable,'Draft while terminal'])assert(!JSON.stringify(logs).includes(payload));report.cases.push('terminal lifecycle logs exclude commands and output');
 await client.tool('rocket_type',{target:'commit-message',text:'',expected_tab:state.active_tab});await settle();await client.tool('rocket_close');state=await client.tool('rocket_state');if(state.dialog?.kind==='quit')await click('dialog-submit');await client.finish();
 assert(gone(report.shutdown_pid),'shell still running after window shutdown');report.cases.push('normal window shutdown reaps its '+shell.name+' process');report.passed=true;
}finally{fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(!client.exited){try{await client.finish();}catch{client.terminateOwnedProcess();}}}}
main().catch(e=>{console.error(e);process.exitCode=1;});
