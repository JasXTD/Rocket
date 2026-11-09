'use strict';
// Actual directory swaps and process exit notifications with signed disposable apps.
// The driver invokes only a cfg(test) entry point; installed Rocket has no trust bypass.
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const b=require('../macos/bundle.cjs'),f=require('./app-update-fixtures.cjs');
assert.equal(process.platform,'darwin');
const [testExe,out]=process.argv.slice(2);assert(testExe&&out);
fs.mkdirSync(out,{recursive:true});const root=fs.mkdtempSync('/private/tmp/rocket-update-native-');
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,label){const end=performance.now()+20000;while(!fn()){assert(performance.now()<end,label+' timeout');await delay(25);}}
const old=f.probe(root,'0.92.1'),next=f.probe(root,'9.0.0'),results=[];
function sign(app){for(const name of b.BINARIES.filter(n=>n!=='rocket'))cp.execFileSync('/usr/bin/codesign',['--force','--sign','-','--timestamp=none',path.join(app,'Contents/MacOS',name)],{stdio:'pipe'});cp.execFileSync('/usr/bin/codesign',['--force','--sign','-','--timestamp=none','--identifier',b.ID,app],{stdio:'pipe'});}
function app(dest,version,bytes){fs.mkdirSync(path.join(dest,'Contents/MacOS'),{recursive:true});for(const name of b.BINARIES){const file=path.join(dest,'Contents/MacOS',name);fs.writeFileSync(file,bytes);fs.chmodSync(file,0o755);}fs.writeFileSync(path.join(dest,'Contents/Info.plist'),b.plist(version));fs.mkdirSync(path.join(dest,'Contents/Resources'));fs.writeFileSync(path.join(dest,'Contents/Resources/資料.md'),'keep '+version);sign(dest);}
async function scenario(mode){const folder=path.join(root,mode);fs.mkdirSync(folder);const target=path.join(folder,'Rocket.app'),stage=path.join(folder,'.rocket-next-test.app'),backup=path.join(folder,'.rocket-previous-test.app'),lock=path.join(folder,'.Rocket.app.rocket-update-lock'),job=path.join(folder,'job');fs.mkdirSync(job,{mode:0o700});app(target,'0.92.1',old);app(stage,'9.0.0',next);
 const oldMap=f.inventory(target),newMap=f.inventory(stage),plan=path.join(job,'install.json');fs.writeFileSync(lock,job);
 const script=`const fs=require('fs'),cp=require('child_process');const [exe,plan]=process.argv.slice(1);const data=JSON.parse(fs.readFileSync(plan));data.parent=process.pid;fs.writeFileSync(plan,JSON.stringify(data));const child=cp.spawn(exe,['--exact','app_updates::install::tests::mac_transaction_child','--nocapture'],{env:{...process.env,ROCKET_MAC_UPDATE_TEST_PLAN:plan},stdio:'ignore'});child.unref();process.stdin.once('data',()=>process.exit(0));process.stdin.resume();`;
 fs.writeFileSync(plan,JSON.stringify({schema:1,target,stage,backup,lock,old:oldMap,new:newMap,parent:1,version:'9.0.0',repositories:[]}));
 const parent=cp.spawn(process.execPath,['-e',script,testExe,plan],{stdio:['pipe','ignore','pipe']});
 try {await until(()=>fs.existsSync(path.join(job,'ready')),'parent notification ready');
 if(mode==='cancel')fs.writeFileSync(path.join(job,'cancel'),'cancel');else if(mode!=='unapproved')fs.writeFileSync(path.join(job,'approved.json'),JSON.stringify({install:true}));
 if(mode==='stage-edit')fs.appendFileSync(path.join(stage,'Contents/Resources/資料.md'),'external');
 if(mode==='target-edit')fs.appendFileSync(path.join(target,'Contents/Resources/資料.md'),'external');
 if(mode==='restart-failure')fs.chmodSync(path.join(stage,'Contents/MacOS/rocket'),0o644);
 const expected=f.inventory(target);if(mode!=='cancel')parent.stdin.end('exit');
 // The receipt is published before restart; wait for transaction completion too.
 await until(()=>fs.existsSync(path.join(job,'result.json'))&&!fs.existsSync(lock),'transaction receipt');const receipt=JSON.parse(fs.readFileSync(path.join(job,'result.json')));
 const code=mode==='install'?'installed':mode==='restart-failure'?'rolled_back':['cancel','unapproved'].includes(mode)?'cancelled':'unchanged';assert.equal(receipt.code,code);
 assert.deepEqual(f.inventory(target),mode==='install'?newMap:expected);
 if(mode==='install'){assert.deepEqual(f.inventory(backup),oldMap);assert(!fs.existsSync(stage));}
 else {assert(!fs.existsSync(backup));assert(fs.existsSync(stage));}
 if(!['cancel','target-edit'].includes(mode)){await until(()=>fs.existsSync(path.join(job,'restarted')),'actual executable restarted');assert.equal(fs.readFileSync(path.join(job,'restarted'),'utf8').split('\n')[0],mode==='install'?'9.0.0':'0.92.1');}else{if(mode==='cancel')assert.equal(parent.exitCode,null);assert(!fs.existsSync(path.join(job,'restarted')));}
 assert(!fs.existsSync(lock));results.push({mode,code});console.log('PASS Mac update '+mode);
 }finally{if(parent.exitCode===null)parent.stdin.end('exit');await until(()=>parent.exitCode!==null,'owned parent exit');}}
(async()=>{let passed=false;try{for(const mode of ['install','cancel','unapproved','stage-edit','target-edit','restart-failure'])await scenario(mode);passed=true;}finally{fs.writeFileSync(path.join(out,'update-transactions.json'),JSON.stringify({passed,root,cases:results},null,2));}})().catch(error=>{console.error(error);process.exitCode=1;});
