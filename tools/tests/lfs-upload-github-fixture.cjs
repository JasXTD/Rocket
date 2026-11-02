// Preparation and independent read-back only. Rocket's native UI performs uploads.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {create}=require('./mcp-github-fixture.cjs');
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const name='素材/アップロード 🚀 e\u0301.dat';
const cache=(repo,oid)=>path.join(repo,'.git/lfs/objects',oid.slice(0,2),oid.slice(2,4),oid);
function snapshot(f){return{head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.client,'.git/index')),refs:f.git(['for-each-ref','--format=%(refname) %(objectname)']),files:[name,'.gitattributes','ROCKET_PRIVATE_file','untracked 文'].map(n=>[n,hash(path.join(f.client,n))]),config:f.git(['config','--null','--list','--includes']).split('\0').filter(r=>!/^lfs\.(repositoryformatversion\n0$|https:\/\/github\.com\/JasXTD\/rocket-auth-test\.git\/info\/lfs\.access\n(basic|none|negotiate)$)/.test(r)).join('\0')}}
function prepare(mode){const f=create(mode);try{
 f.run('setup');const payload=Buffer.from('Rocket synthetic LFS upload '+mode+' '+crypto.randomUUID()+'\n');assert(payload.length<128);const oid=crypto.createHash('sha256').update(payload).digest('hex');
 const pointer=`version https://git-lfs.github.com/spec/v1\noid sha256:${oid}\nsize ${payload.length}\n`;
 fs.mkdirSync(path.dirname(path.join(f.client,name)),{recursive:true});fs.writeFileSync(path.join(f.client,name),pointer);fs.writeFileSync(path.join(f.client,'.gitattributes'),'*.dat filter=lfs diff=lfs merge=lfs -text\n');
 f.git(['add','--','.gitattributes',name]);f.git(['commit','-m','Synthetic LFS upload test']);const head=f.git(['rev-parse','HEAD']);f.r.owned.push(head);f.r.upload={oid,head,bytes:payload.length,path:name};f.save();
 fs.mkdirSync(path.dirname(cache(f.client,oid)),{recursive:true});fs.writeFileSync(cache(f.client,oid),payload);
 fs.writeFileSync(path.join(f.client,'.git/rocket-lfs-upload-fixture'),`ROCKET-LFS-UPLOAD-1\n${head}\n${oid}\n${payload.length}\n`);
 fs.writeFileSync(path.join(f.client,'ROCKET_PRIVATE_file'),'PRIVATE staged sentinel\n');f.git(['add','--','ROCKET_PRIVATE_file']);fs.writeFileSync(path.join(f.client,'ROCKET_PRIVATE_file'),'PRIVATE unstaged sentinel\n');fs.writeFileSync(path.join(f.client,'untracked 文'),'PRIVATE keep untracked\n');
 return Object.assign(f,{payload,oid,head,pointer});
 }catch(e){f.r.failure=e.message;f.save();try{if(f.r.baseline)f.run('cleanup')}catch{}throw e}}
function publish(f){const protocol=f.env.GIT_ALLOW_PROTOCOL;try{f.env.GIT_ALLOW_PROTOCOL='file';f.git(['fetch','--no-tags','--no-recurse-submodules','--',f.client,f.head],f.publisher)}finally{f.env.GIT_ALLOW_PROTOCOL=protocol}f.leasedWrite(f.head,'');f.r.expected_tip=f.head;f.save();f.assertRemote(f.head);}
function download(f){const reader=path.join(f.root,'fresh-download');assert(!fs.existsSync(reader));const auth=[];
 if(f.mode==='https'){auth.push('-c','credential.helper=');const helper=f.git(['config','--get-all','credential.helper'],f.publisher).trim();assert(helper.startsWith('!f()'));auth.push('-c','credential.helper='+helper)}
 if(f.mode==='ssh')auth.push('-c','core.sshCommand='+f.git(['config','--get','core.sshCommand'],f.publisher));
 f.git([...auth,'clone','--no-checkout','--single-branch','--no-tags','--branch',f.branch,'--',f.remote,reader],f.publisher);f.configureAuth(reader,true);
 assert.equal(f.git(['rev-parse','HEAD'],reader),f.head);assert(!fs.existsSync(cache(reader,f.oid)));f.git(['lfs','fetch','origin',f.head],reader);assert.equal(hash(cache(reader,f.oid)),f.oid);assert.deepEqual(fs.readFileSync(cache(reader,f.oid)),f.payload);return reader;}
module.exports={prepare,snapshot,publish,download,hash,cache,name};
