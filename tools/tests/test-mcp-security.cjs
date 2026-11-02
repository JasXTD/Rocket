// Owned local fixtures only. Default state must not disclose private editor/shell text.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {RocketMcpClient}=require('../mcp/mcp-client.cjs');
const root=path.resolve('work/mcp-fixtures/security-'+crypto.randomUUID()),repo=path.join(root,'client'),branch='rocket-ui-test/security';
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'hooks'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_|ROCKET_MCP_HTTP_PROFILE)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'hooks')});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8'}).trimEnd();
git(['init','--initial-branch='+branch]);
for(const[k,v]of[['user.name','Synthetic Security Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',path.join(root,'hooks')]])git(['config',k,v]);
const write=s=>fs.writeFileSync(path.join(repo,'conflict.txt'),s);
const commit=m=>{git(['add','.']);git(['commit','-m',m]);};
write('base\n');commit('fixture');git(['checkout','-b','other']);write('theirs\n');commit('side');git(['checkout',branch]);write('ours\n');commit('main');
assert.throws(()=>git(['merge','other']));fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const executable=path.resolve(process.env.ROCKET_SECURITY_MCP_EXE||'target/debug/rocket-mcp.exe');
const client=new RocketMcpClient(executable,root,env);
const report={passed:false,root,executable,binarySha256:crypto.createHash('sha256').update(fs.readFileSync(executable)).digest('hex'),ownedPid:client.child.pid,cases:[]};let state;
const wait=ms=>new Promise(r=>setTimeout(r,ms));
async function until(test,label){const start=Date.now();for(;;){state=await client.tool('rocket_state');if(test(state))return;assert(Date.now()-start<30000,label+' timed out');await wait(70);}}
const settle=()=>until(s=>!s.busy&&!s.input_pending,'settled UI');
const active=()=>state.tabs.find(t=>t.id===state.active_tab);
async function click(target){await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();}
async function close(){await client.tool('rocket_close');await until(s=>s.window==='closed'||s.window==='not_started'||s.dialog?.kind==='quit','close');if(state.dialog?.kind==='quit'){await click('dialog-submit');await until(s=>s.window==='closed','closed');}}
async function run(){try{
 report.serverInfo=(await client.initialize()).serverInfo;await client.tool('rocket_open',{path:repo});await settle();
 await click('view-changes');await click('change-unstaged-'+Buffer.from('conflict.txt').toString('hex'));await click('conflict-open');
 await client.tool('rocket_type',{target:'conflict-result',text:'PRIVATE_SECURITY_DRAFT\n',expected_tab:state.active_tab});await settle();
 assert.equal(active().conflict_editor.draft,undefined,'Default MCP state disclosed a private conflict draft');
 assert.equal(state.external_tools.arguments,undefined);assert.equal(state.external_tools.result,undefined);
 assert.equal(active().conflict_editor.draft_bytes,23);assert(!JSON.stringify(state).includes('PRIVATE_SECURITY_DRAFT'));
 report.cases.push('default state omits editor content and retains useful metadata');
 await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal?.running,'terminal started');
 assert.equal(state.terminal.tail,undefined);assert.equal(state.terminal.grid,undefined);assert.equal(state.private_state_opt_in,false);
 report.cases.push('default state omits shell output and grid text');
 const marker=path.join(root,'rocket-private-state');fs.writeFileSync(marker,'synthetic-content-only-v1\n');
 await until(s=>s.private_state_opt_in===true&&s.terminal?.tail?.includes('PS '),'explicit fixture consent');
 assert.equal(active().conflict_editor.draft,'PRIVATE_SECURITY_DRAFT\n');
 await client.tool('rocket_type',{target:'terminal-command',text:"Write-Output ('ROCKET_' + 'TERMINAL_OK')",expected_tab:state.active_tab});await settle();await click('terminal-send');await until(s=>s.terminal.tail.includes('ROCKET_TERMINAL_OK'),'synthetic output');
 fs.unlinkSync(marker);await click('terminal-hide');
 assert.equal(state.private_state_opt_in,false);assert.equal(state.terminal.tail,undefined);assert.equal(state.terminal.grid,undefined);assert(!JSON.stringify(state).includes('PRIVATE_SECURITY_DRAFT'));assert(!JSON.stringify(state).includes('ROCKET_TERMINAL_OK'));
 report.cases.push('revoking local consent immediately hides editor and hidden terminal text');
 await click('menu-terminal');if(state.controls.some(c=>c.id==='terminal-end'&&c.enabled))await click('terminal-end');
 await close();await client.finish();report.cleanExit=client.exited&&client.exitCode===0;assert(report.cleanExit);report.passed=true;
}finally{if(!client.exited)client.terminateOwnedProcess();fs.writeFileSync(path.join(root,'report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));}}
run().catch(e=>{console.error(e.stack);process.exitCode=1;});
