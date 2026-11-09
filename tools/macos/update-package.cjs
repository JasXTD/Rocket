'use strict';
// Run after Developer ID signing and stapling. Manifest stays OUTSIDE the sealed app.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const bundle=require('./bundle.cjs');
const required=bundle.BINARIES.map(n=>'Contents/MacOS/'+n);
function records(app) {
  const files=bundle.walk(app); assert(files.length<=20000,'Too many update files');
  const entries=files.map(file=>{ const bytes=bundle.read(path.join(app,file)); return {path:file,size:bytes.length,sha256:crypto.createHash('sha256').update(bytes).digest('hex')}; });
  for(const name of required) assert(entries.some(e=>e.path===name),'Missing helper: '+name);
  assert(entries.reduce((n,e)=>n+e.size,0)<=1024*1024*1024,'Update exceeds unpacked limit');
  assert(entries.every(e=>e.size<=256*1024*1024),'Update file exceeds limit');
  return entries;
}
function metadata(app,version) {
  assert(/^\d+\.\d+\.\d+$/.test(version),'Invalid release version');
  return {schema:1,version,platform:'macos-arm64',entrypoint:required[0],files:records(app)};
}
function run(program,args) { return cp.execFileSync(program,args,{encoding:'utf8',stdio:['ignore','pipe','pipe'],timeout:60000,maxBuffer:1024*1024}); }
function publish(app,destination,team) {
  assert(process.platform==='darwin'&&process.arch==='arm64','Use an Apple Silicon Mac');
  assert(/^[A-Z0-9]{10}$/.test(team),'Expected the reviewed Developer ID Team ID');
  app=fs.realpathSync(app);destination=path.resolve(destination);assert(!fs.existsSync(destination),'Output already exists');
  const rule='=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = "'+team+'" and identifier "'+bundle.ID+'" and notarized';
  run('/usr/bin/codesign',['--verify','--deep','--strict','--all-architectures','--test-requirement',rule,app]);
  run('/usr/bin/xcrun',['stapler','validate',app]);
  const version=run('/usr/bin/plutil',['-extract','CFBundleShortVersionString','raw','-o','-',path.join(app,'Contents/Info.plist')]).trim();
  const data=metadata(app,version),name='Rocket-'+version+'-macos-arm64';
  for(const file of required) bundle.macho(bundle.read(path.join(app,file)));
  fs.mkdirSync(destination,{mode:0o700});const stage=path.join(destination,name);fs.mkdirSync(stage);const target=path.join(stage,'Rocket.app');fs.mkdirSync(target);
  for(const entry of data.files) { const dest=path.join(target,entry.path);fs.mkdirSync(path.dirname(dest),{recursive:true});fs.copyFileSync(path.join(app,entry.path),dest,fs.constants.COPYFILE_EXCL);fs.chmodSync(dest,fs.statSync(path.join(app,entry.path)).mode&0o777); }
  assert.deepEqual(records(target),data.files,'Input changed during packaging');
  run('/usr/bin/codesign',['--verify','--deep','--strict','--all-architectures','--test-requirement',rule,target]);
  run('/usr/bin/xcrun',['stapler','validate',target]);
  fs.writeFileSync(path.join(stage,'rocket-update.json'),JSON.stringify(data,null,2),{flag:'wx'});
  const archive=path.join(destination,name+'.tar.gz');
  run('/usr/bin/tar',['--format=ustar','--no-xattrs','--no-acls','--no-fflags','-czf',archive,'-C',destination,name]);
  const bytes=bundle.read(archive);assert(bytes.length<=512*1024*1024,'Archive exceeds update limit');
  const sha=crypto.createHash('sha256').update(bytes).digest('hex');fs.writeFileSync(archive+'.sha256',sha+'  '+path.basename(archive)+'\n',{flag:'wx'});
  return {archive,sha256:sha,version};
}
module.exports={metadata,records,publish};
if(require.main===module) {try{const args=process.argv.slice(2);assert(args.length===3,'usage: node tools/macos/update-package.cjs SIGNED_APP NEW_OUTPUT_FOLDER TEAM_ID');console.log(JSON.stringify(publish(...args)));}catch(error){console.error(error.message);process.exitCode=1;}}
