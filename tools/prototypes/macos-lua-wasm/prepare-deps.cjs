'use strict';
// Exact copies of the archives approved, hashed, source-reviewed and scanned on
// 2026-11-09. Stages a private offline Cargo directory, never a system install.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const zlib = require('node:zlib'), assert = require('node:assert/strict');
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const cstring = b => b.subarray(0, b.indexOf(0) < 0 ? b.length : b.indexOf(0)).toString('utf8');
function extractCrate(archive, pkg, parent) {
  assert.equal(sha(archive), pkg.checksum, 'Crate checksum mismatch');
  const name = pkg.name + '-' + pkg.version;
  assert(/^[A-Za-z0-9_.+-]+$/.test(name));
  const tar = zlib.gunzipSync(archive, { maxOutputLength: 96 * 1024 * 1024 });
  const files = Object.create(null), entries = new Set(), planned = [];
  let longName = null;
  // Validate the complete archive before creating any file. Links and extension
  // headers other than bounded GNU long names are refused.
  for (let offset = 0; offset + 512 <= tar.length;) {
    const h = tar.subarray(offset, offset + 512);
    if (h.every(b => b === 0)) break;
    assert.equal([...h].reduce((sum, b, i) => sum + (i >= 148 && i < 156 ? 32 : b), 0),
      Number.parseInt(cstring(h.subarray(148, 156)).trim(), 8), 'TAR checksum');
    const size = Number.parseInt(cstring(h.subarray(124, 136)).trim() || '0', 8);
    const type = String.fromCharCode(h[156]);
    assert(Number.isSafeInteger(size) && size >= 0 && size <= 64 * 1024 * 1024 && offset + 512 + size <= tar.length);
    const bytes = tar.subarray(offset + 512, offset + 512 + size);
    offset += 512 + Math.ceil(size / 512) * 512;
    if (type === 'L') { assert(longName === null && size > 0 && size <= 4096); longName = cstring(bytes); continue; }
    const prefix = h.subarray(257, 263).equals(Buffer.from('ustar\0')) ? cstring(h.subarray(345, 500)) : '';
    const entry = longName ?? ((prefix ? prefix + '/' : '') + cstring(h.subarray(0, 100))); longName = null;
    assert(entry.startsWith(name + '/') && !entry.includes('\\') && !entry.includes(':'));
    const rel = entry.slice(name.length + 1).replace(/\/$/, '');
    assert(rel && rel.split('/').every(part => part && part !== '.' && part !== '..'));
    assert(!entries.has(rel), 'Duplicate source path'); entries.add(rel);
    assert(type === '0' || type === '\0' || type === '5', 'Non-regular source entry');
    planned.push({ rel, bytes, directory: type === '5' });
    if (type !== '5') files[rel] = sha(bytes);
  }
  assert(longName === null && files['Cargo.toml'], 'Incomplete crate archive');
  const dir = path.join(parent, name); fs.mkdirSync(dir);
  for (const entry of planned) {
    const dest = path.join(dir, entry.rel); assert(dest.startsWith(dir + path.sep));
    if (entry.directory) fs.mkdirSync(dest, { recursive: true });
    else { fs.mkdirSync(path.dirname(dest), { recursive: true }); fs.writeFileSync(dest, entry.bytes, { flag: 'wx' }); }
  }
  fs.writeFileSync(path.join(dir, '.cargo-checksum.json'), JSON.stringify({ files, package: pkg.checksum }), { flag: 'wx' });
}
async function download(dir, name, url, checksum, limit) {
  assert(/^[A-Za-z0-9_.+-]+$/.test(name) && /^[a-f0-9]{64}$/.test(checksum));
  let source = new URL(url);
  assert(source.protocol === 'https:' && !source.username && !source.password);
  assert(['static.crates.io', 'static.rust-lang.org', 'github.com'].includes(source.hostname));
  assert(!/\.txt$/i.test(decodeURIComponent(source.pathname)));
  let response = await fetch(source, { redirect: 'manual', signal: AbortSignal.timeout(180000) });
  if (response.status === 302) {
    assert.equal(source.hostname, 'github.com'); source = new URL(response.headers.get('location'));
    assert(source.protocol === 'https:' && source.hostname === 'release-assets.githubusercontent.com' && !source.username && !source.password);
    assert(!/\.txt$/i.test(decodeURIComponent(source.pathname)));
    response = await fetch(source, { redirect: 'error', signal: AbortSignal.timeout(180000) });
  }
  assert(response.ok && !/\.txt(?:["';\s]|$)/i.test(response.headers.get('content-disposition') || ''), 'Download refused');
  assert(Number(response.headers.get('content-length') || 0) <= limit);
  const output = path.join(dir, name), fd = fs.openSync(output, 'wx', 0o600), hash = crypto.createHash('sha256');
  let bytes = 0;
  try { for await (const chunk of response.body) { bytes += chunk.length; assert(bytes <= limit); hash.update(chunk); fs.writeSync(fd, chunk); } }
  finally { fs.closeSync(fd); }
  assert.equal(hash.digest('hex'), checksum, 'Download checksum mismatch');
  return output;
}
async function main() {
  const [root, mode] = process.argv.slice(2);
  assert(root && path.isAbsolute(root) && ['guest', 'host'].includes(mode));
  fs.mkdirSync(root, { mode: 0o700 }); // Fail on existing state; no overwrite/cleanup.
  const archives = path.join(root, 'archives'), vendor = path.join(root, 'vendor'), cargo = path.join(root, 'cargo-home');
  for (const dir of [archives, vendor, cargo]) fs.mkdirSync(dir, { mode: 0o700 });
  const packages = require('./approved-crates.json').concat(mode === 'guest' ? require('./approved-unwind-crates.json') : []);
  for (let start = 0; start < packages.length; start += 4) {
    const results = await Promise.allSettled(packages.slice(start, start + 4).map(async pkg => {
      const name = pkg.name + '-' + pkg.version + '.crate';
      const file = await download(archives, name, 'https://static.crates.io/crates/' + pkg.name + '/' + name, pkg.checksum, 64 * 1024 * 1024);
      extractCrate(fs.readFileSync(file), pkg, vendor);
    }));
    const failures = results.filter(result => result.status === 'rejected');
    if (failures.length) throw new AggregateError(failures.map(result => result.reason), failures.map(result => result.reason.message).join('; '));
  }
  if (mode === 'guest') for (const tool of require('./approved-tools.json'))
    await download(archives, tool.name, tool.url, tool.sha256, tool.bytes);
  fs.writeFileSync(path.join(cargo, 'config.toml'), '[source.crates-io]\nreplace-with = "approved-local"\n[source.approved-local]\ndirectory = ' + JSON.stringify(vendor) + '\n[net]\noffline = true\n', { flag: 'wx' });
  console.log('Verified and staged ' + packages.length + ' approved source archives for offline builds.');
}
module.exports = { extractCrate, sha };
if (require.main === module) main().catch(error => { console.error(error.message); process.exitCode = 1; });
