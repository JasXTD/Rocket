'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os');
const pack=require('../macos/update-package.cjs'),b=require('../macos/bundle.cjs');
test('Mac update metadata covers exact signed app bytes outside the sealed bundle',()=>{
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'rocket-mac-update-')),app=path.join(root,'Rocket.app');
 try { for(const n of b.BINARIES){const file=path.join(app,'Contents/MacOS',n);fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,'synthetic '+n);}
 const resource=path.join(app,'Contents/Resources/資料.md');fs.mkdirSync(path.dirname(resource),{recursive:true});fs.writeFileSync(resource,'Unicode');
 const first=pack.metadata(app,'9.0.0');assert.equal(first.platform,'macos-arm64');assert.equal(first.entrypoint,'Contents/MacOS/rocket');assert(first.files.some(f=>f.path.endsWith('資料.md')));assert(!fs.existsSync(path.join(app,'rocket-update.json')));
 fs.appendFileSync(resource,'changed');assert.notDeepEqual(pack.metadata(app,'9.0.0').files,first.files);
 fs.unlinkSync(path.join(app,'Contents/MacOS/rocket-ssh'));assert.throws(()=>pack.metadata(app,'9.0.0'),/Missing helper/);
 assert.throws(()=>pack.metadata(app,'9.0.0-beta'),/Invalid release/);
 } finally {fs.rmSync(root,{recursive:true,force:true});}
});
