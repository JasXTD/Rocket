// Complete packaged Mac consumer acceptance. No credentials, external APIs or user profiles.
'use strict';
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),http=require('node:http'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
assert.equal(process.platform,'darwin');assert.equal(process.arch,'arm64');
const exe=fs.realpathSync(process.env.ROCKET_MCP_EXE||''),out=path.resolve(process.env.ROCKET_MCP_RESULTS||'');
assert(exe.endsWith('/Rocket.app/Contents/MacOS/rocket-mcp'),'Run the helper inside the verified app');
fs.mkdirSync(out,{recursive:true});
const root=fs.realpathSync(fs.mkdtempSync(path.join(out,'mac-package-'))),repo=path.join(root,'資料 日本語 é'),profile=path.join(root,'profile'),home=path.join(root,'home');
for(const p of [repo,home,path.join(root,'template')])fs.mkdirSync(p,{mode:0o700});
fs.writeFileSync(path.join(root,'empty.config'),'');fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');fs.writeFileSync(path.join(root,'rocket-terminal-shell'),'zsh-isolated-v1\n');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|SSH_|GCM_|GH_|GITHUB_TOKEN|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{HOME:home,ZDOTDIR:home,XDG_CONFIG_HOME:path.join(home,'.config'),XDG_STATE_HOME:path.join(home,'.local/state'),GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_OPTIONAL_LOCKS:'0',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'template')});
const git=args=>cp.execFileSync('/usr/bin/git',['-C',repo,...args],{env,encoding:'utf8',stdio:['ignore','pipe','pipe']}).trimEnd();
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const branch='rocket-ui-test/mac-package';git(['init','-b',branch]);for(const [k,v]of [['user.name','Mac Fixture'],['user.email','fixture@example.invalid'],['core.autocrlf','false'],['commit.gpgsign','false']])git(['config',k,v]);
const file=path.join(repo,'資料.rs');fs.writeFileSync(file,'// initial\n');git(['add','.']);git(['commit','-m','Synthetic Mac package']);fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
fs.appendFileSync(file,'// PRIVATE_MAC_STAGED\n');git(['add','.']);fs.appendFileSync(file,'// PRIVATE_MAC_UNSTAGED\n');fs.writeFileSync(path.join(repo,'rocket-completion-fixture'),'PRIVATE_MAC_UNTRACKED');
const fingerprint=()=>({head:git(['rev-parse','HEAD']),index:sha(path.join(repo,'.git/index')),config:sha(path.join(repo,'.git/config')),file:sha(file),untracked:sha(path.join(repo,'rocket-completion-fixture'))}),before=fingerprint();
let client,service,state,mode='unavailable',metadata=0,closed=0;const sockets=new Set();
const report={passed:false,cases:[],timings:{},binary_sha256:sha(exe),root};
const delay=ms=>new Promise(r=>setTimeout(r,ms)),pass=s=>{report.cases.push(s);console.log('PASS '+s)};
const server=http.createServer((req,res)=>{
 assert(!req.headers.authorization&&!req.headers.cookie,'Fixture must not receive login credentials');assert(req.url.endsWith('/releases/latest'),'No asset download expected');metadata++;
 if(mode==='slow'){res.on('close',()=>closed++);return;}
 if(mode==='unavailable'){res.writeHead(404);res.end('{}');return;}
 if(mode==='error'){res.writeHead(503);res.end('PRIVATE_MAC_SERVER_ERROR');return;}
 const version='99.0.0';res.setHeader('Content-Type','application/json');res.end(JSON.stringify({tag_name:'v'+version,draft:false,prerelease:false,html_url:'https://github.com/JasXTD/Rocket/releases/tag/v'+version,body:'Synthetic Mac release · 中文 日本語 한국어 🦀\n\n'+('Reviewed release notes.\n'.repeat(40)),assets:[{id:7,name:`Rocket-${version}-macos-arm64.tar.gz`,state:'uploaded',size:1234,digest:'sha256:'+'a'.repeat(64)}]}));
});server.on('connection',s=>{sockets.add(s);s.on('close',()=>sockets.delete(s))});
const control=id=>state.controls?.find(c=>c.id===id),menu=id=>state.native_menus?.find(c=>c.id===id),play=()=>state.integrations?.playground;
async function until(fn,label='state',ms=45000){const at=performance.now();for(;;){state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-at<ms,label+' timeout; '+JSON.stringify({dialog:state.dialog?.kind,updates:state.updates,play:play(),spelling:state.spelling}));await delay(45)}}
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading))}
async function stable(){let previous='',n=0;await until(()=>{const next=JSON.stringify(state.controls);n=next===previous?n+1:0;previous=next;return n>=2&&!state.input_pending},'stable layout')}
async function click(id,wait=true){await stable();assert(control(id)?.enabled,'Missing/disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(wait)await settled()}
async function native(id){await settled();assert(menu(id)?.enabled,'Native row disabled/missing '+id);const r=await client.tool('rocket_menu',{target:id,expected_tab:state.active_tab});assert(r.native_action_queued);await settled()}
async function type(id,text){await stable();await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settled()}
async function probe(name){await stable();await client.tool('rocket_terminal_probe',{probe:name,expected_tab:state.active_tab});await settled()}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height,'resize');await stable()}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'))}
async function dismiss(){await client.tool('rocket_escape');await until(s=>!s.dialog,'dialog dismissed')}
const line=text=>state.terminal?.tail.split(/\r?\n/).filter(l=>l.trim()===text).length||0;
const prompt=()=>state.terminal?.tail.trimEnd().endsWith('>');
async function run(){try{
 await new Promise(r=>server.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-update-api'),'http://127.0.0.1:'+server.address().port);
 cp.execFileSync(exe,['--init-http',profile,root],{env,stdio:'pipe'});const port=net.createServer();await new Promise(r=>port.listen(0,'127.0.0.1',r));const number=port.address().port;await new Promise(r=>port.close(r));fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${number}\n${root}\n`);
 const fd=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
 for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(n>150)throw e;await delay(100)}}
 await client.initialize();await client.tool('rocket_open',{path:repo});await settled();await resize(1180,620);
 await until(s=>s.native_menus?.length>60&&s.updates.startup_checked&&!s.updates.running,'native menu and startup check');assert.equal(metadata,1);
 assert(!control('menu-file'));assert(menu('repository-merge').parent);assert(menu('repository-rebase').parent);assert(menu('repository-playground'));
 await assert.rejects(client.tool('rocket_menu',{target:'preferences',expected_tab:state.active_tab+999}),/Active repository changed/);
 for(const target of ['shortcut-push','file-exit','9003'])await assert.rejects(client.tool('rocket_menu',{target,expected_tab:state.active_tab}),/reviewed navigation/);
 const sidebar=state.preferences.sidebar;await native('shortcut-sidebar');await until(s=>s.preferences.sidebar!==sidebar);await native('shortcut-sidebar');await until(s=>s.preferences.sidebar===sidebar);
 for(const id of ['repository-merge','repository-rebase','repository-stash','repository-settings']){await native(id);await until(s=>!!s.dialog,'review opens '+id);await assert.rejects(client.tool('rocket_menu',{target:'repository-merge',expected_tab:state.active_tab}),/disabled/);await capture(id);await dismiss();assert.deepEqual(fingerprint(),before)}
 pass('Actual nested Cocoa actions, stale/unsafe/disabled refusal and cancelled reviews preserve dirty Unicode repository');
 await resize(780,620);mode='available';await native('help-updates');await until(s=>s.updates.open&&!s.updates.running&&s.updates.status==='available','available update');assert.equal(state.updates.release.version,'99.0.0');
 // Worker state can arrive before the frame containing its new buttons is painted.
 await until(()=>control('update-download')&&control('update-later'),'available update controls');await stable();
 for(const id of ['update-download','update-later']){const r=control(id).rect;assert(r[0]>=0&&r[1]>=0&&r[2]<=780&&r[3]<=620,id+' clipped')}
 await capture('compact-update');await click('update-later');mode='error';await native('help-updates');await until(s=>!s.updates.running&&s.updates.status==='error','update error');assert(state.updates.error);await capture('compact-update-error');await click('update-later');
 mode='slow';await native('help-updates');await until(s=>s.updates.running&&control('update-stop')?.enabled,'pending check');const stop=performance.now();await click('update-stop',false);await until(s=>!s.updates.running,'cancelled check',5000);report.timings.update_cancel_ms=Math.round(performance.now()-stop);await until(()=>closed>0,'cancelled request closed',5000);await click('update-later');
 pass('Packaged updater: one launch check, native Help recheck, compact available/error states and cancellation close the loopback request');
 await resize(1180,620);await native('preferences');await until(()=>control('pref-page-commit')?.enabled);await click('pref-page-commit');await click('pref-spell-check');await click('dialog-submit');await native('shortcut-changes');await type('commit-message','Ths sentnce has misspelled wrds');await until(s=>s.spelling.ready&&!s.spelling.waiting&&s.spelling.checked_texts>0&&s.spelling.misspellings>0,'native spelling results');assert(!state.spelling.failed);await capture('native-spelling');
 await type('commit-message','');await native('preferences');await click('pref-page-commit');await click('pref-spell-check');await click('dialog-submit');await until(s=>!s.spelling.ready&&!s.spelling.waiting,'spelling stopped');
 pass('Cocoa spelling checks actual commit input and stops cleanly without learning dictionary words');
 await native('repository-playground');await until(()=>play()?.loaded&&control('int-play-run')?.enabled,'Lua scratch editor');await click('int-play-run');await until(()=>!play().busy&&play().has_preview,'bound Lua preview');assert.equal(play().error,null);assert(play().widget_ids.includes('increment'));await click('int-play-ui-increment');await until(()=>!play().busy&&play().text?.labels?.some(t=>t.includes('updated 1 times')),'Lua interaction');await capture('bound-lua-preview');
 await type('int-play-source','return function(ctx) return { view = ui.text("資料 Mac helper") } end');await click('int-play-run');await until(()=>!play().busy&&play().has_preview&&play().preview_current,'Unicode Lua preview');assert.equal(play().error,null);
 await type('int-play-source','return function( -- invalid');await click('int-play-run');await until(()=>!play().busy&&play().error,'Lua diagnostic');await capture('lua-error');
 await type('int-play-source','return function(ctx) return { view = ui.text("Recovered") } end');await click('int-play-run');await until(()=>!play().busy&&play().preview_current&&!play().error,'Lua recovers');await click('int-play-save');await until(()=>!play().dirty&&!play().busy);await native('shortcut-changes');
 pass('Packaged, hash-bound Lua helper renders and updates real UI, handles Unicode and syntax errors, and recovers');
 await native('shortcut-terminal');await until(s=>s.terminal?.running,'zsh starts');await probe('zsh-initialize');await until(s=>s.terminal.tail.includes('ROCKET_ZSH_')&&prompt(),'isolated zsh initialized');assert(state.terminal.tail.includes(path.join(root,'rocket-ui-state','terminal-history.log')));
 await probe('typing');await until(()=>line('ROCKET_TYPING_OK')>0&&prompt(),'zsh typing');await probe('history-seed');await until(()=>line('ROCKET_HISTORY_SECOND')>0&&prompt());const count=line('ROCKET_HISTORY_SECOND');await probe('history-previous');await probe('submit');await until(()=>line('ROCKET_HISTORY_SECOND')>count&&prompt(),'zsh history');
 await probe('completion-command');await probe('completion-argument');await until(()=>line('ROCKET_COMPLETION_OK')>0&&prompt(),'zsh command completion');await probe('completion-path');await probe('submit');await until(()=>line('rocket-completion-fixture')>0&&prompt(),'zsh path completion');
 await probe('redraw');await probe('resize-draft');await resize(780,620);await probe('resize-edit');await until(()=>line('ROCKET_RESIZE_中文_🚀_é_OK')>0&&prompt(),'Unicode draft survives resize');await capture('compact-zsh');
 await probe('interrupt-start');await until(()=>line('ROCKET_INTERRUPT_READY')>0);const interrupt=performance.now();await probe('cancel-line');await until(prompt,'zsh interrupt prompt',4000);report.timings.interrupt_ms=Math.round(performance.now()-interrupt);assert.equal(line('ROCKET_INTERRUPT_UNEXPECTED'),0);await probe('interrupt-status');await until(()=>line('ROCKET_SHELL_True')>0&&line('ROCKET_STATE_42')>0&&prompt(),'same zsh after interrupt');
 await probe('typing');await until(()=>line('ROCKET_TYPING_OK')>0&&prompt());const history=fs.readFileSync(path.join(root,'rocket-ui-state','terminal-history.log'),'utf8');assert(history.includes('ROCKET_HISTORY_SECOND'));assert(!fs.existsSync(path.join(home,'.zsh_history')));
 pass('zsh editing, history, command/path completion, Unicode resize and Ctrl+C work in the same shell with isolated persisted history');
 assert.deepEqual(fingerprint(),before);await capture('final');report.passed=true;
}catch(e){report.error=e.stack;try{await capture('failure');fs.writeFileSync(path.join(root,'failure-state.json'),JSON.stringify(state,null,2))}catch{}throw e}finally{
 try{if(client&&service&&service.exitCode===null){if(state?.window==='running'){await client.tool('rocket_close');await until(s=>s.window==='closed'||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await click('dialog-submit');await until(s=>s.window==='closed')}}await client.raw('POST','/shutdown');const at=performance.now();while(service.exitCode===null&&service.signalCode===null){if(performance.now()-at>15000)throw Error('Owned MCP service did not stop');await delay(50)}}}catch(e){report.passed=false;report.cleanup_error=e.stack;if(service?.exitCode===null)service.kill();process.exitCode=1}
 if(client)await client.finish();for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));
 const logs=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).map(n=>path.join(root,n)).concat(path.join(profile,'logs')).filter(p=>fs.existsSync(p));
 const events=logs.flatMap(p=>fs.readdirSync(p).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n)).flatMap(n=>fs.readFileSync(path.join(p,n),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 report.events=events.length;if(!events.length||events.some(e=>e.dropped_before!==0)||JSON.stringify(events).includes('PRIVATE_MAC_')){report.passed=false;report.log_error='Missing, dropped or private diagnostic events';process.exitCode=1}
 fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log('RESULT '+path.join(root,'result.json'));
}}
run().catch(e=>{console.error(e);process.exitCode=1});
