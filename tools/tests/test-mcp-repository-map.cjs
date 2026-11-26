// Native Map acceptance via authenticated localhost MCP, synthetic local Git only.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/repository-map-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),branch='rocket-ui-test/repository-map';
const focused=[
 {flag:'--scripts-only',directory:'script-code-map',run:scriptLoads},
 {flag:'--native-only',directory:'native-code-map',run:nativeBridges},
 {flag:'--lua-only',directory:'lua-code-map',run:luaModules},
 {flag:'--workflows-only',directory:'workflow-code-map',run:workflowLinks},
 {flag:'--web-only',directory:'web-code-map',run:webAssets},
 {flag:'--references-only',directory:'code-map-references',run:assetReferences},
 {flag:'--rust-import-references-only',directory:'rust-import-references',run:rustImportReferences},
 {flag:'--rust-assets-only',directory:'rust-embedded-code-map',run:rustAssets},
].find(test=>process.argv.includes(test.flag));
const evidence=path.resolve('tools/maintenance',focused?.directory??'repository-map-redesign-20261105');
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const edgesOnly=process.argv.includes('--edges-only'),privateIds=[];
const report={passed:false,version,root,scope:focused?.flag.slice(2)??(edgesOnly?'edges-only':'full-map'),cases:[]};
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
function diagnosticRecords(){return fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));}
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
async function folders(f,tip){
 await open(f);
 if(control('map-mode-folders')) await click('map-mode-folders');
 await until(()=>map().data?.tip===tip && !map().alternate && map().code_map.links && !map().code_map.running,'committed file links');
}
async function expand(paths){
 for(const p of paths){
  const folder=node(p);
  if(!map().collapsed.includes(p)) continue;
  const toggle='map-toggle-'+folder.index;
  if(!control(toggle)) await click('map-fit');
  await click(toggle);
 }
}
function node(p){const n=map().data.nodes.find(n=>n.path===p);assert(n,p);return n;}
function arrow(a,b){return map().code_map.picture.arrows.find(e=>e.from===node(a).index && e.to===node(b).index);}
async function stopThenReload(light,heavy){
 await settle();await load(light);
 if(control('map-mode-folders')) await click('map-mode-folders');
 await until(()=>map().data?.tip===light && map().code_map.links && !map().code_map.running,'small map');
 await openRevision();await type('map-revision',heavy);
 await client.tool('rocket_click',{target:'map-load',expected_tab:state.active_tab});
 await until(()=>map().code_map.reading_links && control('map-code-stop')?.enabled,'active code read');
 await client.tool('rocket_click',{target:'map-code-stop',expected_tab:state.active_tab});
 await until(()=>!state.input_pending && map().code_map.stopped && !map().code_map.running,'stopped code read');
 await load(light);await until(()=>map().data?.tip===light && map().code_map.links && !map().code_map.running,'reload after Stop');
}
async function luaModules(){
 const f=fixture('Lua modules 資料');
 const repoRoot=path.resolve(__dirname,'../..');
 for(const file of ['integration.json','main.lua']){
  const source='crates/rocket-integrations/examples/jira/'+file;
  f.write('jira/'+file,cp.execFileSync('git',['-C',repoRoot,'show','HEAD:'+source],{env,windowsHide:true}));
 }
 f.write('runner.lua',[
  'local panel = require("ui.面板")', "local lib = require 'lib'", "require('ordinary.main')", "require('outside.tool')",
  'object:require("decoy")', 'local example = [=[require("decoy")]=]', '-- require("decoy")',
  'require("decoy" .. suffix)', 'require("ambiguous")', 'require("linked")',
 ].join('\n'));
 f.write('ui/面板.lua','local file = io.open("map-executed", "w"); file:close()\n');
 for(const p of ['lib/init.lua','decoy.lua','ambiguous.lua','ambiguous/init.lua']) f.write(p,'return {}\n');
 f.write('shadow.lua','local require = function(name) return name end\nrequire("decoy")\n');
 f.write('ordinary/integration.json','{"kind":"project metadata"}\n');
 f.write('ordinary/main.lua','return {}\n');
 f.commit('Rocket Jira and ordinary Lua');
 const link=f.git(['hash-object','-w','--stdin'],'decoy.lua');
 f.git(['update-index','--add','--cacheinfo','120000',link,'linked.lua']);
 f.git(['commit','-m','Link fixture without a filesystem symlink']);
 const tip=f.git(['rev-parse','HEAD']);
 f.write('runner.lua',"require('untracked')\n");f.write('untracked.lua','return 42\n');
 const original=snapshot(f);
 await folders(f,tip);await expand(['jira','ui','lib','ordinary']);
 assert.equal(map().code_map.links.links,4);
 assert.deepEqual(arrow('runner.lua','ui/面板.lua')?.names,['ui.面板']);
 assert.deepEqual(arrow('runner.lua','lib/init.lua')?.names,['lib']);
 assert.deepEqual(arrow('jira/integration.json','jira/main.lua')?.names,['main.lua']);
 assert.deepEqual(arrow('runner.lua','ordinary/main.lua')?.names,['ordinary.main']);
 passed('Folders connects the committed Jira manifest to main.lua and both ordinary module layouts, including a Unicode filename');
 await click('map-mode-modules');await until(()=>!map().running && map().data?.kind==='Modules','Lua groups');
 assert.deepEqual(map().data.module_packages.filter(p=>p.language==='Lua').map(p=>p.folder).sort(),['','jira']);
 assert.equal(map().data.module_packages.find(p=>p.language==='Lua'&&p.folder==='jira').files,1);
 for(const [folder,caller,targets] of [['','runner.lua',['lib/init.lua','ordinary/main.lua','ui/面板.lua']],['jira','jira/integration.json',['jira/main.lua']]]){
  const i=map().data.module_packages.findIndex(p=>p.language==='Lua'&&p.folder===folder);
  assert(i>=0,'Lua group '+folder);
  if(map().package!==folder||map().package_language!=='Lua'){
   await click('map-module-package');await click('map-module-package-'+i);
  }
  await until(()=>!map().running && map().package===folder && map().data?.modules?.language==='Lua','Lua group '+folder);
  const from=node(caller), data=map().data;
  assert.deepEqual(data.links.filter(l=>l.kind==='DependsOn').map(l=>{
   assert.equal(l.from,from.index);return data.nodes[l.to].path;
  }).sort(),targets);
  if(folder===''){
   assert.deepEqual(data.modules.outside.find(o=>o.node===from.index)?.names,['linked','outside.tool']);
   assert(!data.nodes.some(n=>n.path==='untracked.lua'));
  }
  await click('map-module-'+from.index);assert.equal(map().selected,from.index);
  await click('map-node-'+node(targets[0]).index);assert.equal(map().selected,node(targets[0]).index);
 }
 await resize(780,620);assert(control('map-module-switcher'));checkBounds();
 assert.deepEqual(snapshot(f),original);assert(!fs.existsSync(path.join(f.repo,'map-executed')));
 passed('Modules navigates between caller and target, keeps dynamic and ambiguous references out of the graph, preserves local edits and fits the compact window');
 const h=fixture('Lua Stop and reload');
 h.write('start.lua','return require("helper")');h.write('helper.lua','return 1');
 const light=h.commit('Small Lua map');
 const bulk='local x = require("helper")\n'.repeat(4500);
 for(let i=0;i<36;i++) h.write('bulk/caller'+i+'.lua','-- '+i+'\n'+bulk);
 const heavy=h.commit('Larger Lua map'), before=snapshot(h);
 await open(h);await stopThenReload(light,heavy);
 assert.deepEqual(arrow('start.lua','helper.lua')?.names,['helper']);
 assert.deepEqual(snapshot(h),before);
 passed('Stop interrupts an active Lua map read, and reloading the older revision restores its original link and leaves the checkout intact');
}
async function workflowLinks(){
 const f=fixture('workflows 資料'), directory='.github/workflows', mac=directory+'/macos-arm64.yml';
 const primary=directory+'/review.yml', shared=directory+'/shared.yaml';
 const action='.github/actions/check/action.yaml', script='ci/二 words.ps1', build='tools/build-macos.sh', arch='tools/macos/architecture.sh';
 f.write(mac,cp.execFileSync('git',['-C',path.resolve(__dirname,'../..'),'show','HEAD:'+mac],{env,windowsHide:true}));
 f.write(primary,[
  'jobs:', '  checks:', '    runs-on: windows-latest', '    steps:',
  '      - run: |', "          & '.\\二 words.ps1'", '        working-directory: ci',
  '      - uses: ./.github/actions/check', '      - uses: actions/checkout@v4',
  '        with:', '          run: bash decoy.sh',
  '      - run: bash decoy.sh', '        working-directory: ${{ matrix.directory }}',
  '  reuse:', '    uses: $/.github/workflows/shared.yaml',
 ].join('\n'));
 f.write(shared,'on: workflow_call\njobs: {}\n');f.write(action,'name: Local check\nruns: {using: composite, steps: []}\n');
 f.write(build,'echo unexpected > map-executed\n');f.write(arch,'# fixture\n');
 f.write(script,"Set-Content map-executed unexpected\n");f.write('decoy.sh','# fixture\n');
 f.commit('Local workflow targets');
 const link=f.git(['hash-object','-w','--stdin'],'macos-arm64.yml');
 f.git(['update-index','--add','--cacheinfo','120000',link,directory+'/linked.yml']);
 f.git(['commit','-m','Tracked symlink fixture']);
 const tip=f.git(['rev-parse','HEAD']);
 f.write(primary,'jobs: {}\n');f.write('ci/untracked.ps1','# local work\n');
 const before=snapshot(f);
 await folders(f,tip);
 assert.equal(map().code_map.links.links,5);
 await expand(['.github',directory,'.github/actions','.github/actions/check','tools','tools/macos','ci']);
 await click('map-fit');
 for(const [caller,target,kind] of [[mac,build,'run'],[mac,arch,'run'],[primary,script,'run'],[primary,action,'uses'],[primary,shared,'uses']]){
  assert.deepEqual(arrow(caller,target)?.names,[kind],caller+' -> '+target);
 }
 await capture('workflow-folders');fs.copyFileSync(path.join(root,'workflow-folders.png'),path.join(evidence,'folders.png'));
 passed('Folders draws the macOS workflow build and architecture scripts, a Unicode PowerShell path, a local action and a reusable workflow from the selected commit');
 await click('map-mode-modules');await until(()=>!map().running&&map().data?.kind==='Modules','workflow picker');
 const i=map().data.module_packages.findIndex(p=>p.language==='GitHubActions'&&p.folder===directory);
 assert(i>=0);assert.equal(map().data.module_packages[i].files,3);
 if(map().package_language!=='GitHubActions'){await click('map-module-package');await click('map-module-package-'+i);}
 await until(()=>!map().running&&map().data?.modules?.language==='GitHubActions','workflow modules');
 const data=map().data, from=node(primary), helper=node(script);
 const targets=data.links.filter(l=>l.from===from.index&&l.kind==='DependsOn').map(l=>data.nodes[l.to].path).sort();
 assert.deepEqual(targets,[action,shared,script].sort());
 assert(!data.nodes.some(n=>n.path==='decoy.sh'||n.path==='ci/untracked.ps1'));
 await click('map-module-'+from.index);assert.equal(map().selected,from.index);
 await click('map-node-'+helper.index);
 assert.equal(map().selected,helper.index);assert(map().visible.includes(from.index)&&map().captions.some(c=>c.what==='UsedBy'&&c.count===1));
 await resize(780,620);assert(control('map-module-switcher'));checkBounds();
 await capture('workflow-compact');fs.copyFileSync(path.join(root,'workflow-compact.png'),path.join(evidence,'compact.png'));
 assert.deepEqual(snapshot(f),before);assert(!fs.existsSync(path.join(f.repo,'map-executed')));
 passed('Modules navigates in both directions, fits a compact window, and preserves the working tree, index and configuration');
 const h=fixture('workflow Stop and reload');
 const header='jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n';
 h.write(primary,header+'      - run: bash helper.sh\n');h.write('helper.sh','# fixture\n');
 const light=h.commit('Small workflow map'), bulk='      - run: bash helper.sh\n'.repeat(4400);
 for(let j=0;j<36;j++)h.write(directory+'/bulk'+j+'.yml','# '+j+'\n'+header+bulk);
 const heavy=h.commit('Large workflow map'), original=snapshot(h);
 await open(h);await stopThenReload(light,heavy);assert.equal(map().code_map.links.links,1);
 await expand(['.github',directory]);assert.deepEqual(arrow(primary,'helper.sh')?.names,['run']);
 assert.deepEqual(snapshot(h),original);
 passed('Stop cancels an active workflow map read and reloading an older revision restores its original link');
}
function committedFile(p){
 return cp.execFileSync('git',['-C',path.resolve(__dirname,'../..'),'show','HEAD:'+p],{env,windowsHide:true});
}
async function sharedWebsiteFolders(){
 const f=fixture('Rocket website'), style='docs/site.css', script='docs/site.js', logo='docs/images/rocket-logo.png';
 const pages=['docs/get.html','docs/index.html','docs/privacy.html'], nested='docs/guide/start.htm';
 for(const p of [...pages,'docs/404.html',style,script,logo]){
  f.write(p,committedFile(p));
 }
 f.write(nested,'<link rel=stylesheet href="../site.css?v=2"><script src="../site.js"></script><img src="../images/rocket-logo.png#logo">');
 const callers=[...pages,nested].sort(), tip=f.commit('Shared website assets');
 f.write(pages[1],'<img src="untracked.png">');f.write('docs/untracked.png','local work');
 const before=snapshot(f);
 await folders(f,tip);assert.equal(map().code_map.links.links,12);
 await expand(['docs','docs/guide','docs/images']);await click('map-fit');
 for(const p of callers)for(const [asset,kind] of [[style,'stylesheet'],[script,'script'],[logo,'image']]){
  assert.deepEqual(arrow(p,asset)?.names,[kind],p+' -> '+asset);
 }
 // The actual 404 page uses a deployment mount (/Rocket/), absent from this Git tree.
 assert(!arrow('docs/404.html',style));
 return {f,style,nested,callers,before};
}
async function webAssets(){
 const {f,style,nested,callers,before}=await sharedWebsiteFolders();
 await capture('web-folders');fs.copyFileSync(path.join(root,'web-folders.png'),path.join(evidence,'folders.png'));
 passed('Folders shows twelve links from three committed Rocket pages and a nested page; the deployment-only 404 base stays unresolved');
 await click('map-mode-modules');await until(()=>!map().running&&map().data?.kind==='Modules','web picker');
 const i=map().data.module_packages.findIndex(p=>p.language==='HtmlCss');
 assert(i>=0);assert.equal(map().data.module_packages[i].files,6);
 if(map().package_language!=='HtmlCss'){await click('map-module-package');await click('map-module-package-'+i);}
 await until(()=>!map().running&&map().data?.modules?.language==='HtmlCss','HTML / CSS modules');
 const data=map().data, sheet=node(style), page=node(nested);
 assert.deepEqual(data.links.filter(l=>l.kind==='DependsOn'&&l.to===sheet.index).map(l=>data.nodes[l.from].path).sort(),callers);
 await click('map-module-'+sheet.index);
 assert.equal(map().selected,sheet.index);assert(map().captions.some(c=>c.what==='UsedBy'&&c.count===4));
 await click('map-node-'+page.index);
 assert.equal(map().selected,page.index);assert(map().captions.some(c=>c.what==='Uses'&&c.count===3));
 await click('map-node-'+sheet.index);assert.equal(map().selected,sheet.index);
 await capture('web-modules');fs.copyFileSync(path.join(root,'web-modules.png'),path.join(evidence,'modules.png'));
 await resize(780,620);assert(control('map-module-switcher'));checkBounds();
 await capture('web-compact');fs.copyFileSync(path.join(root,'web-compact.png'),path.join(evidence,'compact.png'));
 assert.deepEqual(snapshot(f),before);
 passed('Modules shows all four users of a shared stylesheet, navigates to a page and back, and fits at 780 × 620 with local edits intact');

 const h=fixture('Web assets 資料'), img='img/雪 #1.png', font='fonts/字体.woff2', sub='css/theme/sub.css';
 const html='<link rel=stylesheet href="css/site.css?v=2#main"><script src=local.js></script>'+
  '<picture><source srcset="data:image/png;base64,AA 1x, img/%E9%9B%AA%20%231.png?x 2x, img/large.png 3x"></picture>'+
  '<!-- <img src=decoy.png> --><img src=linked.png>';
 h.write('index.html',html);h.write('external.htm','<img src=decoy.png><base href="https://example.invalid/">');
 h.write('css/site.css','@import "theme/sub.css?v=2"; body{background:url(../img/%E9%9B%AA%20%231.png#crop)}');
 h.write(sub,'@font-face { src:url("../../fonts/字体.woff2?#v") }');
 h.write('local.js',"require('node:fs').writeFileSync('map-executed','unexpected')\n");
 for(const p of [img,'img/large.png',font,'decoy.png'])h.write(p,Buffer.from([0,255,128,3]));
 h.commit('Unicode and binary assets');
 const link=h.git(['hash-object','-w','--stdin'],'decoy.png');
 h.git(['update-index','--add','--cacheinfo','120000',link,'linked.png']);h.git(['commit','-m','Tracked link']);
 const light=h.git(['rev-parse','HEAD']);
 // Build both revisions before opening the checkout so Stop targets the map
 // read initiated below, independently of filesystem watcher refreshes.
 h.git(['switch','--quiet','-c','large-site']);
 const bulk='<img src="../img/large.png">'.repeat(4000);
 for(let j=0;j<40;j++)h.write('bulk/page'+j+'.html','<!-- '+j+' -->\n'+bulk);
 const heavy=h.commit('Large site for Stop');
 h.git(['switch','--quiet',branch]);h.write('index.html','working edit\n');const original=snapshot(h);
 await folders(h,light);assert.equal(map().code_map.links.links,7);
 await expand(['css','css/theme','img','fonts']);await click('map-fit');
 for(const [from,to,kind] of [['index.html','css/site.css','stylesheet'],['index.html','local.js','script'],
  ['index.html',img,'srcset'],['index.html','img/large.png','srcset'],['css/site.css',sub,'@import'],
  ['css/site.css',img,'url()'],[sub,font,'url()']]){
  assert.deepEqual(arrow(from,to)?.names,[kind],from+' -> '+to);
 }
 assert.deepEqual(snapshot(h),original);assert(!fs.existsSync(path.join(h.repo,'map-executed')));
 passed('A second checkout resolves responsive images, percent-encoded Unicode, CSS imports and a binary font, skipping remote, commented and symlink targets');
 await stopThenReload(light,heavy);assert.equal(map().code_map.links.links,7);
 await expand(['css','css/theme','fonts']);assert.deepEqual(arrow(sub,font)?.names,['url()']);
 assert.deepEqual(snapshot(h),original);
 passed('Stop interrupts an active read; loading the older revision restores its seven links and preserves HEAD, index, config and working files');
}
async function keepCapture(name){
 await capture(name);fs.copyFileSync(path.join(root,name+'.png'),path.join(evidence,name+'.png'));
}
function checkFitCounts(){
 const {arrows,count_labels:labels}=map().code_map.picture;
 assert.equal(arrows.length,12);
 const cards=state.controls.filter(c=>/^map-(node-\d+|toggle-\d+|files-(\d+|top))$/.test(c.id));
 assert(cards.length>=8,'fitted website cards must be available for hit testing');
 const intersects=(a,b)=>a[0]<b[2]&&a[2]>b[0]&&a[1]<b[3]&&a[3]>b[1];
 for(const [i,label] of labels.entries()){
  assert(label[0]>=0&&label[1]>=0&&label[2]<=state.viewport[0]&&label[3]<=state.viewport[1]);
  assert(!cards.some(c=>intersects(label,c.rect)),'count touches a card '+JSON.stringify(label));
  assert(!labels.slice(0,i).some(other=>intersects(label,other)),'two counts overlap');
 }
 return {width:state.viewport[0],height:state.viewport[1],zoom_percent:Math.round(map().zoom*100),
  arrows:arrows.length,visible_counts:labels.length};
}
async function selectReferenceRevision(tip){
 await load(tip);
 await until(()=>map().data?.tip===tip&&!map().running &&
  (map().kind==='Modules'||map().code_map.links&&!map().code_map.running),'selected revision and links');
}
async function openReferencesFor(file){
 const i=node(file).index;
 await click((map().kind==='Modules'?'map-module-':'map-node-')+i);
 assert.equal(map().selected,i);
 await click('map-go-to-reference');reads();
}
// The first reference opens at once; rows and arrows each read another committed source.
function reads(n=1){report.reference_reads=(report.reference_reads??0)+n;}
async function expectReference(tip,source,target,line,kind,text){
 const ref=map().reference;
 assert.equal(ref?.commit,tip);
 const index=ref.sites.findIndex(s=>s.source===source&&s.target===target&&s.line===line&&s.kind===kind);
 assert(index>=0,JSON.stringify({source,target,line,sites:ref.sites}));
 if(ref.selected!==index){
  if(!control('map-reference-'+index)&&control('map-reference-list'))await click('map-reference-list');
  await click('map-reference-'+index);reads();
 }
 await until(()=>map().reference?.selected===index&&!map().reference.loading,'committed source');
 assert.equal(map().reference.error,null);
 assert.equal(map().reference.text,text);
 // Asset references mark the target's name in the line; module ones mark it only where the line spells it.
 const marked=map().reference.marked;
 if(['include_str!','include_bytes!','srcset','url()','@import','stylesheet','script','image','font'].includes(kind))assert.equal(marked,target.split('/').pop());
 else if(marked!==null)assert(text.includes(marked),JSON.stringify({marked,text}));
 await until(()=>!!control('map-reference-source-line-'+line),'highlighted row in viewport');
 checkBounds();
 report.reference_opens=(report.reference_opens??0)+1;
}
// Back to the map: a narrow window with several references goes back to their list first.
async function closeReference(){
 if(control('map-reference-list'))await click('map-reference-list');
 await click('map-reference-close');
 await until(()=>!map().reference,'reference view closed');
}
async function openReferenceModules(language,tip){
 await click('map-mode-modules');
 await until(()=>map().data?.kind==='Modules'&&!map().running);
 if(map().data.modules?.language!==language){
  const i=map().data.module_packages.findIndex(p=>p.language===language);assert(i>=0);
  await click('map-module-package');await click('map-module-package-'+i);
 }
 await selectReferenceRevision(tip);
}
async function assetReferences(){
 const rust=fixture('Rust locations 雪'), src='src/lib.rs', guide='入門.md', single='one.bin';
 const earlier='const FIRST: &str = include_str!("../入門.md");';
 const later='const SECOND: &str = include_str!("../入門.md");';
 const bytes='const ONE: &[u8] = include_bytes!("../one.bin");';
 const body=['// include_str!("../入門.md")',earlier,...Array(725).fill('// padding'),later,...Array(55000).fill(''),bytes].join('\r\n')+'\r\n';
 rust.write('Cargo.toml',"[package]\nname='reference_demo'\nversion='0.1.0'\n");
 rust.write(src,body);rust.write(guide,'hello');rust.write(single,Buffer.from([0,255]));
 const first=rust.commit('Original embedded locations');
 rust.write(src,'// moved\n// again\n'+body.replace('const SECOND','pub const SECOND'));
 const second=rust.commit('Move reference lines');
 rust.write(src,'// staged Rust source\n');rust.git(['add','--',src]);rust.write(src,'// local Rust source\n');
 await folders(rust,second);await selectReferenceRevision(first);await expand(['src']);await click('map-fit');
 const before=snapshot(rust);await openReferencesFor(guide);assert.equal(map().reference.count,2);
 await until(()=>map().reference.selected===0&&!map().reference.loading,'first reference opens at once');
 assert.equal(map().reference.listing,false);
 await click('map-reference-next');reads();await until(()=>map().reference.selected===1&&!map().reference.loading,'next reference');
 assert.equal(map().reference.text,later);assert.equal(map().reference.window,true);
 await click('map-reference-previous');reads();await until(()=>map().reference.selected===0&&!map().reference.loading,'previous reference');
 assert.equal(map().reference.text,earlier);
 await closeReference();assert.equal(map().selected,node(guide).index);
 await click('map-go-to-reference');reads();
 await expectReference(first,src,guide,2,'include_str!',earlier);
 await expectReference(first,src,guide,728,'include_str!',later);
 await keepCapture('rust-reference');await closeReference();assert.equal(map().selected,node(guide).index);
 await openReferencesFor(src);assert.equal(map().reference.count,3);await expectReference(first,src,single,55729,'include_bytes!',bytes);assert.equal(map().reference.window,true);
 await closeReference();assert.deepEqual(snapshot(rust),before);
 passed('Folders opens both occurrences of a Rust embed and the byte embed at the selected older commit, including visible highlights on lines 728 and 55729');
 await openReferenceModules('Rust',second);await openReferencesFor(guide);
 await expectReference(second,src,guide,730,'include_str!',later.replace('const SECOND','pub const SECOND'));
 await closeReference();assert.deepEqual(snapshot(rust),before);
 passed('Modules follows the same asset to its moved line at a newer commit while the index and checkout contain different text');

 const web=fixture('Web locations 雪'), page='頁.html', sheet='site.css', base='base.css', pic='雪.png';
 const html=['<!-- <img src="雪.png"> -->','<link rel="stylesheet"',' href="site.css">','<img srcset="missing.png 1x,',' 雪.png 2x">','<div style="color: red&#59;',' background: url(&quot;雪.png&quot;)"></div>'];
 const css=['/* url(雪.png) */','@import',' "base.css";','body { background: url(',' 雪.png) }'];
 web.write(page,html.join('\r\n'));web.write(sheet,css.join('\r\n'));web.write(base,'/* base */');web.write(pic,Buffer.from([0,254]));
 const original=web.commit('Original web references');
 web.write(page,'\n\n'+html.join('\n'));web.write(sheet,'\n'+css.join('\n'));
 const head=web.commit('Move web references');web.write(page,'<p>Staged</p>');web.git(['add','--',page]);web.write(page,'<p>Unstaged</p>');
 await folders(web,head);await selectReferenceRevision(original);
 // An external edit after the map has loaded must not change the source we open.
 fs.unlinkSync(path.join(web.repo,sheet));const unchanged=snapshot(web);
 await openReferencesFor(pic);assert.equal(map().reference.count,3);
 await expectReference(original,page,pic,5,'srcset',html[4]);
 await expectReference(original,page,pic,7,'url()',html[6]);
 await expectReference(original,sheet,pic,5,'url()',css[4]);await closeReference();
 passed('Shared Unicode image opens the exact HTML srcset, entity-encoded inline CSS and standalone CSS lines after the working stylesheet is deleted');
 await openReferenceModules('HtmlCss',original);await openReferencesFor(base);assert.equal(map().reference.count,1);
 await expectReference(original,sheet,base,3,'@import',css[2]);await closeReference();
 await openReferencesFor(page);await resize(780,620);await expectReference(original,page,sheet,3,'stylesheet',html[2]);assert.equal(map().reference.window,false);
 await keepCapture('compact-reference');
 // Narrow: back goes to the list first, and a row opens the next reference in its place.
 await click('map-reference-list');await until(()=>map().reference.listing&&!!control('map-reference-1'),'narrow list');
 assert(!control('map-reference-copy')&&!control('map-reference-next'));await keepCapture('compact-reference-list');
 await expectReference(original,page,pic,5,'srcset',html[4]);assert.equal(map().reference.listing,false);
 await closeReference();assert.equal(map().selected,node(page).index);
 assert.deepEqual(snapshot(web),unchanged);
 passed('A single CSS import opens directly. A compact window shows the selected commit and highlighted HTML line, returns to the same module and preserves local files, index, config and HEAD');
}
async function rustImportReferences(){
 const f=fixture('Rust import locations 雪'), lib='src/lib.rs', user='src/client.rs', model='src/型.rs';
 const decl=['mod client;', '#[path = "型.rs"]', 'pub mod model;', 'pub mod facade {',
  '    pub(crate) use super::model::{', '        Alpha as One,', '        Beta,', '    };', '}'];
 const imports=['use crate::facade::{', '    One as Local,', '    Beta,', '};', 'pub use crate::model::*;',
  'use crate::model::Missing;', 'const TEXT: &str = stringify! { use crate::model::Alpha; };'];
 f.write('Cargo.toml',"[package]\nname='reference_sites'\nversion='0.1.0'\n");
 f.write(lib,decl.join('\r\n'));f.write(user,imports.join('\n'));f.write(model,'pub struct Alpha;\npub struct Beta;\n');
 const original=f.commit('Rust module, imports and re-exports');
 f.write(lib,'// moved\n'+decl.join('\n'));f.write(user,'// changed\n\n'+imports.join('\n'));
 const head=f.commit('Shift the committed source lines');
 for(const file of [lib,user]){f.write(file,'// staged replacement\n');f.git(['add','--',file]);f.write(file,'// local replacement\n');}
 await folders(f,head);await selectReferenceRevision(original);await expand(['src']);await click('map-fit');
 fs.unlinkSync(path.join(f.repo,user));const before=snapshot(f);
 await openReferencesFor(model);assert.equal(map().reference.count,6);
 await expectReference(original,lib,model,3,'mod',decl[2]);
 await expectReference(original,lib,model,6,'pub(crate) use',decl[5]);
 await expectReference(original,user,model,2,'use',imports[1]);
 await expectReference(original,user,model,5,'pub use',imports[4]);await closeReference();
 assert.equal(map().selected,node(model).index);
 passed('Folders opens the Unicode target’s declaration, restricted re-export, aliased grouped import and glob at their older committed lines with staged replacements and a deleted working source');
 await openReferenceModules('Rust',head);
 await openReferencesFor(lib);await expectReference(head,lib,model,4,'mod',decl[2]);
 await expectReference(head,lib,model,8,'pub(crate) use',decl[6]);await closeReference();
 await openReferencesFor(user);await expectReference(head,user,model,5,'use',imports[2]);
 await expectReference(head,user,model,7,'pub use',imports[4]);await closeReference();
 assert.deepEqual(snapshot(f),before);
 passed('Modules opens references from the importing files at their new committed lines, follows local re-exports, and preserves HEAD, index, configuration and remaining files');

 const g=fixture('Nested Rust locations'), parent='src/nested/mod.rs', leaf='src/nested/group/type.rs';
 const declaration='        r#type;';
 g.write('Cargo.toml',"[package]\nname='nested_sites'\nversion='0.1.0'\n");
 g.write(lib,'mod nested;\n');g.write(parent,['mod group {','    pub mod',declaration,'}'].join('\n'));
 g.write(leaf,'pub struct Marker;\n');const tip=g.commit('Raw module name in an inline module');
 g.write(parent,'// unrelated working lines\n');const secondBefore=snapshot(g);
 await folders(g,tip);await expand(['src','src/nested','src/nested/group']);await click('map-fit');
 await openReferencesFor(leaf);assert.equal(map().reference.count,1);assert.equal(map().reference.selected,0);
 await expectReference(tip,parent,leaf,3,'mod',declaration);await closeReference();
 assert.deepEqual(snapshot(g),secondBefore);
 passed('A single nested mod declaration opens directly on its raw identifier after the working parent changes in a second repository');
}
async function rustAssets(){
 const f=fixture('Rust resources'), guide='docs/入門 #1.md', script='src/app_update/extract.ps1', image='assets/themes/astronaut.jpg';
 const update='src/app_update.rs', picture='src/theme/picture.rs', help='src/theme/help.rs', settings='src/theme/settings.rs';
 f.write('Cargo.toml',"[package]\nname='rocket_map_demo'\nversion='0.1.0'\n");
 f.write('src/lib.rs','mod app_update; mod theme;\n');f.write('src/theme/mod.rs','mod help; mod settings; mod picture;\n');
 for(const p of [update,script])f.write(p,committedFile('crates/rocket-git/'+p));
 for(const p of [picture,image])f.write(p,committedFile('crates/rocket-ui/'+p));
 f.write(help,'#![doc = std::include_str!("../../docs/入門 #1.md")]\n');
 f.write(settings,'const GUIDE: &[u8] = include_bytes![r#"../../docs/入門 #1.md"#,];\n'+
  'const NEVER: &str = stringify!(include_str!("../../decoy"));\n'+
  'const DYNAMIC: &str = include_str!(concat!("../../","decoy"));\n'+
  'const LINK: &str = include_str!("../../linked");\n');
 f.write(guide,'Shared guide\n');f.write('decoy','ordinary file\n');f.commit('Rust modules and their resources');
 const link=f.git(['hash-object','-w','--stdin'],'decoy');
 f.git(['update-index','--add','--cacheinfo','120000',link,'linked']);f.git(['commit','-m','Tracked symlink']);
 const light=f.git(['rev-parse','HEAD']);
 f.git(['switch','--quiet','-c','large-rust']);
 const bulk='const _: &[u8] = include_bytes!("../docs/入門 #1.md");\n'.repeat(1900);
 for(let n=0;n<40;n++)f.write('src/bulk'+n+'.rs','// file '+n+'\n'+bulk);
 const heavy=f.commit('Many candidate reads for Stop');
 f.git(['switch','--quiet',branch]);f.write(help,'// local changes\n');f.write('untracked.bin',Buffer.from([0,255,9]));
 const before=snapshot(f);
 await folders(f,light);
 const edges=[[update,script,'include_str!'],[picture,image,'include_bytes!'],[help,guide,'include_str!'],[settings,guide,'include_bytes!']];
 await expand(['src','src/app_update','src/theme','assets','assets/themes','docs']);await click('map-fit');
 for(const [from,to,kind] of edges)assert.deepEqual(arrow(from,to)?.names,[kind],from+' -> '+to);
 assert(!arrow(settings,'decoy'));
 const symlink=map().data.nodes.find(n=>n.path==='linked');
 assert(!symlink||!map().code_map.picture.arrows.some(e=>e.from===node(settings).index&&e.to===symlink.index));
 await keepCapture('rust-folders');
 passed('Folders traces the committed Rocket updater to its PowerShell helper, its theme to the binary image, and two modules to a shared Unicode guide');
 await click('map-mode-modules');await until(()=>!map().running&&map().data?.kind==='Modules','Modules picker');
 if(map().data.modules?.language!=='Rust'){
  const i=map().data.module_packages.findIndex(p=>p.language==='Rust');assert(i>=0);
  await click('map-module-package');await click('map-module-package-'+i);
 }
 await until(()=>!map().running&&map().data?.modules?.language==='Rust','Rust modules');
 const data=map().data, file=node(guide);
 assert.deepEqual(data.links.filter(e=>e.kind==='DependsOn'&&e.to===file.index).map(e=>data.nodes[e.from].path).sort(),[help,settings]);
 for(const [from,to] of [[update,script],[picture,image],[help,guide]]){
  const caller=node(from),asset=node(to);
  await click('map-module-'+asset.index);assert.equal(map().selected,asset.index);
  if(to===guide){
   assert(map().captions.some(c=>c.what==='UsedBy'&&c.count===2));
   assert(control('map-node-'+node(settings).index)?.enabled);
  }
  await click('map-node-'+caller.index);assert.equal(map().selected,caller.index);
  await click('map-node-'+asset.index);assert.equal(map().selected,asset.index);
 }
 await keepCapture('rust-modules');
 await resize(780,620);assert(control('map-module-switcher'));checkBounds();await keepCapture('rust-compact');
 assert.deepEqual(snapshot(f),before);
 passed('Modules navigates from each resource to its caller and back, distinguishes sibling callers and fits at 780 × 620');
 await resize(1480,920);await stopThenReload(light,heavy);
 await expand(['src','src/theme','docs']);await click('map-fit');assert.deepEqual(arrow(help,guide)?.names,['include_str!']);
 assert.deepEqual(snapshot(f),before);
 passed('One Stop click interrupts an active read; the original embed returns on revision reload with HEAD, config, index and working files intact');
 const site=await sharedWebsiteFolders();
 report.fit=[checkFitCounts()];assert(report.fit[0].visible_counts>0,'the wide example must display counts');
 await keepCapture('fit-wide');await resize(780,620);
 if(control('map-close-details')?.enabled)await click('map-close-details');
 await click('map-fit');checkBounds();
 const compact=checkFitCounts();assert(compact.visible_counts>0,'compact Fit must display counts where space permits');
 report.fit.push(compact);await keepCapture('fit-compact');
 const privacy=node('docs/privacy.html');await click('map-node-'+privacy.index);assert.equal(map().selected,privacy.index);
 assert.deepEqual(snapshot(site.f),site.before);
 passed('All twelve website arrows survive Fit at both sizes; counts clear every visible card and each other, and the compact privacy.html card can be selected');
}
async function nativeBridges(){
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
 await open(h);await stopThenReload(light,heavy);
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
 if(focused){await focused.run();return;}
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
 const recordsLog=diagnosticRecords();
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
   if(['--references-only','--rust-import-references-only'].includes(focused.flag)){
    assert(report.reference_opens>0,'no committed source was opened');
    const records=diagnosticRecords().filter(r=>r.operation==='map-open-reference');
    const starts=records.filter(r=>r.event==='request_start');
    assert.equal(starts.length,report.reference_reads);
    for(const start of starts) assert(records.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request));
    const logged=JSON.stringify(records);
    for(const privateText of ['入門.md','頁.html','雪.png','const SECOND','型.rs','One as Local','r#type',root])assert(!logged.includes(privateText));
    passed('Each source read has paired diagnostic records without source text or fixture paths');
   }
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
