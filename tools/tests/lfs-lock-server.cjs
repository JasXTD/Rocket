// Synthetic, memory-only LFS locking API. Binds numeric loopback; no forwarding.
const http=require('node:http'),crypto=require('node:crypto');
async function start(){
 const state={mode:'normal',locks:[],requests:[],writes:0,held:[],next:100,endpoint:''};
 const prefix='/rocket-lfs-locks-'+crypto.randomUUID().replaceAll('-','')+'.git';
 const server=http.createServer(async(req,res)=>{
  let raw='';for await(const b of req){raw+=b;if(raw.length>65536){res.writeHead(413);res.end();return}}
  const url=new URL(req.url,'http://127.0.0.1'),route=url.pathname.slice((prefix+'/info/lfs').length);let body={};try{body=raw?JSON.parse(raw):{}}catch{res.writeHead(400);res.end();return}
  const send=(code,data)=>{if(res.destroyed)return;res.writeHead(code,{'Content-Type':'application/vnd.git-lfs+json'});res.end(JSON.stringify(data))};
  if(!url.pathname.startsWith(prefix+'/info/lfs/'))return send(404,{message:'Unknown fixture route'});
  state.requests.push({method:req.method,route});
  if(state.mode==='unsupported')return send(501,{message:'LFS locking support is not implemented'});
  if(state.mode==='denied')return send(403,{message:'Permission denied for PRIVATE_LOCK_FIXTURE'});
  if(state.mode==='auth')return send(401,{message:'Authentication required PRIVATE_LOCK_FIXTURE'});
  if(state.mode==='malformed'){res.writeHead(200,{'Content-Type':'application/json'});res.end('{broken');return}
  if(state.mode==='pause-read'&&route==='/locks/verify'){state.held.push(()=>send(200,{ours:[],theirs:[]}));return}
  const publicLock=l=>({id:l.id,path:l.path,locked_at:l.locked_at,owner:{name:l.owner}});
  if(route==='/locks/verify'&&req.method==='POST'){
   let all=state.locks;if(body.limit>0)all=all.slice(0,body.limit);
   return send(200,{ours:all.filter(l=>l.ours).map(publicLock),theirs:all.filter(l=>!l.ours).map(publicLock),next_cursor:''});
  }
  if(route==='/locks'&&req.method==='GET')return send(200,{locks:state.locks.filter(l=>(!url.searchParams.has('id')||l.id===url.searchParams.get('id'))&&(!url.searchParams.has('path')||l.path===url.searchParams.get('path'))).map(publicLock),next_cursor:''});
  if(route==='/locks'&&req.method==='POST'){
   state.writes++;
   const exists=state.locks.find(l=>l.path===body.path);if(exists)return send(409,{message:'Lock already exists',lock:publicLock(exists)});
   const lock={id:String(state.next++),path:body.path,owner:'PRIVATE_LOCK_OWNER',ours:true,locked_at:'2026-10-19T10:00:00Z'};state.locks.push(lock);
   if(state.mode==='pause-after-lock'){state.held.push(()=>send(201,{lock:publicLock(lock)}));return;}
   return send(201,{lock:publicLock(lock)});
  }
  const unlock=route.match(/^\/locks\/([^/]+)\/unlock$/);
  if(unlock&&req.method==='POST'){
   state.writes++;const lock=state.locks.find(l=>l.id===unlock[1]);if(!lock)return send(404,{message:'Lock not found'});
   if(!lock.ours&&!body.force)return send(403,{message:'Not the owner'});
   if(state.mode==='reject-unlock')return send(403,{message:'Permission denied for force unlock'});
   state.locks=state.locks.filter(l=>l!==lock);return send(200,{lock:publicLock(lock)});
  }
  send(404,{message:'Unknown fixture action'});
 });
 await new Promise(r=>server.listen(0,'127.0.0.1',r));state.endpoint='http://127.0.0.1:'+server.address().port+prefix;
 state.release=()=>{for(const f of state.held.splice(0))f()};
 state.stop=async()=>{state.release();server.closeAllConnections();await new Promise(r=>server.close(r))};
 return state;
}
module.exports={start};
