'use strict';
// First-party development packaging only. No dependency, download or credential.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const cp = require('node:child_process'), assert = require('node:assert/strict');
const ROOT = path.resolve(__dirname, '../..');
const ID = 'io.github.jasxtd.rocket', MINIMUM = '15.0';
const BINARIES = ['rocket', 'rocket-cli', 'rocket-mcp', 'rocket-rebase-editor', 'rocket-ssh'];
const LUA = require('./lua-helper.cjs');
const RESOURCES = new Map([
  ['LICENSE-MIT', 'LICENSE-MIT'], ['LICENSE-APACHE', 'LICENSE-APACHE'],
  ['THIRD-PARTY-NOTICES.txt', 'THIRD-PARTY-NOTICES.txt'],
  ['Noto-CJK-LICENSE', 'runtime/fonts/noto-sans-cjk/2.004/LICENSE'],
  ['Noto-CJK-NOTICE', 'runtime/fonts/noto-sans-cjk/2.004/NOTICE'],
  ['Noto-Emoji-LICENSE', 'runtime/fonts/noto-emoji/3.002/LICENSE'],
  ['Noto-Emoji-NOTICE', 'runtime/fonts/noto-emoji/3.002/NOTICE'],
]);
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
function version() {
  const found = fs.readFileSync(path.join(ROOT, 'Cargo.toml'), 'utf8')
    .match(/\[workspace\.package\][\s\S]*?\bversion\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"/);
  assert(found, 'A numeric workspace version is required'); return found[1];
}
function read(file, limit = 512 * 1024 * 1024) {
  const s = fs.lstatSync(file);
  assert(s.isFile() && !s.isSymbolicLink() && s.nlink === 1 && s.size <= limit, 'Refuse linked/non-file/oversized input: '+file);
  const fd = fs.openSync(file, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW || 0));
  try {
    const opened = fs.fstatSync(fd);
    assert(opened.isFile() && opened.dev === s.dev && opened.ino === s.ino, 'Input changed while opening: '+file);
    const bytes = fs.readFileSync(fd), after = fs.fstatSync(fd);
    assert(bytes.length === s.size && after.size === s.size && after.mtimeMs === s.mtimeMs, 'Input changed while reading: '+file);
    return bytes;
  } finally { fs.closeSync(fd); }
}
function macho(bytes, expectedArch) {
  assert(bytes.length >= 32 && bytes.readUInt32LE(0) === 0xfeedfacf, 'Expected thin 64-bit Mach-O');
  const cpu = bytes.readUInt32LE(4);
  const arch = cpu === 0x0100000c ? 'arm64' : cpu === 0x01000007 ? 'x64' : undefined;
  assert(arch, 'Expected Apple Silicon arm64 or Intel x86_64');
  if (expectedArch !== undefined) assert.equal(arch, expectedArch, 'Mixed Mac executable architectures');
  assert.equal(bytes.readUInt32LE(12), 2, 'Expected Mach-O executable');
  const count = bytes.readUInt32LE(16), end = 32 + bytes.readUInt32LE(20);
  assert(count > 0 && count <= 1024 && end <= bytes.length, 'Invalid Mach-O load commands');
  let at = 32, build = false;
  for (let i = 0; i < count; i++) {
    assert(at + 8 <= end, 'Truncated Mach-O command');
    const kind = bytes.readUInt32LE(at), size = bytes.readUInt32LE(at+4);
    assert(size >= 8 && size % 8 === 0 && at + size <= end, 'Invalid Mach-O command size');
    if (kind === 0x32) {
      assert(size >= 24 && bytes.readUInt32LE(at+8) === 1, 'Expected macOS build platform');
      assert(bytes.readUInt32LE(at+12) <= 0x000f0000, 'Binary requires newer macOS than the bundle declares');
      assert.equal(size, 24 + 8 * bytes.readUInt32LE(at+20), 'Invalid build-version tools'); build = true;
    }
    at += size;
  }
  assert(at === end && build, 'Missing/inconsistent macOS build-version command');
  return arch;
}
function icns(bytes) {
  assert(bytes.length > 16 && bytes.toString('ascii',0,4) === 'icns' && bytes.readUInt32BE(4) === bytes.length, 'Invalid ICNS container');
  let at = 8, image = false;
  while (at < bytes.length) {
    assert(at+8 <= bytes.length, 'Truncated ICNS element');
    const type = bytes.toString('ascii',at,at+4), size = bytes.readUInt32BE(at+4);
    assert(size >= 8 && at+size <= bytes.length, 'Invalid ICNS element length');
    if (/^ic(?:0[7-9]|1[0-4]|p[4-6])$/.test(type)) image = true;
    at += size;
  }
  assert(image, 'ICNS is missing an application image');
}
function plist(v) {
  assert(/^\d+\.\d+\.\d+$/.test(v), 'Invalid application version');
  const strings = {
    CFBundleDevelopmentRegion:'en', CFBundleDisplayName:'Rocket', CFBundleName:'Rocket',
    CFBundleExecutable:'rocket', CFBundleIdentifier:ID, CFBundleInfoDictionaryVersion:'6.0',
    CFBundlePackageType:'APPL', CFBundleShortVersionString:v, CFBundleVersion:v,
    CFBundleIconFile:'Rocket.icns', LSMinimumSystemVersion:MINIMUM, NSPrincipalClass:'NSApplication',
  };
  return '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0">\n<dict>\n'
    + Object.entries(strings).map(([k,v])=>`  <key>${k}</key><string>${v}</string>\n`).join('')
    + '  <key>NSHighResolutionCapable</key><true/>\n</dict>\n</plist>\n';
}
function walk(root, relative = '') {
  const directory = path.join(root,relative), s = fs.lstatSync(directory);
  assert(s.isDirectory() && !s.isSymbolicLink(), 'Refuse linked/non-directory bundle folder: '+relative);
  if (process.platform !== 'win32') assert((s.mode & 0o022) === 0, 'Writable bundle directory: '+relative);
  return fs.readdirSync(directory).sort().flatMap(name=>{
    assert(!/[\r\n\\]/.test(name), 'Invalid bundle filename');
    const rel = relative ? relative+'/'+name : name, stat = fs.lstatSync(path.join(root,rel));
    assert(!stat.isSymbolicLink(), 'Bundle symlink refused: '+rel);
    if (stat.isDirectory()) return walk(root,rel);
    assert(stat.isFile() && stat.nlink === 1, 'Bundle special/hardlinked file refused: '+rel);
    if (process.platform !== 'win32') assert((stat.mode & 0o022) === 0, 'Writable bundle file: '+rel);
    return [rel];
  });
}
function expected(lua = false) {
  return ['Rocket.app/Contents/Info.plist','Rocket.app/Contents/PkgInfo','Rocket.app/Contents/Resources/Rocket.icns',
    ...BINARIES.map(n=>'Rocket.app/Contents/MacOS/'+n), ...[...RESOURCES.keys()].map(n=>'Rocket.app/Contents/Resources/'+n),
    ...(lua ? [...LUA.FILES.map(n=>LUA.PREFIX+n),'Rocket.app/Contents/Resources/LUA-NOTICES'] : []),
    'README.md','BUILD.md'].sort();
}
function luaInfo(directory, arch = architecture(directory)) {
  const app=path.join(directory,LUA.PREFIX);
  if(!fs.existsSync(app)) return 'disabled';
  const digest=LUA.inspect(app,read,walk,bytes=>macho(bytes,arch));
  for(const name of ['rocket','rocket-mcp']) assert(read(path.join(directory,'Rocket.app/Contents/MacOS',name)).includes(Buffer.from(digest)), 'Host is not bound to this signed Lua helper: '+name);
  assert(read(path.join(directory,'Rocket.app/Contents/Resources/LUA-NOTICES'),4*1024*1024).length>0,'Missing Lua licenses');
  return {runtime:'pulley',sha256:digest};
}
function architecture(directory) {
  return macho(read(path.join(directory,'Rocket.app/Contents/MacOS/rocket')));
}
function structure(directory, v = version()) {
  const arch=architecture(directory), lua=luaInfo(directory,arch);
  const actual = walk(directory).filter(n=>!['bundle-manifest.json','SHA256SUMS','Rocket.app/Contents/_CodeSignature/CodeResources'].includes(n));
  assert.deepEqual(actual.sort(), expected(lua!=='disabled'), 'Bundle is incomplete or contains unexpected files');
  assert.equal(read(path.join(directory,'Rocket.app/Contents/Info.plist'),32768).toString(), plist(v), 'Bundle metadata mismatch');
  assert.equal(read(path.join(directory,'Rocket.app/Contents/PkgInfo'),32).toString(),'APPL????');
  icns(read(path.join(directory,'Rocket.app/Contents/Resources/Rocket.icns'),16*1024*1024));
  for (const name of BINARIES) {
    const file=path.join(directory,'Rocket.app/Contents/MacOS',name); macho(read(file),arch);
    if(process.platform!=='win32') assert((fs.statSync(file).mode&0o111)===0o111,'Helper is not executable: '+name);
  }
  for (const [name,source] of RESOURCES) assert(read(path.join(directory,'Rocket.app/Contents/Resources',name)).equals(read(path.join(ROOT,source))), 'License/notice mismatch: '+name);
  return arch;
}
function stage(binaries, icon, directory, buildInfo, luaApp = process.env.ROCKET_MACOS_LUA_APP) {
  const v=version(), input = new Map();
  const arch=macho(read(path.join(binaries,'rocket')));
  for (const name of BINARIES) { const data=read(path.join(binaries,name));macho(data,arch);input.set('Rocket.app/Contents/MacOS/'+name,data); }
  const image=read(icon,16*1024*1024);icns(image);input.set('Rocket.app/Contents/Resources/Rocket.icns',image);
  for (const [name,source] of RESOURCES) input.set('Rocket.app/Contents/Resources/'+name,read(path.join(ROOT,source)));
  input.set('Rocket.app/Contents/Info.plist',Buffer.from(plist(v)));input.set('Rocket.app/Contents/PkgInfo',Buffer.from('APPL????'));
  input.set('README.md',read(path.join(ROOT,'docs/MacOS-port.md')));input.set('BUILD.md',read(buildInfo,1024*1024));
  if(luaApp) {
    const digest=LUA.inspect(luaApp,read,walk,bytes=>macho(bytes,arch));
    for(const name of ['rocket','rocket-mcp']) assert(input.get('Rocket.app/Contents/MacOS/'+name).includes(Buffer.from(digest)),'Host is not bound to this signed Lua helper: '+name);
    for(const name of LUA.FILES) input.set(LUA.PREFIX+name,read(path.join(luaApp,name)));
    input.set('Rocket.app/Contents/Resources/LUA-NOTICES',read(path.join(path.dirname(luaApp),'LUA-NOTICES'),4*1024*1024));
  }
  directory=path.resolve(directory);assert(!fs.existsSync(directory) && !fs.lstatSync(directory,{throwIfNoEntry:false}), 'Output exists; preserve it and choose a new directory');
  fs.mkdirSync(directory,{mode:0o755});
  for(const rel of ['','Rocket.app','Rocket.app/Contents','Rocket.app/Contents/MacOS','Rocket.app/Contents/Resources']) {
    const dir=path.join(directory,rel);fs.mkdirSync(dir,{recursive:true,mode:0o755});
    if(process.platform!=='win32')fs.chmodSync(dir,0o755);
  }
  for (const [rel,data] of input) {
    const file=path.join(directory,rel);fs.mkdirSync(path.dirname(file),{recursive:true,mode:0o755});
    fs.writeFileSync(file,data,{flag:'wx',mode:rel.includes('/MacOS/')?0o755:0o644});
    if(process.platform!=='win32') fs.chmodSync(file,rel.includes('/MacOS/')?0o755:0o644);
  }
  structure(directory,v);return directory;
}
function manifest(directory) {
  const arch=structure(directory);
  const files=walk(directory).filter(n=>!['bundle-manifest.json','SHA256SUMS'].includes(n)).map(file=>({path:file,sha256:sha(read(path.join(directory,file)))}));
  const record={schema:1,version:version(),platform:'macos-'+arch,minimum_macos:MINIMUM,identity:ID,lua:luaInfo(directory,arch),files};
  fs.writeFileSync(path.join(directory,'bundle-manifest.json'),JSON.stringify(record,null,2)+'\n',{flag:'wx',mode:0o644});
  fs.writeFileSync(path.join(directory,'SHA256SUMS'),[...files,{path:'bundle-manifest.json',sha256:sha(read(path.join(directory,'bundle-manifest.json')))}]
    .map(f=>f.sha256+'  '+f.path+'\n').join(''),{flag:'wx',mode:0o644});
  verify(directory);return record;
}
function verify(directory) {
  const data=JSON.parse(read(path.join(directory,'bundle-manifest.json'),1024*1024));
  assert(data.schema===1&&['macos-arm64','macos-x64'].includes(data.platform)&&data.identity===ID&&data.minimum_macos===MINIMUM,'Invalid bundle manifest');
  assert.deepEqual(data.lua,luaInfo(directory),'Invalid Lua bundle binding');
  assert.equal(data.platform,'macos-'+structure(directory,data.version),'Manifest architecture does not match the app');
  const files=walk(directory).filter(n=>!['bundle-manifest.json','SHA256SUMS'].includes(n));
  assert.deepEqual(data.files.map(f=>f.path),files,'Manifest inventory mismatch');
  for(const f of data.files) assert.equal(sha(read(path.join(directory,f.path))),f.sha256,'Hash mismatch: '+f.path);
  const expectedSums=[...data.files,{path:'bundle-manifest.json',sha256:sha(read(path.join(directory,'bundle-manifest.json')))}].map(f=>f.sha256+'  '+f.path+'\n').join('');
  assert.equal(read(path.join(directory,'SHA256SUMS'),1024*1024).toString(),expectedSums,'Checksum sidecar mismatch');return data;
}
function run(program,args) {
  const result=cp.spawnSync(program,args,{encoding:'utf8',timeout:60000,maxBuffer:4*1024*1024});
  if(result.error)throw result.error;assert.equal(result.status,0,path.basename(program)+': '+result.stderr);return result.stdout;
}
function nativeVerify(directory) {
  assert.equal(process.platform,'darwin','Native bundle validation requires macOS');const arch=structure(directory);
  const app=path.join(directory,'Rocket.app');run('/usr/bin/plutil',['-lint',path.join(app,'Contents/Info.plist')]);
  const executables=BINARIES.map(name=>path.join(app,'Contents/MacOS',name));
  if(luaInfo(directory)!=='disabled') {
    LUA.native(path.join(directory,LUA.PREFIX));
    executables.push(path.join(directory,LUA.PREFIX,LUA.EXE));
  }
  for(const file of executables) {
    assert.equal(run('/usr/bin/lipo',['-archs',file]).trim(),arch==='arm64'?'arm64':'x86_64','Unexpected architecture');
    const lines=run('/usr/bin/otool',['-L',file]).trimEnd().split('\n').slice(1);
    for(const line of lines) assert(/^\s+(\/System\/Library\/|\/usr\/lib\/)/.test(line),'Non-system dynamic dependency: '+line.trim());
    assert(!/cmd LC_RPATH\b/.test(run('/usr/bin/otool',['-l',file])),'Unexpected dynamic-library search path');
    run('/usr/bin/codesign',['--verify','--strict','--all-architectures',file]);
  }
  run('/usr/bin/codesign',['--verify','--deep','--strict','--all-architectures',app]);
}
function iconset(directory) {
  const expected=[];
  for(const points of [16,32,128,256,512])for(const scale of [1,2]) {
    const name=`icon_${points}x${points}${scale===2?'@2x':''}.png`;expected.push(name);
    const png=read(path.join(directory,name),8*1024*1024);
    assert(png.length>=33&&png.subarray(0,8).toString('hex')==='89504e470d0a1a0a','Invalid icon PNG');
    assert.equal(png.readUInt32BE(16),points*scale,'Icon width mismatch');assert.equal(png.readUInt32BE(20),points*scale,'Icon height mismatch');
  }
  assert.deepEqual(walk(directory).sort(),expected.sort(),'Iconset inventory mismatch');
}
function signDevelopment(directory) {
  assert.equal(process.platform,'darwin','Development signing requires macOS');structure(directory);
  assert(!fs.existsSync(path.join(directory,'bundle-manifest.json')),'Never sign over a finalized package');
  const app=path.join(directory,'Rocket.app');
  // Sign nested executable code first, then the app. No key/certificate, no
  // host entitlements, no --deep signing and no Gatekeeper/security-policy changes.
  // The already-signed Lua sub-app MUST stay byte-identical to the host binding.
  const luaBefore=luaInfo(directory);
  for(const name of BINARIES.filter(n=>n!=='rocket')) run('/usr/bin/codesign',['--force','--sign','-','--timestamp=none','--identifier',ID+'.'+name,path.join(app,'Contents/MacOS',name)]);
  run('/usr/bin/codesign',['--force','--sign','-','--timestamp=none','--identifier',ID,app]);
  assert.deepEqual(luaInfo(directory),luaBefore,'Signing changed the bound Lua helper');
  nativeVerify(directory);manifest(directory);
}
module.exports={BINARIES,ID,version,read,macho,architecture,icns,iconset,plist,walk,stage,structure,manifest,verify,nativeVerify,signDevelopment};
if(require.main===module) {
  try {
    const [command,...args]=process.argv.slice(2);
    if(command==='stage'&&args.length===4)stage(...args);
    else if(command==='finalize'&&args.length===1)signDevelopment(path.resolve(args[0]));
    else if(command==='verify'&&args.length===1){verify(path.resolve(args[0]));if(process.platform==='darwin')nativeVerify(path.resolve(args[0]));}
    else if(command==='iconset'&&args.length===1)iconset(path.resolve(args[0]));
    else throw Error('usage: node tools/macos/bundle.cjs stage BINARY_DIR ICON.icns NEW_PACKAGE_DIR BUILD.md | finalize PACKAGE_DIR | verify PACKAGE_DIR | iconset DIRECTORY');
    console.log('PASS macOS bundle '+command);
  }catch(error){console.error(error.message);process.exitCode=1;}
}
