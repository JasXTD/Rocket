const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {RocketMcpClient}=require('../mcp/mcp-client.cjs');
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const root=path.resolve('work/mcp-fixtures/review-'+crypto.randomUUID().replaceAll('-',''));
const repo=path.join(root,'client'),branch='rocket-ui-test/review';
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'hooks'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'hooks')});
function git(args,ok=true){const r=cp.spawnSync('git',args,{cwd:repo,env,windowsHide:true,encoding:'utf8'});if(ok)assert.equal(r.status,0,r.stderr);return r;}
const write=(p,s)=>fs.writeFileSync(path.join(repo,p),s);
git(['init','--initial-branch='+branch]);git(['config','user.name','Rocket UI Fixture']);git(['config','user.email','rocket@example.invalid']);git(['config','commit.gpgsign','false']);git(['config','core.autocrlf','false']);git(['config','core.hooksPath',path.join(root,'hooks')]);
const base=Array.from({length:20},(_,i)=>'line '+(i+1)).join('\n')+'\n';
write('review.txt',base);git(['add','.']);git(['commit','-m','Synthetic review baseline']);
fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const working=base.replace('line 2\n','line TWO\nextra\n').replace('line 18\n','line EIGHTEEN\n');write('review.txt',working);write('new.txt','Untracked preview\nSecond line\n');
const client=new RocketMcpClient(path.resolve('target/release/rocket-mcp.exe'),path.resolve('work/mcp-fixtures'),env);
const report={root,cases:[]};let state;
const settle=async()=>{for(let n=0;n<8;n++){state=await client.tool('rocket_wait',{timeout_ms:15000});if(!state.busy&&!state.input_pending){assert(!state.tabs[0].error);return state;}}throw Error('UI did not settle');};
const click=async target=>{await client.tool('rocket_click',{target,expected_tab:state.active_tab});return settle();};
const select=async(name,staged=false)=>click('change-'+(staged?'staged-':'unstaged-')+Buffer.from(name).toString('hex'));
async function capture(name){const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);const image=r.content.find(x=>x.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});for(let n=0;;n++){state=await client.tool('rocket_state');if(state.viewport[0]===width&&state.viewport[1]===height)return;assert(n<100);await new Promise(r=>setTimeout(r,50));}}
async function main(){try{
 const init=await client.initialize();assert.equal(init.serverInfo.version,version);await client.tool('rocket_open',{path:repo});await settle();assert.equal(state.app_version,version);
 await click('menu-help');await click('help-about');assert.equal(state.dialog.kind,'about');await capture('about');await click('dialog-submit');
 await click('menu-help');await click('help-changelog');assert.equal(state.dialog.kind,'changelog');await capture('changelog');await click('dialog-submit');report.cases.push('shared version, About, changelog');
 await click('view-changes');await select('new.txt');assert.equal(state.tabs[0].patch_added,2);assert.equal(state.tabs[0].patch_error,null);assert.equal(state.tabs[0].partial_selection,false);await capture('untracked');report.cases.push('untracked preview without staging');
 await select('review.txt');assert.equal(state.tabs[0].partial_selection,true);
 const patch=git(['diff','--','review.txt']).stdout.split('\n');
 for(const text of ['-line 2','+line TWO'])await click('diff-line-'+patch.indexOf(text));
 assert.equal(state.tabs[0].selected_lines,2);await click('diff-split');assert.equal(state.tabs[0].split_diff,true);await capture('split-selection');
 await resize(780,620);await capture('split-narrow');await resize(1480,920);await settle();
 await click('diff-apply-selection');assert(state.activities.some(a=>a.operation==='stage-selection'&&a.command_succeeded));
 assert.equal(git(['show',':review.txt']).stdout,base.replace('line 2\n','line TWO\n'));assert.equal(fs.readFileSync(path.join(repo,'review.txt'),'utf8'),working);
 await select('review.txt',true);const hunk=state.controls.find(c=>c.id.startsWith('diff-hunk-')&&c.enabled);assert(hunk);await click(hunk.id);await click('diff-apply-selection');assert.equal(git(['show',':review.txt']).stdout,base);report.cases.push('line staging, hunk unstaging, split view, worktree preserved');
 // Build a real conflict externally; automatic refresh must notice it.
 git(['restore','--','review.txt']);git(['checkout','-b','other']);write('review.txt','theirs\n');git(['add','review.txt']);git(['commit','-m','Synthetic other']);git(['checkout',branch]);write('review.txt','ours\n');git(['add','review.txt']);git(['commit','-m','Synthetic ours']);assert.notEqual(git(['merge','other'],false).status,0);
 await new Promise(r=>setTimeout(r,600));await settle();await select('review.txt');
 assert(!state.controls.some(c=>c.id==='change-staged-'+Buffer.from('review.txt').toString('hex')),'Conflicted file must not offer a misleading unstage action');
 for(const side of [1,2,3,0]){await click('conflict-side-'+side);assert.equal(state.tabs[0].conflict_side,side);assert.equal(state.tabs[0].patch_error,null);}
 await capture('conflict');await click('conflict-resolved');assert(state.activities.some(a=>a.operation==='mark-resolved'&&!a.command_succeeded&&JSON.stringify(a).includes('markers remain')));assert(git(['ls-files','--unmerged']).stdout);
 write('review.txt','resolved together\n');await new Promise(r=>setTimeout(r,600));await settle();await select('review.txt');await click('conflict-resolved');assert.equal(git(['ls-files','--unmerged']).stdout,'');assert.equal(git(['show',':review.txt']).stdout,'resolved together\n');assert(fs.existsSync(path.join(repo,'.git/MERGE_HEAD')));report.cases.push('base/ours/theirs previews, marker rejection, resolved index without committing');
 const logs=await client.tool('rocket_logs',{limit:1000});assert.equal(state.dropped_events,0);assert(!JSON.stringify(logs).includes('resolved together'));assert(logs.records.every(r=>!r.dropped_before));report.cases.push('structured diagnostics, no file contents, no dropped events');
 await client.tool('rocket_close');state=await client.tool('rocket_state');if(state.dialog?.kind==='quit')await click('dialog-submit');await client.finish();report.passed=true;
}finally{fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(!client.exited){try{await client.finish();}catch{client.terminateOwnedProcess();}}}}
main().catch(e=>{console.error(e);process.exitCode=1;});
