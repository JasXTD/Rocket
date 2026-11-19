'use strict';
// Package-only regressions; generated credentials never leave the temporary fixture.
const {test}=require('node:test'), assert=require('node:assert/strict');
const fs=require('node:fs'), path=require('node:path'), os=require('node:os'), crypto=require('node:crypto');
const {audit,secretKinds,sensitivePath,publicFixture}=require('../windows/audit-payload.cjs');
const pem=()=>crypto.generateKeyPairSync('ed25519').privateKey.export({type:'pkcs8',format:'pem'});

test('normal code, placeholder labels and public keys do not look like credentials',()=>{
  const pub=crypto.generateKeyPairSync('ed25519').publicKey.export({type:'spki',format:'pem'});
  // Generate a code sample containing a synthetic acknowledgment, not a credential.
  const message=['AV_REQUESTED','malwarebytes'].join(':')+'\n';
  const sample=`const token = ${JSON.stringify(message)};\n${pub}`;
  assert.deepEqual(secretKinds(Buffer.from(sample)),[]);
  assert.deepEqual(secretKinds(Buffer.from('-----BEGIN PRIVATE KEY-----\nexample\n-----END PRIVATE KEY-----')),[]);
  assert.deepEqual(secretKinds(Buffer.from('AK'+'IAIOSFODNN7EXAMPLE AK'+'IAXXXXXXXXXXXXXXXX')),[]);
});

test('private keys in ordinary or escaped content and provider tokens are refused',()=>{
  const key=pem();
  assert.deepEqual(secretKinds(Buffer.from(key)),['private_key']);
  assert.deepEqual(secretKinds(Buffer.from(JSON.stringify({value:key}))),['private_key']);
  assert.deepEqual(secretKinds(Buffer.from(['ghp',crypto.randomBytes(18).toString('hex')].join('_'))),['github_token']);
  assert.deepEqual(secretKinds(Buffer.from(['sk','ant-api03',crypto.randomBytes(24).toString('hex')].join('-'))),['ai_provider_token']);
  assert.deepEqual(secretKinds(Buffer.from('AK'+'IA'+crypto.randomBytes(8).toString('hex').toUpperCase())),['aws_access_key']);
});

test('OpenAI keys require their complete fingerprint, not adjacent compiled labels',()=>{
  const generated=n=>crypto.randomBytes(n).toString('base64url').slice(0,n);
  for(const prefix of ['proj','svcacct','admin']) {
    // Linkers may pack a key-format prefix directly beside unrelated constants.
    const labels=['branch','request','worktree','status','record','provider','arguments','changes'].join('');
    assert.deepEqual(secretKinds(Buffer.from(['sk',prefix,labels].join('-'))),[]);
    for(const before of [58,74])for(const after of [58,74]) {
      const key=['sk',prefix,generated(before)+'T3BlbkFJ'+generated(after)].join('-');
      assert.deepEqual(secretKinds(Buffer.from(key)),['ai_provider_token']);
      assert.deepEqual(secretKinds(Buffer.from(JSON.stringify({key}))),['ai_provider_token']);
      assert.deepEqual(secretKinds(Buffer.from(key+'x')),[]);
    }
  }
  const legacy='sk-'+crypto.randomBytes(10).toString('hex')+'T3BlbkFJ'+crypto.randomBytes(10).toString('hex');
  assert.deepEqual(secretKinds(Buffer.from(legacy)),['ai_provider_token']);
});

test('development folders, keys and profiles are never package inputs',()=>{
  for(const name of ['.git/config','docs/.ssh/config','target/release/x','work/result.json','.env','id_ed25519','identity.pfx','settings.bin'])assert(sensitivePath(name),name);
  for(const name of ['rocket.exe','docs/Git.md','git/2.56.0/cmd/git.exe','runtime/README.md'])assert(!sensitivePath(name),name);
});

test('inventory and local-key comparison are read-only, with no credential values in findings',t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'rocket-installer-audit-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const bundle=path.join(root,'bundle'),ssh=path.join(root,'ssh');fs.mkdirSync(bundle);fs.mkdirSync(ssh);
  const key=pem(),pub='ssh-ed25519 '+crypto.randomBytes(48).toString('base64')+' synthetic';
  fs.writeFileSync(path.join(ssh,'key'),key);fs.writeFileSync(path.join(ssh,'key.pub'),pub);
  fs.writeFileSync(path.join(bundle,'ordinary.md'),'Documentation');
  const clean=audit(bundle,{sshHome:ssh});assert(clean.passed);assert.equal(clean.files,1);
  assert.equal(clean.localPrivateKeysCompared,1);assert.equal(clean.localPublicKeysCompared,1);
  fs.writeFileSync(path.join(bundle,'private.md'),key);fs.writeFileSync(path.join(bundle,'public.md'),pub);
  const dirty=audit(bundle,{sshHome:ssh});assert(!dirty.passed);assert(dirty.localKeysUnchanged);
  assert(dirty.findings.some(f=>f.kind==='local_private_key_match'));
  assert(dirty.findings.some(f=>f.kind==='local_public_key_match'));
  assert(!JSON.stringify(dirty).includes(key));assert(!JSON.stringify(dirty).includes(pub));
  assert.equal(fs.readFileSync(path.join(ssh,'key'),'utf8'),key);
  assert.equal(fs.readFileSync(path.join(ssh,'key.pub'),'utf8'),pub);
});

test('linked package paths are refused instead of traversing outside',t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'rocket-installer-links-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const bundle=path.join(root,'bundle'),outside=path.join(root,'outside');fs.mkdirSync(bundle);fs.mkdirSync(outside);
  fs.writeFileSync(path.join(outside,'sentinel'),'preserve');
  fs.symlinkSync(outside,path.join(bundle,'linked'),process.platform==='win32'?'junction':'dir');
  assert.throws(()=>audit(bundle),/linked/);assert.equal(fs.readFileSync(path.join(outside,'sentinel'),'utf8'),'preserve');
});

test('the reviewed vendor fixture record cannot hide changed bytes or another finding',()=>{
  const name='git/2.56.0.windows.4/usr/bin/msys-gnutls-30.dll';
  const hash='5f3019ca853a642a5d26b81661040f295cc11cfc588a43500fbcb2fbee7cc444';
  assert(publicFixture(name,hash,'private_key'));
  assert(publicFixture('git/2.56.1.windows.1/usr/bin/msys-gnutls-30.dll',hash,'private_key'));
  assert(!publicFixture('git/unreviewed/usr/bin/msys-gnutls-30.dll',hash,'private_key'));
  assert(!publicFixture(name,hash.slice(0,-1)+'5','private_key'));
  assert(!publicFixture('other.dll',hash,'private_key'));
  assert(!publicFixture(name,hash,'github_token'));
});
