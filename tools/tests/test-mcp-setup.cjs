// Native acceptance through the authenticated loopback HTTP transport only.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const version=fs.readFileSync('Cargo.toml','utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const root=path.resolve('work/mcp-fixtures/setup-'+crypto.randomUUID().replaceAll('-',''));
const profile=path.join(root,'http-profile'),seed=path.join(root,'seed'),branch='rocket-ui-test/setup';
const report={passed:false,version,root,cases:[]};let client,service,state;
fs.mkdirSync(seed,{recursive:true});fs.mkdirSync(path.join(root,'hooks'));fs.writeFileSync(path.join(root,'global.config'),'[user]\n name = PRIVATE Inherited User\n email = PRIVATE-inherited@example.invalid\n');fs.writeFileSync(path.join(root,'system.config'),'');
const env={...process.env};for(const key of Object.keys(env))if(/^(GIT_|SSH_)|_PROXY$/i.test(key))delete env[key];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'global.config'),GIT_CONFIG_SYSTEM:path.join(root,'system.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_CONFIG_COUNT:'1',GIT_CONFIG_KEY_0:'commit.gpgsign',GIT_CONFIG_VALUE_0:'false'});
function git(at,args,options={}){return cp.execFileSync('git',['-C',at,...args],{env,windowsHide:true,encoding:'utf8',...options}).trim();}
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const globalHash=hash(path.join(root,'global.config')),systemHash=hash(path.join(root,'system.config'));
git(seed,['init','--template=','--initial-branch='+branch]);git(seed,['config','commit.gpgsign','false']);git(seed,['config','core.hooksPath',path.join(root,'hooks')]);
fs.writeFileSync(path.join(seed,'file.txt'),'base\n');git(seed,['add','file.txt']);git(seed,['commit','-m','Synthetic setup baseline']);
fs.writeFileSync(path.join(seed,'.git/rocket-ui-fixture'),branch);fs.writeFileSync(path.join(seed,'file.txt'),'staged\n');git(seed,['add','file.txt']);fs.writeFileSync(path.join(seed,'file.txt'),'working\n');
const preserved={head:git(seed,['rev-parse','HEAD']),index:hash(path.join(seed,'.git/index')),file:hash(path.join(seed,'file.txt'))};
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function settle(){for(let n=0;n<600;n++){state=await client.tool('rocket_state');if(!state.busy&&!state.input_pending)return state;await delay(50);}throw Error('UI did not settle');}
const click=async target=>{await client.tool('rocket_click',{target,expected_tab:state.active_tab});return settle();};
const type=async(target,text)=>{await client.tool('rocket_type',{target,text,expected_tab:state.active_tab});return settle();};
const pass=message=>{report.cases.push(message);console.error('PASS '+message);};
async function capture(name){const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(x=>x.type==='image').data,'base64'));}
async function resize(w,h){await client.tool('rocket_resize',{width:w,height:h});for(let n=0;n<100;n++){await delay(30);await settle();if(state.viewport[0]===w&&state.viewport[1]===h)return;}throw Error('Resize did not settle');}
async function settings(){await click('menu-repository');await click('repository-settings-menu');const visible=()=>{const c=state.controls.find(c=>c.id==='repository-settings');return c&&c.rect[3]-c.rect[1]>=28;};for(let i=0;i<8&&!visible();i++){await settle();await delay(100);await settle();}await click('repository-settings');assert.equal(state.dialog.kind,'identity');assert(state.dialog.identity_loaded);}
async function create(target,initial=branch){fs.writeFileSync(target+'.rocket-init-fixture',initial);await click('menu-file');await click('repository-new');await type('dialog-init-path',target);await type('dialog-init-branch',initial);}
async function finishCreation(target){await click('dialog-submit');if(state.scope_error){assert.equal(git(target,['symbolic-ref','--short','HEAD']),branch);assert(fs.statSync(path.join(target,'.git')).isDirectory());fs.writeFileSync(path.join(target,'.git/rocket-ui-fixture'),branch,{flag:'wx'});await settle();}assert(state.activities.at(-1).command_succeeded,JSON.stringify(state.activities.at(-1)));}
async function closed(){await client.tool('rocket_close');for(let n=0;n<200;n++){const s=await client.tool('rocket_state');if(['closed','not_started'].includes(s.window))return;await delay(50);}throw Error('Test window did not close');}
async function main(){
 try {
  const exe=path.resolve('target/release/rocket-mcp.exe');cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
  const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
  fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);report.endpoint=`http://127.0.0.1:${port}/mcp`;
  const serviceLog=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',serviceLog]});fs.closeSync(serviceLog);client=new RocketHttpClient(profile);
  for(let n=0;;n++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(n===100)throw e;await delay(100);}}
  assert.equal((await client.initialize()).serverInfo.version,version);await client.tool('rocket_open',{path:seed});await settle();
  await click('view-changes');await type('commit-message','PRIVATE preserved draft');
  await settings();await capture('identity-inherited');assert.equal(state.dialog.first,null);assert.equal(state.dialog.second,null);
  const beforeInvalid=hash(path.join(seed,'.git/config'));await type('dialog-identity-name','PRIVATE Local 文');await type('dialog-identity-email','not-an-email');await click('dialog-submit');assert(state.dialog.error.includes('email'));assert.equal(hash(path.join(seed,'.git/config')),beforeInvalid);
 } finally {}
}
// The remainder is kept in one acceptance run; no real account configuration is read.
async function run(){try{
 await main();
 await type('dialog-identity-email','PRIVATE-local@example.invalid');assert.equal(state.dialog.error,null);await capture('identity-edit');await click('dialog-submit');assert(state.activities.at(-1).command_succeeded);
 assert.equal(git(seed,['config','--local','user.name']),'PRIVATE Local 文');assert.equal(git(seed,['config','--local','user.email']),'PRIVATE-local@example.invalid');
 assert.deepEqual({head:git(seed,['rev-parse','HEAD']),index:hash(path.join(seed,'.git/index')),file:hash(path.join(seed,'file.txt'))},preserved);assert(state.tabs.find(t=>t.id===state.active_tab).draft_present);
 pass('native identity form loads inherited values, validates email and saves local-only settings without changing HEAD/index/files/draft');
 await settings();const config=path.join(seed,'.git/config');git(seed,['config','custom.external','preserve']);const changed=hash(config);await type('dialog-identity-name','PRIVATE stale');await click('dialog-submit');assert(!state.activities.at(-1).command_succeeded);assert(JSON.stringify(state.activities.at(-1)).includes('changed'));assert.equal(hash(config),changed);
 await settings();await type('dialog-identity-name','PRIVATE locked');fs.writeFileSync(path.join(seed,'.git/config.lock'),'external lock');await click('dialog-submit');assert(!state.activities.at(-1).command_succeeded);assert(JSON.stringify(state.activities.at(-1)).includes('lock repository config'));assert.equal(hash(config),changed);assert.equal(fs.readFileSync(path.join(seed,'.git/config.lock'),'utf8'),'external lock');fs.unlinkSync(path.join(seed,'.git/config.lock'));
 await settings();await type('dialog-identity-name','PRIVATE cancelled');await click('dialog-cancel');assert.equal(hash(config),changed);pass('stale edits, config-lock errors and Cancel preserve configuration and other writers locks');
 await settings();await type('dialog-identity-name','');await type('dialog-identity-email','');await click('dialog-submit');assert(state.activities.at(-1).command_succeeded);assert.equal(git(seed,['config','user.name']),'PRIVATE Inherited User');assert.equal(git(seed,['config','user.email']),'PRIVATE-inherited@example.invalid');pass('clearing local overrides restores inherited identity');
 await settings();await resize(780,620);await capture('identity-narrow');await click('dialog-cancel');await resize(1480,920);
 await click('view-changes');await type('commit-message','');
 const project=path.join(root,'existing project 文');fs.mkdirSync(project);fs.mkdirSync(path.join(project,'src'));fs.writeFileSync(path.join(project,'src/data.bin'),Buffer.from([0,255,13,10]));fs.writeFileSync(path.join(project,'README.md'),'PRIVATE keep every byte\n');fs.writeFileSync(path.join(project,'.gitignore'),'ignored\n');fs.writeFileSync(path.join(project,'ignored'),'PRIVATE ignored bytes');
 const files=['src/data.bin','README.md','.gitignore','ignored'];const hashes=files.map(f=>hash(path.join(project,f)));
 await create(project);await capture('new-repository');await resize(780,620);await capture('new-repository-narrow');await resize(1480,920);await finishCreation(project);assert.deepEqual(files.map(f=>hash(path.join(project,f))),hashes);assert.equal(git(project,['symbolic-ref','--short','HEAD']),branch);assert(!fs.existsSync(path.join(project,'.git/index')));assert(!fs.existsSync(path.join(project,'.git/hooks')));
 fs.writeFileSync(path.join(project,'.git/rocket-ui-fixture'),branch);git(project,['config','commit.gpgsign','false']);git(project,['config','core.hooksPath',path.join(root,'hooks')]);await settle();pass('Create initializes a populated folder, opens an unborn branch and preserves tracked-candidate/ignored/binary files without staging');
 await settings();await type('dialog-identity-name','PRIVATE New Author');await type('dialog-identity-email','PRIVATE-new@example.invalid');await click('dialog-submit');assert(state.activities.at(-1).command_succeeded);
 await click('view-changes');await click('stage-all');await type('commit-message','Synthetic first UI commit');await click('commit');assert(state.activities.at(-1).command_succeeded);assert.equal(git(project,['log','-1','--format=%an <%ae>']),'PRIVATE New Author <PRIVATE-new@example.invalid>');pass('first commit through native UI uses the repository-local identity');
 const configHash=hash(path.join(project,'.git/config')),head=git(project,['rev-parse','HEAD']);await create(project);await click('dialog-submit');assert(!state.activities.at(-1).command_succeeded);assert(JSON.stringify(state.activities.at(-1)).includes('already a repository'));assert.equal(hash(path.join(project,'.git/config')),configHash);assert.equal(git(project,['rev-parse','HEAD']),head);assert.deepEqual(files.map(f=>hash(path.join(project,f))),hashes);await capture('existing-repository-error');pass('existing repository is rejected without changing its config, history or files');
 const cancelled=path.join(root,'cancelled');await create(cancelled);await click('dialog-cancel');assert(!fs.existsSync(cancelled));
 const fresh=path.join(root,'brand new');await create(fresh);await finishCreation(fresh);assert(!fs.existsSync(path.join(fresh,'.git/index')));pass('new folder creation succeeds; cancelling the creation dialog writes nothing');
 const unsafe=path.join(root,'unmarked');await click('menu-file');await click('repository-new');await type('dialog-init-path',unsafe);await type('dialog-init-branch',branch);const refused=await client.request('tools/call',{name:'rocket_click',arguments:{target:'dialog-submit',expected_tab:state.active_tab}});assert(refused.isError);assert(!fs.existsSync(unsafe));await click('dialog-cancel');pass('HTTP automation rejects creation without the independent fixture ownership marker');
 assert.equal(hash(path.join(root,'global.config')),globalHash);assert.equal(hash(path.join(root,'system.config')),systemHash);
 const records=[];let after_seq=0;for(let n=0;;n++){assert(n<20);const page=await client.tool('rocket_logs',{after_seq,limit:1000});assert(!page.retention_gap);records.push(...page.records);if(!page.more)break;after_seq=page.next_seq;}
 const serialized=JSON.stringify(records);for(const secret of ['PRIVATE','existing project','brand new'])assert(!serialized.includes(secret));for(const operation of ['init','identity-read','identity-save'])assert(records.some(r=>r.operation===operation));assert.equal(state.dropped_events,0);report.log_records=records.length;
 assert(!JSON.stringify(state).includes('PRIVATE-new@example.invalid'));pass('diagnostics identify initialization and identity phases without names/emails/contents; global/system configuration unchanged');
 await closed();await client.raw('POST','/shutdown');await client.finish();report.passed=true;
 }finally{
  fs.writeFileSync(path.join(root,'last-state.json'),JSON.stringify(state??null,null,2));fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
  if(service&&service.exitCode===null){try{await closed();await client.raw('POST','/shutdown');}catch{} await delay(150);if(service.exitCode===null)service.kill();}
  if(client&&!client.exited)await client.finish();
 }}
run().catch(e=>{console.error(e);process.exitCode=1;});
