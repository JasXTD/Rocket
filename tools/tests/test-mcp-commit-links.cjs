// Focused Commit link acceptance. Real local Git, isolated MCP profile, no hosting requests.
'use strict';
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const baseline=process.argv.includes('--baseline');
const root=path.resolve('work/mcp-fixtures/commit-links-'+crypto.randomUUID());
const repo=path.join(root,'資料 repository'),profile=path.join(root,'http-profile'),exe=path.resolve('target/release/rocket-mcp.exe');
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'template'));
for(const name of ['global.config','system.config'])fs.writeFileSync(path.join(root,name),'');
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'global.config'),GIT_CONFIG_SYSTEM:path.join(root,'system.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'template')});
const git=(...args)=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:'pipe'}).trimEnd();
const hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const branch='rocket-ui-test/commit-links';git('init','-b',branch);
for(const [key,value]of [['user.name','Link Fixture'],['user.email','fixture@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false']])git('config',key,value);
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
for(const name of ['表示.rs','staged.txt'])fs.writeFileSync(path.join(repo,name),'PRIVATE_LINK_FIXTURE\n');
git('add','.');git('commit','-m','Keep commit links clear 日本語');
git('remote','add','origin','https://github.com/example/rocket.git');git('remote','add','upstream','git@gitlab.com:team/rocket.git');
fs.appendFileSync(path.join(repo,'表示.rs'),'local edit\n');fs.appendFileSync(path.join(repo,'staged.txt'),'staged edit\n');git('add','staged.txt');fs.writeFileSync(path.join(repo,'untracked.txt'),'untracked\n');
const config=fs.readFileSync(path.join(repo,'.git/config'));
const fingerprint=()=>({head:git('rev-parse','HEAD'),index:git('ls-files','--stage','-z'),status:git('status','--porcelain=v2','-z'),files:['表示.rs','staged.txt','untracked.txt'].map(name=>hash(path.join(repo,name))),config:hash(path.join(repo,'.git/config'))});
const before=fingerprint(),relay=gitRelay(root,env,'tools/helpers/file-history-preview/git-relay.rs');
const report={passed:false,baseline,root,exe_hash:hash(exe),cases:[]};let service,client,state;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const control=id=>state.controls.find(c=>c.id===id),pass=text=>{report.cases.push(text);console.log('PASS '+text)};
async function until(test,label='state',timeout=60000){const end=performance.now()+timeout;for(;;){state=await client.tool('rocket_state');if(test(state))return;assert(performance.now()<end,label+' timeout');await delay(70)}}
async function settled(){await until(s=>!s.busy&&!s.input_pending&&!s.tabs?.some(t=>t.loading),'settled')}
async function stable(){let previous='',count=0;await until(s=>{const next=JSON.stringify(s.controls);count=next===previous?count+1:0;previous=next;return count>=2},'stable controls')}
async function click(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settled()}
async function type(target,text){await reveal(target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settled()}
async function scroll(direction,pixels=350){await client.tool('rocket_scroll',{target:'commit-link-scroll',direction,pixels,expected_tab:state.active_tab});await settled();await stable()}
async function reveal(target){await stable();for(let i=0;i<8;i++){const c=control(target);if(c&&c.rect[3]-c.rect[1]>16)return;await scroll('down')}throw Error('Cannot reveal '+target)}
async function links(){await until(s=>s.commit_link&&!s.commit_link.loading,'link result');await stable()}
async function open(){await click('view-commits');await click('commit-'+before.head);await click('commit-browser-link');await links()}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const image=result.content.find(c=>c.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2))}
function bounds(){const [width,height]=state.viewport;for(const c of state.controls.filter(c=>c.id.startsWith('commit-link-'))){assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=width&&c.rect[3]<=height,'Visible bounds: '+c.id)}const viewport=control('commit-link-scroll');for(const id of ['commit-link-copy','commit-link-open']){const c=control(id);assert(c,'Persistent action '+id);assert(c.rect[1]>=viewport.rect[3],'Footer stays outside scrolling content: '+id)}}
async function start(){
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true,stdio:'pipe'});
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));
 fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
 const fd=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',fd]});fs.closeSync(fd);client=new RocketHttpClient(profile);
 for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(n>120)throw e;await delay(100)}}
 report.version=(await client.initialize()).serverInfo.version;await client.tool('rocket_open',{path:repo});await settled();relay.check(state);
 await client.tool('rocket_resize',{width:1440,height:1000});await until(s=>s.viewport[0]===1440);await open();
}
async function run(){
 assert.equal(state.commit_link.links.length,2);assert.equal(state.commit_link.selected,null);
 if(!baseline){assert.equal(control('commit-link-open').enabled,false);assert.equal(control('commit-link-copy').enabled,false);assert(!control('commit-link-name'));bounds()}
 await capture(baseline?'before-choice':'choice');await click('commit-link-remote-0');
 const url=state.commit_link.links[0].url;assert.equal(url,'https://github.com/example/rocket/commit/'+before.head);
 await click('commit-link-copy');assert.match(state.commit_link.notice,/copied/);await click('commit-link-open');assert(state.github.opened_urls.includes(url));await capture(baseline?'before-selected':'selected');
 if(baseline)return;
 pass('Explicit remote choice; exact pinned URL copied/opened through intercepted browser action');
 await client.tool('rocket_resize',{width:780,height:620});await until(s=>s.viewport[0]===780);await stable();bounds();await capture('compact-selected');
 await click('commit-link-options');await type('commit-link-name','missing');assert.equal(state.commit_link.links.length,0);assert.equal(control('commit-link-open').enabled,false);
 await reveal('commit-link-refresh');await click('commit-link-refresh');await links();assert(state.commit_link.error);await scroll('down',1000);bounds();await capture('compact-error');
 await type('commit-link-name','upstream');assert.equal(state.commit_link.error,null,'editing clears stale validation error');await reveal('commit-link-custom');await click('commit-link-custom');await type('commit-link-server','http://gitlab.com');await reveal('commit-link-refresh');await scroll('up',2000);await reveal('commit-link-refresh');await click('commit-link-refresh');await links();assert(state.commit_link.error);assert.equal(state.commit_link.links.length,0);
 await type('commit-link-server','https://gitlab.com');assert.equal(state.commit_link.error,null);await scroll('up',2000);await reveal('commit-link-refresh');await click('commit-link-refresh');await links();assert.equal(state.commit_link.error,null);assert.equal(state.commit_link.selected,0);assert.equal(state.commit_link.links[0].url,'https://gitlab.com/team/rocket/-/commit/'+before.head);await scroll('down',2000);bounds();await capture('compact-custom');
 pass('Compact scrolling keeps actions visible; invalid remote/server errors recover without stale links');
 await client.tool('rocket_escape');await until(s=>!s.commit_link);await open();
 const gateMode='^remote\\..*\\.(url|pushurl|fetch)$';fs.writeFileSync(relay.gate,gateMode);if(fs.existsSync(relay.marker))fs.unlinkSync(relay.marker);
 await click('commit-link-options');await reveal('commit-link-refresh');await click('commit-link-refresh');await until(s=>s.commit_link?.loading&&fs.existsSync(relay.marker),'gated link read');
 const pids=fs.readFileSync(relay.marker,'utf8').trim().split(/\r?\n/).map(Number);await scroll('up',2000);await capture('loading');const started=performance.now();await click('commit-link-cancel');await links();report.cancel_feedback_ms=Math.round(performance.now()-started);assert.match(state.commit_link.notice,/cancelled/);assert.equal(state.commit_link.links.length,0);assert.equal(control('commit-link-open').enabled,false);
 const deadline=performance.now()+5000;while(pids.some(pid=>{try{process.kill(pid,0);return true}catch{return false}})){assert(performance.now()<deadline,'Cancelled Git process must exit');await delay(50)}
 fs.unlinkSync(relay.gate);await reveal('commit-link-refresh');await click('commit-link-refresh');await links();assert.equal(state.commit_link.links.length,2);assert.equal(state.commit_link.selected,null);pass('First Stop cancels a gated real Git read; refreshing recovers with no stale result');
 await client.tool('rocket_escape');await until(s=>!s.commit_link);
 git('remote','remove','origin');git('remote','remove','upstream');await open();assert.equal(state.commit_link.links.length,0);assert.equal(control('commit-link-open').enabled,false);await reveal('commit-link-name');await capture('empty');await click('commit-link-close');await until(s=>!s.commit_link);
 fs.writeFileSync(path.join(repo,'.git/config'),config);
 for(let i=0;i<9;i++)git('remote','add','extra-'+i,'https://github.com/example/extra-'+i+'.git');
 git('remote','add','上流-'+('very-long-remote-'.repeat(6)),'https://github.com/example/'+('long-project-'.repeat(9))+'.git');
 await open();assert.equal(state.commit_link.links.length,12);await reveal('commit-link-remote-11');await click('commit-link-remote-11');assert(state.commit_link.links[11].remote.startsWith('上流-'));await scroll('down',2000);bounds();await capture('compact-many-remotes');await click('commit-link-copy');assert.match(state.commit_link.notice,/copied/);await click('commit-link-close');await until(s=>!s.commit_link);
 fs.writeFileSync(path.join(repo,'.git/config'),config);pass('Empty state, long Unicode remote names and many destinations stay usable in a compact window');
 assert.deepEqual(fingerprint(),before);pass('HEAD, index, staged/unstaged/untracked files and repository configuration preserved');
}
async function logs(){const dirs=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-'));const records=dirs.flatMap(dir=>fs.readdirSync(path.join(root,dir)).filter(n=>n.endsWith('.jsonl')).flatMap(n=>fs.readFileSync(path.join(root,dir,n),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));assert(records.some(r=>r.operation==='commit-link'));assert(records.some(r=>r.operation==='commit-link-open'));assert(records.every(r=>!r.dropped_before));assert(!JSON.stringify(records).includes('PRIVATE_LINK_FIXTURE'));report.log_records=records.length;pass('Commit-link steps recorded with no dropped diagnostics or file payloads')}
(async()=>{try{await start();await run();await logs();report.passed=true}catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client)try{await capture('failure')}catch{}}
finally{
 if(fs.existsSync(relay.gate))fs.unlinkSync(relay.gate);
 if(client)try{await client.tool('rocket_escape');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window));await client.raw('POST','/shutdown');await client.finish()}catch(e){report.cleanup_error=String(e)}
 if(service){for(let n=0;n<100&&service.exitCode===null;n++)await delay(100);if(service.exitCode===null){report.forced_cleanup=true;cp.execFileSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true,stdio:'ignore'})}}
 const token=path.join(profile,'access-token');if(fs.existsSync(token))fs.unlinkSync(token);report.cleanup={service_exit:service?.exitCode,token_removed:!fs.existsSync(token)};
 if(report.cleanup_error||report.forced_cleanup||service?.exitCode!==0){report.passed=false;process.exitCode=1}
 fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.mkdirSync('work/results/commit-links',{recursive:true});fs.writeFileSync('work/results/commit-links/'+(baseline?'baseline':'latest')+'.json',JSON.stringify(report,null,2));console.log('Evidence: '+root);
}})();
