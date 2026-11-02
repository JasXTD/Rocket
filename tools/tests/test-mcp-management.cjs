const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {RocketMcpClient}=require('../mcp/mcp-client.cjs');
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const root=path.resolve('work/mcp-fixtures/management-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'client'),bare1=path.join(root,'PRIVATE-upstream-one.git'),bare2=path.join(root,'PRIVATE-upstream-two.git'),branch='rocket-ui-test/management',name='fixture-control';
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'hooks'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'hooks')});
function git(args,options={}){const r=cp.spawnSync('git',args,{cwd:repo,env,windowsHide:true,encoding:'utf8',...options});assert.equal(r.status,0,r.stderr);return r.stdout.trim();}
git(['init','--initial-branch='+branch]);git(['config','user.name','Rocket Fixture']);git(['config','user.email','fixture@example.invalid']);git(['config','commit.gpgsign','false']);git(['config','core.autocrlf','false']);git(['config','core.hooksPath',path.join(root,'hooks')]);
git(['init','--bare',bare1]);git(['init','--bare',bare2]);
let stream='blob\nmark :20001\ndata 8\noriginal\n';
for(let i=0;i<1210;i++){
 const message=i===2?'PRIVATE_needle [literal].*\n\nUnicode body 文':'Synthetic fixture commit '+i;
 const author=i===3?'Rare Author <rare@example.invalid>':'Fixture <fixture@example.invalid>';
 stream+=`commit refs/heads/${branch}\nmark :${i+1}\nauthor ${author} ${1700000000+i} +0000\ncommitter Fixture <fixture@example.invalid> ${1700000000+i} +0000\ndata ${Buffer.byteLength(message)}\n${message}\n`;
 if(i)stream+=`from :${i}\n`;else stream+='M 100644 :20001 file.txt\n';stream+='\n';
}
git(['fast-import','--quiet'],{input:stream+'done\n'});git(['reset','--hard',branch]);
const mainTip=git(['rev-parse','HEAD']),deepId=git(['rev-parse','HEAD~1207']);
function commit(message,year=2000,parent){return git(['commit-tree','HEAD^{tree}',...(parent?['-p',parent]:[]),'-m',message],{env:{...env,GIT_AUTHOR_DATE:`${year}-01-01T00:00:00Z`,GIT_COMMITTER_DATE:`${year}-01-01T00:00:00Z`}});}
const side=commit('PRIVATE_side old'),remoteOnly=commit('PRIVATE_remote only',2001),feature=commit('Synthetic feature fork',2024,'HEAD~3');
git(['update-ref','refs/heads/old-side',side]);git(['update-ref',`refs/remotes/${name}/isolated`,remoteOnly]);git(['update-ref','refs/heads/feature',feature]);
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
fs.writeFileSync(path.join(repo,'file.txt'),'staged fixture\n');git(['add','file.txt']);fs.writeFileSync(path.join(repo,'file.txt'),'working fixture\n');fs.writeFileSync(path.join(repo,'untracked.txt'),'untracked fixture\n');
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const preserved={head:mainTip,index:hash(path.join(repo,'.git/index')),file:hash(path.join(repo,'file.txt')),untracked:hash(path.join(repo,'untracked.txt'))};
const client=new RocketMcpClient(path.resolve('target',process.env.ROCKET_TEST_PROFILE==='debug'?'debug':'release','rocket-mcp.exe'),path.resolve('work/mcp-fixtures'),env);
const report={root,cases:[]};let state;
const current=()=>state.tabs.find(t=>t.id===state.active_tab);
const waitInput=async()=>{for(let n=0;n<150;n++){state=await client.tool('rocket_state');if(!state.input_pending)return;await new Promise(r=>setTimeout(r,20));}throw Error('Input did not settle');};
const settle=async()=>{for(let n=0;n<8;n++){state=await client.tool('rocket_wait',{timeout_ms:15000});if(!state.busy&&!state.input_pending){assert(!current()?.error);return state;}}throw Error('UI did not settle');};
const click=async(target,wait=true)=>{await client.tool('rocket_click',{target,expected_tab:state.active_tab});return wait?settle():waitInput();};
const type=async(target,text,wait=true)=>{await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});return wait?settle():waitInput();};
const scope=async(all)=>{await click('history-scope');await click(all?'history-all':'history-head');};
const search=async text=>type('commit-filter',text);
async function capture(name){const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(x=>x.type==='image').data,'base64'));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});for(let n=0;n<100;n++){state=await client.tool('rocket_state');if(state.viewport[0]===width&&state.viewport[1]===height)return;await new Promise(r=>setTimeout(r,30));}throw Error('Resize did not settle');}
async function main(){try{
 const init=await client.initialize();assert.equal(init.serverInfo.version,version);await client.tool('rocket_open',{path:repo});await settle();assert.equal(state.app_version,version);assert.equal(current().commit_count,200);
 await click('view-changes');await type('commit-message','PRIVATE draft retained');await click('view-commits');
 await search('PRIVATE_needle [literal].*');assert(current().search.complete);assert.deepEqual(current().history_ids,[deepId]);assert.equal(current().search.scanned,1210);await capture('deep-search');
 await search('rare@EXAMPLE.invalid');assert.equal(current().commit_count,1);await search(deepId.slice(0,10));assert.deepEqual(current().history_ids,[deepId]);report.cases.push('repository-wide literal message/body, author/email and ID search beyond loaded commits');
 await search('Synthetic');assert.equal(current().commit_count,200);assert(!current().search.complete);await click('history-more');assert.equal(current().commit_count,400);assert.equal(new Set(current().history_ids).size,400);report.cases.push('bounded search pages continue without duplicates');
 await search('');await scope(true);assert.equal(current().history_scope,'all');
 while(current().history_more)await click('history-more');
 const expected=git(['rev-list','--branches','--remotes','HEAD']).split('\n');assert.deepEqual([...current().history_ids].sort(),expected.sort());assert.equal(current().commit_count,1213);await capture('all-branches');report.cases.push('combined local and remote branch graph, complete paging and deduplicated ancestry');
 await search('PRIVATE_side');assert.deepEqual(current().history_ids,[side]);
 const oldFirst=feature,sideNew=commit('PRIVATE_side new');git(['update-ref','refs/heads/old-side',sideNew]);
 for(let i=0;i<120;i++){await new Promise(r=>setTimeout(r,100));await settle();if(current().history_ids.includes(sideNew))break;}
 assert.deepEqual(current().history_ids,[sideNew]);assert(current().draft_present);assert.equal(git(['rev-list','--topo-order','--branches','--remotes','HEAD']).split('\n')[0],oldFirst);
 await scope(false);assert.equal(current().commit_count,0);assert(current().search.complete);await scope(true);assert.deepEqual(current().history_ids,[sideNew]);report.cases.push('nonfirst branch update refreshes search, scope switching and draft preservation');
 await type('commit-filter','PRIVATE no match',false);
 for(let i=0;i<100&&!current().search.running;i++){await new Promise(r=>setTimeout(r,10));state=await client.tool('rocket_state');}
 assert(current().search.running);await click('history-stop');assert(current().search.paused);assert(!current().search.complete);await capture('search-stopped');await click('history-more');assert(current().search.complete);assert.equal(current().commit_count,0);
 await type('commit-filter','PRIVATE old query',false);const inputStart=performance.now();await type('commit-filter','PRIVATE_needle [literal].*',false);report.query_input_ms=performance.now()-inputStart;await settle();assert.deepEqual(current().history_ids,[deepId]);report.cases.push('native stop/resume and replacement search rejects stale results');
 await search('x'.repeat(1025));assert(current().search.error?.includes('1024'));assert(!current().search.complete);await capture('search-error');await search('');
 await click('view-remotes');assert.equal(current().remotes.length,0);await click('remote-add');await type('dialog-remote-name',name);
 const outside=await client.request('tools/call',{name:'rocket_type',arguments:{target:'dialog-remote-url',text:'https://example.invalid/not-a-fixture',expected_tab:state.active_tab}});assert(outside.isError);
 await type('dialog-remote-url',bare1);await capture('add-remote');await click('dialog-submit');assert(state.activities.at(-1).command_succeeded);assert.equal(current().remotes[0].name,name);
 await click('remote-add');await type('dialog-remote-name',name);await type('dialog-remote-url',bare1);await click('dialog-submit');assert(!state.activities.at(-1).command_succeeded);assert(JSON.stringify(state.activities.at(-1)).includes('already exists'));
 await click('remote-edit-fetch-0');await type('dialog-remote-url',bare2);await capture('edit-remote');await click('dialog-submit');assert.equal(git(['config',`remote.${name}.url`]),bare2);
 await click('remote-edit-push-0');assert(state.dialog.flag);await click('dialog-remote-fallback');await type('dialog-remote-url',bare1);await click('dialog-submit');assert.equal(current().remotes[0].push_urls,1);
 await click('remote-edit-push-0');await click('dialog-remote-fallback');await click('dialog-submit');assert.equal(current().remotes[0].push_urls,0);report.cases.push('add/edit fetch and push URLs, push fallback, duplicate-name rejection');
 await click('remote-edit-fetch-0');await type('dialog-remote-url',bare1);git(['config',`remote.${name}.url`,bare1]);await click('dialog-submit');assert(!state.activities.at(-1).command_succeeded);assert(JSON.stringify(state.activities.at(-1)).includes('changed'));
 const lock=path.join(repo,'.git/config.lock');fs.writeFileSync(lock,'owned lock');await click('remote-edit-fetch-0');await type('dialog-remote-url',bare2);await click('dialog-submit');assert(!state.activities.at(-1).command_succeeded);assert(JSON.stringify(state.activities.at(-1)).includes('locked or not writable'));assert.equal(fs.readFileSync(lock,'utf8'),'owned lock');fs.unlinkSync(lock);await settle();
 await capture('remotes-errors');await resize(780,620);await capture('remotes-narrow');await resize(1480,920);report.cases.push('stale edit and config lock errors preserve settings and lock ownership');
 await click('remote-remove-0');await capture('remove-confirmation');await click('dialog-cancel');assert.equal(current().remotes.length,1);await click('remote-remove-0');await click('dialog-submit');assert(state.activities.at(-1).command_succeeded);assert.equal(current().remotes.length,0);assert(!git(['for-each-ref','--format=%(refname)',`refs/remotes/${name}`]));
 assert.deepEqual({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),file:hash(path.join(repo,'file.txt')),untracked:hash(path.join(repo,'untracked.txt'))},preserved);report.cases.push('confirmed local remote removal preserves HEAD, index and working files');
 await click('view-commits');await search('PRIVATE_remote');assert.equal(current().commit_count,0);assert(current().search.complete);await search('');await resize(780,620);await capture('history-narrow');await resize(1480,920);
 const records=[];let after_seq=0;for(let page=0;;page++){assert(page<20);const logs=await client.tool('rocket_logs',{after_seq,limit:1000});assert(!logs.retention_gap);records.push(...logs.records);if(!logs.more)break;assert(logs.next_seq>after_seq);after_seq=logs.next_seq;}
 const text=JSON.stringify(records);for(const secret of ['PRIVATE_needle','PRIVATE_side','PRIVATE_remote','PRIVATE-upstream','PRIVATE draft'])assert(!text.includes(secret));assert.equal(state.dropped_events,0);assert(records.every(r=>!r.dropped_before));
 for(const operation of ['remote-add','remote-edit','remote-remove','history-search'])assert(records.some(r=>r.operation===operation));assert(!records.some(r=>['fetch','push','pull'].includes(r.operation)));report.log_records=records.length;report.cases.push('structured local-operation diagnostics exclude URLs, queries and drafts');
 await click('view-changes');await type('commit-message','');await client.tool('rocket_close');await client.finish();report.passed=true;
}finally{report.log_directory=state?.log_directory;fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(!client.exited){try{await client.finish();}catch{client.terminateOwnedProcess();}}}}
main().catch(e=>{console.error(e);process.exitCode=1;});
