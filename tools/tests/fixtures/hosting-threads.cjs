// Additional discussion contracts on the existing loopback provider fixtures.
// The synthetic service rejects unexpected scopes/bodies and never forwards URLs.
const assert=require('node:assert/strict');
exports.create=(provider,token)=>{
 const azure=provider.startsWith('azure'),server=provider==='bitbucket-server',lab=provider==='gitlab',gitea=provider==='gitea';
 const factory=require(lab||gitea?'./hosting-inline.cjs':'./hosting-other-inline.cjs');
 const t={resolved:false,body:'PRIVATE original 資料 🚀',version:7,replies:[],writes:[],mode:'ok',empty:false};
 const name=(azure?'/':'')+'資料/新 é 🚀.rs';
 const own=()=>lab?{username:'fixture'}:gitea?{login:'fixture'}:azure?{id:'11111111-1111-4111-8111-111111111111',displayName:'fixture'}:server?{name:'fixture',displayName:'fixture'}:{uuid:'{fixture}',display_name:'fixture'};
 const author=()=>lab?{username:'contributor'}:gitea?{login:'contributor'}:azure?{id:'22222222-2222-4222-8222-222222222222',displayName:'contributor'}:server?{name:'contributor',displayName:'contributor'}:{uuid:'{contributor}',display_name:'contributor'};
 const note=(id,text,mine=false)=>({id,body:text,text,content:azure?text:{raw:text},author:mine?own():author(),user:mine?own():author(),created_at:'2026-11-10T00:00:00Z',created_on:'2026-11-10T00:00:00Z',publishedDate:'2026-11-10T00:00:00Z',commentType:'text',isDeleted:false});
 const root=()=>{
  const n=note(8,t.body);return lab?{...n,resolvable:true,resolved:t.resolved,system:false,position:{new_path:name,new_line:2}}:
   gitea?{...n,pull_request_review_id:90,position:2,original_position:2,path:name,resolver:t.resolved?own():null,commit_id:'a'.repeat(40),original_commit_id:'a'.repeat(40)}:
   server?{...n,version:t.version,threadResolved:t.resolved,comments:t.replies.map(r=>({...note(r.id,r.body,true),parent:{id:8},version:0,comments:[]})),anchor:{path:{components:name.split('/')},line:2},pending:false}:
   azure?{...n,parentCommentId:0}:{...n,deleted:false,inline:{path:name,to:2},parent:null,resolution:t.resolved?{type:'comment_resolution',user:own()}:null,pending:false};
 };
 const collection=()=>{
  if(t.empty)return [];
  if(lab)return[{id:'discussion-fixture',individual_note:false,notes:[root(),...t.replies.map(r=>({...note(r.id,r.body,true),resolvable:false,system:false}))]}];
  if(gitea)return [root()];
  if(azure)return [{id:7,status:t.resolved?'fixed':'active',isDeleted:false,lastUpdatedDate:String(t.version),threadContext:{filePath:name,rightFileStart:{line:2},rightFileEnd:{line:2}},comments:[root(),...t.replies.map(r=>({...note(r.id,r.body,true),parentCommentId:8}))]}];
  return [root(),...t.replies.map(r=>({...note(r.id,r.body,true),parent:{id:8},deleted:false}))];
 };
 const fixture=factory.create(provider,token,({state,route,pr,req,data,u,send,slow})=>{
  const respond=(code,value,headers)=>{send(code,value,headers);return true;};
  const reads=lab?route===pr+'/discussions':gitea?(route===pr+'/reviews'||route===pr+'/reviews/90/comments'):server?(route===pr+'/activities'||route===pr+'/comments/8'):azure?route===pr+'/threads':route===pr+'/comments';
  if(req.method==='GET'&&reads){
   if(t.mode==='slow-read'){slow();return true;}
   if(t.mode==='auth')return respond(401,{message:token+' PRIVATE auth'});
   let values=collection();
   if(t.mode==='duplicate'&&!server)values.push(structuredClone(values[0]));
   if(lab)return respond(200,values,t.mode==='partial'?{'X-Next-Page':'2'}:{});
   if(gitea)return respond(200,route.endsWith('/reviews')?(t.empty?[]:[{id:90,user:author(),body:'PRIVATE review',state:'COMMENT',commit_id:state.head,comments_count:1,submitted_at:'2026-11-10'}]):values);
   if(server)return respond(200,route.endsWith('/activities')?{values:t.empty?[]:[{comment:root(),commentAnchor:{path:name,line:2}}],isLastPage:t.mode!=='partial',start:0}:root());
   if(azure)return respond(200,{value:values});
   return respond(200,{values,...(t.mode==='partial'?{next:'https://never-follow.example.invalid/PRIVATE'}:{})});
  }
  const reply=lab?route===pr+'/discussions/discussion-fixture/notes':azure?route===pr+'/threads/7/comments':!gitea&&route===pr+'/comments';
  const change=lab?route===pr+'/discussions/discussion-fixture':gitea?/^\/pulls\/comments\/8\/(unresolve|resolve)$/.test(route):server?route===pr+'/comments/8':azure?route===pr+'/threads/7':route===pr+'/comments/8/resolve';
  if(req.method==='GET'||(!reply&&!change))return false;
  if(azure)assert(['6.0','7.0','7.1'].includes(u.searchParams.get('api-version')));
  t.writes.push({method:req.method,route,body:data});
  if(t.mode==='forbidden')return respond(403,{message:token+' PRIVATE permissions'});
  if(t.mode==='reject')return respond(409,{message:token+' PRIVATE changed'});
  const b=data?JSON.parse(data):{};let result;
  if(reply){
   assert.equal(req.method,'POST');const text=lab?b.body:azure?b.content:server?b.text:b.content.raw;
   if(!lab)assert.equal(azure?b.parentCommentId:b.parent.id,8);if(azure)assert.equal(b.commentType,1);
   const n=t.mode==='reused-reply'?t.replies.find(r=>r.body===text):{id:90+t.replies.length,body:text};assert(n,'Existing synthetic reply');if(t.mode!=='reused-reply')t.replies.push(n);result={...note(n.id,text,true),...(azure?{parentCommentId:8}:lab?{}:{parent:{id:8}})};
  }else{
   if(lab){assert.equal(req.method,'PUT');assert.deepEqual(Object.keys(b),['resolved']);t.resolved=b.resolved;}
   else if(server){assert.equal(req.method,'PUT');assert.deepEqual(Object.keys(b).sort(),['threadResolved','version']);assert.equal(b.version,t.version);t.resolved=b.threadResolved;}
   else if(azure){assert.equal(req.method,'PATCH');assert(['active','fixed'].includes(b.status));assert.deepEqual(Object.keys(b),['status']);t.resolved=b.status==='fixed';}
   else if(gitea){assert.equal(req.method,'POST');t.resolved=route.endsWith('/resolve');}
   else {assert(['POST','DELETE'].includes(req.method));t.resolved=req.method==='POST';}
   t.version++;result=lab?collection()[0]:server?root():azure?collection()[0]:{type:'comment_resolution',user:own(),created_on:'2026-11-10'};
  }
  if(t.mode==='slow-write'){slow();return true;}
  if(t.mode==='bad-response')return respond(200,{});
  if(t.mode==='readback-failed'){t.mode='auth';return respond(reply?201:200,result);}
  if(change&&(gitea||(!server&&!lab&&!azure&&!t.resolved)))return respond(204,undefined);
  return respond(reply?201:200,result);
 });
 return Object.assign(fixture,{threads:t,threadPath:name});
};
