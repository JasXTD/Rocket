'use strict';
// Local, value-free package audit. Never uploads content or prints matching credentials.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const digest = b => crypto.createHash('sha256').update(b).digest('hex');
// All 12 embedded key blocks in this exact already-pinned Git DLL match the
// upstream public known-answer tests. A changed DLL is never covered by this record.
// https://gitlab.com/gnutls/gnutls/-/blob/master/lib/crypto-selftests-pk.c
function publicFixture(name,hash,kind) {
  return kind==='private_key' && ['2.56.0.windows.4','2.56.1.windows.1'].some(version=>
    name===`git/${version}/usr/bin/msys-gnutls-30.dll`) &&
    hash==='5f3019ca853a642a5d26b81661040f295cc11cfc588a43500fbcb2fbee7cc444';
}
function sensitivePath(name) {
  return /(?:^|\/)(?:\.git|\.ssh|\.aws|\.codex|\.claude|target|work)(?:\/|$)/i.test(name) ||
    /(?:^|\/)(?:\.env(?:\..*)?|id_(?:rsa|dsa|ecdsa|ed25519)(?:\..*)?|credentials|credentials\.json|settings\.bin)$/i.test(name) ||
    /\.(?:pfx|p12|ppk|pdb)$/i.test(name);
}
function secretKinds(bytes) {
  const text=bytes.toString('latin1').replaceAll('\\r','').replaceAll('\\n','\n'), kinds=[];
  const pem=/-----BEGIN ((?:OPENSSH |RSA |EC |DSA |ENCRYPTED )?PRIVATE KEY)-----([\s\S]{48,65536}?)-----END \1-----/g;
  for(const m of text.matchAll(pem)) {
    const body=m[2].replace(/\s/g,''); if(!/^[A-Za-z0-9+/]+=*$/.test(body))continue;
    const decoded=Buffer.from(body,'base64');
    if((m[1]==='OPENSSH PRIVATE KEY' && decoded.subarray(0,15).equals(Buffer.from('openssh-key-v1\0'))) ||
       (m[1]!=='OPENSSH PRIVATE KEY' && decoded[0]===0x30)) kinds.push('private_key');
  }
  if(/PuTTY-User-Key-File-[23]:[^\r\n]+[\s\S]{0,16384}Private-Lines:\s*[1-9]/.test(text)) kinds.push('putty_private_key');
  if(/\bgh[pousr]_[A-Za-z0-9]{36}\b|\bgithub_pat_[A-Za-z0-9_]{60,255}\b/.test(text))kinds.push('github_token');
  // As in rocket-git's guard, OpenAI keys have a complete payload and embedded
  // fingerprint. A linker can place a format prefix beside ordinary labels;
  // matching an arbitrary run after that prefix mistakes code for a credential.
  const openai=/\bsk-(?:proj|svcacct|admin)-(?:[A-Za-z0-9_-]{58}|[A-Za-z0-9_-]{74})T3BlbkFJ(?:[A-Za-z0-9_-]{58}|[A-Za-z0-9_-]{74})(?![A-Za-z0-9_-])|\bsk-[A-Za-z0-9]{20}T3BlbkFJ[A-Za-z0-9]{20}(?![A-Za-z0-9_-])/;
  if(openai.test(text)||/\bsk-ant-(?:api|admin)\d+-[A-Za-z0-9_-]{32,512}\b/.test(text))kinds.push('ai_provider_token');
  // AWS publishes example IDs with this suffix; repeated filler is not a credential.
  for(const m of text.matchAll(/\b(?:AKIA|ASIA|ABIA|ACCA)([A-Z0-9]{16})\b/g))
    if(!m[1].endsWith('EXAMPLE') && !/^(.)\1{15}$/.test(m[1])){kinds.push('aws_access_key');break;}
  return [...new Set(kinds)];
}
function audit(root,{sshHome}={}) {
  root=fs.realpathSync(root); const files=[],findings=[],keys=[],reviewedPublicFixtures=[];
  let sshEntries=[];
  if(sshHome)try { sshEntries=fs.readdirSync(sshHome,{withFileTypes:true}); }
  catch(e){if(e.code!=='ENOENT')throw Error('Local SSH comparison could not read the key directory; audit incomplete');}
  for(const e of sshEntries) {
    const p=path.join(sshHome,e.name); if(!e.isFile() || fs.lstatSync(p).isSymbolicLink() || fs.statSync(p).size>1024*1024)continue;
    const bytes=fs.readFileSync(p), pub=bytes.toString('utf8').match(/^(?:ssh-[\w-]+|ecdsa-[\w-]+|sk-[\w@.-]+)\s+([A-Za-z0-9+/]+=*)/);
    if(secretKinds(bytes).some(k=>k.includes('private_key')))keys.push({path:p,hash:digest(bytes),bytes,kind:'private'});
    else if(pub)keys.push({path:p,hash:digest(bytes),bytes:Buffer.from(pub[1]),kind:'public'});
  }
  let count=0, total=0; const pending=[''];
  while(pending.length) {
    const rel=pending.pop(),full=path.join(root,rel),s=fs.lstatSync(full);
    if(s.isSymbolicLink())throw Error('Package contains a linked path');
    if(s.isDirectory()){for(const n of fs.readdirSync(full))pending.push(rel?rel+'/'+n:n);continue;}
    if(!s.isFile() || s.size>256*1024*1024)throw Error('Package contains an unsupported file');
    const bytes=fs.readFileSync(full),hash=digest(bytes);count++;total+=bytes.length;files.push({path:rel,sha256:hash,size:bytes.length});
    if(sensitivePath(rel))findings.push({path:rel,kind:'sensitive_or_development_path'});
    for(const kind of secretKinds(bytes)) {
      if(publicFixture(rel,hash,kind))reviewedPublicFixtures.push({path:rel,kind,sha256:hash,reason:'Upstream public cryptographic self-test vectors'});
      else findings.push({path:rel,kind});
    }
    for(const kind of ['private','public'])if(keys.some(k=>k.kind===kind && (hash===k.hash || bytes.includes(k.bytes))))findings.push({path:rel,kind:'local_'+kind+'_key_match'});
  }
  for(const k of keys)if(digest(fs.readFileSync(k.path))!==k.hash)throw Error('A local key changed during the read-only comparison');
  files.sort((a,b)=>a.path.localeCompare(b.path,'en'));
  return {schema:1,checkedUtc:new Date().toISOString(),files:count,bytes:total,localPrivateKeysCompared:keys.filter(k=>k.kind==='private').length,localPublicKeysCompared:keys.filter(k=>k.kind==='public').length,localKeysUnchanged:true,findings,
    passed:findings.length===0,reviewedPublicFixtures,inventory:files};
}
module.exports={sensitivePath,secretKinds,publicFixture,audit};
if(require.main===module) {
  try {
    const [root,output,flag]=process.argv.slice(2);if(!root||!output)throw Error('Usage: node audit-payload.cjs BUNDLE REPORT [--compare-local-ssh]');
    if(flag && flag!=='--compare-local-ssh')throw Error('Unknown audit option');
    const r=audit(root,{sshHome:flag?path.join(process.env.USERPROFILE,'.ssh'):undefined});
    const report=Buffer.from(JSON.stringify(r,null,2)+'\n');
    fs.writeFileSync(output,report,{flag:'wx'});
    console.log(JSON.stringify({reportSha256:digest(report),passed:r.passed,files:r.files,bytes:r.bytes,localPrivateKeysCompared:r.localPrivateKeysCompared,localPublicKeysCompared:r.localPublicKeysCompared,localKeysUnchanged:r.localKeysUnchanged,findings:r.findings}));
    if(!r.passed)process.exitCode=1;
  }catch(e){console.error(e.message);process.exitCode=1;}
}
