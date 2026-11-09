// Focused GitLab search acceptance. The marked fixture permits only a loopback
// API, synthetic credentials and a disposable repository; no real hosting calls.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const http=require('node:http'),net=require('node:net'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
const root=path.resolve('work/mcp-fixtures/gitlab-search-'+crypto.randomUUID());
const repo=path.join(root,'repo 資料'),profile=path.join(root,'profile'),branch='rocket-ui-test/gitlab-search';
const exe=path.resolve(process.env.ROCKET_HOSTING_EXE||'target/release/rocket-mcp.exe');
const token='fixture-search-'+crypto.randomBytes(12).toString('hex'),phrase='界: "literal" &state=all';
const env={...process.env};for(const k of Object.keys(env))if(/^(GIT_|GCM_|SSH_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k))delete env[k];
fs.mkdirSync(repo,{recursive:true});fs.mkdirSync(path.join(root,'template'));fs.writeFileSync(path.join(root,'empty.config'),'');
Object.assign(env,{GIT_CONFIG_GLOBAL:path.join(root,'empty.config'),GIT_CONFIG_SYSTEM:path.join(root,'empty.config'),GIT_CONFIG_NOSYSTEM:'1',GIT_ALLOW_PROTOCOL:'file',GIT_TERMINAL_PROMPT:'0',GIT_TEMPLATE_DIR:path.join(root,'template'),http_proxy:'http://127.0.0.1:1',all_proxy:'http://127.0.0.1:1',NO_PROXY:''});
const git=args=>cp.execFileSync('git',['-C',repo,...args],{env,windowsHide:true,encoding:'utf8',stdio:'pipe'}).trimEnd();
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
git(['init','--initial-branch='+branch]);git(['config','user.name','Search Fixture']);git(['config','user.email','fixture@example.invalid']);git(['config','commit.gpgsign','false']);
fs.writeFileSync(path.join(repo,'資料.txt'),'committed\n');git(['add','--all']);git(['commit','-m','Fixture']);
fs.writeFileSync(path.join(repo,'資料.txt'),'local change\n');fs.writeFileSync(path.join(repo,'.git/rocket-ui-fixture'),branch);
const snapshot=()=>({head:git(['rev-parse','HEAD']),index:hash(path.join(repo,'.git/index')),config:hash(path.join(repo,'.git/config')),file:hash(path.join(repo,'資料.txt'))});
const before=snapshot(),requests=[],report={passed:false,root,cases:[]};
let client,service,state,mode='ok',closedAt=null,serverError=null;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const server=http.createServer((req,res)=>{try{
    const u=new URL(req.url,'http://127.0.0.1'),send=(status,body,headers={})=>{res.writeHead(status,{'Content-Type':'application/json',...headers});res.end(JSON.stringify(body));};
    assert.equal(req.method,'GET');assert.equal(req.headers.authorization,'Bearer '+token);
    requests.push({path:u.pathname,query:u.search});
    if(u.pathname==='/gitlab/user')return send(200,{username:'fixture'});
    assert(['/gitlab/projects/team%2F%E8%B3%87%E6%96%99/merge_requests','/gitlab/projects/team%2F%E8%B3%87%E6%96%99/issues'].includes(u.pathname),u.pathname);
    if(mode==='slow'){res.on('close',()=>closedAt=performance.now());return;}
    if(mode==='401')return send(401,{message:token});
    if(mode==='429')return send(429,{message:token});
    if(mode==='bad-link')return send(200,[],{'Link':'<http://127.0.0.1:1/never>; rel="next"'});
    if(mode==='bad-json')return send(200,{unexpected:token});
    const search=u.searchParams.get('search');
    if(!search)return send(200,[]);
    assert.equal(search,phrase);assert.equal(u.searchParams.get('scope'),'all');
    assert.equal(u.searchParams.get('author_username'),'author.name');assert.equal(u.searchParams.get('assignee_username[]'),'reviewer_one');
    assert.equal(u.searchParams.get('in'),'title,description');assert.equal(u.searchParams.get('per_page'),'50');
    if(mode==='empty')return send(200,[],{'x-next-page':''});
    const page=u.searchParams.get('page'),row={iid:page==='2'?51:1,title:'PRIVATE search result 界',state:'opened',author:{username:'author.name'},source_branch:'feature',target_branch:'main'};
    // A continuation tries to remove the filters. Rocket must validate its
    // destination and rebuild the original search for the second page.
    const next='http://'+req.headers.host+u.pathname+'?page=2&search=changed';
    send(200,[row],page==='1'?{'Link':'<'+next+'>; rel="next"'}:{'x-next-page':''});
}catch(e){serverError=e;res.writeHead(500);res.end('{}');}});
const control=id=>state.controls?.find(c=>c.id===id),pass=s=>{report.cases.push(s);console.log('PASS '+s);};
async function until(fn,label='state'){const started=performance.now();for(;;){if(serverError)throw serverError;state=await client.tool('rocket_state');if(fn(state))return;assert(performance.now()-started<60000,label+' timeout '+JSON.stringify({hosting:state.hosting,dialog:state.dialog?.kind}));await delay(65);}}
async function settle(){await until(s=>!s.busy&&!s.input_pending&&!s.hosting?.busy&&!(s.tabs||[]).some(t=>t.loading),'settled');}
async function stable(){let last='',count=0;await until(s=>{const now=JSON.stringify(s.controls);count=now===last?count+1:0;last=now;return count>=2;},'stable');}
async function scroll(target,direction,pixels=300){await client.tool('rocket_scroll',{target,direction,pixels,expected_tab:state.active_tab});await until(s=>!s.input_pending);await stable();}
async function reveal(id){await stable();const visible=()=>control(id)&&control(id).rect[3]-control(id).rect[1]>18;if(visible())return;
    const region=id.startsWith('repository-')?'repository-menu-scroll':'hosting-scroll';
    if(control(region)){await scroll(region,'up',2000);for(let i=0;i<30&&!visible();i++)await scroll(region,'down',220);}assert(visible(),'Missing '+id);}
