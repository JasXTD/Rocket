const http=require('node:http'),net=require('node:net'),fs=require('node:fs'),path=require('node:path');
// Local acceptance client. Tokens remain in memory; never print request headers.
class RocketHttpClient {
  constructor(profile){this.profile=profile;this.calls=0;this.nextId=0;this.exited=false;this.reload();this.agent=new http.Agent({keepAlive:false});this.agent.createConnection=()=>net.createConnection({host:'127.0.0.1',port:this.port});}
  reload(){const lines=fs.readFileSync(path.join(this.profile,'settings'),'utf8').trimEnd().split(/\r?\n/);if(lines[0]!=='ROCKET-MCP-1')throw Error('Invalid local MCP profile');this.port=Number(lines[1]);this.token=fs.readFileSync(path.join(this.profile,'access-token'),'utf8');}
  raw(method,route,body='',headers={}){return new Promise((resolve,reject)=>{
    // Always create a direct numeric loopback socket; never inherit a system or Node proxy.
    const request=http.request({agent:this.agent,host:'127.0.0.1',port:this.port,path:route,method,headers:{Authorization:'Bearer '+this.token,Accept:'application/json, text/event-stream','Content-Type':'application/json','MCP-Protocol-Version':'2025-11-25','Content-Length':Buffer.byteLength(body),...headers}},response=>{
      const chunks=[];let size=0;response.on('data',b=>{size+=b.length;if(size>32*1024*1024){response.destroy();reject(Error('MCP response exceeded limit'));}else chunks.push(b);});
      response.on('end',()=>resolve({status:response.statusCode,headers:response.headers,text:Buffer.concat(chunks).toString('utf8')}));response.on('error',reject);
    });request.setTimeout(30000,()=>request.destroy(Error('MCP response timed out; inspect state before retrying')));request.on('error',reject);request.end(body);
  });}
  async request(method,params={}){const id=++this.nextId;const response=await this.raw('POST','/mcp',JSON.stringify({jsonrpc:'2.0',id,method,params}));if(response.status!==200)throw Error(`MCP HTTP ${response.status}: ${response.text}`);const value=JSON.parse(response.text);if(value.id!==id)throw Error('MCP response ID mismatch');if(value.error)throw Error(value.error.message);return value.result;}
  async notify(method,params={}){const response=await this.raw('POST','/mcp',JSON.stringify({jsonrpc:'2.0',method,params}));if(response.status!==202)throw Error('MCP notification rejected');}
  async initialize(){const value=await this.request('initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'Rocket HTTP acceptance',version:'1'}});await this.notify('notifications/initialized');return value;}
  async tool(name,args={}){this.calls++;const value=await this.request('tools/call',{name,arguments:args});if(value.isError)throw Error(value.content.map(c=>c.text||'').join('\n'));return value.structuredContent??value;}
  async finish(){this.agent.destroy();this.exited=true;} // A client disconnect must not stop the service.
  terminateOwnedProcess(){} // This client never owns the shared service process.
}
module.exports={RocketHttpClient};
