'use strict';
const { test } = require('node:test'), assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), os = require('node:os'), zlib = require('node:zlib');
const { extractCrate, sha } = require('./prepare-deps.cjs');
function archive(entries) {
  const chunks = [];
  for (const [name, data = '', type = '0'] of entries) {
    const bytes = Buffer.from(data), header = Buffer.alloc(512);
    header.write(name); header.write(bytes.length.toString(8).padStart(11, '0'), 124);
    header[156] = type.charCodeAt(0); header.fill(32, 148, 156);
    header.write([...header].reduce((sum, byte) => sum + byte, 0).toString(8).padStart(6, '0') + '\0 ', 148);
    chunks.push(header, bytes, Buffer.alloc((512 - bytes.length % 512) % 512));
  }
  return zlib.gzipSync(Buffer.concat([...chunks, Buffer.alloc(1024)]));
}
function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rocket-prototype-extract-'));
  const owned = fs.realpathSync(root);
  t.after(() => { assert.equal(fs.realpathSync(root), owned); fs.rmSync(root, { recursive: true }); });
  return root;
}
test('extracts verified regular sources with directory-source checksums', t => {
  const root = fixture(t), bytes = archive([['example-1.0.0/Cargo.toml', '[package]'], ['example-1.0.0/src/lib.rs', '// fixture']]);
  extractCrate(bytes, { name: 'example', version: '1.0.0', checksum: sha(bytes) }, root);
  const checks = JSON.parse(fs.readFileSync(path.join(root, 'example-1.0.0/.cargo-checksum.json')));
  assert.equal(checks.files['src/lib.rs'], sha(Buffer.from('// fixture')));
  assert.equal(checks.package, sha(bytes));
});
for (const [label, entry] of [
  ['traversal', ['example-1.0.0/../../escape', 'x']],
  ['absolute path', ['/outside/escape', 'x']],
  ['symbolic link', ['example-1.0.0/link', '', '2']],
  ['hard link', ['example-1.0.0/link', '', '1']],
  ['duplicate file', ['example-1.0.0/Cargo.toml', 'second']],
]) test('refuses ' + label + ' before writing source', t => {
  const root = fixture(t), bytes = archive([['example-1.0.0/Cargo.toml', '[package]'], entry]);
  assert.throws(() => extractCrate(bytes, { name: 'example', version: '1.0.0', checksum: sha(bytes) }, root));
  assert.deepEqual(fs.readdirSync(root), []);
});
test('refuses changed archive bytes before decompression', t => {
  const root = fixture(t), bytes = archive([['example-1.0.0/Cargo.toml', 'fixture']]);
  assert.throws(() => extractCrate(bytes, { name: 'example', version: '1.0.0', checksum: '0'.repeat(64) }, root));
  assert.deepEqual(fs.readdirSync(root), []);
});
