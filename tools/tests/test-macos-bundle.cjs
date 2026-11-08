'use strict';
// Metadata fixtures are deliberately non-runnable Mach-O data, never downloaded code.
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs');
const path=require('node:path'),os=require('node:os'),cp=require('node:child_process');
const b=require('../macos/bundle.cjs');
const names=['rocket','rocket-cli','rocket-mcp','rocket-rebase-editor','rocket-ssh'];
function binary(){
  const bytes=Buffer.alloc(72);bytes.writeUInt32LE(0xfeedfacf,0);bytes.writeUInt32LE(0x0100000c,4);
  bytes.writeUInt32LE(2,12);bytes.writeUInt32LE(1,16);bytes.writeUInt32LE(24,20);
  bytes.writeUInt32LE(0x32,32);bytes.writeUInt32LE(24,36);bytes.writeUInt32LE(1,40);
  bytes.writeUInt32LE(0x000f0000,44);bytes.writeUInt32LE(0x000f0000,48);return bytes;
}
function fixture(fn){
  const parent=fs.realpathSync(os.tmpdir()),root=fs.realpathSync(fs.mkdtempSync(path.join(parent,'rocket-macos-bundle-')));
  const bins=path.join(root,'build 資料'),icon=path.join(root,'Rocket.icns'),out=path.join(root,'package 中文'),info=path.join(root,'BUILD.md');
  fs.mkdirSync(bins);for(const name of names)fs.writeFileSync(path.join(bins,name),binary(),{mode:0o755});
  const png=fs.readFileSync(path.join(__dirname,'../../docs/images/rocket-logo.png'));
  const bytes=Buffer.alloc(16+png.length);bytes.write('icns',0);bytes.writeUInt32BE(bytes.length,4);bytes.write('ic08',8);bytes.writeUInt32BE(png.length+8,12);png.copy(bytes,16);
  fs.writeFileSync(icon,bytes);fs.writeFileSync(info,'Synthetic bundle metadata fixture; not a runnable application.\n');
  const stage=()=>b.stage(bins,icon,out,info),seal=()=>{stage();b.manifest(out);};
  try{fn({root,bins,icon,out,stage,seal});assert.equal(path.dirname(root),parent);assert.equal(fs.realpathSync(root),root);assert(path.basename(root).startsWith('rocket-macos-bundle-'));fs.rmSync(root,{recursive:true});}
  catch(error){console.error('Failed package fixture preserved: '+root);throw error;}
}
test('Complete Unicode-path bundle has exact metadata, existing artwork and five sibling helpers',()=>fixture(f=>{
  f.seal();const data=b.verify(f.out);assert.equal(data.lua,'disabled');assert.equal(data.minimum_macos,'15.0');
  assert.equal(data.version,b.version());assert.equal(data.identity,'io.github.jasxtd.rocket');
  const dir=path.join(f.out,'Rocket.app/Contents/MacOS');assert.deepEqual(fs.readdirSync(dir).sort(),names);
  for(const name of names)assert(fs.readFileSync(path.join(dir,name)).equals(fs.readFileSync(path.join(f.bins,name))));
  const plist=fs.readFileSync(path.join(f.out,'Rocket.app/Contents/Info.plist'),'utf8');
  assert(plist.includes('<key>CFBundleExecutable</key><string>rocket</string>'));
  assert(plist.includes('<key>NSHighResolutionCapable</key><true/>'));
  assert(plist.includes('<key>CFBundleIconFile</key><string>Rocket.icns</string>'));
  assert(fs.readFileSync(path.join(f.out,'Rocket.app/Contents/Resources/Rocket.icns')).equals(fs.readFileSync(f.icon)));
  assert(!data.files.some(x=>/lua|git\.exe|token|credential/.test(x.path)));
}));
test('Existing outputs and source bytes are preserved',()=>fixture(f=>{
  f.seal();const before=fs.readFileSync(path.join(f.out,'SHA256SUMS'));
  assert.throws(f.stage,/Output exists/);assert.throws(()=>b.manifest(f.out),/EEXIST/);
  assert(before.equals(fs.readFileSync(path.join(f.out,'SHA256SUMS'))));b.verify(f.out);
  for(const name of names)assert(fs.readFileSync(path.join(f.bins,name)).equals(binary()));
}));
test('Missing helper fails before output publication',()=>fixture(f=>{
  fs.unlinkSync(path.join(f.bins,'rocket-ssh'));assert.throws(f.stage,/ENOENT/);assert(!fs.existsSync(f.out));
}));
test('Windows/Linux/Intel/wrong-platform/newer-OS/truncated input is refused',()=>fixture(f=>{
  const variants=[Buffer.from('MZ Windows'),Buffer.from('\x7fELF Linux')];
  for(const [offset,value]of [[4,0x01000007],[12,6],[40,2],[44,0x00100000],[36,0xffffffff],[20,0xffffffff]]){const x=binary();x.writeUInt32LE(value,offset);variants.push(x);}
  variants.push(binary().subarray(0,40));
  for(const bytes of variants){fs.writeFileSync(path.join(f.bins,'rocket'),bytes);assert.throws(f.stage);assert(!fs.existsSync(f.out));}
}));
test('Broken or empty icon containers are refused before publication',()=>fixture(f=>{
  for(const data of [Buffer.from('not an icon'),Buffer.from('icns\0\0\0\x08')]){fs.writeFileSync(f.icon,data);assert.throws(f.stage,/ICNS/);assert(!fs.existsSync(f.out));}
}));
test('Tampering, missing helpers, unexpected Lua workers and altered plist fail verification',()=>fixture(f=>{
  f.seal();const file=path.join(f.out,'Rocket.app/Contents/MacOS/rocket-cli'),original=fs.readFileSync(file);
  const edited=Buffer.from(original);edited[70]=99;fs.writeFileSync(file,edited);assert.throws(()=>b.verify(f.out),/Hash mismatch/);
  fs.unlinkSync(file);assert.throws(()=>b.verify(f.out),/incomplete/);fs.writeFileSync(file,original,{mode:0o755});
  const lua=path.join(f.out,'Rocket.app/Contents/MacOS/rocket-lua-host');fs.writeFileSync(lua,original);assert.throws(()=>b.verify(f.out),/unexpected/);fs.unlinkSync(lua);
  const info=path.join(f.out,'Rocket.app/Contents/Info.plist');fs.appendFileSync(info,'\n');assert.throws(()=>b.verify(f.out),/metadata/);
}));
test('Manifest traversal, duplicate entries, downgrade and checksum changes are refused',()=>fixture(f=>{
  f.seal();const file=path.join(f.out,'bundle-manifest.json'),before=fs.readFileSync(file);
  for(const change of [d=>d.files[0].path='../outside',d=>d.files.push(d.files[0]),d=>d.version='0.1.0',d=>d.platform='linux',d=>d.lua='enabled']){
    const data=JSON.parse(before);change(data);fs.writeFileSync(file,JSON.stringify(data));assert.throws(()=>b.verify(f.out));
  }
  fs.writeFileSync(file,before);fs.appendFileSync(path.join(f.out,'SHA256SUMS'),'\n');assert.throws(()=>b.verify(f.out),/sidecar/);
}));
test('Hard-linked source helpers and bundle directory redirects are refused',()=>fixture(f=>{
  fs.linkSync(path.join(f.bins,'rocket-ssh'),path.join(f.root,'shared'));assert.throws(f.stage,/linked/);fs.unlinkSync(path.join(f.root,'shared'));
  f.seal();const dir=path.join(f.out,'Rocket.app/Contents/Resources'),saved=path.join(f.root,'resources');fs.renameSync(dir,saved);
  fs.symlinkSync(saved,dir,process.platform==='win32'?'junction':'dir');assert.throws(()=>b.verify(f.out),/symlink|linked/);
}));
test('Non-executable helpers and writable package content fail on Unix',{skip:process.platform==='win32'},()=>fixture(f=>{
  f.seal();const file=path.join(f.out,'Rocket.app/Contents/MacOS/rocket');fs.chmodSync(file,0o644);assert.throws(()=>b.verify(f.out),/not executable/);
  fs.chmodSync(file,0o777);assert.throws(()=>b.verify(f.out),/Writable/);
}));
test('Signing/native validation refuses other operating systems',{skip:process.platform==='darwin'},()=>{
  assert.throws(()=>b.signDevelopment('/unused'),/macOS/);assert.throws(()=>b.nativeVerify('/unused'),/macOS/);
});
test('The icon exporter writes ten correctly sized PNGs and never overwrites an existing directory',()=>{
  assert(process.env.ROCKET_MACOS_ICON_TOOL,'Set ROCKET_MACOS_ICON_TOOL to the built macos_icon example');
  fixture(f=>{
    const out=path.join(f.root,'artwork 資料.iconset'),exe=path.resolve(process.env.ROCKET_MACOS_ICON_TOOL);
    const first=cp.spawnSync(exe,[out],{encoding:'utf8',timeout:60000,windowsHide:true});assert.ifError(first.error);assert.equal(first.status,0,first.stderr);
    const expected=[];
    for(const points of [16,32,128,256,512])for(const scale of [1,2]){
      const name=`icon_${points}x${points}${scale===2?'@2x':''}.png`;expected.push(name);const data=fs.readFileSync(path.join(out,name));
      assert.equal(data.subarray(0,8).toString('hex'),'89504e470d0a1a0a');assert.equal(data.readUInt32BE(16),points*scale);assert.equal(data.readUInt32BE(20),points*scale);
    }
    assert.deepEqual(fs.readdirSync(out).sort(),expected.sort());const before=fs.readFileSync(path.join(out,'icon_512x512@2x.png'));
    const second=cp.spawnSync(exe,[out],{encoding:'utf8',timeout:60000,windowsHide:true});assert.notEqual(second.status,0);assert(before.equals(fs.readFileSync(path.join(out,'icon_512x512@2x.png'))));
  });
});
