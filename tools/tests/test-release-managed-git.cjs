// Reuses only the disposable bundle produced by test-mcp-managed-git.cjs.
// No downloads, host changes, network Git or execution of damaged binaries.
'use strict';
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const cp=require('node:child_process'),assert=require('node:assert/strict');
const latest=path.resolve('tools/maintenance/managed-git-20261030/resumed/native-latest.json');
const native=JSON.parse(fs.readFileSync(process.argv[2]||latest));
assert(native.passed,'Native fixture must have passed and closed its own service');
const allowed=fs.realpathSync('tools/maintenance/managed-git-20261030/resumed/runs')+path.sep;
const root=fs.realpathSync(native.root),bundle=fs.realpathSync(native.bundle);
assert(root.startsWith(allowed)&&bundle.startsWith(root+path.sep));
assert.equal(fs.readFileSync(path.join(root,'rocket-private-state'),'utf8'),'synthetic-content-only-v1\n');
assert(!fs.existsSync(path.join(root,'http-profile/access-token')));
const report={passed:false,root,bundle,cases:[]},version='2.56.0.windows.4';
const hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const manifest=path.join(bundle,'SHA256SUMS.txt'),helper=path.join(bundle,'tools/Git-Runtime.ps1');
const runtime=path.join(bundle,'git',version),inventory=path.join(bundle,'git/manifest.tsv');
const pwsh='C:/Program Files/PowerShell/7/pwsh.exe';
function files(directory=''){
 return fs.readdirSync(path.join(bundle,directory),{withFileTypes:true}).flatMap(entry=>{
  const relative=path.join(directory,entry.name);assert(!entry.isSymbolicLink(),'Fixture contains a link');
  return entry.isDirectory()?files(relative):relative==='SHA256SUMS.txt'?[]:[relative];
 });
}
function refresh(){fs.writeFileSync(manifest,files().sort().map(file=>hash(path.join(bundle,file))+'  '+file.replaceAll('\\','/')).join('\n')+'\n');}
function command(script,args,failure){
 const result=cp.spawnSync(pwsh,['-NoProfile','-File',script,...args],{windowsHide:true,encoding:'utf8',timeout:180000});
 if(result.error)throw result.error;const output=result.stdout+result.stderr;
 if(failure){assert.notEqual(result.status,0,'Invalid bundle was accepted');assert.match(output,failure);}
 else assert.equal(result.status,0,output);
 return output;
}
function verify(failure){return command(path.join(bundle,'tools/Verify-Release.ps1'),['-Bundle',bundle,'-RequireBundledGit'],failure);}
function assemble(failure){return command(path.resolve('tools/Assemble-GitRuntime.ps1'),['-Destination',bundle],failure);}
function mutate(file,bytes,check){const original=fs.readFileSync(file),sums=fs.readFileSync(manifest);
 try{if(bytes===null)fs.unlinkSync(file);else fs.writeFileSync(file,bytes);refresh();check();}
 finally{fs.writeFileSync(file,original);fs.writeFileSync(manifest,sums);}
}
function pass(name){report.cases.push(name);console.log('PASS '+name);}
try{
 assert.equal(hash(path.join(bundle,'rocket-mcp.exe')),native.executable_sha256,'Native proof must match the packaged host');
 fs.mkdirSync(path.dirname(helper),{recursive:true});
 fs.copyFileSync('tools/Verify-Release.ps1',path.join(bundle,'tools/Verify-Release.ps1'));
 fs.copyFileSync('tools/Git-Runtime.ps1',helper);
 fs.mkdirSync(path.join(bundle,'runtime/git'),{recursive:true});
 fs.copyFileSync('runtime/git/README.md',path.join(bundle,'runtime/git/README.md'));
 refresh();assert.match(verify(),/Pinned bundled Git 2\.56\.0\.windows\.4 verified \(9612 files\)/);
 pass('Complete bundle, compiled Git inventory, Lua host/worker pair and signed ConPTY pair verify');
 const prior=hash(inventory);assert.match(assemble(),/Existing files were not overwritten/);assert.equal(hash(inventory),prior);
 pass('Assembly reuses an unchanged reviewed runtime without overwriting existing files');
 const readme=path.join(runtime,'README.portable');const changed=Buffer.concat([fs.readFileSync(readme),Buffer.from('\nsynthetic alteration\n')]);
 mutate(readme,changed,()=>{verify(/Bundled Git file changed/);assemble(/Bundled Git file changed/);assert.deepEqual(fs.readFileSync(readme),changed);});
 pass('Changed runtime is rejected despite regenerated release checksums; assembly preserves it');
 mutate(inventory,Buffer.concat([fs.readFileSync(inventory),Buffer.from('\n')]),()=>verify(/inventory is not the reviewed version/));
 pass('Modified runtime inventory is rejected despite regenerated release checksums');
 mutate(helper,Buffer.concat([fs.readFileSync(helper),Buffer.from('\nthrow "UNTRUSTED_HELPER_EXECUTED"\n')]),()=>{
  const output=verify(/Unreviewed bundled Git verification helper/);assert(!output.includes('UNTRUSTED_HELPER_EXECUTED'));
 });
 pass('Changed verification helper is refused before its code can execute');
 const cli=path.join(bundle,'rocket-cli.exe'),bytes=Buffer.from(fs.readFileSync(cli));
 const digest=Buffer.from('94395c3c041d75fd66b5d768844e8545fc35010e368537aaede718eaf4e7ee96');
 let offset,count=0;while((offset=bytes.indexOf(digest))>=0){bytes.fill('f',offset,offset+digest.length);count++;}assert(count>0);
 mutate(cli,bytes,()=>verify(/Mismatched compiled Git runtime inventory: rocket-cli.exe/));
 pass('Mixed host/runtime build is rejected even with matching ordinary release checksums');
 const extra=path.join(runtime,'unexpected.exe'),sums=fs.readFileSync(manifest);
 try{fs.writeFileSync(extra,'synthetic non-executable fixture',{flag:'wx'});refresh();verify(/Unexpected bundled Git file/);}
 finally{fs.unlinkSync(extra);fs.writeFileSync(manifest,sums);}
 pass('Unreviewed executable in the runtime is refused');
 mutate(inventory,null,()=>verify(/required bundled Git inventory is missing/));
 pass('Missing published inventory is refused');
 assert.match(verify(),/Rocket binaries may be unsigned/);report.passed=true;
 report.host_sha256=hash(path.join(bundle,'rocket-mcp.exe'));report.manifest_sha256=hash(manifest);
 report.files=files().length;report.driver_sha256=hash(__filename);
}catch(error){report.error=error.stack;console.error(error.stack);process.exitCode=1;}
finally{fs.writeFileSync(path.join(root,'release-result.json'),JSON.stringify(report,null,2));console.log('Evidence: '+path.join(root,'release-result.json'));}
