const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const assert = require('node:assert/strict');
const test = require('node:test');
const { readDraftPayload } = require('./state-fixture.cjs');

function fixture(run) {
  const parent = path.resolve(os.tmpdir());
  const root = fs.mkdtempSync(path.join(parent, 'rocket-state-reader-'));
  try { run(root); } finally {
    assert.equal(path.dirname(path.resolve(root)), parent);
    assert(path.basename(root).startsWith('rocket-state-reader-'));
    fs.rmSync(root, { recursive: true });
  }
}
function pointer(name) {
  const bytes = Buffer.from(name), size = Buffer.alloc(4);
  size.writeUInt32LE(bytes.length);
  return Buffer.concat([Buffer.from('ROCKET-STATE\0\x12'), size, bytes]);
}
function archive(message) {
  const name = Buffer.from('C:\\fixture', 'utf16le'), text = Buffer.from(message);
  const number = value => { const bytes = Buffer.alloc(4); bytes.writeUInt32LE(value); return bytes; };
  return Buffer.concat([Buffer.from('ROCKET-DRAFTS\0\x01'), number(1), number(name.length), name, number(text.length), text]);
}
test('legacy state keeps exact-content checks working', () => fixture(root => {
  const state = Buffer.from('ROCKET-STATE\0\x11legacy draft 文');
  fs.writeFileSync(path.join(root, 'state.bin'), state);
  assert.deepEqual(readDraftPayload(root), state);
}));
test('only the referenced generation supplies current draft text', () => fixture(root => {
  fs.writeFileSync(path.join(root, 'drafts-1-1.bin'), archive('PRIVATE_MUST_NOT_TYPE'));
  const current = archive('draft stays unchanged 文');
  fs.writeFileSync(path.join(root, 'drafts-1-2.bin'), current);
  fs.writeFileSync(path.join(root, 'state.bin'), pointer('drafts-1-2.bin'));
  assert.deepEqual(readDraftPayload(root), current);
  assert(!readDraftPayload(root).includes(Buffer.from('PRIVATE_MUST_NOT_TYPE')));
}));
test('invalid references cannot read outside the fixture', () => fixture(root => {
  for (const name of ['../drafts-1-2.bin', 'drafts-1-2.bin/other', 'C:\\drafts-1-2.bin']) {
    fs.writeFileSync(path.join(root, 'state.bin'), pointer(name));
    assert.throws(() => readDraftPayload(root), /Invalid draft archive name/);
  }
}));
test('missing or corrupt archives and unsupported state fail explicitly', () => fixture(root => {
  fs.writeFileSync(path.join(root, 'state.bin'), pointer('drafts-1-2.bin'));
  assert.throws(() => readDraftPayload(root), { code: 'ENOENT' });
  fs.writeFileSync(path.join(root, 'drafts-1-2.bin'), 'draft stays unchanged');
  assert.throws(() => readDraftPayload(root), /Invalid draft archive header/);
  fs.writeFileSync(path.join(root, 'state.bin'), Buffer.from('ROCKET-STATE\0\x13'));
  assert.throws(() => readDraftPayload(root), /Unsupported state version/);
}));
