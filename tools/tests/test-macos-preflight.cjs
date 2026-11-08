'use strict';
// Exercise the build script's real shell preflight, then stop before compilation.
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs');
const path=require('node:path'),os=require('node:os'),cp=require('node:child_process');
const script=path.resolve(__dirname,'../build-macos.sh');
function run(mode){
  const parent=fs.realpathSync(os.tmpdir()),root=fs.realpathSync(fs.mkdtempSync(path.join(parent,'rocket-macos-preflight-')));
  assert.equal(path.dirname(root),parent);assert(path.basename(root).startsWith('rocket-macos-preflight-'));
  const bin=path.join(root,'bin'),out=path.join(root,'output'),marker=path.join(root,'cargo-called');fs.mkdirSync(bin);
  const tools={
    uname:'if [ "$1" = -s ]; then echo Darwin; else echo arm64; fi',
    id:'echo 501',
    'xcode-select':'echo /fixture/SDK',
    xcrun:'echo 15.0',
    node:'exit 0',
    sw_vers:'echo fixture-macOS',
    git:'case "$1" in rev-parse) echo 0000000000000000000000000000000000000000 ;; --version) echo "git version 2.56.0" ;; status) exit 0 ;; *) exit 90 ;; esac',
    rustup:`set -e
test "$1" = run && test "$2" = 1.99.1
if [ "$3" = rustc ]; then
  if [ "$ROCKET_PREFLIGHT_FIXTURE" = missing ]; then exit 78; fi
  if [ "$ROCKET_PREFLIGHT_FIXTURE" = wrong ]; then host=x86_64-apple-darwin; else host=aarch64-apple-darwin; fi
  printf 'rustc 1.99.1\\nhost: %s\\n' "$host"
  # Force the writer to remain alive after grep -q has found the host line.
  sleep 0.1
  printf 'release: 1.99.1\\nLLVM version: fixture\\n'
elif [ "$3" = cargo ] && [ "$4" = check ]; then
  printf 'preflight reached Cargo\\n' > "$ROCKET_PREFLIGHT_MARKER"
  exit 73
else exit 90
fi`,
  };
  for(const [name,body]of Object.entries(tools))fs.writeFileSync(path.join(bin,name),'#!/bin/bash\n'+body+'\n',{mode:0o755});
  const result=cp.spawnSync('/bin/bash',[script,'--offline'],{encoding:'utf8',timeout:15000,env:{...process.env,PATH:bin+':/usr/bin:/bin',ROCKET_MACOS_OUTPUT:out,ROCKET_PREFLIGHT_FIXTURE:mode,ROCKET_PREFLIGHT_MARKER:marker}});
  try{
    assert.ifError(result.error);
    if(mode==='valid'){
      assert.equal(result.status,73,'Valid compiler was rejected before Cargo: '+result.stdout+result.stderr);
      assert(fs.existsSync(marker));assert(fs.readFileSync(path.join(out,'build-info.md'),'utf8').includes('host: aarch64-apple-darwin'));
    }else{
      assert.equal(result.status,2,result.stdout+result.stderr);assert(!fs.existsSync(marker));assert(!fs.existsSync(out));
    }
    assert.equal(path.dirname(root),parent);fs.rmSync(root,{recursive:true});
  }catch(error){console.error('Preserved failed fixture: '+root);throw error;}
}
for(const mode of ['valid','wrong','missing'])test('Build preflight: '+mode+' toolchain, with full producer output',{skip:process.platform==='win32'},()=>run(mode));
