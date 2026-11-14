'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),os=require('node:os'),path=require('node:path');
const {zip}=require('./app-update-fixtures.cjs'),reader=require('../macos/read-package-zip.cjs');
const {inputs,sourceDiff}=require('../macos/test-retained-package.cjs');
function removeFixture(temp){
 assert.equal(path.dirname(path.resolve(temp)),path.resolve(os.tmpdir()));assert(path.basename(temp).startsWith('rocket-zip-'));assert(!fs.lstatSync(temp).isSymbolicLink());
 fs.rmSync(temp,{recursive:true,force:true});
}
test('retained package inputs reject shell syntax, wrong architectures and production changes',()=>{
 const env={ROCKET_REQUESTED_MAC_ARCH:'x64',ROCKET_SOURCE_RUN:'1234567',ROCKET_ARCHIVE_SHA256:'a'.repeat(64)};
 assert.equal(inputs(env).arch,'x64');
 for(const value of ['0','-1','1; pwd','$(pwd)','1\n2','1'.repeat(17)])assert.throws(()=>inputs({...env,ROCKET_SOURCE_RUN:value}));
 assert.throws(()=>inputs({...env,ROCKET_REQUESTED_MAC_ARCH:'self-hosted'}));assert.throws(()=>inputs({...env,ROCKET_ARCHIVE_SHA256:'A'.repeat(64)}));
 sourceDiff(['tools/tests/test-mcp-macos-package.cjs']);
 for(const changed of ['Cargo.lock','crates/rocket-ui/src/app.rs','tools/macos/bundle.cjs','tools/macos/build-lua-helper.sh','README.md'])assert.throws(()=>sourceDiff([changed]));
});
test('bounded ZIP roundtrip preserves Unicode bytes and refuses replacing existing files',()=>{
 const temp=fs.mkdtempSync(path.join(os.tmpdir(),'rocket-zip-')),dest=path.join(temp,'out');
 try{const bytes=zip({'package/資料.bin':Buffer.from([0,1,2,255])});reader.extract(bytes,dest);assert.deepEqual(fs.readFileSync(path.join(dest,'package/資料.bin')),Buffer.from([0,1,2,255]));assert.throws(()=>reader.extract(bytes,dest));}
 finally{removeFixture(temp);}
});
test('archive traversal, aliases, symlinks, bad CRC, truncation and oversized output are rejected before writing',()=>{
 const temp=fs.mkdtempSync(path.join(os.tmpdir(),'rocket-zip-refuse-')),dest=path.join(temp,'out');
 try{
  const refuse=bytes=>{assert.throws(()=>reader.extract(bytes,dest));assert(!fs.existsSync(dest));};
  for(const name of ['../escape','/escape','a/../escape','a\\escape','C:/escape','a//b'])refuse(zip({[name]:Buffer.from('x')}));
  refuse(zip({'A':Buffer.from('one'),'a':Buffer.from('two')}));
  const good=zip({'package/file':Buffer.from('payload')}),central=good.readUInt32LE(good.length-6);
  const link=Buffer.from(good);link.writeUInt32LE((0o120777*65536)>>>0,central+38);refuse(link);
  const corrupt=Buffer.from(good);corrupt[30+'package/file'.length]^=1;refuse(corrupt);
  const huge=Buffer.from(good);huge.writeUInt32LE(0x7fffffff,central+24);refuse(huge);
  refuse(good.subarray(0,good.length-1));
 }finally{fs.rmdirSync(temp);}
});
test('retained workflow is manual, read-only and restricted to standard public runners',()=>{
 const cp=require('node:child_process'),source=fs.readFileSync(path.resolve(__dirname,'../../.github/workflows/macos-package-acceptance.yml'),'utf8');
 assert.match(source,/^on:\r?\n  workflow_dispatch:/m);assert.doesNotMatch(source,/^\s*(?:push|pull_request(?:_target)?|workflow_run|schedule):/m);
 assert.doesNotMatch(source,/write-all|:\s*write\b|secrets\.|self-hosted/);assert.match(source,/github\.event\.repository\.private == false/);
 assert.match(source,/inputs\.architecture == 'x64' && 'macos-15-intel' \|\| 'macos-15'/);assert.match(source,/timeout-minutes: 17/);
 for(const match of source.matchAll(/uses:\s*(\S+)/g))assert.match(match[1],/^actions\/(checkout|upload-artifact)@[a-f0-9]{40}$/);
 assert.match(source,/persist-credentials: false/);
 const guard=source.match(/run: \|\r?\n([\s\S]*?)\r?\n      - name:/)[1].split(/\r?\n/).map(s=>s.slice(10)).join('\n');assert(!guard.includes('${{'));
 const shell=process.platform==='win32'?path.join(process.env.ProgramFiles,'Git/bin/bash.exe'):'bash';
 const base={...process.env,ROCKET_REQUESTED_MAC_ARCH:'x64',ROCKET_SOURCE_RUN:'123',ROCKET_ARCHIVE_SHA256:'a'.repeat(64)};delete base.BASH_ENV;delete base.ENV;
 for(const change of [{},{ROCKET_REQUESTED_MAC_ARCH:'arm64'},{ROCKET_REQUESTED_MAC_ARCH:'self-hosted'},{ROCKET_SOURCE_RUN:'$(exit 0)'},{ROCKET_SOURCE_RUN:'1\n2'},{ROCKET_ARCHIVE_SHA256:'a; exit 0'}]){
  const r=cp.spawnSync(shell,['--noprofile','--norc','-c',guard],{env:{...base,...change},encoding:'utf8',windowsHide:true,timeout:10000});assert.ifError(r.error);assert.equal(r.status,Object.keys(change).length===0||change.ROCKET_REQUESTED_MAC_ARCH==='arm64'?0:2);
 }
});
