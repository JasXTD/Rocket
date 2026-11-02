'use strict';
// Validate the moved launcher in a disposable LOCALAPPDATA, never the user's profile.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const crypto=require('node:crypto'),net=require('node:net'),assert=require('node:assert/strict');
const {RocketHttpClient}=require('../mcp/mcp-http-client.cjs');
process.chdir(path.resolve(__dirname,'../..'));
const root=path.resolve('work/launcher-smoke-'+crypto.randomUUID());
const local=path.join(root,'local-app-data'),profile=path.join(local,'Rocket/state/mcp');
const report={passed:false,root};let client;
async function main(){try{
  fs.mkdirSync(root,{recursive:true});
  const exe=path.resolve(process.argv[2]||'tools/releases/0.81.1/rocket-mcp.exe');
  cp.execFileSync(exe,['--init-http',profile,root],{windowsHide:true,stdio:'pipe'});
  const server=net.createServer();await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;await new Promise(r=>server.close(r));
  fs.writeFileSync(path.join(profile,'settings'),`ROCKET-MCP-1\n${port}\n${root}\n`);
  client=new RocketHttpClient(profile);
  const env={...process.env,LOCALAPPDATA:local};
  let launchCount=0;
  const run=()=>{
    // A detached Windows child may inherit extra pipe handles. Wait for the
    // launcher process, not EOF from every descendant, by capturing into files.
    const output=path.join(root,'launch-'+(++launchCount)+'.log'),errors=output+'.stderr';
    const out=fs.openSync(output,'w'),err=fs.openSync(errors,'w');
    try{cp.execFileSync('pwsh.exe',['-NoProfile','-File','tools/mcp/start-local-mcp.ps1'],{env,windowsHide:true,timeout:60000,stdio:['ignore',out,err]});}
    finally{fs.closeSync(out);fs.closeSync(err);}
    return fs.readFileSync(output,'utf8');
  };
  const first=run();assert(first.includes('Started local MCP process '),first);
  await client.initialize();const a=await client.tool('rocket_state');
  const second=run();assert(!second.includes('Started local MCP process '),second);
  const b=await client.tool('rocket_state');assert.equal(a.pid,b.pid,'Launcher must reuse its existing service');
  report.cases=['Launcher finds the preserved bundle when target is empty','Health-check script works from its new location','Repeated launch reuses the same isolated service'];
  await client.raw('POST','/shutdown');
  const stopped=cp.spawnSync(process.execPath,['tools/mcp/check-local-mcp.cjs'],{env,windowsHide:true,encoding:'utf8',timeout:30000});
  // The service may still be completing its graceful shutdown when first checked.
  if(stopped.status!==2){let down=false;for(let i=0;i<100;i++){try{await client.raw('GET','/health');}catch(e){if(e.code==='ECONNREFUSED'){down=true;break;}throw e;}await new Promise(r=>setTimeout(r,50));}assert(down,'Service did not stop');}
  report.passed=true;
}catch(error){report.error=error.stack;report.launcher_output=error.stdout?.toString().slice(-1500);report.launcher_error=error.stderr?.toString().slice(-1500);process.exitCode=1;}finally{
  if(client){try{await client.raw('POST','/shutdown');}catch(error){if(error.code!=='ECONNREFUSED')report.cleanup_note=error.message;}await client.finish();}
  fs.mkdirSync('work/results',{recursive:true});fs.writeFileSync('work/results/launcher-latest.json',JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
}}
main().catch(error=>{console.error(error);process.exitCode=1;});
