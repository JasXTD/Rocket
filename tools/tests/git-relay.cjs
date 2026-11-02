// Holds a Git command for a native runner. Hooks can't: MCP turns them off (SECURITY.md).
// A relay on PATH alone isn't enough either, because find_git prefers the bundle beside
// rocket-mcp.exe. MCP does accept a saved Git choice that is the first Git on PATH, so the
// relay is both, in this run's own profile, and the bundle and production code stay as they are.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const normal=p=>path.resolve(p.replace(/^\\\\\?\\/,'')).toLowerCase();
// state is the folder that holds the first repository the run opens, where MCP keeps its profile.
function gitRelay(root,env,source,state=root){
 const folder=path.join(root,'git-relay'),exe=path.join(folder,'git.exe'),selection=path.join(state,'rocket-ui-state','git-selection');
 const gate=path.join(root,'git-gate'),marker=path.join(root,'git-marker');
 const realGit=cp.execFileSync('where.exe',['git.exe'],{windowsHide:true,encoding:'utf8'}).trim().split(/\r?\n/)[0];
 fs.mkdirSync(folder);fs.mkdirSync(path.dirname(selection),{recursive:true});
 cp.execFileSync('rustc',['--edition=2024','-C','debuginfo=0','-o',exe,path.resolve(source)],{windowsHide:true,stdio:'pipe'});
 fs.writeFileSync(selection,'ROCKET-GIT-1\n'+exe);
 const key=Object.keys(env).find(k=>k.toUpperCase()==='PATH');
 return{gate,marker,folder,exe,
  env:{...env,ROCKET_FILE_TEST_GIT:realGit,ROCKET_FILE_TEST_GATE:gate,ROCKET_FILE_TEST_MARKER:marker,[key]:folder+path.delimiter+env[key]},
  // Fails early and by name if a run would otherwise reach a gate that never holds.
  check(state){const active=state.git_settings?.active_executable??'none';if(normal(active)!==normal(exe))throw new Error('Rocket is not running the Git relay: '+active);},
  // Before starting the service without the relay: MCP refuses a saved choice not on PATH.
  direct(){fs.rmSync(selection,{force:true});},
 };
}
module.exports={gitRelay};
