// Synthetic fixture setup only. Native Rocket performs downloads under test.
// Existing GCM entries are get-only; SSH uses the existing agent and public key.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {create,Fixture}=require('./mcp-github-fixture.cjs');
const payload=Buffer.from('Rocket disposable LFS authentication fixture v1\nSynthetic content only.\n');
const oid=crypto.createHash('sha256').update(payload).digest('hex');
const name='素材/認証 🚀 e\u0301.dat';
const pointer=`version https://git-lfs.github.com/spec/v1\noid sha256:${oid}\nsize ${payload.length}\n`;
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const cache=f=>path.join(f.client,'.git/lfs/objects',oid.slice(0,2),oid.slice(2,4),oid);
function clearCache(f){const p=cache(f);if(fs.existsSync(p)){assert.equal(hash(p),oid);assert(p.startsWith(f.client+path.sep));fs.unlinkSync(p)}}
function snapshot(f){return {head:f.git(['rev-parse','HEAD']),index:hash(path.join(f.client,'.git/index')),refs:f.git(['for-each-ref','--format=%(refname) %(objectname)']),files:[name,'.gitattributes','ROCKET_PRIVATE_file','untracked 文'].map(n=>[n,hash(path.join(f.client,n))])}}
function prepare(mode){
 const f=create(mode);try{
  f.run('setup');
  fs.mkdirSync(path.dirname(path.join(f.client,name)),{recursive:true});
  fs.writeFileSync(path.join(f.client,'.gitattributes'),'*.dat filter=lfs diff=lfs merge=lfs -text\n');
  fs.writeFileSync(path.join(f.client,name),pointer);
  f.git(['add','--','.gitattributes',name]);f.git(['commit','-m','Synthetic LFS authentication fixture']);
  const head=f.git(['rev-parse','HEAD']);f.r.owned.push(head);f.r.expected_tip=head;f.r.lfs={oid,bytes:payload.length,path:name,upload_attempted:false};f.save();
  fs.mkdirSync(path.dirname(cache(f)),{recursive:true});fs.writeFileSync(cache(f),payload);
  f.r.lfs.upload_attempted=true;f.save();
  // Exactly one known sub-1-KB object; no hooks, --all or history migration.
  f.git(['lfs','push','--object-id','origin',oid]);f.r.lfs.uploaded=true;f.save();
  const protocol=f.env.GIT_ALLOW_PROTOCOL;
  try { f.env.GIT_ALLOW_PROTOCOL='file';f.git(['fetch','--no-tags','--no-recurse-submodules','--',f.client,head],f.publisher); }
  finally { f.env.GIT_ALLOW_PROTOCOL=protocol; }
  f.leasedWrite(head,'');f.assertRemote(head);
  f.git(['fetch','--no-tags','--no-prune','--no-recurse-submodules','origin',`${f.ref}:${f.tracking}`]);
  fs.writeFileSync(path.join(f.client,'ROCKET_PRIVATE_file'),'Staged fixture bytes\n');f.git(['add','--','ROCKET_PRIVATE_file']);
  fs.writeFileSync(path.join(f.client,'ROCKET_PRIVATE_file'),'Unstaged fixture bytes\n');fs.writeFileSync(path.join(f.client,'untracked 文'),'Preserve synthetic untracked content\n');
  clearCache(f);f.r.lfs.before=snapshot(f);f.save();return f;
 }catch(e){f.r.failure=e.message;f.save();try{if(f.r.baseline)f.run('cleanup')}catch(cleanup){f.r.cleanup_error=cleanup.message;f.save()}throw e}
}
module.exports={prepare,Fixture,payload,oid,name,pointer,hash,cache,clearCache,snapshot};
if(require.main===module){const mode=process.argv[2]||'local';const f=prepare(mode);try{assert(!fs.existsSync(cache(f)));assert.equal(fs.readFileSync(path.join(f.client,name),'utf8'),pointer);console.log(JSON.stringify({root:f.root,bytes:payload.length,mode,prepared:true}));}finally{f.run('cleanup')}}
