// Native Map acceptance via authenticated localhost MCP, synthetic local Git only.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/repository-map-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/repository-map';
const scriptsOnly=process.argv.includes('--scripts-only');
const nativeOnly=process.argv.includes('--native-only');
const focused=scriptsOnly||nativeOnly;
const evidence=path.resolve('tools/maintenance',nativeOnly?'native-code-map':scriptsOnly?'script-code-map':'repository-map-redesign-20261105');
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const edgesOnly=process.argv.includes('--edges-only'),privateIds=[];
const report={passed:false,version,root,scope:nativeOnly?'native-only':scriptsOnly?'scripts-only':edgesOnly?'edges-only':'full-map',cases:[]};
fs.mkdirSync(root,{recursive:true});fs.mkdirSync(evidence,{recursive:true});
fs.writeFileSync(path.join(root,'rocket-private-state'),'synthetic-content-only-v1\n');
fs.mkdirSync(path.join(root,'empty-template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'empty-template')});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function fixture(name,empty=false){
 const repo=path.join(root,name),hooks=path.join(root,name+'-hooks');fs.mkdirSync(repo);fs.mkdirSync(hooks);
 const git=(args,input)=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',input,maxBuffer:32*1024*1024}).trimEnd();
 const write=(name,body)=>{const p=path.join(repo,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,body);};
 const commit=message=>{git(['add','--all']);git(['commit','-m',message]);return git(['rev-parse','HEAD']);};
 git(['init','--initial-branch='+branch]);
 for(const [key,value]of [['user.name','PRIVATE_Alias'],['user.email','alias@example.invalid'],['commit.gpgsign','false'],['core.autocrlf','false'],['core.hooksPath',hooks]])git(['config',key,value]);
 fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
 if(!empty){write('base.txt','base\n');commit('PRIVATE_BASE 中文');}
 return {repo,git,write,commit};
}
let client,service,state;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const passed=text=>{report.cases.push(text);console.error('PASS '+text);};
function active(){return state.tabs.find(t=>t.id===state.active_tab);}
function map(){return active().repository_map;}
function control(id){return state.controls.find(c=>c.id===id);}
async function until(predicate,label='UI state'){
 const start=performance.now();for(;;){state=await client.tool('rocket_state');if(predicate(state))return state;assert(performance.now()-start<60000,label+' timed out: '+JSON.stringify({dialog:state.dialog,notice:state.notice,map:active()?.repository_map?.notice}));await delay(60);}
}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!(s.tabs||[]).some(t=>t.loading||t.search.running));}
async function stable(){let last='',count=0;await until(s=>{const next=JSON.stringify(s.controls);count=next===last?count+1:0;last=next;return count>=2;},'Stable controls');}
async function click(target){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_click',{target,expected_tab:state.active_tab});await settle();await stable();}
async function type(target,text){await stable();assert(control(target)?.enabled,'Missing/disabled '+target);await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});await settle();await stable();}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function capture(name){await stable();const result=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!result.isError);const img=result.content.find(c=>c.type==='image');assert(img);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(img.data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function open(f){if(state?.dialog)await click('dialog-cancel');await client.tool('rocket_open',{path:f.repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');await click('view-map');assert.equal(active().view,'Map');}
// Refresh can leave the revision popup open; only open it when its field is absent.
async function openRevision(){await stable();if(!control('map-revision'))await click('map-revision-pick');assert(control('map-revision')?.enabled,'Revision picker did not open');}
async function load(revision){await openRevision();await type('map-revision',revision);await click('map-load');}
async function closeWindow(){await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit'&&!s.input_pending);if(state.dialog?.kind==='quit'){await client.tool('rocket_click',{target:'dialog-submit',expected_tab:state.active_tab});await until(s=>['closed','not_started'].includes(s.window));}}
function snapshot(f){const files={};function walk(dir){for(const e of fs.readdirSync(dir,{withFileTypes:true})){if(e.name==='.git')continue;const p=path.join(dir,e.name);if(e.isDirectory())walk(p);else files[path.relative(f.repo,p)]=hash(p);}}walk(f.repo);return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.repo,'.git/index')),config:hash(path.join(f.repo,'.git/config')),files};}
function checkBounds(){for(const c of state.controls.filter(c=>c.id.startsWith('map-')))assert(c.rect[0]>=0&&c.rect[1]>=0&&c.rect[2]<=state.viewport[0]+1&&c.rect[3]<=state.viewport[1]+1,'Offscreen '+JSON.stringify(c));}
const FILE='应用/src/結果 🚀 é.rs';
async function scriptLoads(){
 const f=fixture('script helpers 資料');
 f.write('tools/build.ps1',[
  ". (Join-Path $PSScriptRoot 'lib/補 助.ps1')",
  'Import-Module -Name "$PSScriptRoot/../shared/flags.psm1"',
  '& "$PSScriptRoot/run.ps1"',
  "$example = @'",". './decoy.ps1'","'@",
 ].join('\n'));
 f.write('tools/lib/補 助.ps1','Set-Content "$PSScriptRoot/map-executed" unexpected\n');
 f.write('tools/run.ps1','# helper\n'); f.write('tools/decoy.ps1','# quoted example\n');
 f.write('shared/flags.psm1','# static module\n');
 f.write('tools/build.sh',[
  'root=$(cd "$(dirname "$0")/.." && pwd -P)','source "$root/shared/flags.bash"',
  ". './lib/本 地.sh'","cat <<'EXAMPLE'",'source ./decoy.sh','EXAMPLE',
  'source "$runtime/decoy.sh"',
 ].join('\n'));
 f.write('tools/lib/本 地.sh','echo unexpected > map-executed\n');
 f.write('tools/decoy.sh','# quoted example\n'); f.write('shared/flags.bash','# helper\n');
 const tip=f.commit('helper map fixture');
 f.write('tools/build.ps1',". './untracked.ps1'\n"); f.write('tools/untracked.ps1','# keep me\n');
 const before=snapshot(f);
 await open(f);
 await until(()=>map().data?.tip===tip && map().code_map?.links && !map().code_map.running,'Folder links');
 assert.equal(map().code_map.links.links,5);
 const shared=map().data.nodes.find(n=>n.path==='shared');
 const tools=map().data.nodes.find(n=>n.path==='tools');
 assert(map().code_map.picture.arrows.some(a=>a.from===tools.index && a.to===shared.index && a.files===2),'both callers load helpers from shared');
 passed('Folders draws five committed loads, including the two callers that use shared');
 await click('map-mode-modules');
 await until(()=>map().data?.kind==='Modules' && !map().running,'Module picker');
 for(const [language,caller,helpers,outside] of [
  ['PowerShell','tools/build.ps1',['tools/lib/補 助.ps1','tools/run.ps1'],'shared/flags.psm1'],
  ['Bash','tools/build.sh',['tools/lib/本 地.sh'],'shared/flags.bash'],
 ]){
  const index=map().data.module_packages.findIndex(p=>p.language===language && p.folder==='tools');
  assert(index>=0,language+' group missing');
  if(map().package!=='tools'||map().package_language!==language){
   await click('map-module-package'); await click('map-module-package-'+index);
  }
  await until(()=>!map().running && map().data?.modules?.language===language && map().package==='tools',language+' scripts');
  const data=map().data, from=data.nodes.find(n=>n.path===caller);
  assert(from,caller); assert(!data.nodes.some(n=>n.path==='tools/untracked.ps1'));
  assert.deepEqual(data.links.filter(l=>l.kind==='DependsOn').map(l=>{
   assert.equal(l.from,from.index); return data.nodes[l.to].path;
  }).sort(),helpers);
  assert.deepEqual(data.modules.outside.find(o=>o.node===from.index)?.names,[outside]);
  await click('map-module-'+from.index); assert.equal(map().selected,from.index);
  for(const helper of helpers) assert(map().visible.includes(data.nodes.find(n=>n.path===helper).index));
  const helper=data.nodes.find(n=>n.path===helpers[0]);
  await click('map-node-'+helper.index); assert.equal(map().selected,helper.index);
  assert(map().visible.includes(from.index) && map().captions.some(c=>c.what==='UsedBy' && c.count===1));
  passed(language+' picker, directed file links, cross-folder helper and navigation from caller to helper and back');
 }
 assert.deepEqual(snapshot(f),before);
 const g=fixture('ambiguous shell');
 g.write('start/main.sh','source ./lib.sh\nsource global.sh\nsource "$(missing)/choice.sh"\n. ./link.sh\n. "${BASH_SOURCE[0]%/*}/../shared/environment"\n');
 g.write('lib.sh','# root candidate\n'); g.write('start/lib.sh','# sibling candidate\n');
 g.write('global.sh','# PATH candidate\n');
 g.write('shared/environment','# explicitly sourced extensionless helper\n');
 g.commit('ambiguous paths');
 const linkBlob=g.git(['hash-object','-w','--stdin'],'../global.sh');
 g.git(['update-index','--add','--cacheinfo','120000',linkBlob,'start/link.sh']);
 g.git(['commit','-m','symlink in snapshot']);
 const second=g.git(['rev-parse','HEAD']);
 await open(g);
 await until(()=>map().data?.tip===second && map().code_map?.links && !map().code_map.running,'Second repository');
 assert.equal(map().code_map.links.links,1);
 const folderId=name=>map().data.nodes.find(n=>n.path===name).index;
 assert(map().code_map.picture.arrows.some(a=>a.from===folderId('start') && a.to===folderId('shared')));
 passed('A second checkout links an explicit extensionless helper and skips ambiguous, PATH, dynamic and symlink loads. Local edits and helper contents were preserved');
}
async function nativeBridges(){
 async function folders(f,tip){
  await open(f);
  if(control('map-mode-folders')) await click('map-mode-folders');
  await until(()=>map().data?.tip===tip && !map().alternate && map().code_map.links && !map().code_map.running,'committed native file links');
 }
 async function expand(paths){
  for(const p of paths){
   const folder=map().data.nodes.find(n=>n.path===p);
   assert(folder,p);
   if(map().collapsed.includes(p)) await click('map-toggle-'+folder.index);
  }
 }
 const node=p=>{const n=map().data.nodes.find(n=>n.path===p); assert(n,p);return n;};
 const arrow=(a,b)=>map().code_map.picture.arrows.find(e=>e.from===node(a).index && e.to===node(b).index);
 const f=fixture('native imports 資料');
 f.write('Cargo.toml',"[package]\nname='preview'\n");
 f.write('build.rs','fn main() { cc::Build::new().file("native/介面.mm").compile("native"); }\n');
 f.write('src/lib.rs','mod ffi; pub fn run() { unsafe { ffi::read() } }\n');
 f.write('src/ffi.rs','unsafe extern "C" { #[link_name="native_read"] pub fn read(); fn hidden(); fn cpp_only(); fn external(); }\n');
 f.write('native/CMakeLists.txt','project(Preview)\n');
 f.write('native/介面.mm','#import "標頭.h"\n#import <AppKit/AppKit.h>\n#include <Foundation/Foundation.h>\nextern "C" int native_read() { return PREVIEW_VALUE; }\nstatic void hidden() {}\nvoid cpp_only() {}\n');
 f.write('native/標頭.h','#define PREVIEW_VALUE 7\n');
 f.write('unrelated/decoy.m','void native_read() {} void external() {}\n');
 const tip=f.commit('Native imports and foreign calls');
 f.write('native/介面.mm','local edit to keep\n'); f.write('src/unstaged.m','void never_indexed() {}\n');
 const before=snapshot(f);
 await folders(f,tip); await expand(['src','native']);
 for(const p of ['src/lib.rs','src/ffi.rs']){
  assert.deepEqual(arrow(p,'native/介面.mm')?.names,['native_read'],p+' resolves only the reviewed native symbol');
 }
 assert(arrow('native/介面.mm','native/標頭.h')?.names.includes('PREVIEW_VALUE'));
 assert(!map().data.nodes.some(n=>n.path==='src/unstaged.m'));
 await click('map-node-'+node('src/ffi.rs').index);
 assert(control('map-link-0'),'binding details show a native dependency');
 passed('Folders connects the Rust caller and its renamed binding to the C-linkage Objective-C++ definition and shows the imported Unicode header');
 await click('map-mode-modules');await until(()=>!map().running && map().data?.kind==='Modules','native package picker');
 const i=map().data.module_packages.findIndex(p=>p.dialect==='ObjectiveCpp' && p.folder==='native');
 assert(i>=0,'Objective-C++ package');assert(map().data.module_packages.some(p=>p.dialect==='ObjectiveC'));
 if(map().package!=='native'||map().package_language!=='C'){
  await click('map-module-package');await click('map-module-package-'+i);
 }
 await until(()=>!map().running && map().package==='native' && map().data?.modules?.language==='C','Objective-C++ modules');
 const source=node('native/介面.mm'), header=node('native/標頭.h');
 assert.equal(source.detail,'Objective-C++ source');
 assert(map().data.links.some(e=>e.kind==='DependsOn' && e.from===source.index && e.to===header.index));
 assert.deepEqual(map().data.modules.outside.find(e=>e.node===source.index)?.names,['AppKit','Foundation']);
 await click('map-module-'+source.index);assert.equal(map().selected,source.index);
 assert(map().visible.includes(header.index));checkBounds();
 await click('map-node-'+header.index);assert.equal(map().selected,header.index);
 assert(map().visible.includes(source.index),'header shows the file that imports it');
 await click('map-node-'+source.index);assert.equal(map().selected,source.index);
 await resize(780,620);assert(control('map-module-switcher'));checkBounds();
 assert.deepEqual(snapshot(f),before);
 passed('Modules distinguishes both Objective-C dialects, navigates from the header back to its importer, and lists AppKit and Foundation under Outside. Compact controls and working edits survived');
 const g=fixture('Rocket spelling snapshot');
 const repoRoot=path.resolve(__dirname,'../..');
 for(const p of ['crates/rocket-spell/Cargo.toml','crates/rocket-spell/build.rs','crates/rocket-spell/src/macos.rs','crates/rocket-spell/src/macos.m','tools/build-objc.rs']){
  g.write(p,cp.execFileSync('git',['-C',repoRoot,'show','HEAD:'+p],{env,windowsHide:true}));
 }
 g.write('crates/rocket-spell/src/lib.rs','mod macos;\n');
 const spellTip=g.commit('Committed Rocket spelling adapter');const spellBefore=snapshot(g);
 await folders(g,spellTip);await expand(['crates','crates/rocket-spell','crates/rocket-spell/src']);
 const bridge=arrow('crates/rocket-spell/src/macos.rs','crates/rocket-spell/src/macos.m');
 assert(bridge?.names.includes('rocket_spell_check'),'the real spelling bridge is visible');
 assert.deepEqual(snapshot(g),spellBefore);
 passed('A separate temporary repository displays rocket_spell_check from Rocket’s committed Rust declaration to its Cocoa implementation');
 const h=fixture('native cancellation');
 h.write('Cargo.toml',"[package]\nname='cancel_native'\n");
 h.write('src/lib.rs','unsafe extern "C" { fn bridge(); }\n');
 h.write('build.rs','fn main() { cc::Build::new().file("native.m").compile("bridge"); }\n');
 h.write('native.m','void bridge(void) {}\n');const light=h.commit('Small native map');
 const bulkSource=Array.from({length:3000},(_,i)=>'int item_'+i+'(void) { return '+i+'; }').join('\n');
 for(let i=0;i<32;i++) h.write('bulk/bridge'+i+'.mm','// source '+i+'\n'+bulkSource);
 const heavy=h.commit('Larger native map');const unchanged=snapshot(h);
 await open(h);await load(light);
 if(control('map-mode-folders')) await click('map-mode-folders');
 await until(()=>map().data?.tip===light && map().code_map.links && !map().code_map.running,'small native map');
 await openRevision();await type('map-revision',heavy);
 await client.tool('rocket_click',{target:'map-load',expected_tab:state.active_tab});
 await until(()=>map().code_map.reading_links && control('map-code-stop')?.enabled,'active native code read');
 await client.tool('rocket_click',{target:'map-code-stop',expected_tab:state.active_tab});
 await until(()=>!state.input_pending && map().code_map.stopped && !map().code_map.running,'stopped native code read');
 await load(light);await until(()=>map().data?.tip===light && map().code_map.links && !map().code_map.running,'reload after Stop');
 await expand(['src']);assert.deepEqual(arrow('src/lib.rs','native.m')?.names,['bridge']);
 assert.deepEqual(snapshot(h),unchanged);
 passed('Stop cancels an in-flight native map read; the earlier revision can be reloaded with its original bridge and unchanged working files');
}
async function run(){try{
 const exe=path.resolve('target/release/rocket-mcp.exe');report.executable_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
 fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
 const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
 for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i===100)throw e;await delay(100);}}
 assert.equal((await client.initialize()).serverInfo.version,version);
 if(scriptsOnly){await scriptLoads();return;}
 if(nativeOnly){await nativeBridges();return;}
 if(!edgesOnly){
 const f=fixture('PRIVATE_MAP_検索 🚀');
 f.write('Cargo.toml',"[workspace]\nmembers=['应用','core']\n");f.write('应用/Cargo.toml',"[package]\nname='界面'\n[dependencies]\ncore={path='../core'}\n");f.write('core/Cargo.toml',"[package]\nname='engine'\n");
 f.write(FILE,'// PRIVATE_INITIAL 界\n');f.write('.mailmap','PRIVATE_林安 <canonical@example.invalid> PRIVATE_Alias <alias@example.invalid>\n');f.commit('PRIVATE_STRUCTURE');
 f.git(['switch','-c','feature-map']);f.write('feature.txt','feature\n');f.side=f.commit('PRIVATE_SIDE');f.git(['switch',branch]);
 f.git(['config','user.name','PRIVATE_Béa']);f.git(['config','user.email','bea@example.invalid']);f.write(FILE,'// PRIVATE_SHARED\n');f.commit('PRIVATE_MAIN');f.git(['merge','--no-ff','feature-map','-m','PRIVATE_MERGE']);
 f.git(['switch','-c','unmerged-map']);f.write('unmerged.txt','unmerged\n');f.unmerged=f.commit('PRIVATE_UNMERGED');f.git(['switch',branch]);f.tip=f.git(['rev-parse','HEAD']);
 privateIds.push(f.tip);
 f.write(FILE,'PRIVATE_STAGED\n');f.git(['add','--',FILE]);f.write(FILE,'PRIVATE_UNSTAGED\n');f.write('PRIVATE_UNTRACKED','unchanged');const before=snapshot(f);
 await open(f);assert.equal(map().data.tip,f.tip);assert(map().alternate);let nodes=map().data.nodes;
 const ui=nodes.find(n=>n.label==='界面'),core=nodes.find(n=>n.label==='engine');assert(ui&&core);assert(map().data.links.some(e=>e.from===ui.index&&e.to===core.index&&e.kind==='DependsOn'));
 assert(nodes.some(n=>n.path===FILE));assert(!nodes.some(n=>n.path==='PRIVATE_UNTRACKED'));await capture('code-packages-wide');
 await click('map-node-'+ui.index);assert.equal(map().selected,ui.index);await capture('code-package-details');
 // A button that runs out of row starts the next one; it once broke its label ("Comm / its / here") into a tall narrow button.
 {const height=c=>c.rect[3]-c.rect[1],line=height(control('map-fit'));for(const id of ['map-show-folders','map-show-modules','map-history'])assert(control(id)&&height(control(id))<=line+1,id+' breaks its label: '+JSON.stringify(control(id)));}
 assert(control('map-details-scroll'),'wide maps keep the details card');assert(control('map-link-0'),'the package card lists what it uses');
 await click('map-show-folders');assert(!map().alternate);assert.equal(map().selected,ui.index);assert(!map().collapsed.includes(map().data.nodes[ui.index].label));await capture('code-folders-from-package');
 // Folders: a file counts on its folder's card; search selects that folder.
 await type('map-search','結果 🚀 é');const src=map().data.nodes.find(n=>n.path==='应用/src');assert(map().visible.includes(src.index));assert.equal(map().selected,src.index);assert(map().code_map.picture.placed.some(p=>p.open),'an open folder is a box');await capture('code-unicode-search');
 // A file row in the folder card shows the commits that changed it.
 await click('map-path-0');assert.equal(map().kind,'History');assert.equal(map().path_scope,FILE);await until(s=>!map().running,'path history');assert(map().data.nodes.every(n=>n.label!=='PRIVATE_MERGE'||n.count===2));await capture('history-file-scope');
 await type('map-path-scope','');await click('map-apply-scope');await until(s=>!map().running,'whole history');assert.equal(map().path_scope,'');await click('map-view-code');
 await click('map-clear');await load('PRIVATE_UNKNOWN');assert(map().error.includes('Check the revision'));assert.equal(map().data.tip,f.tip);await capture('error-retains-code');await load('HEAD');assert(!map().error);
 await click('map-view-history');nodes=map().data.nodes;const merge=nodes.find(n=>n.label==='PRIVATE_MERGE');assert.equal(merge.count,2);assert.equal(map().data.links.filter(e=>e.to===merge.index&&e.kind==='Parent').length,2);assert(!nodes.some(n=>n.commit===f.unmerged));assert.equal(map().condensed,true);assert.equal(map().line,0);assert(control('map-commit-0'),'a merge line lists its merged commits');assert(control('map-inspect-commit'));await capture('history-merge-wide');
 await click('map-all-branches');assert(map().data.nodes.some(n=>n.commit===f.unmerged));assert(control('map-tip-0'),'a branch ahead of the main line shows above it');await click('map-tip-0');assert.equal(map().data.nodes[map().selected].commit,f.unmerged);await capture('history-branch-ahead');
 await click('map-mode-commits');assert(map().visible.includes(map().data.nodes.find(n=>n.label==='PRIVATE_MERGE').index));await capture('history-every-commit');
 await type('map-search','PRIVATE_UNMERGED');await click('map-node-'+map().data.nodes.find(n=>n.commit===f.unmerged).index);await click('map-inspect-commit');
 await until(s=>s.review_windows.length===1&&!s.review_windows[0].loading,'Out-of-list commit review');const review=state.review_windows[0];assert.equal(review.commit,f.unmerged);assert(review.patch_files.includes('unmerged.txt'));assert(!review.error);await client.tool('rocket_review_click',{window:review.id,target:'review-close'});await until(s=>s.review_windows.length===0&&!s.review_input_pending);
 await type('map-search','PRIVATE_STRUCTURE');const commit=map().data.nodes.find(n=>n.label==='PRIVATE_STRUCTURE');await click('map-node-'+commit.index);await click('map-inspect-commit');assert.equal(active().view,'Commits');assert.equal(active().selected_commit,commit.commit);
 await click('view-map');await click('map-view-people');nodes=map().data.nodes;const a=nodes.find(n=>n.label==='PRIVATE_林安'),b=nodes.find(n=>n.label==='PRIVATE_Béa');assert(a&&b);assert.equal(a.count,3);assert.equal(b.count,2);assert(nodes.every(n=>n.label!=='PRIVATE_Alias'));
 assert(map().data.links.some(e=>e.kind==='SharedFiles'&&[e.from,e.to].includes(a.index)&&[e.from,e.to].includes(b.index)));await capture('people-areas-wide');
 const cell=state.controls.find(c=>c.id.startsWith('map-cell-'+a.index+'-'));assert(cell,'a person has cells in the grid');await click(cell.id);assert.deepEqual(map().cell,cell.id.split('-').slice(2).map(Number));assert(control('map-history')&&control('map-path-0'));await capture('people-cell');
 await click('map-mode-shared');await capture('people-shared-files');await click('map-node-'+a.index);await click('map-history');assert.equal(map().kind,'History');assert.equal(map().query,'PRIVATE_林安');assert(map().visible.some(i=>map().data.nodes[i].label==='PRIVATE_STRUCTURE'));
 assert.deepEqual(snapshot(f),before);passed('Real committed Cargo edges, Unicode paths, pinned revision errors, merge parents, all branches, canonical authors, shared-file evidence and cross-view navigation; local changes preserved');


 for(const kind of ['history','people']){
  await click('map-view-'+kind);const tip=map().data.tip;await stable();await client.tool('rocket_click',{target:'map-load',expected_tab:state.active_tab});
  await until(s=>!s.input_pending&&map().running&&control('map-cancel')?.enabled,'In-flight '+kind+' read');
  const start=performance.now();await client.tool('rocket_click',{target:'map-cancel',expected_tab:state.active_tab});await settle();
  report[kind+'_stop_ms']=performance.now()-start;assert(map().notice?.includes('stopped'));assert.equal(map().data.tip,tip);assert(report[kind+'_stop_ms']<10000);
 }
 passed('First-click History and People cancellation retains their completed results');

 await click('map-view-code');await type('map-search','結果 🚀 é');await resize(780,620);await capture('code-compact');checkBounds();
 const folder=map().data.nodes.find(n=>n.path==='应用/src');if(map().show_details)await click('map-close-details');await click('map-focus');await click('map-node-'+folder.index);assert(map().show_details);await capture('code-compact-details');checkBounds();await click('map-close-details');
 const zoom=map().zoom;await click('map-zoom-in');assert(map().zoom>zoom);await click('map-fit');await click('map-focus');
 for(const kind of ['history','people']){await click('map-view-'+kind);if(map().show_details)await click('map-close-details');await capture(kind+'-compact');checkBounds();if(kind==='history'){await click('map-mode-topology');await click('map-line-0');}else{await click('map-mode-grid');await click('map-node-'+map().data.nodes.find(n=>n.kind==='Person').index);}assert(map().show_details);await capture(kind+'-compact-details');checkBounds();await click('map-close-details');}
 await type('map-search','PRIVATE_NO_MATCH');assert.equal(map().visible.length,0);await capture('empty-search');await click('map-clear');
 await resize(1480,920);await click('map-view-code');await click('map-clear');
 await click('map-export');assert.equal(map().export.name,'rocket-folders.mmd');assert(control('map-export-copy')?.enabled&&control('map-export-save')?.enabled);await capture('export-dialog');
 await click('map-export-save');await until(s=>!map().export.running,'export saved');assert.match(map().export.notice||'',/Saved rocket-folders\.mmd/,map().export.error);const mermaid=path.join(map().export.dir,'rocket-folders.mmd');assert(fs.readFileSync(mermaid,'utf8').includes('flowchart'));
 await click('map-export-save');await until(s=>!map().export.running,'second save');assert.match(map().export.error||'',/already exists/);
 await type('map-export-name','map.txt');await click('map-export-save');assert.match(map().export.error||'',/\.mmd/);await capture('export-errors');
 await client.tool('rocket_escape');await settle();assert(!map().export,'Esc closes the export dialog');fs.unlinkSync(mermaid);
 const old=map().data.tip;
 f.write('external 新.rs','// PRIVATE_EXTERNAL\n');f.git(['add','--','external 新.rs']);f.git(['commit','--only','-m','PRIVATE_EXTERNAL','--','external 新.rs']);const afterExternal=snapshot(f);
 await click('refresh');assert.equal(map().data.tip,old);assert(map().stale);await click('map-load');assert.notEqual(map().data.tip,old);assert(map().data.nodes.some(n=>n.path==='external 新.rs'));assert.deepEqual(snapshot(f),afterExternal);
 passed('780×620 layouts, readable Unicode nodes and detail drawers, zoom/focus, empty search, Mermaid export dialog, external-change detection and explicit snapshot refresh');

 const kinds=fixture('PRIVATE_REFERENCES');const link=kinds.git(['hash-object','-w','--stdin'],'PRIVATE_DO_NOT_FOLLOW\n');const ref=kinds.git(['rev-parse','HEAD']);
 kinds.git(['update-index','--add','--cacheinfo','120000',link,'Cargo.toml']);kinds.git(['update-index','--add','--cacheinfo','120000',link,'.mailmap']);kinds.git(['update-index','--add','--cacheinfo','160000',ref,'module']);kinds.git(['commit','-m','PRIVATE_REFERENCES']);
 kinds.git(['config','diff.external','false']);kinds.git(['config','filter.test.smudge','false']);kinds.git(['config','filter.test.required','true']);kinds.write('.gitattributes','* filter=test\n');const beforeKinds=snapshot(kinds);
 await open(kinds);assert(map().data.nodes.some(n=>n.path==='Cargo.toml'&&n.detail.includes('not followed')));assert(!map().data.nodes.some(n=>n.kind==='Crate'));assert(map().data.nodes.some(n=>n.path==='module'&&n.detail.includes('Submodule')));await click('map-view-history');assert(!map().error);await click('map-view-people');assert(!map().error);assert.deepEqual(snapshot(kinds),beforeKinds);
 passed('Committed symlinks and submodules remain references; configured filters/external diff are not executed; missing working files and index remain unchanged');

 // Code → Modules: one Rust package's modules, read from committed files only.
 const mods=fixture('PRIVATE_MODULES 模块');
 mods.write('Cargo.toml',"[workspace]\nmembers=['crates/ui','crates/core']\n");mods.write('crates/ui/Cargo.toml',"[package]\nname='界面'\n");mods.write('crates/core/Cargo.toml',"[package]\nname='engine'\n");mods.write('crates/core/src/lib.rs','pub fn run() {}\n');
 mods.write('crates/ui/src/lib.rs','mod app;\npub mod 検索;\n#[cfg(test)]\nmod tests;\n#[path = "../../shared/common.rs"]\nmod common;\npub use app::Rocket;\n');
 mods.write('crates/ui/src/app.rs','pub struct Rocket;\nimpl Rocket { pub fn new() -> Self { Self } fn draw(&self) {} }\npub fn start() { crate::検索::find(); }\n');
 mods.write('crates/ui/src/検索.rs','use crate::app::*;\npub fn find() { let _ = Rocket::new(); start(); crate::common::helper(); }\n');
 mods.write('crates/ui/src/tests.rs','use super::*;\n');mods.write('crates/shared/common.rs','pub fn helper() {}\n');mods.write('crates/ui/src/main.rs','fn main() {}\n');mods.write('crates/ui/src/stray.rs','// PRIVATE_STRAY\n');
 // Section 46: a TypeScript project and a Go module beside the Rust workspace.
 mods.write('web/package.json','{"name":"web-app"}\n');mods.write('web/src/app.ts',"import React from 'react';\nimport { view } from './view';\nexport function app() { return view(); }\n");mods.write('web/src/view.ts','export function view() { return 1; }\n');
 mods.write('svc/go.mod','module example.com/svc\n\ngo 1.22\n');mods.write('svc/main.go','package main\n\nimport "example.com/svc/store"\n\nfunc main() { store.Open() }\n');mods.write('svc/store/db.go','package store\n\ntype DB struct{}\n\nfunc Open() *DB { return nil }\nfunc (d *DB) Close() {}\n');mods.write('svc/store/gen.go','//go:build ignore\n\npackage main\n');
 // A Kotlin project on Gradle, a C project on CMake and a C# project beside them.
 mods.write('jvm/settings.gradle.kts','rootProject.name = "shop"\n');mods.write('jvm/app/build.gradle.kts','plugins { kotlin("jvm") }\n');mods.write('jvm/app/src/main/kotlin/com/acme/app/Main.kt','package com.acme.app\n\nimport com.acme.app.ui.Screen\n\nfun main() { Screen().show() }\n');mods.write('jvm/app/src/main/kotlin/com/acme/app/ui/Screen.kt','package com.acme.app.ui\n\nclass Screen {\n    fun show() {}\n}\n');
 mods.write('native/CMakeLists.txt','project(acme C)\n');mods.write('native/src/acme.c','#include "acme.h"\n\nint acme_open(void) { return ACME_OK; }\n');mods.write('native/src/acme.h','#define ACME_OK 0\nint acme_open(void);\n');
 mods.write('dotnet/Web/Web.csproj','<Project Sdk="Microsoft.NET.Sdk" />\n');mods.write('dotnet/Web/Controllers/Home.cs','using Acme.Web.Models;\n\nnamespace Acme.Web.Controllers;\n\npublic class Home { User user; }\n');mods.write('dotnet/Web/Models/User.cs','namespace Acme.Web.Models;\n\npublic class User { }\n');
 mods.commit('PRIVATE_MODULES');mods.write('crates/ui/src/app.rs','PRIVATE_UNCOMMITTED\n');mods.write('crates/ui/src/untracked.rs','fn private() {}\n');const beforeModules=snapshot(mods);
 await open(mods);assert(map().alternate,'Code opens on Packages');assert(control('map-mode-modules'));await click('map-mode-modules');await until(s=>!map().running&&map().data?.kind==='Modules','modules read');
 assert.equal(map().kind,'Modules');assert.equal(map().package,'crates/ui');assert.equal(map().code_shows_modules,true);assert(map().whole,'a small package starts on the whole package');
 nodes=map().data.nodes;const mod=k=>nodes.find(n=>n.kind==='Module'&&n.label===k);const app=mod('app'),search=mod('検索'),common=mod('common');assert(app&&search&&common,'modules read');
 assert.equal(common.path,'crates/shared/common.rs','#[path] outside src');assert(!nodes.some(n=>n.path==='crates/ui/src/untracked.rs'));assert(map().data.links.some(e=>e.kind==='DependsOn'&&e.from===search.index&&e.to===app.index));
 assert.deepEqual(map().data.modules.unlinked.map(u=>[u.path,u.reason]),[['crates/ui/src/main.rs','Program'],['crates/ui/src/stray.rs','NotDeclared']]);
 assert(control('map-module-whole')&&control('map-module-package')&&control('map-module-'+app.index));checkBounds();await capture('modules-whole-wide');
 await click('map-module-'+app.index);assert(!map().whole);assert.equal(map().selected,app.index);assert(map().visible.includes(search.index));assert(map().captions.some(c=>c.what==='UsedBy'));
 assert(control('map-module-section-outline'),'Details lists what the file declares');assert(control('map-outline-0'));checkBounds();await capture('modules-around-wide');
 await click('map-node-'+search.index);assert.equal(map().selected,search.index);assert(map().visible.includes(common.index),'the chosen card moves to the middle');
 // Other languages in the same snapshot, picked on the package button: a TypeScript project, then a Go module.
 const pickLanguage=async language=>{const k=map().data.module_packages.findIndex(p=>p.language===language);assert(k>=0,language+' is listed');await click('map-module-package');await click('map-module-package-'+k);await until(s=>!map().running&&map().package_language===language&&map().data?.modules?.language===language,language+' read');};
 await pickLanguage('JavaScript');nodes=map().data.nodes;const tsApp=nodes.find(n=>n.kind==='Module'&&n.label==='app.ts'),tsView=nodes.find(n=>n.kind==='Module'&&n.label==='view.ts');assert(tsApp&&tsView,'TypeScript modules read');assert(map().data.links.some(e=>e.kind==='DependsOn'&&e.from===tsApp.index&&e.to===tsView.index),'an import connects them');
 assert(map().data.modules.outside.some(o=>o.node===tsApp.index&&o.names.includes('react')),'react is listed as outside');assert(map().data.module_packages.find(p=>p.language==='JavaScript').typescript);checkBounds();await capture('modules-typescript-wide');
 await pickLanguage('Go');nodes=map().data.nodes;const goMain=nodes.find(n=>n.kind==='Module'&&n.label==='main'),store=nodes.find(n=>n.kind==='Module'&&n.label==='store');assert(goMain&&store,'Go packages read');assert(map().data.links.some(e=>e.kind==='DependsOn'&&e.from===goMain.index&&e.to===store.index),'an import connects them');
 assert.deepEqual(map().data.modules.unlinked.map(u=>[u.path,u.reason]),[['svc/store/gen.go','BuildIgnore']]);await click('map-module-'+store.index);assert.equal(map().selected,store.index);assert(control('map-module-section-outline'),"Details lists the package's types and functions");checkBounds();await capture('modules-go-wide');
 // Kotlin packages stand alone like Go's; C cards are files; C# cards are namespaces.
 await pickLanguage('Java');nodes=map().data.nodes;const ktApp=nodes.find(n=>n.kind==='Module'&&n.label==='app'&&n.detail==='Java package'),ktUi=nodes.find(n=>n.kind==='Module'&&n.label==='ui'&&n.detail==='Java package');assert(ktApp&&ktUi,'Kotlin packages read, labelled below com.acme.app');assert(map().data.links.some(e=>e.kind==='DependsOn'&&e.from===ktApp.index&&e.to===ktUi.index),'an import connects them');
 assert.equal(map().data.module_packages.find(p=>p.language==='Java').dialect,'Kotlin');await click('map-module-'+ktUi.index);assert(control('map-module-section-outline'),"Details lists the package's class");checkBounds();await capture('modules-kotlin-wide');
 await pickLanguage('C');nodes=map().data.nodes;const cFile=nodes.find(n=>n.kind==='Module'&&n.label==='acme.c'),hFile=nodes.find(n=>n.kind==='Module'&&n.label==='acme.h');assert(cFile&&hFile,'C files read');assert(map().data.links.some(e=>e.kind==='DependsOn'&&e.from===cFile.index&&e.to===hFile.index&&e.weight===2),'an include connects them through acme_open and ACME_OK');checkBounds();await capture('modules-c-wide');
 await pickLanguage('CSharp');nodes=map().data.nodes;const home=nodes.find(n=>n.kind==='Module'&&n.label==='Controllers'&&n.detail==='C# namespace'),models=nodes.find(n=>n.kind==='Module'&&n.label==='Models'&&n.detail==='C# namespace');assert(home&&models,'C# namespaces read');assert(map().data.links.some(e=>e.kind==='DependsOn'&&e.from===home.index&&e.to===models.index&&e.weight===1),'a using line connects them through User');checkBounds();await capture('modules-csharp-wide');
 await click('map-module-package');await click('map-module-package-0');await until(s=>!map().running&&map().package==='crates/core','other package');assert(map().data.nodes.some(n=>n.kind==='Module'&&n.label==='lib.rs'));
 await click('map-mode-packages');assert.equal(map().kind,'Code');assert(map().alternate);const uiPackage=map().data.nodes.find(n=>n.label==='界面');await click('map-node-'+uiPackage.index);await click('map-show-modules');
 await until(s=>!map().running&&map().package==='crates/ui','Show modules');assert.equal(map().kind,'Modules');await click('map-view-history');await click('map-view-code');assert.equal(map().kind,'Modules','Code returns to Modules');
 await resize(780,620);assert(control('map-module-switcher'),'a narrow window folds the list into a button');checkBounds();await capture('modules-compact');await resize(1480,920);
 await click('map-export');assert.equal(map().export.name,'rocket-modules.mmd');await client.tool('rocket_escape');await settle();assert(!map().export);
 assert.deepEqual(snapshot(mods),beforeModules);passed('Code → Modules: committed module tree with #[path], connections, unlinked files, list, picture, package switch, a TypeScript project, a Go module, a Kotlin project, a C project and a C# project, Show modules, narrow switcher and export; local changes preserved');
 // Section 47: Folders is the code map.
 await click('map-mode-folders');await until(s=>map().kind==='Code'&&!map().alternate&&map().code_map.links&&!map().code_map.running,'code map links');
 assert.equal(map().code_map.colour,'Kind');assert(map().code_map.links.packages>=4,'Rust, TypeScript and Go packages read: '+JSON.stringify(map().code_map.links));
 nodes=map().data.nodes;const cratesFolder=nodes.find(n=>n.path==='crates'),uiFolder=nodes.find(n=>n.path==='crates/ui'),sharedFolder=nodes.find(n=>n.path==='crates/shared');
 if(map().collapsed.includes('crates'))await click('map-toggle-'+cratesFolder.index);
 const arrows=()=>map().code_map.picture.arrows;
 assert(arrows().some(a=>a.from===uiFolder.index&&a.to===sharedFolder.index&&a.files>=1),'ui uses shared through #[path]: '+JSON.stringify(arrows()));
 assert(map().code_map.picture.placed.some(p=>p.item===cratesFolder.index&&p.open),'crates opens as a box');checkBounds();await capture('code-map-kind');
 await click('map-node-'+uiFolder.index);assert(control('map-link-0'),'Details lists what ui uses');
 await click('map-colour');await click('map-colour-activity');await until(s=>map().code_map.history&&!map().code_map.running,'code map history');
 assert.equal(map().code_map.colour,'Activity');assert(map().code_map.picture.placed.some(p=>/commit/.test(p.sub)),'cards count commits');assert(map().code_map.picture.legend.most>=1);await capture('code-map-activity');
 await click('map-colour');await click('map-colour-owner');assert.equal(map().code_map.picture.legend.owners_from,null,'no CODEOWNERS here');assert(map().code_map.picture.legend.owners.length>=1);
 await click('map-colour');await click('map-colour-changes');await type('map-code-base','HEAD~1');await click('map-code-compare');
 await until(s=>map().code_map.changes&&!map().code_map.running,'code map changes');assert.equal(map().code_map.changes.outcome,'Ready');assert(map().code_map.picture.legend.added>=1,'the modules commit adds files');await capture('code-map-changes');
 await click('map-changed-only');assert(map().code_map.changed_only);
 assert(map().code_map.picture.placed.filter(p=>typeof p.item==='number'&&nodes[p.item].kind==='File').every(p=>p.badge||p.hatched),'Changed only keeps changed files and those that use them');checkBounds();await capture('code-map-changed-only');
 await type('map-code-base','PRIVATE_NO_BASE');await click('map-code-compare');await until(s=>map().code_map.changes?.outcome==='BaseNotFound','unknown base');
 await click('map-changed-only');await click('map-colour');await click('map-colour-kind');
 assert.deepEqual(snapshot(mods),beforeModules);passed('Code → Folders as a code map: links across Rust, TypeScript and Go packages, #[path] between folders, boxes, Kind, Commit activity, Owner by most commits, Changes since a commit with Changed only, an unknown base; local changes preserved');
 }

 const ownerLimit=fixture('PRIVATE_OWNER_LIMIT');
 ownerLimit.write('CODEOWNERS',Array.from({length:10000},(_,i)=>`/missing-${i}/** @owner\n`).join(''));
 for(let i=0;i<200;i++)ownerLimit.write(`src/file-${i}.md`,'data\n');
 ownerLimit.commit('Bound owner matching');const ownerBefore=snapshot(ownerLimit);
 await open(ownerLimit);if(control('map-mode-folders'))await click('map-mode-folders');await until(s=>map().code_map.links&&!map().code_map.running,'bounded owners');
 assert(map().code_map.links.owners_limited&&map().code_map.links.limited);
 assert.equal(map().code_map.links.owners,null,'partial ownership is discarded');
 await click('map-colour');await click('map-colour-owner');await until(s=>!map().code_map.running,'owner fallback');
 await capture('code-map-owner-limit');assert.deepEqual(snapshot(ownerLimit),ownerBefore);
 passed('CODEOWNERS matching stops at its budget, reports the limit and preserves the repository');
 const empty=fixture('PRIVATE_EMPTY',true);await open(empty);for(const kind of ['code','history','people']){await click('map-view-'+kind);assert.equal(map().data.nodes.length,0);assert(!map().running);await capture('empty-'+kind);}
 const oldRepo=fixture('PRIVATE_OLD');const oldTree=oldRepo.git(['rev-parse','HEAD^{tree}']);
 const oldCommit=cp.execFileSync('git',['-C',oldRepo.repo,'commit-tree',oldTree],{env:{...env,GIT_AUTHOR_DATE:'2000-01-01T00:00:00Z',GIT_COMMITTER_DATE:'2000-01-01T00:00:00Z'},input:'PRIVATE_OLD_COMMIT\n',windowsHide:true,encoding:'utf8'}).trim();oldRepo.git(['update-ref','refs/heads/'+branch,oldCommit]);
 await open(oldRepo);await click('map-view-people');assert(map().data.nodes.length>0);await click('map-period-90');assert.equal(map().data.nodes.length,0);assert.equal(map().data.scanned,1);await capture('people-empty-period');
 const longRepo=fixture('PRIVATE_LONG');const longTree=longRepo.git(['rev-parse','HEAD^{tree}']);const longCommit=longRepo.git(['commit-tree',longTree,'-p',longRepo.git(['rev-parse','HEAD'])],'PRIVATE_LONG_'+ '界🚀'.repeat(4000)+'\n');longRepo.git(['update-ref','refs/heads/'+branch,longCommit]);
 await open(longRepo);await click('map-view-history');const longNode=map().data.nodes.find(n=>n.commit===longCommit);assert(Buffer.byteLength(longNode.label)<=4096);assert(!longNode.label.includes('\ufffd'));assert(map().data.notes.some(n=>n.includes('shortened')));await capture('bounded-long-label-wide');await resize(780,620);await capture('bounded-long-label-compact');checkBounds();
 const big=fixture('PRIVATE_LARGE');const blob=big.git(['hash-object','-w','--stdin'],'PRIVATE_SMALL\n');let records='';for(let i=0;i<50000;i++)records+=`100644 blob ${blob}\tfile-${String(i).padStart(6,'0')}.rs\0`;
 const tree=big.git(['mktree','-z'],records),large=big.git(['commit-tree',tree],'PRIVATE_BIG_TREE');const beforeBig=snapshot(big);
 await open(big);await load(large);assert.equal(map().data.scanned,50000);assert.equal(map().data.node_count,4096);assert(map().data.limited);assert.equal(map().visible.length,0,'files share one card');assert.deepEqual(map().code_map.picture.placed.map(p=>p.item),['files:top']);assert(map().zoom>=0.75);await capture('bounded-large-code');await click('map-fit');assert(map().zoom>=0.75&&map().zoom<=1);await capture('bounded-large-code-fit');
 if(edgesOnly){await load('PRIVATE_BAD_REVISION');assert(map().error?.includes('Check the revision'));assert.equal(map().data.tip,large);await load(large);}
 await stable();await client.tool('rocket_click',{target:'map-load',expected_tab:state.active_tab});await until(s=>!s.input_pending&&map().running&&control('map-cancel')?.enabled,'In-flight map read');
 // Exactly one Stop click; no retry conceals a missed cancellation.
 const started=performance.now();await client.tool('rocket_click',{target:'map-cancel',expected_tab:state.active_tab});await until(s=>!s.input_pending&&(map().cancelling||!map().running),'Immediate Stop acknowledgement');report.stop_feedback_ms=performance.now()-started;
 await settle();report.stop_settled_ms=performance.now()-started;assert(map().notice?.includes('stopped'));assert.equal(map().data.tip,large);assert(report.stop_settled_ms<10000);await capture('stopped-retains-code');
 await stable();await client.tool('rocket_click',{target:'map-load',expected_tab:state.active_tab});await until(s=>!s.input_pending&&map().running,'Map reload running');
 // Stop before opening another popup: the bounded read can finish during menu navigation.
 // The stopped read must never replace the subsequent HEAD result.
 await client.tool('rocket_click',{target:'map-cancel',expected_tab:state.active_tab});await until(s=>!s.input_pending&&!map().running,'stopped reload');await load('HEAD');await until(s=>!map().running,'HEAD read');await settle();assert.equal(map().data.tip,big.git(['rev-parse','HEAD']));assert.equal(map().data.node_count,1);await delay(250);await settle();assert.equal(map().data.node_count,1);assert.deepEqual(snapshot(big),beforeBig);
 passed('Empty views and date scope, bounded Unicode labels, readable large-tree focus and complete Fit, one-click Code cancellation, a stopped read that never publishes, and unchanged Git/files');

 await closeWindow();await client.raw('POST','/shutdown');
 const recordsLog=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(recordsLog.length);assert(recordsLog.every(r=>r.dropped_before===0));const payload=JSON.stringify(recordsLog);for(const secret of ['PRIVATE_','検索',FILE,root,blob,...privateIds])assert(!payload.includes(secret),'Diagnostic payload leak');
 for(const op of ['map-code','map-history','map-people',...(edgesOnly?[]:['map-modules','map-code-links','map-code-history','map-code-changes'])]){const starts=recordsLog.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const start of starts)assert(recordsLog.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired '+op);}
 for(const start of recordsLog.filter(r=>r.event==='step_start'&&r.operation?.startsWith('map-')))assert(recordsLog.some(r=>r.event==='step_end'&&r.step===start.step&&r.session===start.session&&r.request===start.request),'Unpaired step');
 for(const op of (edgesOnly?['map-code']:['map-code','map-history','map-people']))assert(recordsLog.some(r=>r.operation===op&&r.outcome==='cancelled'),'Missing cancellation log '+op);assert(recordsLog.some(r=>r.operation==='map-code'&&r.outcome==='git_error'));report.log_records=recordsLog.length;
 passed('Paired request/step logs with named map operations, error/cancellation outcomes, zero dropped events and no private payloads');report.passed=true;
}catch(error){report.failure=error.stack;if(client)try{await capture('failure');}catch{}throw error;}
finally{
 fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(evidence,'mcp-last-result.json'),JSON.stringify(report,null,2));if(!focused)console.log(JSON.stringify(report,null,2));
 if(service&&service.exitCode===null){try{await closeWindow();await client.raw('POST','/shutdown');}catch{}for(let n=0;n<40&&service.exitCode===null;n++)await delay(100);if(service.exitCode===null){report.forced_service_stop=true;service.kill();}}if(client)await client.finish();
 const token=path.join(profile,'access-token');if(fs.existsSync(token))fs.unlinkSync(token);
 report.cleanup={service_exit:service?.exitCode,token_removed:!fs.existsSync(token)};
 fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));fs.writeFileSync(path.join(evidence,'mcp-last-result.json'),JSON.stringify(report,null,2));
 if(focused && !report.failure){
  try{
   assert.equal(report.cleanup.service_exit,0,'isolated MCP service did not exit cleanly');
   assert(report.cleanup.token_removed && !report.forced_service_stop);
   const base=fs.realpathSync(path.resolve('work/mcp-fixtures')), owned=fs.realpathSync(root);
   assert.equal(path.dirname(owned),base);
   assert.match(path.basename(owned),/^repository-map-[0-9a-f]{32}$/);
   assert.equal(fs.readFileSync(path.join(owned,'rocket-private-state'),'utf8'),'synthetic-content-only-v1\n');
   fs.rmSync(owned,{recursive:true,force:false,maxRetries:3});
   report.cleanup.fixtures_removed=!fs.existsSync(owned);
   assert(report.cleanup.fixtures_removed);
   report.passed=true;
  }catch(error){report.failure=error.stack;throw error;}
  finally{
   fs.writeFileSync(path.join(evidence,'mcp-last-result.json'),JSON.stringify(report,null,2));
   console.log(JSON.stringify(report,null,2));
  }
 }
}}
run().catch(e=>{console.error(e);process.exitCode=1});
