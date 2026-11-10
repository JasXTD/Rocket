// Inert loopback contracts for CLI/native hosting tests. Never contacts a host.
const http=require('node:http'),assert=require('node:assert/strict');
exports.create=(provider,token,threadHandler)=>{
 const lab=provider==='gitlab',prefix=lab?'/gitlab/projects/team%2Fsample':'/gitea/repos/team/sample';
 const state={mode:'ok',head:'a'.repeat(40),base:'b'.repeat(40),start:'b'.repeat(40),diffBase:'c'.repeat(40),merged:false,requests:[],reviews:[],notes:[],pending:null,closedAt:null,error:null};
 const name='資料/新 é 🚀.rs',old='資料/旧 é 🚀.rs',patch='@@ -10,2 +20,2 @@\n context\n-old\n+new 界\n';
 const summary=()=>lab?{iid:42,title:'PRIVATE rename 界',description:'PRIVATE detail',author:{username:'contributor'},state:state.merged?'merged':'opened',draft:false,sha:state.head,source_branch:'feature/界',target_branch:'main',detailed_merge_status:'mergeable',diff_refs:state.mode==='no-refs'?null:{base_sha:state.diffBase,head_sha:state.head,start_sha:state.start},merge_commit_sha:'e'.repeat(40)}:
 {number:42,title:'PRIVATE rename 界',body:'PRIVATE detail',user:{login:'contributor'},state:state.merged?'closed':'open',merged:state.merged,draft:false,mergeable:true,head:{sha:state.head,ref:'feature/界'},base:{sha:state.base,ref:'main'},merge_commit_sha:'e'.repeat(40),commits:1,changed_files:2,additions:1,deletions:1};
 const server=http.createServer(async(req,res)=>{try{
  let data='';for await(const b of req){data+=b;assert(data.length<200*1024);}
  const u=new URL(req.url,'http://127.0.0.1'),p=u.pathname;
  state.requests.push({method:req.method,path:p,query:u.search,body:data});
  const send=(code,value,headers={})=>{res.writeHead(code,{'Content-Type':'application/json',...headers});res.end(JSON.stringify(value));};
  const slow=()=>{res.on('close',()=>{state.closedAt=performance.now();});};
  assert.equal(req.headers.authorization,(lab?'Bearer ':'token ')+token);
  if(state.mode==='401')return send(401,{message:token+' PRIVATE rejection'});
  if(p===`/${provider}/user`)return send(200,lab?{username:'fixture'}:{login:'fixture'});
  assert(p.startsWith(prefix),'unexpected scope '+p);
  const route=p.slice(prefix.length),noun=lab?'/merge_requests':'/pulls',pr=noun+'/42';
  if(state.mode==='429')return send(429,{message:token});
  if(state.mode==='redirect')return send(302,{}, {Location:'http://127.0.0.1:1/never-follow'});
  if(state.mode==='slow-read'&&req.method==='GET')return slow();
  if(threadHandler?.({provider,state,route,pr,req,data,u,send,slow}))return;
  if(route.includes('/branches/'))return send(200,{commit:{id:route.endsWith('/main')?state.base:state.head}});
  if(route===''&&!lab)return send(200,{full_name:'team/sample',archived:false,allow_merge_commits:true,allow_squash_merge:true,allow_rebase:true,permissions:{push:true}});
  if(req.method==='GET'&&(route===noun||route==='/issues')){
   if(state.mode==='empty')return send(200,[]);
   const headers=state.mode==='pages'&&u.searchParams.get('page')!=='2'?(lab?{'X-Next-Page':'2'}:{Link:`<http://127.0.0.1:${server.address().port}${p}?page=2&limit=50>; rel="next"`}):{};
   return send(200,[{...summary(),number:u.searchParams.get('page')==='2'?43:42,iid:u.searchParams.get('page')==='2'?43:42}],headers);
  }
  if(req.method==='POST'&&route===noun){
   const b=JSON.parse(data);assert.equal(lab?b.source_branch:b.head,'feature/界');assert.equal(lab?b.target_branch:b.base,'main');
   if(state.mode==='reject')return send(422,{message:token});if(state.mode==='slow-create')return slow();
   return send(201,{...summary(),title:b.title});
  }
  if(route===pr+'/merge'&&req.method!=='GET'){
   const b=JSON.parse(data);assert.equal(lab?b.sha:b.head_commit_id,state.head);
   if(state.mode==='reject')return send(409,{message:token});
   state.merged=true;if(state.mode==='slow-merge')return slow();
   if(lab)return send(200,summary());res.writeHead(200);return res.end();
  }
  if(req.method==='POST'){
   const b=JSON.parse(data);if(state.mode==='reject'){
    if(!lab){state.pending={id:900,user:{login:'fixture'},body:'PRIVATE pending',state:'PENDING',commit_id:state.head};const c=b.comments[0];state.notes.push({id:900,user:{login:'fixture'},body:c.body,path:c.path,position:c.new_position,original_position:c.old_position,review_id:900});}
    return send(422,{message:token+' PRIVATE rejection'});
   }
   let value;
   if(lab&&route===pr+'/discussions'){
    assert.equal(b.position.head_sha,state.head);assert.equal(b.position.base_sha,state.diffBase);assert.equal(b.position.start_sha,state.start);
    assert.equal(b.position.old_path,old);assert.equal(b.position.new_path,name);assert.equal(b.position.position_type,'text');
    const n={id:100+state.notes.length,author:{username:'fixture'},body:b.body,position:b.position,created_at:'2026-11-10T00:00:00Z'};
    state.notes.push(n);value={id:'discussion-'+n.id,notes:[n]};
    if(state.mode==='wrong-anchor')value={...value,notes:[{...n,position:{...b.position,new_line:999}}]};
   }else if(lab&&route===pr+'/notes'){
    value={id:250,author:{username:'fixture'},body:b.body,created_at:'2026-11-10T00:00:00Z'};
   }else if(!lab&&route===pr+'/reviews'){
    assert.equal(b.commit_id,state.head);
    for(const c of b.comments||[]){assert.equal(c.path,name);assert(Number.isInteger(c.old_position));assert(Number.isInteger(c.new_position));assert((c.old_position===0)!==(c.new_position===0));}
    value={id:100+state.reviews.length,user:{login:'fixture'},body:b.body,state:b.event,commit_id:b.commit_id,comments_count:(b.comments||[]).length,submitted_at:'2026-11-10T00:00:00Z'};
    assert.equal(state.pending,null,'Rocket must not consume an existing pending review');
    state.reviews.push(value);for(const c of b.comments||[])state.notes.push({id:200+state.notes.length,user:{login:'fixture'},body:c.body,path:c.path,position:c.new_position,original_position:c.old_position,review_id:value.id,created_at:'2026-11-10T00:00:00Z'});
    if(state.mode==='wrong-anchor')value={...value,comments_count:value.comments_count+1};
   }else {assert.fail('Unexpected write '+route);}
   if(state.mode==='slow-submit')return slow();if(state.mode==='bad-response')return send(201,{});
   return send(201,value);
  }
  if(route===pr)return send(200,summary());
  if(route===pr+'/commits')return send(200,[lab?{id:state.head,message:'PRIVATE commit',author_name:'作者'}:{sha:state.head,commit:{message:'PRIVATE commit',author:{name:'作者'}}}]);
  if(route===pr+'/diffs')return send(200,[{new_path:name,old_path:old,renamed_file:true,diff:state.mode==='truncated'?patch.slice(0,-7):patch},{new_path:'asset.png',old_path:'asset.png',diff:null}]);
  if(route===pr+'/files')return send(200,[{filename:name,previous_filename:old,status:'renamed',additions:1,deletions:1},{filename:'asset.png',status:'modified'}]);
  if(route===pr+'.diff'){
   const diff=`diff --git a/old b/new\nrename from ${old}\nrename to ${name}\n--- a/${old}\n+++ b/${name}\n${state.mode==='truncated'?patch.slice(0,-7):patch}diff --git a/asset.png b/asset.png\nBinary files a/asset.png and b/asset.png differ\n`;
   res.writeHead(200,{'Content-Type':'text/plain; charset=utf-8'});return res.end(diff);
  }
  if(route===pr+'/discussions')return send(200,state.notes.map(n=>({id:'discussion-'+n.id,notes:[n]})));
  if(route===pr+'/approvals')return send(200,{approved_by:[]});
  if(!lab&&/^\/commits\/[a-f0-9]{40}\/status$/.test(route))return send(200,{sha:state.head,state:'success',total_count:0,statuses:[]});
  if(!lab&&route==='/actions/runs')return send(200,{total_count:0,workflow_runs:[]});
  if(route===pr+'/reviews'){
   if(state.mode==='unbounded-reviews')return send(200,Array.from({length:50},(_,i)=>({id:i+1,user:{login:'other'},state:'COMMENT'})));
   return send(200,[...state.reviews,...(state.pending?[state.pending]:[])]);
  }
  const m=route.match(/^\/pulls\/42\/reviews\/(\d+)\/comments$/);if(m)return send(200,state.notes.filter(n=>n.review_id===Number(m[1])));
  if(route.endsWith('/notes')||route.endsWith('/comments')||route.endsWith('/pipelines')||route.endsWith('/statuses'))return send(200,[]);
  assert.fail('Unexpected route '+route);
 }catch(e){state.error=e.stack;res.writeHead(500);res.end('{}');}});
 return {server,state,name,old,patch,async listen(){await new Promise(r=>server.listen(0,'127.0.0.1',r));return server.address().port;},close(){server.closeAllConnections();server.close();}};
};
