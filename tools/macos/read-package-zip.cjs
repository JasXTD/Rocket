'use strict';
// Bounded extraction for our own digest-pinned CI packages. No archive links,
// special files, duplicate paths, encryption or ZIP64 are needed by this format.
const fs=require('node:fs'),path=require('node:path'),zlib=require('node:zlib'),assert=require('node:assert/strict');
const utf8=new TextDecoder('utf-8',{fatal:true});
function inventory(bytes){
 assert(bytes.length>=22&&bytes.length<=512*1024*1024,'Archive size out of bounds');
 let end=-1;
 for(let at=bytes.length-22;at>=Math.max(0,bytes.length-65557);at--){
  if(bytes.readUInt32LE(at)===0x06054b50&&at+22+bytes.readUInt16LE(at+20)===bytes.length){end=at;break;}
 }
 assert(end>=0,'Missing ZIP directory');assert.equal(bytes.readUInt16LE(end+4),0);assert.equal(bytes.readUInt16LE(end+6),0);
 const count=bytes.readUInt16LE(end+10),offset=bytes.readUInt32LE(end+16),size=bytes.readUInt32LE(end+12);
 assert(count>0&&count<=20000&&count===bytes.readUInt16LE(end+8));assert.equal(offset+size,end,'Unsupported directory layout');
 const entries=[],seen=new Set();let at=offset,total=0;
 for(let i=0;i<count;i++){
  assert(at+46<=end&&bytes.readUInt32LE(at)===0x02014b50,'Invalid directory entry');
  const flags=bytes.readUInt16LE(at+8),method=bytes.readUInt16LE(at+10),crc=bytes.readUInt32LE(at+16),packed=bytes.readUInt32LE(at+20),length=bytes.readUInt32LE(at+24);
  const n=bytes.readUInt16LE(at+28),extra=bytes.readUInt16LE(at+30),comment=bytes.readUInt16LE(at+32),attrs=bytes.readUInt32LE(at+38),local=bytes.readUInt32LE(at+42);
  assert((flags&~0x80e)===0&&[0,8].includes(method),'Encrypted or unsupported ZIP');assert.equal(bytes.readUInt16LE(at+34),0);
  assert(n>0&&at+46+n+extra+comment<=end);const rawName=bytes.subarray(at+46,at+46+n),name=utf8.decode(rawName),directory=name.endsWith('/');
  const components=(directory?name.slice(0,-1):name).split('/');
  assert(!/[\\:\x00-\x1f\x7f]/.test(name)&&!name.startsWith('/')&&components.every(c=>c&&c!=='.'&&c!=='..'),'Unsafe archive path');
  const key=components.join('/').normalize('NFD').toLowerCase();assert(!seen.has(key),'Duplicate archive path');seen.add(key);
  const mode=attrs>>>16,type=mode&0xf000;
  assert([0,0x4000,0x8000].includes(type)&&!(mode&0xe00)&&!(attrs&0x400),'Archive link or special file');
  assert(type!==0x4000||directory);assert(type!==0x8000||!directory);
  total+=length;assert(length<=256*1024*1024&&total<=1024*1024*1024&&packed<=bytes.length,'Expanded archive too large');
  assert(local+30<=offset&&bytes.readUInt32LE(local)===0x04034b50,'Invalid local header');
  assert.equal(bytes.readUInt16LE(local+6),flags);assert.equal(bytes.readUInt16LE(local+8),method);
  const localName=bytes.readUInt16LE(local+26),localExtra=bytes.readUInt16LE(local+28),data=local+30+localName+localExtra;
  assert(localName===n&&bytes.subarray(local+30,local+30+n).equals(rawName),'Conflicting archive names');assert(data+packed<=offset,'Overlapping archive directory');
  if(directory)assert.equal(length,0);if(method===0)assert.equal(length,packed);
  entries.push({name,directory,mode:mode&0o777,length,packed,data,crc,method});at+=46+n+extra+comment;
 }
 assert.equal(at,end);return entries;
}
function content(bytes,entry){
 const packed=bytes.subarray(entry.data,entry.data+entry.packed);
 const data=entry.method===0?packed:zlib.inflateRawSync(packed,{maxOutputLength:Math.max(1,entry.length)});
 assert.equal(data.length,entry.length);assert.equal(zlib.crc32(data),entry.crc,'Archive CRC differs');return data;
}
function extract(bytes,destination){
 const entries=inventory(bytes);assert(!fs.existsSync(destination),'Extraction already exists');
 // Verify all data before writing; failures cannot leave a half-accepted tree.
 for(const entry of entries)content(bytes,entry);
 fs.mkdirSync(destination,{recursive:false,mode:0o700});const base=fs.realpathSync(destination)+path.sep;
 for(const entry of entries){
  const target=path.resolve(destination,...entry.name.split('/'));assert(target.startsWith(base));
  if(entry.directory)fs.mkdirSync(target,{recursive:true,mode:0o700});
  else{fs.mkdirSync(path.dirname(target),{recursive:true,mode:0o700});fs.writeFileSync(target,content(bytes,entry),{flag:'wx',mode:entry.mode||0o600});if(process.platform!=='win32')fs.chmodSync(target,entry.mode||0o600);}
 }
 return entries;
}
module.exports={inventory,content,extract};
