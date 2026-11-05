// Native Windows acceptance through authenticated localhost MCP; no hosted Git remotes.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const {gitRelay}=require('./git-relay.cjs');
const root=path.resolve('work/mcp-fixtures/diff-review-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/diff-review';
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const focused=process.argv.includes('--preview-only');
const report={passed:false,version,root,focused,cases:[]},fixtures=[];
fs.mkdirSync(root,{recursive:true});
// Explicit local consent: this runner creates only synthetic shell/editor data.
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_OPTIONAL_LOCKS:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function fixture(name){
  const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
  const git=args=>cp.execFileSync('git',['-c','diff.autoRefreshIndex=false','-C',repo,...args],{env,windowsHide:true,encoding:'utf8'}).trimEnd();
  const write=(name,body)=>{const p=path.join(repo,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,body);};
  const commit=message=>{git(['add','.']);git(['commit','-m',message]);return git(['rev-parse','HEAD']);};
  git(['init','--initial-branch='+branch]);
  for(const [key,value]of [['user.name','Rocket Commit Fixture'],['user.email','rocket@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',key,value]);
  write('base.txt','base\n');commit('PRIVATE_SOURCE baseline 中文');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
  const f={repo,hooks,git,write,commit,config:hash(path.join(repo,'.git/config'))};fixtures.push(f);return f;
}
let client,service,state;
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const passed=text=>{report.cases.push(text);console.error('PASS '+text);};
async function until(predicate,label='UI state'){
  const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<45000,label+' timed out; '+JSON.stringify({dialog:state.dialog,write:state.write,notice:state.notice}));await delay(80);}
}
async function settle(){return until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running));}
function active(){return state.tabs.find(t=>t.id===state.active_tab);}
function control(id){return state.controls.find(c=>c.id===id);}
// Under 440 px the diff header keeps Words in its ··· menu; then the menu button is where it is reached.
function wordsControl(){const c=control('diff-words')??control('diff-tools-menu');assert(c,'Words or the ··· menu that holds it');return c;}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=3;},'Stable widget geometry');}
async function click(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();if(target==='image-reload')await waitForImage(active().patch_files[active().patch_file].path);}
async function type(target,text){await stable();assert(control(target)?.enabled,'Missing/disabled input '+target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const image=result.content.find(c=>c.type==='image');assert(image);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(image.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function scroll(target,direction='down',pixels=600){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await settle();await stable();}
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();relay.check(state);await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-commits');}
async function refresh(){await click('refresh');await settle();}
async function submit(){await click('dialog-submit');await settle();}
function latest(){return state.activities.at(-1);}
async function closeWindow(){if(state?.dialog&&!state.write)await click('dialog-cancel');await client.tool('rocket_close');let confirmed=false;await until(s=>{if(['closed','not_started'].includes(s.window))return true;return s.dialog?.kind==='quit'&&!s.input_pending;},'Close window or exit confirmation');if(state.dialog?.kind==='quit'){assert.equal(state.state_error,null);await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});confirmed=true;await until(s=>['closed','not_started'].includes(s.window),'Closed window');}return confirmed;}

const IMAGE='assets/PRIVATE_ロゴ 🚀 é.png',TEXT='src/検索/表示.rs',RENAMED='assets/名前変更.png';
const relay=gitRelay(root,env,'tools/helpers/file-history-preview/git-relay.rs'),{gate,marker}=relay,assets=path.join(root,'synthetic-images');
const imageState=()=>active().image_diff;
const words=()=>active().word_diff;
const alive=pid=>{try{process.kill(pid,0);return true}catch{return false}};
function snapshot(f){return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),files:Object.fromEntries(f.git(['ls-files','--cached','--others','--exclude-standard','-z']).split('\0').filter(Boolean).map(n=>[n,fs.existsSync(path.join(f.repo,n))?hash(path.join(f.repo,n)):null]))}}
// A diff can be idle for one frame before its selected image decoder starts.
// Wait for the observable result; keep failures available for the assertions.
async function waitForImage(name){await until(()=>{const t=active(),file=t.patch_files?.[t.patch_file];return file?.path===name&&(!file.image||t.image_diff.loaded||t.image_diff.error||t.image_diff.cancelled);},'Selected file/image result');}
async function change(name,staged=false){await click('view-changes');const key='change-'+(staged?'staged':'unstaged')+'-'+Buffer.from(name).toString('hex');for(let n=0;!control(key)&&n<30;n++)await scroll('changes-scroll','down',350);assert(control(key),'Missing change '+name);const start=performance.now();await click(key);await waitForImage(name);return Math.round(performance.now()-start);}
async function commitImage(name){await click('view-commits');await click('detail-files');const i=active().patch_files.findIndex(f=>f.path===name);assert(i>=0,JSON.stringify(active().patch_files));await click('patch-file-'+i);await waitForImage(name);assert(imageState().loaded,JSON.stringify(imageState()));}
async function pauseImage(){fs.writeFileSync(gate,'--batch=%(objectname) %(objecttype) %(objectsize)');if(fs.existsSync(marker))fs.unlinkSync(marker);await stable();await client.tool('rocket_click',{target:'image-reload',expected_tab:state.active_tab});await until(s=>!s.input_pending&&imageState().running&&fs.existsSync(marker),'Paused image Git read');assert.deepEqual(active().warnings,[],'Only the image read should be paused, without history timeouts');return fs.readFileSync(marker,'utf8').trim().split(/\s+/).map(Number);}
async function processesGone(pids){const end=performance.now()+6000;while(pids.some(alive)){assert(performance.now()<end,'Owned cancelled helper still running');await delay(50);}}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');
 report.executable_sha256=hash(exe);cp.execFileSync(path.resolve('target/release/examples/image_fixtures.exe'),[assets],{env,windowsHide:true});
 cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');report.endpoint='http://127.0.0.1:'+port+'/mcp';
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env:relay.env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break}catch(e){if(i===100)throw e;await delay(100)}}assert.equal((await client.initialize()).serverInfo.version,version);
 const f=fixture('PRIVATE_REPO_画像 🚀');const asset=n=>fs.readFileSync(path.join(assets,n));
 f.write(IMAGE,asset('before.png'));f.write('assets/old.png',asset('before.png'));f.write('assets/deleted.png',asset('before.png'));
 f.write(TEXT,'fn main() {\n    let greeting = "Hello 👩‍💻 é";\n    render(greeting, 12);\n}\n');
 f.write('dense.rs',('let value = '+ 'a '.repeat(140)+';\n').repeat(100));f.commit('PRIVATE_IMAGE initial');
 f.write(IMAGE,asset('after.png'));f.git(['mv','assets/old.png',RENAMED]);f.commit('PRIVATE_IMAGE reviewed change');
 f.write(IMAGE,asset('before.png'));f.git(['add','--',IMAGE]);f.write(IMAGE,asset('external.png'));
 f.write(TEXT,'fn main() {\n    let greeting = "你好 👨‍💻 é";\n    render(greeting, 16);\n}\n');
 f.write('dense.rs',('let value = '+ 'b '.repeat(140)+';\n').repeat(100));
 f.write('assets/added.png',asset('after.png'));fs.unlinkSync(path.join(f.repo,'assets/deleted.png'));
 for(const n of ['sample.bmp','sample.jpg','rotated.jpg','wide.png','pixels.png'])f.write('assets/'+n,asset(n));
 f.write('assets/damaged.png',asset('before.png').subarray(0,29));f.write('assets/oversized.png',Buffer.alloc(16*1024*1024+1));
 const original=snapshot(f);await open(f);
 if(focused){
   await change(TEXT);assert(words().enabled&&words().lines===4);await click('diff-split');await capture('final-words');
   await commitImage(RENAMED);assert(active().patch_file>0,'Expected a nonfirst image');await click('image-reload');
   assert.equal(active().patch_files[active().patch_file].path,RENAMED,'Reload must retain the selected committed image');
   assert.equal(imageState().after.width,320);await capture('final-commit-reload');passed('Commit image Reload retains the exact selected Unicode path rather than returning to the first file');
   await change(IMAGE);await click('menu-terminal');await click('terminal-new');await until(s=>s.terminal?.running&&s.terminal.tail.includes('PS '),'Ready PowerShell');await resize(780,620);await capture('final-image-terminal');
   const top=control('terminal-hide').rect[1]-5;for(const id of ['image-fit','image-checker','image-reload','image-side-0','image-side-1']){const c=control(id);assert(c&&c.rect[3]<=top+2,'Clipped image control '+id);}
   await change(TEXT);await capture('final-words-terminal');assert(wordsControl().rect[3]<control('terminal-hide').rect[1]&&control('diff-syntax').rect[3]<control('terminal-hide').rect[1]);assert.deepEqual(snapshot(f),original);
   passed('Final-binary Unicode text/image rendering and 780x620 terminal layout preserve HEAD, index, config and working files');
   await closeWindow();await client.raw('POST','/shutdown');report.passed=true;return;
 }
 report.word_ready_ms=await change(TEXT);assert(words().enabled&&words().lines===4&&words().spans>=4,JSON.stringify(words()));assert(active().partial_selection);await click('diff-split');await capture('word-split');
 await type('diff-search','你好');assert.equal(active().diff_tools.matches,1);await click('diff-words');assert(!words().enabled);assert.equal(active().diff_tools.matches,1);await click('diff-words');assert.equal(words().lines,4);await click('diff-search-clear');await click('diff-unified');await capture('word-unified');
 passed('Unicode word highlights, combining/emoji text, unified/split views, toggles and search coexist with unchanged selectable line identities');
 await change('dense.rs');assert(words().limited,'Expected explicit bounded word-detail fallback');await capture('word-budget');passed('Dense text reaches the word-detail budget and retains whole-line review');
 report.image_ready_ms=await change(IMAGE);assert.equal(imageState().before.width,320);assert.equal(imageState().after.width,280);assert(imageState().before.alpha&&imageState().after.alpha);await capture('image-unstaged');
 await click('image-actual');assert.equal(imageState().zoom,1);assert(!imageState().fit);await click('image-zoom-in');assert(imageState().zoom>1);await click('image-checker');assert(!imageState().checker);await capture('image-zoom-solid');await click('image-checker');await click('image-fit');assert(imageState().fit);
 await change(IMAGE,true);assert.equal(imageState().before.width,400);assert.equal(imageState().after.width,320);await capture('image-staged');
 await commitImage(IMAGE);assert.equal(imageState().before.width,320);assert.equal(imageState().after.width,400);await click('image-reload');await commitImage(IMAGE);assert.equal(imageState().after.width,400);await capture('image-commit');
 await commitImage(RENAMED);assert.equal(imageState().before.width,320);assert.equal(imageState().after.width,320);await click('image-reload');assert.equal(active().patch_files[active().patch_file].path,RENAMED);assert.equal(imageState().after.width,320);
 passed('Index versus working file, HEAD versus index, first-parent commit comparison and renamed Unicode image identities; Fit/100%/zoom/transparency and history Reload');
 await change('assets/added.png');assert.equal(imageState().before,null);assert.equal(imageState().after.width,400);await capture('image-added');await change('assets/deleted.png');assert.equal(imageState().before.width,320);assert.equal(imageState().after,null);await capture('image-deleted');
 for(const [name,format,w,h]of [['sample.bmp','BMP',120,80],['sample.jpg','JPEG',150,100],['rotated.jpg','JPEG',100,150]]){await change('assets/'+name);assert.equal(imageState().after.format,format);assert.equal(imageState().after.width,w);assert.equal(imageState().after.height,h)}
 passed('Added/deleted images, PNG/BMP/JPEG decoding and JPEG EXIF orientation');
 for(const [name,hint]of [['damaged.png','damaged'],['wide.png','limit'],['pixels.png','limit'],['oversized.png','16 MiB']]){await change('assets/'+name);assert(!imageState().loaded);assert(imageState().error?.includes(hint),JSON.stringify(imageState()));assert(!imageState().before&&!imageState().after);await capture('error-'+name)}
 assert.deepEqual(snapshot(f),original);passed('Malformed input, dimensions, pixel-count and encoded-byte limits show errors without stale textures or repository writes');
 await change(IMAGE);let pids=await pauseImage();await capture('image-loading');const start=performance.now();await client.tool('rocket_click',{target:'image-cancel',expected_tab:state.active_tab});await settle();report.cancel_ms=Math.round(performance.now()-start);assert(imageState().cancelled);assert(!imageState().loaded);await processesGone(pids);fs.unlinkSync(gate);await capture('image-cancelled');await click('image-reload');assert(imageState().loaded);
 pids=await pauseImage();const textControl='change-unstaged-'+Buffer.from(TEXT).toString('hex');for(let n=0;!control(textControl)&&n<20;n++)await scroll('changes-scroll','up',350);await client.tool('rocket_click',{target:textControl,expected_tab:state.active_tab});await settle();await processesGone(pids);fs.unlinkSync(gate);assert(!imageState().loaded);assert.equal(words().lines,4);
 passed('Native cancellation stops the owned Git process tree; switching files drops stale image results and preserves typing/navigation responsiveness');
 await change(IMAGE);f.write(IMAGE,asset('after.png'));await until(s=>!s.busy&&imageState().loaded&&imageState().after.width===400,'Automatic refresh after external image edit');assert.equal(imageState().before.width,320);const updated=snapshot(f);assert.equal(updated.head,original.head);assert.equal(updated.index,original.index);assert.equal(updated.config,original.config);original.files[IMAGE]=updated.files[IMAGE];assert.deepEqual(updated,original);passed('External image edits refresh the working version without changing HEAD, index, config or unrelated files');
 await click('menu-terminal');await click('terminal-new');await resize(780,620);await capture('image-small-terminal');const terminalTop=control('terminal-hide').rect[1]-5;for(const c of state.controls.filter(c=>c.id.startsWith('image-')))assert(c.rect[3]<=terminalTop+2,'Image control overlaps terminal '+c.id);await change(TEXT);await capture('words-small-terminal');assert(wordsControl().rect[3]<=control('terminal-hide').rect[1]&&control('diff-syntax').rect[3]<=control('terminal-hide').rect[1]);await click('terminal-hide');await resize(1480,920);
 assert.deepEqual(snapshot(f),original);passed('Compact 780×620 layout with terminal open, text/image switching and preserved original files');
 await closeWindow();await client.raw('POST','/shutdown');
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const payload=JSON.stringify(records);for(const secret of ['PRIVATE_IMAGE','PRIVATE_REPO',IMAGE,TEXT,'Hello','greeting'])assert(!payload.includes(secret),'Diagnostic payload leak');for(const op of ['diff-words','image-preview','image-decode-before','image-decode-after'])assert(records.some(r=>r.operation===op&&r.event==='request_end'),'Missing diagnostic '+op);assert(records.some(r=>r.outcome==='cancelled'));assert(records.some(r=>r.operation==='image-decode-after'&&r.outcome==='invalid_input'));report.log_records=records.length;passed('Named read/decode/highlight request spans, failure and cancellation outcomes, no dropped records or tested private content');
 report.passed=true;
 }catch(error){report.failure=error.stack;if(client)try{await capture('failure')}catch{}throw error;}
 finally{if(fs.existsSync(gate))fs.unlinkSync(gate);fs.writeFileSync('work/diff-review-last-result.txt',path.join(root,'result.json'));fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown')}catch{}await delay(150);if(service.exitCode===null)service.kill()}if(client)await client.finish()}}
run().catch(error=>{console.error(error);process.exitCode=1});
