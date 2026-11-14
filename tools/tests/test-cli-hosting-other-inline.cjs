// Focused executable-level hosting acceptance. Loopback, temporary repositories,
// synthetic credentials and isolated Git config only. No account settings touched.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {create}=require('./fixtures/hosting-other-inline.cjs');
const root=path.resolve(process.env.ROCKET_CLI_RESULTS||'work','cli-other-inline-'+crypto.randomUUID()),repo=path.join(root,'repo 資料'),exe=path.resolve(process.env.ROCKET_CLI_EXE||'target/release/rocket-cli'+(process.platform==='win32'?'.exe':''));
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
 if(fixture.server)command.push('--server',fixture.server);
 const result=await new Promise((resolve,reject)=>{const child=cp.spawn(exe,command,{env:{...env,ROCKET_TEST_TOKEN:token},windowsHide:true,stdio:['ignore','pipe','pipe']});let out='',err='';const timer=setTimeout(()=>{child.kill();reject(new Error('CLI timeout'));},30000);child.stdout.on('data',b=>out+=b);child.stderr.on('data',b=>err+=b);child.on('error',reject);child.on('close',code=>{clearTimeout(timer);resolve({code,out,err});});});
 assert.equal(result.code,exit,result.out+'\n'+result.err);assert.equal(result.err,exit===130?'Stopping; waiting for Git cleanup and final observations...\n':'');const value=JSON.parse(result.out);assert.equal(value.schema_version,1);assert(!(result.out+result.err).includes(token));assert(!result.out.includes('PRIVATE rejection'));assert(!fixture.state.error,fixture.state.error);return value;
}
const scoped=(...args)=>[...args,'--repository','team/sample'];
async function main(){try{
 const boundsOnly=process.argv.includes('--bounds-only'),providers=process.argv.slice(2).filter(p=>p!=='--bounds-only');for(const p of providers)assert(['bitbucket','bitbucket-server','azure','azure-server'].includes(p));
 for(const provider of providers.length?providers:['bitbucket','bitbucket-server','azure','azure-server']){
  fixture=create(provider,env.ROCKET_TEST_TOKEN);const port=await fixture.listen(),s=fixture.state,exec=(args,opts)=>run(provider,port,args,opts),writes=()=>s.requests.filter(r=>r.method!=='GET').length;
  assert.equal((await exec(['account'])).data.login,'fixture');
  assert.equal((await exec(scoped('list'))).data.items.length,1);
  let d=(await exec(scoped('pull','42'))).data;assert(d.capabilities.inline);assert.equal(d.capabilities.inline_limit,1);assert(!d.capabilities.inline_ranges);assert.equal(d.files.items[0].previous,fixture.old);assert.equal(d.files.items[0].lines[3].new,2);assert.equal(d.files.items[1].patch,null);
  const comment=(side,line)=>({path:fixture.name,start:line,line,side,...d.inline_context,body:'PRIVATE inline 界'}),file=path.join(root,provider+'-comments.json'),args=scoped('review','42','--action','comment','--comments',file);
  if(boundsOnly){
   assert(provider.startsWith('azure'));s.mode='row-limit';const bounded=(await exec(scoped('pull','42'))).data;assert.equal(bounded.files.items.length,2);assert.equal(bounded.files.items[0].lines.length,0);assert.equal(bounded.files.items[1].path,'/asset.png');assert(bounded.files.limited);s.mode='ok';
   fs.writeFileSync(file,JSON.stringify([comment('RIGHT',2)]));const pv=(await exec([...args,'--preview'])).data.preview,n=writes();s.mode='opposite-side';const response=await exec([...args,'--expect-review',pv.review_sha256],{exit:1});assert.equal(response.error.code,'hosting_pull_review_unknown');assert.equal(writes(),n+1);assert.deepEqual(local(),before);
   pass(provider+': row-limit fallback preserves all file metadata; contradictory side in acknowledgement is unknown');fixture.close();fixture=null;continue;
  }
  for(const [side,line]of [['RIGHT',2],['LEFT',2],['RIGHT',1]]){
   fs.writeFileSync(file,JSON.stringify([comment(side,line)]));const n=writes(),pv=(await exec([...args,'--preview'])).data.preview;assert.equal(writes(),n);
   assert.equal((await exec([...args,'--expect-review',pv.review_sha256])).data.remote_outcome,'confirmed');assert.equal(writes(),n+1);
   const sent=JSON.parse(s.requests.filter(r=>r.method==='POST').at(-1).body);
   if(provider==='bitbucket')assert.equal(sent.inline[side==='LEFT'?'from':'to'],line);
   else if(provider==='bitbucket-server'){assert.equal(sent.anchor.lineType,line===1?'CONTEXT':side==='LEFT'?'REMOVED':'ADDED');assert.equal(sent.anchor.fileType,side==='LEFT'?'FROM':'TO');}
   else assert.deepEqual(sent.threadContext[side==='LEFT'?'leftFileStart':'rightFileStart'],{line,offset:1});
  }
  d=(await exec(scoped('pull','42'))).data;assert.equal(d.inline_comments.items.length,3);assert.deepEqual(d.inline_comments.items.map(n=>n.line),[2,2,1]);
  pass(provider+': account/list/Unicode rename diff; added, deleted and context lines publish once and read back');
  const original=comment('RIGHT',2);fs.writeFileSync(file,JSON.stringify([original]));let pv=(await exec([...args,'--preview'])).data.preview,n=writes();s.head='d'.repeat(40);
  let r=await exec([...args,'--expect-review',pv.review_sha256],{exit:1});assert(['hosting_pull_review_stale','workflow_review_stale'].includes(r.error.code));assert.equal(writes(),n);s.head='a'.repeat(40);
  if(provider==='bitbucket-server'||provider.startsWith('azure')){
   const field=provider==='bitbucket-server'?'version':'iteration';s[field]++;r=await exec([...args,'--preview'],{exit:1});assert.equal(r.error.code,'hosting_pull_review_stale');assert.equal(writes(),n);s[field]--;
  }
  fs.writeFileSync(file,JSON.stringify([{...original,body:'PRIVATE changed'}]));r=await exec([...args,'--expect-review',pv.review_sha256],{exit:1});assert.equal(r.error.code,'workflow_review_stale');assert.equal(writes(),n);
  fs.writeFileSync(file,JSON.stringify([original,original]));assert.equal((await exec([...args,'--preview'],{exit:1})).error.code,'invalid_input');assert.equal(writes(),n);
  fs.writeFileSync(file,JSON.stringify([original]));assert.equal((await exec([...args,'--body','extra','--preview'],{exit:1})).error.code,'invalid_input');assert.equal(writes(),n);
  pass(provider+': head/version/iteration/text changes and multi-write requests fail before submission');
  for(const [mode,code]of [['reject','hosting_pull_review_rejected'],['wrong-anchor','hosting_pull_review_unknown'],['bad-response','hosting_pull_review_unknown'],['racing-submit','hosting_pull_review_unknown']]){
   pv=(await exec([...args,'--preview'])).data.preview;s.mode=mode;n=writes();r=await exec([...args,'--expect-review',pv.review_sha256],{exit:1});assert.equal(r.error.code,code);assert.equal(writes(),n+1);assert.equal(r.data.remote_outcome,mode==='reject'?'not_confirmed':'unknown');s.mode='ok';s.head='a'.repeat(40);
  }
  if(provider==='bitbucket-server'){pv=(await exec([...args,'--preview'])).data.preview;s.mode='no-anchor';assert.equal((await exec([...args,'--expect-review',pv.review_sha256])).data.remote_outcome,'confirmed');s.mode='ok';}
  pass(provider+': rejection, bad acknowledgements, racing writes and legacy anchor verification have explicit outcomes');
  s.mode='truncated';d=(await exec(scoped('pull','42'))).data;assert.equal(d.files.items[0].lines.length,0);n=writes();assert.equal((await exec([...args,'--preview'],{exit:1})).error.code,'invalid_input');assert.equal(writes(),n);s.mode='ok';
  if(provider.startsWith('azure')){
   s.mode='partial';assert((await exec(scoped('pull','42'))).data.files.limited);
   s.mode='row-limit';const bounded=(await exec(scoped('pull','42'))).data;assert.equal(bounded.files.items.length,2);assert.equal(bounded.files.items[0].lines.length,0);assert.equal(bounded.files.items[1].path,'/asset.png');assert(bounded.files.limited);s.mode='ok';
  }
  if(provider==='azure-server'){
   for(const version of ['6.0','7.0','7.1']){
    s.apiVersion=version;const start=s.requests.length;
    assert((await exec(scoped('pull','42'),{extra:['--api-version',version]})).data.capabilities.inline);
    const statuses=s.requests.slice(start).filter(r=>r.path.endsWith('/pullrequests/42/statuses'));
    assert.equal(statuses.length,1);assert.equal(new URLSearchParams(statuses[0].query).get('api-version'),version==='6.0'?'6.0-preview.1':version);
   }
   s.apiVersion='7.0';
  }
  pass(provider+': binary/incomplete diffs stay read-only; pagination and configured server API versions are preserved');
  s.mode='401';assert.equal((await exec(scoped('pull','42'),{exit:1})).error.code,'hosting_unauthorized');s.mode='slow-read';const at=performance.now();r=await exec(scoped('pull','42'),{exit:130,extra:['--cancel-after-ms','1800']});assert.equal(r.error.code,'cancelled');assert(performance.now()-at<6000);s.mode='ok';
  pass(provider+': authentication errors and cancellation keep JSON/logs private and local state unchanged');
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
