// Official Bitbucket/Azure contracts, served only on loopback. No returned URL
// is usable: tests fail if Rocket follows links instead of constructing scope.
const http=require('node:http'),assert=require('node:assert/strict');
function create(provider,token,threadHandler){
 const azure=provider.startsWith('azure'),server=provider==='bitbucket-server';
 const base='/'+provider+(provider==='azure-server'?'/tfs/Fixture':'');
 const prefix=base+(azure?'/team/_apis/git/repositories/sample':server?'/projects/team/repos/sample':'/repositories/team/sample');
 const pr=(server?'/pull-requests':'/pullrequests')+'/42';
 const name=(azure?'/':'')+'資料/新.rs',old=(azure?'/':'')+'資料/旧.rs';
 const actor=azure?{id:'11111111-1111-4111-8111-111111111111',displayName:'fixture',providerDisplayName:'fixture',isActive:true}:server?{name:'fixture',displayName:'fixture'}:{uuid:'{fixture}',display_name:'fixture'};
 const state={mode:'ok',head:'a'.repeat(40),base:'b'.repeat(40),common:'c'.repeat(40),iteration:3,version:7,apiVersion:provider==='azure-server'?'7.0':'7.1',tracking:9,requests:[],notes:[],closedAt:null,error:null};
 const summary=()=>azure?{pullRequestId:42,title:'PRIVATE PR 界',description:'PRIVATE details',status:'active',createdBy:{...actor,id:'22222222-2222-4222-8222-222222222222'},sourceRefName:'refs/heads/feature/界',targetRefName:'refs/heads/main',lastMergeSourceCommit:{commitId:state.head},lastMergeTargetCommit:{commitId:state.base},reviewers:[]}:
 server?{id:42,version:state.version,title:'PRIVATE PR 界',description:'PRIVATE details',state:'OPEN',author:{user:{name:'author',displayName:'作者'}},fromRef:{id:'refs/heads/feature/界',latestCommit:state.head},toRef:{id:'refs/heads/main',latestCommit:state.base},reviewers:[]}:
 {id:42,title:'PRIVATE PR 界',description:'PRIVATE details',state:'OPEN',author:{uuid:'{author}',display_name:'作者'},source:{branch:{name:'feature/界'},commit:{hash:state.head}},destination:{branch:{name:'main'},commit:{hash:state.base}},participants:[]};
 const page=values=>azure?{value:values}:{values,start:0,size:values.length,isLastPage:true};
 const service=http.createServer(async(req,res)=>{try{
  let data='';for await(const b of req){data+=b;assert(data.length<128*1024);}
  const u=new URL(req.url,'http://127.0.0.1'),p=u.pathname;
  state.requests.push({method:req.method,path:p,query:u.search,body:data});
  const send=(code,value,headers={})=>{res.writeHead(code,{'Content-Type':'application/json',...headers});res.end(JSON.stringify(value));};
  const slow=()=>res.on('close',()=>{state.closedAt=performance.now();});
  assert.equal(req.headers.authorization,azure?'Basic '+Buffer.from(':'+token).toString('base64'):'Bearer '+token);
  if(state.mode==='401')return send(401,{message:token+' PRIVATE rejection'});
  if(p===base+'/user')return send(200,actor);
  if(p===base+'/application-properties')return send(200,{version:'9.4.0'},{'X-AUSERNAME':'fixture'});
  if(p===base+'/_apis/connectionData')return send(200,{authenticatedUser:actor,authorizedUser:actor});
  if(azure){
   // Server 2020's PR statuses contract is 6.0-preview.1; other reads use
   // the configured stable version. Require the exact endpoint contract.
   const expected=state.apiVersion==='6.0'&&p===prefix+pr+'/statuses'?'6.0-preview.1':state.apiVersion;
   assert.equal(u.searchParams.get('api-version'),expected,'incorrect configured API version for '+p);
  }
  if(p===base+'/team/_apis/build/builds')return send(200,{value:[]});
  assert(p.startsWith(prefix),'Unexpected scope '+p);const route=p.slice(prefix.length);
  if(state.mode==='slow-read'&&req.method==='GET')return slow();
  if(threadHandler?.({provider,state,route,pr,req,data,u,send,slow,actor}))return;
  if(route===''&&azure)return send(200,{id:'33333333-3333-4333-8333-333333333333'});
  if(route==='/refs')return send(200,{value:[{name:'refs/heads/main',objectId:state.base}]});
  if(route==='/branches')return send(200,page([{id:'refs/heads/main',latestCommit:state.base}]));
  if(route==='/refs/branches/main')return send(200,{target:{hash:state.base}});
  if(route===pr.slice(0,-3))return send(200,page(state.mode==='empty'?[]:[summary()]));
  if(req.method==='POST'){
   assert.equal(route,pr+(azure?'/threads':'/comments'));const b=JSON.parse(data);
   if(state.mode==='reject')return send(422,{message:token+' PRIVATE rejection'});
   const id=100+state.notes.length;let value;
   if(azure){
    assert.equal(b.threadContext.filePath,name);assert.deepEqual(b.pullRequestThreadContext,{changeTrackingId:state.tracking,iterationContext:{firstComparingIteration:0,secondComparingIteration:state.iteration}});
    const side=b.threadContext.leftFileStart?'left':'right';assert.deepEqual(b.threadContext[side+'FileStart'],{line:b.threadContext[side+'FileEnd'].line,offset:1});
    value={id,comments:[{id,author:actor,content:b.comments[0].content,publishedDate:'2026-11-10T00:00:00Z'}],threadContext:b.threadContext,pullRequestThreadContext:b.pullRequestThreadContext,status:1};
   }else if(server){
    assert.equal(b.anchor.path,name);assert.equal(b.anchor.srcPath,old);assert.equal(b.anchor.fromHash,state.common);assert.equal(b.anchor.toHash,state.head);assert.equal(b.anchor.diffType,'EFFECTIVE');
    assert(['ADDED','REMOVED','CONTEXT'].includes(b.anchor.lineType));assert(['FROM','TO'].includes(b.anchor.fileType));value={id,author:actor,text:b.text,anchor:b.anchor};
   }else{
    assert.equal(b.inline.path,name);assert((b.inline.from===undefined)!==(b.inline.to===undefined));value={id,user:actor,content:b.content,inline:b.inline,pending:false};
   }
   state.notes.push(structuredClone(value));
   if(state.mode==='slow-submit')return slow();
   if(state.mode==='racing-submit')state.head='f'.repeat(40);
   if(state.mode==='wrong-anchor'){
    value=structuredClone(value);if(azure)value.pullRequestThreadContext.changeTrackingId=999;else if(server)value.anchor.toHash='f'.repeat(40);else value.inline.path='wrong.rs';
   }
   if(state.mode==='opposite-side'&&azure){value=structuredClone(value);value.threadContext.leftFileEnd={line:999,offset:1};}
   if(state.mode==='no-anchor'&&server)delete value.anchor;
   if(state.mode==='bad-response')return send(201,{});
   return send(201,value);
  }
  if(route===pr)return send(200,summary());
  if(route===pr+'/commits')return send(200,page([azure?{commitId:state.head,comment:'PRIVATE commit',author:{name:'作者'}}:server?{id:state.head,message:'PRIVATE commit',author:{name:'作者'}}:{hash:state.head,message:'PRIVATE commit',author:{raw:'作者'}}]));
  if(route.startsWith('/diffstat/')){assert.equal(route,`/diffstat/${state.head}..${state.base}`);assert.equal(u.searchParams.get('topic'),'true');return send(200,page([{new:{path:name},old:{path:old},status:'renamed',lines_added:1,lines_removed:1},{new:{path:'asset.png'},old:{path:'asset.png'},status:'modified'}]));}
  if(route.startsWith('/diff/')){
   assert.equal(route,`/diff/${state.head}..${state.base}`);assert.equal(u.searchParams.get('topic'),'true');assert.equal(req.headers.accept,'text/plain');
   const patch='@@ -1,2 +1,2 @@\n context\n-old\n+new\n';
   res.writeHead(200,{'Content-Type':'text/plain'});return res.end(`diff --git a/old b/new\n--- a/${old}\n+++ b/${name}\n${state.mode==='truncated'?patch.slice(0,-5):patch}`);
  }
  if(route===pr+'/changes')return send(200,page([{path:{toString:name},srcPath:{toString:old},type:'MOVE'},{path:{toString:'asset.png'},type:'MODIFY'}]));
  if(route===pr+'/diff')return send(200,{fromHash:state.common,toHash:state.head,truncated:state.mode==='truncated',diffs:[{source:{toString:old},destination:{toString:name},hunks:[{sourceLine:1,sourceSpan:2,destinationLine:1,destinationSpan:2,segments:[{type:'CONTEXT',lines:[{source:1,destination:1,line:'context'}]},{type:'REMOVED',lines:[{source:2,destination:2,line:'old'}]},{type:'ADDED',lines:[{source:3,destination:2,line:'new'}]}]}]}]});
  if(route===pr+'/iterations')return send(200,{value:[{id:state.iteration,commonRefCommit:{commitId:state.common},sourceRefCommit:{commitId:state.head},targetRefCommit:{commitId:state.base}}]});
  if(route===pr+`/iterations/${state.iteration}/changes`){assert.equal(u.searchParams.get('$compareTo'),'0');return send(200,{changeEntries:[{changeTrackingId:state.tracking,changeType:'rename, edit',item:{path:name,objectId:state.head},originalPath:old},{changeTrackingId:10,changeType:'edit',item:{path:'/asset.png'}}],nextSkip:state.mode==='partial'?50:0,nextTop:state.mode==='partial'?50:0});}
  if(route==='/items'){
   assert.equal(u.searchParams.get('versionDescriptor.versionType'),'commit');assert.equal(u.searchParams.get('resolveLfs'),'false');
   const file=u.searchParams.get('path'),commit=u.searchParams.get('versionDescriptor.version');assert([name,old,'/asset.png'].includes(file));assert([state.common,state.head].includes(commit));
   if(state.mode==='redirect')return send(302,{}, {Location:'http://127.0.0.1:1/never'});
   return send(200,{path:file,commitId:commit,gitObjectType:'blob',isFolder:false,contentMetadata:{isBinary:file==='/asset.png'},content:state.mode==='truncated'?'large\n'.repeat(1001):state.mode==='row-limit'?(commit===state.common?'a\n':'b\n').repeat(1000):commit===state.common?'context\nold\n':'context\nnew\n'});
  }
  if(route===pr+'/comments')return send(200,page(state.notes));
  if(route===pr+'/activities')return send(200,page(state.notes.map(n=>({comment:{id:n.id,text:n.text,author:n.author},commentAnchor:n.anchor}))));
  if(route===pr+'/threads')return send(200,{value:state.notes});
  if(route===pr+'/statuses'||/^\/commit[s]?\/[a-f0-9]{40}\/statuses$/.test(route))return send(200,page([]));
  assert.fail('Unexpected route '+req.method+' '+route);
 }catch(e){state.error=e.stack;res.writeHead(500);res.end('{}');}});
 return {state,name,old,server:provider==='azure-server'?'https://devops.example.invalid/tfs/Fixture':provider==='azure'?'https://dev.azure.com/fixture':provider==='bitbucket-server'?'https://bitbucket.example.invalid':null,
  listen:()=>new Promise(r=>service.listen(0,'127.0.0.1',()=>r(service.address().port))),close:()=>{service.closeAllConnections();service.close();}};
}
module.exports={create};