async function click(id,wait=true){await reveal(id);assert(control(id).enabled,'Disabled '+id);await client.tool('rocket_click',{target:id,expected_tab:state.active_tab});if(wait)await settle();}
async function type(id,text){await reveal(id);await client.tool('rocket_type',{target:id,text,expected_tab:state.active_tab});await settle();}
async function resize(width,height){await client.tool('rocket_resize',{width,height});await until(s=>s.viewport[0]===width&&s.viewport[1]===height&&!s.input_pending);await settle();}
async function capture(name){await stable();const r=await client.request('tools/call',{name:'rocket_capture',arguments:{}});assert(!r.isError);fs.writeFileSync(path.join(root,name+'.png'),Buffer.from(r.content.find(c=>c.type==='image').data,'base64'));fs.writeFileSync(path.join(root,name+'.json'),JSON.stringify(state,null,2));}
async function connect(){await click('hosting-tab-accounts');await click('hosting-provider-gitlab');await type('hosting-token',token);if(state.hosting.remember)await click('hosting-remember');await click('hosting-connect');assert(state.hosting.connected,state.hosting.error);}
async function run(){try{
    await new Promise(r=>server.listen(0,'127.0.0.1',r));fs.writeFileSync(path.join(root,'rocket-hosting-api'),'http://127.0.0.1:'+server.address().port);
    report.exe_sha256=hash(exe);cp.execFileSync(exe,['--init-http',profile,root],{env,windowsHide:true});
    const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));
    fs.writeFileSync(path.join(profile,'settings'),'ROCKET-MCP-1\n'+port+'\n'+root+'\n');
    const log=fs.openSync(path.join(root,'service.log'),'w');service=cp.spawn(exe,['--http-profile',profile],{env,windowsHide:true,stdio:['ignore','ignore',log]});fs.closeSync(log);client=new RocketHttpClient(profile);
    for(let i=0;;i++){try{assert.equal((await client.raw('GET','/health')).status,200);break;}catch(e){if(i>=100)throw e;await delay(100);}}
    await client.initialize();await client.tool('rocket_open',{path:repo});await settle();await resize(1480,920);if(control('console-close'))await click('console-close');
    await click('menu-repository');await click('repository-remotes-menu');await click('repository-hosting');await until(s=>s.dialog?.kind==='hosting');await connect();
    await type('hosting-repository','team/資料');await click('hosting-tab-pulls');
    await type('hosting-search-author','author.name');await type('hosting-search-assignee','reviewer_one');await type('hosting-search-text',phrase);
    await click('hosting-search');assert.equal(state.hosting.count,1);assert(state.hosting.more);assert(!state.hosting.error);
    await capture('gitlab-search-wide');await click('hosting-more');assert.equal(state.hosting.count,2);assert(!state.hosting.more);
    pass('MR search and pagination retain literal Unicode text, author, assignee and project scope');
    await click('hosting-tab-issues');await click('hosting-search');assert.equal(state.hosting.count,1);assert(requests.at(-1).path.endsWith('/issues'));
    await resize(780,620);for(const id of ['hosting-search-text','hosting-search-author','hosting-search-assignee','hosting-search']){await reveal(id);const r=control(id).rect;assert(r[0]>=0&&r[2]<=780&&r[3]<=620,id+' clipped');}await capture('gitlab-search-compact');
    pass('Issue search and compact controls work with the same provider filters');
    const retained=state.hosting.items;
    for(const [failure,code] of [['429','hosting_rate_limited'],['bad-link','hosting_invalid_pagination'],['bad-json','hosting_invalid_response']]){mode=failure;await click('hosting-search');assert(state.hosting.error.includes(code),state.hosting.error);assert.deepEqual(state.hosting.items,retained);assert(!JSON.stringify(state).includes(token));}
    await capture('gitlab-search-error');pass('Rate limiting, invalid pagination and malformed data keep prior rows and sanitized errors');
    mode='slow';closedAt=null;const seen=requests.length;await click('hosting-search',false);await until(s=>s.hosting.busy&&requests.length>seen);await capture('gitlab-search-loading');
    const at=performance.now();await click('hosting-cancel',false);await until(s=>!s.input_pending&&(s.hosting.stopping||!s.hosting.busy));report.cancel_feedback_ms=Math.round(performance.now()-at);await settle();
    for(let i=0;i<100&&closedAt===null;i++)await delay(50);assert(closedAt!==null);report.cancel_disconnect_ms=Math.round(closedAt-at);assert.deepEqual(state.hosting.items,retained);assert(!state.hosting.uncertain);
    pass('Cancellation stops the request, retains earlier rows and does not report an uncertain write');
    mode='empty';await click('hosting-search');assert.equal(state.hosting.count,0);assert(!state.hosting.more);await capture('gitlab-search-empty');pass('Empty search has no stale rows or load-more action');
    mode='401';await click('hosting-search');assert(state.hosting.error.includes('hosting_unauthorized'));assert(!state.hosting.connected);assert(!JSON.stringify(state).includes(token));pass('Expired credentials disconnect safely without exposing token text');
    assert.deepEqual(snapshot(),before);report.passed=true;
}catch(e){report.error=e.stack;console.error(e.stack);process.exitCode=1;if(client&&state)try{await capture('failure');}catch{}}
finally{
    mode='ok';if(client&&state)try{if(state.hosting?.busy)await click('hosting-cancel');if(state.dialog)await click('dialog-cancel');await client.tool('rocket_close');await until(s=>['closed','not_started'].includes(s.window)||s.dialog?.kind==='quit');if(state.dialog?.kind==='quit'){await click('dialog-submit');await until(s=>['closed','not_started'].includes(s.window));}await client.raw('POST','/shutdown');}catch(e){report.cleanup_error=e.message;}
    if(client)await client.finish();if(service){for(let i=0;i<100&&service.exitCode===null;i++)await delay(100);if(service.exitCode===null){report.cleanup_error='Owned MCP host did not exit';cp.spawnSync('taskkill',['/PID',String(service.pid),'/T','/F'],{windowsHide:true});}}
    server.closeAllConnections();server.close();const access=path.join(profile,'access-token');if(fs.existsSync(access))fs.unlinkSync(access);
    try{const rows=[];for(const folder of fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')))for(const file of fs.readdirSync(path.join(root,folder)).filter(n=>/^rocket(?:\.\d+)?\.jsonl$/.test(n)))rows.push(...fs.readFileSync(path.join(root,folder,file),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse));
        assert(rows.length);assert(rows.every(r=>r.dropped_before===0));const text=JSON.stringify(rows);for(const secret of[token,phrase,'PRIVATE search result','author.name','reviewer_one'])assert(!text.includes(secret),'Diagnostic leak');
        assert(rows.some(r=>r.operation==='hosting-search'));for(const start of rows.filter(r=>r.event==='request_start'))assert(rows.some(r=>r.event==='request_end'&&r.session===start.session&&r.request===start.request),'Unpaired request');for(const code of ['hosting_unauthorized','hosting_rate_limited','hosting_invalid_pagination','hosting_invalid_response'])assert(rows.some(r=>r.failure_code===code),'Missing failure '+code);assert.deepEqual(snapshot(),before);report.logs=rows.length;
    }catch(e){report.log_error=e.message;report.passed=false;process.exitCode=1;}
    if(report.cleanup_error||service?.exitCode!==0){report.passed=false;process.exitCode=1;}
    fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log('Evidence: '+root);
}}
run().catch(e=>{console.error(e);process.exitCode=1;});
