'use strict';
// Retest an exact package produced by this repository without rebuilding the app.
// Native tests compile their first-party Git relay with the installed Rust compiler.
// No dependency installs, account changes, arbitrary download URLs or result overrides.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const zip=require('./read-package-zip.cjs');
const ROOT=path.resolve(__dirname,'../..'),REPO='JasXTD/Rocket',BASE='/repos/'+REPO;
const TEST_ONLY=new Set([
 '.github/workflows/macos-package-acceptance.yml','tools/macos/test-retained-package.cjs','tools/macos/read-package-zip.cjs',
 'tools/macos/test-lua-integration.sh','tools/tests/test-retained-package.cjs','tools/tests/test-mcp-macos-package.cjs',
 'tools/tests/test-mcp-agents.cjs',
 'tools/tests/fixtures/hosting-other-inline.cjs','tools/tests/test-cli-hosting-other-inline.cjs','tools/tests/test-hosting-inline-fixture.cjs',
]);
const hash=b=>crypto.createHash('sha256').update(b).digest('hex');
function sourceDiff(paths){assert(paths.every(p=>TEST_ONLY.has(p)),'Package application/build inputs changed; rebuild it');}
function inputs(env){
 assert(['arm64','x64'].includes(env.ROCKET_REQUESTED_MAC_ARCH));
 assert(/^[1-9][0-9]{0,15}$/.test(env.ROCKET_SOURCE_RUN||''));
 assert(/^[a-f0-9]{64}$/.test(env.ROCKET_ARCHIVE_SHA256||''));
 return {arch:env.ROCKET_REQUESTED_MAC_ARCH,run:env.ROCKET_SOURCE_RUN,sha:env.ROCKET_ARCHIVE_SHA256};
}
async function main(){
 const input=inputs(process.env);assert.equal(process.platform,'darwin');assert.equal(process.arch,input.arch);
 assert.equal(process.env.GITHUB_ACTIONS,'true');assert.equal(process.env.GITHUB_REPOSITORY,REPO);
 assert.equal(process.env.ROCKET_MACOS_SANDBOX_DISPOSABLE_USER,'1');
 const temp=fs.realpathSync(process.env.RUNNER_TEMP),out=path.join(temp,'rocket-retained');assert(!fs.existsSync(out));fs.mkdirSync(out,{mode:0o700});
 const report={passed:false,architecture:input.arch,source_run:Number(input.run),archive_sha256:input.sha,checks:[]};
 const token=process.env.ROCKET_READ_ARTIFACT_TOKEN;assert(token);delete process.env.ROCKET_READ_ARTIFACT_TOKEN;
 const env={...process.env};for(const k of Object.keys(env))if(/^(?:GITHUB_|GH_|ACTIONS_|DYLD_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)/.test(k)||k==='ROCKET_SPELL_OBSERVER')delete env[k];
 Object.assign(env,{GITHUB_ACTIONS:'true',RUNNER_OS:'macOS',GIT_TERMINAL_PROMPT:'0',GCM_INTERACTIVE:'never'});
 const git=args=>cp.execFileSync('/usr/bin/git',args,{cwd:ROOT,env,encoding:'utf8',timeout:30000}).trimEnd();
 const headers={Authorization:'Bearer '+token,Accept:'application/vnd.github+json','X-GitHub-Api-Version':'2026-03-10','User-Agent':'Rocket-package-acceptance'};
 async function api(route){assert(route===BASE||route.startsWith(BASE+'/actions/'));const r=await fetch('https://api.github.com'+route,{headers,redirect:'error',signal:AbortSignal.timeout(45000)});assert(r.ok,'GitHub metadata HTTP '+r.status);const body=await r.text();assert(body.length<4*1024*1024);return JSON.parse(body);}
 function run(name,args){
  const log=path.join(out,name+'.log'),fd=fs.openSync(log,'wx');let result;
  try{result=cp.spawnSync(process.execPath,args,{cwd:ROOT,env,stdio:['ignore',fd,fd],timeout:8*60*1000});}finally{fs.closeSync(fd);}
  const ok=!result.error&&result.status===0;report.checks.push({name,passed:ok,status:result.status,error:result.error?.code||null});console.log((ok?'PASS ':'FAIL ')+name);return ok;
 }
 try{
  const repository=await api(BASE);assert(repository.full_name===REPO&&!repository.private&&!repository.archived);
  const source=await api(BASE+'/actions/runs/'+input.run);assert.equal(source.status,'completed');assert.equal(source.event,'workflow_dispatch');assert.equal(source.path,'.github/workflows/macos-wasm-prototype.yml');assert(/^[a-f0-9]{40}$/.test(source.head_sha));
  report.source_commit=source.head_sha;report.test_commit=git(['rev-parse','HEAD']);git(['merge-base','--is-ancestor',source.head_sha,report.test_commit]);
  report.test_only_diff=git(['diff','--name-only','-z',source.head_sha,report.test_commit]).split('\0').filter(Boolean);sourceDiff(report.test_only_diff);
  const artifacts=await api(BASE+'/actions/runs/'+input.run+'/artifacts?per_page=100');assert.equal(artifacts.total_count,artifacts.artifacts.length);
  const found=artifacts.artifacts.filter(a=>a.name===`lua-mac-${input.arch}-${input.run}-${source.run_attempt}`);assert.equal(found.length,1);
  const artifact=found[0];assert(!artifact.expired&&artifact.size_in_bytes<=512*1024*1024&&/^sha256:[a-f0-9]{64}$/.test(artifact.digest));
  const first=await fetch('https://api.github.com'+BASE+'/actions/artifacts/'+artifact.id+'/zip',{headers,redirect:'manual',signal:AbortSignal.timeout(45000)});assert.equal(first.status,302);
  const url=new URL(first.headers.get('location'));await first.body?.cancel();
  assert(url.protocol==='https:'&&!url.username&&!url.password&&(/\.blob\.core\.windows\.net$|\.githubusercontent\.com$/.test(url.hostname))&&!/\.txt$/i.test(decodeURIComponent(url.pathname)));
  const response=await fetch(url,{redirect:'error',signal:AbortSignal.timeout(180000)});assert(response.ok&&!/\.txt(?:[";\s]|$)/i.test(response.headers.get('content-disposition')||''));
  const chunks=[];let size=0;for await(const chunk of response.body){size+=chunk.length;assert(size<=artifact.size_in_bytes);chunks.push(chunk);}assert.equal(size,artifact.size_in_bytes);
  const bytes=Buffer.concat(chunks);assert.equal('sha256:'+hash(bytes),artifact.digest);report.artifact={id:artifact.id,sha256:hash(bytes)};
  const version=require('./bundle.cjs').version(),name=`Rocket-${version}-macos-${input.arch}-preview.zip`,entries=zip.inventory(bytes);
  const packages=entries.filter(e=>e.name==='prototype-results/'+name);assert.equal(packages.length,1);
  const archive=zip.content(bytes,packages[0]);assert.equal(hash(archive),input.sha,'Reviewed portable archive differs');
  const destination=path.join(out,'extracted');zip.extract(archive,destination);
  const bundle=path.join(destination,'package');assert(run('bundle-verification',['tools/macos/bundle.cjs','verify',bundle]));
  const manifest=JSON.parse(fs.readFileSync(path.join(bundle,'bundle-manifest.json')));assert.equal(manifest.platform,'macos-'+input.arch);assert.equal(manifest.version,version);assert.equal(manifest.lua.runtime,'pulley');
  assert(fs.readFileSync(path.join(bundle,'BUILD.md'),'utf8').includes('Source: '+source.head_sha));
  env.ROCKET_MCP_EXE=path.join(bundle,'Rocket.app/Contents/MacOS/rocket-mcp');env.ROCKET_MCP_RESULTS=path.join(out,'fixtures');
  env.ROCKET_AGENTS_EXE=env.ROCKET_MCP_EXE;env.ROCKET_AGENTS_REPORT_ROOT=env.ROCKET_MCP_RESULTS;
  env.ROCKET_CLI_EXE=path.join(bundle,'Rocket.app/Contents/MacOS/rocket-cli');env.ROCKET_CLI_RESULTS=env.ROCKET_MCP_RESULTS;
  assert(run('fixture-contract',['--test','tools/tests/test-hosting-inline-fixture.cjs']));
  assert(run('desktop',['tools/tests/test-mcp-desktop.cjs']));
  assert(run('package',['tools/tests/test-mcp-macos-package.cjs']));
  assert(run('agents',['tools/tests/test-mcp-agents.cjs']));
  assert(run('hosting',['tools/tests/test-cli-hosting-other-inline.cjs']));
  assert(run('bundle-after-tests',['tools/macos/bundle.cjs','verify',bundle]));
  report.passed=true;
 }catch(error){report.error=error.message;process.exitCode=1;}
 finally{
  const evidence=path.join(out,'evidence');fs.mkdirSync(evidence);const fixtures=path.join(out,'fixtures');
  if(fs.existsSync(fixtures))for(const name of fs.readdirSync(fixtures)){const dir=path.join(fixtures,name);if(!fs.statSync(dir).isDirectory())continue;for(const file of fs.readdirSync(dir))if(file==='result.json'||file==='failure-state.json'||file==='failure.json'||file.endsWith('.png'))fs.copyFileSync(path.join(dir,file),path.join(evidence,name+'-'+file));}
  fs.writeFileSync(path.join(out,'retained.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
 }
}
module.exports={inputs,sourceDiff};
if(require.main===module)main().catch(e=>{console.error(e.message);process.exitCode=1});
