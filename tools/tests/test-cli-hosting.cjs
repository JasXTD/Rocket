// Focused executable-level hosting acceptance. Loopback, temporary repositories,
// synthetic credentials and isolated Git config only. No account settings touched.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {create}=require('./fixtures/hosting-inline.cjs');
const root=path.resolve('work/cli-hosting-'+crypto.randomUUID()),repo=path.join(root,'repo 資料'),exe=path.resolve('target/release/rocket-cli.exe');
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'template'));fs.writeFileSync(path.join(root,'empty.config'),'');
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|GCM_|SSH_|CODEX_|OPENAI_|ANTHROPIC_)|_TOKEN$|_PROXY$/i.test(k))delete env[k];
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_TERMINAL_PROMPT:'0',GIT_ALLOW_PROTOCOL:'file',GIT_TEMPLATE_DIR:path.join(root,'template'),http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:'',ROCKET_TEST_TOKEN:'rocket-disposable-fixture'});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8'}).trim(),hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
git(['init','--initial-branch=rocket-ui-test/hosting']);git(['config','user.name','Hosting Fixture']);git(['config','user.email','fixture@example.invalid']);git(['config','commit.gpgsign','false']);
fs.writeFileSync(path.join(repo,'資料.txt'),'committed\n');git(['add','--all']);git(['commit','-m','Synthetic']);fs.appendFileSync(path.join(repo,'資料.txt'),'keep dirty\n');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),'rocket-ui-test/hosting');
const local=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt'))}),before=local();
const report={passed:false,root,cases:[],binary:hash(exe)},logs=[];let fixture;
const pass=s=>{report.cases.push(s);console.log('PASS '+s);};
async function run(provider,port,args,{exit=0,extra=[],token=env.ROCKET_TEST_TOKEN}={}){
 const log=path.join(root,'logs-'+logs.length);logs.push(log);
 const command=['--repo',repo,'--json','--log-dir',log,'hosting',...args,'--provider',provider,'--token-env','ROCKET_TEST_TOKEN','--fixture-api-port',String(port),...extra];
 if(provider==='gitea')command.push('--server','https://gitea.example.invalid');
 const result=await new Promise((resolve,reject)=>{const child=cp.spawn(exe,command,{env:{...env,ROCKET_TEST_TOKEN:token},windowsHide:true,stdio:['ignore','pipe','pipe']});let out='',err='';const timer=setTimeout(()=>{child.kill();reject(new Error('CLI timeout'));},30000);child.stdout.on('data',b=>out+=b);child.stderr.on('data',b=>err+=b);child.on('error',reject);child.on('close',code=>{clearTimeout(timer);resolve({code,out,err});});});
 assert.equal(result.code,exit,result.out+'\n'+result.err);assert.equal(result.err,exit===130?'Stopping; waiting for Git cleanup and final observations...\n':'');const value=JSON.parse(result.out);assert.equal(value.schema_version,1);assert(!(result.out+result.err).includes(token));assert(!result.out.includes('PRIVATE rejection'));assert(!fixture.state.error,fixture.state.error);return value;
}
const scoped=(...args)=>[...args,'--repository','team/sample'];
async function main(){try{
 for(const provider of ['gitlab','gitea']){
  fixture=create(provider,env.ROCKET_TEST_TOKEN);const port=await fixture.listen(),s=fixture.state;
  const exec=(args,opts)=>run(provider,port,args,opts),writes=()=>s.requests.filter(r=>r.method!=='GET').length;
  assert.equal((await exec(['account'])).data.login,'fixture');
  s.mode='pages';let listed=await exec(scoped('list','--pages','2'));assert.equal(listed.data.items.length,2);assert(!listed.data.limited);
  listed=await exec(scoped('list'));assert(listed.data.limited);s.mode='ok';assert.equal((await exec(scoped('list','--kind','issues'))).data.items.length,1);
  pass(provider+': verified account, PR/issue lists and bounded pagination');
  let d=(await exec(scoped('pull','42'))).data;assert(d.capabilities.inline);assert(!d.capabilities.inline_ranges);assert.equal(d.files.items[0].previous,fixture.old);assert.equal(d.files.items[0].lines[1].new,20);assert.equal(d.files.items[1].patch,null);
  const comment=(side,line)=>({path:fixture.name,start:line,line,side,...d.inline_context,body:'PRIVATE inline 界'}),file=path.join(root,provider+'-comments.json');
  const reviewArgs=scoped('review','42','--action','comment','--comments',file);
  for(const [side,line]of [['RIGHT',21],['LEFT',11],['RIGHT',20]]){
   fs.writeFileSync(file,JSON.stringify([comment(side,line)]));const n=writes();const p=(await exec([...reviewArgs,'--preview'])).data.preview;assert.equal(writes(),n);assert.equal(p.review_sha256.length,64);
   const result=await exec([...reviewArgs,'--expect-review',p.review_sha256]);assert.equal(result.data.remote_outcome,'confirmed');assert.equal(writes(),n+1);
   const sent=JSON.parse(s.requests.filter(r=>r.method==='POST').at(-1).body);
   if(provider==='gitlab'){assert.equal(sent.position.old_line,side==='LEFT'?11:line===20?10:undefined);assert.equal(sent.position.new_line,side==='RIGHT'?line:undefined);}else{assert.equal(sent.comments[0].old_position,side==='LEFT'?line:0);assert.equal(sent.comments[0].new_position,side==='RIGHT'?line:0);}
  }
  d=(await exec(scoped('pull','42'))).data;assert.equal(d.inline_comments.items.length,3);assert.deepEqual(d.inline_comments.items.map(n=>n.line),[21,11,20]);
  pass(provider+': Unicode renamed paths and added/deleted/context lines round-trip via reviewed single writes');
  const p=(await exec([...reviewArgs,'--preview'])).data.preview,n=writes();s.head='d'.repeat(40);
  let stale=await exec([...reviewArgs,'--expect-review',p.review_sha256],{exit:1});assert(['hosting_pull_review_stale','workflow_review_stale'].includes(stale.error.code));assert.equal(writes(),n);s.head='a'.repeat(40);
  const altered=[comment('RIGHT',21)];altered[0].body='PRIVATE edited body';fs.writeFileSync(file,JSON.stringify(altered));stale=await exec([...reviewArgs,'--expect-review',p.review_sha256],{exit:1});assert.equal(stale.error.code,'workflow_review_stale');assert.equal(stale.data.remote_outcome,'not_attempted');assert.equal(writes(),n);
  if(provider==='gitlab'){s.start='f'.repeat(40);stale=await exec([...reviewArgs,'--preview'],{exit:1});assert.equal(stale.error.code,'hosting_pull_review_stale');s.start='b'.repeat(40);}
  pass(provider+': changed revisions, diff version and edited review text refuse before writing');
  fs.writeFileSync(file,JSON.stringify([comment('RIGHT',21)]));
  if(provider==='gitea'){
   const count=writes();s.pending={id:900,user:{login:'fixture'},state:'PENDING'};
   assert.equal((await exec([...reviewArgs,'--preview'],{exit:1})).error.code,'hosting_pending_review');assert.equal(writes(),count);s.pending=null;
   s.mode='unbounded-reviews';assert.equal((await exec([...reviewArgs,'--preview'],{exit:1})).error.code,'hosting_review_state_unavailable');assert.equal(writes(),count);s.mode='ok';
   pass('gitea: existing pending reviews and incomplete review scans refuse before writing');
  }
  let preview=(await exec([...reviewArgs,'--preview'])).data.preview;s.mode='reject';let failure=await exec([...reviewArgs,'--expect-review',preview.review_sha256],{exit:1});assert.equal(failure.error.code,provider==='gitlab'?'hosting_pull_review_rejected':'hosting_pull_review_unknown');assert.equal(failure.data.remote_outcome,provider==='gitlab'?'not_confirmed':'unknown');s.mode='ok';
  if(provider==='gitea'){
   const count=writes();assert.equal((await exec([...reviewArgs,'--preview'],{exit:1})).error.code,'hosting_pending_review');assert.equal(writes(),count);
   const partial=(await exec(scoped('pull','42'))).data;assert(partial.reviews.items.some(r=>r.state==='PENDING'));assert(partial.inline_comments.items.some(n=>n.id===900));
   // Simulate the user discarding that pending review on the provider.
   s.pending=null;s.notes=s.notes.filter(n=>n.review_id!==900);
  }
  preview=(await exec([...reviewArgs,'--preview'])).data.preview;s.mode='wrong-anchor';failure=await exec([...reviewArgs,'--expect-review',preview.review_sha256],{exit:1});assert.equal(failure.error.code,'hosting_pull_review_unknown');assert.equal(failure.data.remote_outcome,'unknown');s.mode='ok';
  pass(provider+': rejected and unverifiable submissions have distinct JSON recovery outcomes');
  const createArgs=scoped('create','--source','feature/界','--target','main','--title','PRIVATE new PR','--body','PRIVATE description');
  preview=(await exec([...createArgs,'--preview'])).data.preview;const cn=writes();failure=await exec([...createArgs,'--expect-review','0'.repeat(64)],{exit:1});assert.equal(failure.error.code,'workflow_review_stale');assert.equal(writes(),cn);
  assert.equal((await exec([...createArgs,'--expect-review',preview.review_sha256])).data.remote_outcome,'confirmed');
  const mergeArgs=scoped('merge','42','--method','squash');preview=(await exec([...mergeArgs,'--preview'])).data.preview;
  s.base='f'.repeat(40);failure=await exec([...mergeArgs,'--expect-review',preview.review_sha256],{exit:1});assert.equal(failure.error.code,'workflow_review_stale');s.base='b'.repeat(40);
  assert.equal((await exec([...mergeArgs,'--expect-review',preview.review_sha256])).data.remote_outcome,'confirmed');s.merged=false;
  pass(provider+': create and merge require reviewed pins; remote target races refuse');
  for(const[mode,code]of [['401','hosting_unauthorized'],['429','hosting_rate_limited'],['redirect','hosting_invalid_response']]){s.mode=mode;failure=await exec(scoped('pull','42'),{exit:1});assert.equal(failure.error.code,code);}s.mode='ok';
  s.mode='slow-read';const started=performance.now();failure=await exec(scoped('pull','42'),{exit:130,extra:['--cancel-after-ms','1800']});assert.equal(failure.error.code,'cancelled');assert(performance.now()-started<6000);s.mode='ok';
  const count=s.requests.length;failure=await exec(['account'],{exit:1,token:'some-real-looking-credential'});assert.equal(s.requests.length,count);assert.equal(failure.error.code,'invalid_input');
  pass(provider+': auth, rate limits, redirect refusal, read cancellation and fixture credential isolation');
  assert.deepEqual(local(),before);fixture.close();fixture=null;
 }
 report.passed=true;
 }catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;}finally{
  fixture?.close();const events=[];
  try{for(const dir of logs)if(fs.existsSync(dir))for(const file of fs.readdirSync(dir).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n)))events.push(...fs.readFileSync(path.join(dir,file),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse));
   assert(events.length);assert(events.every(e=>e.dropped_before===0));for(const secret of [env.ROCKET_TEST_TOKEN,'PRIVATE','資料/新','feature/界'])assert(!JSON.stringify(events).includes(secret),'log leaked payload');
   for(const start of events.filter(e=>e.event==='request_start'))assert(events.some(e=>e.event==='request_end'&&e.session===start.session&&e.request===start.request));assert.deepEqual(local(),before);report.log_events=events.length;
  }catch(e){report.log_error=e.stack;report.passed=false;process.exitCode=1;}
  fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log('Evidence: '+root);
 }}main();
