'use strict';
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const lock = fs.readFileSync(path.join(__dirname, 'Cargo.lock'), 'utf8');
const actual = [];
for (const match of lock.matchAll(/^\[\[package\]\]\r?\n(.*?)(?=^\[\[package\]\]|$(?![\s\S]))/gms)) {
  const block = match[1];
  if (!/^source = /m.test(block)) continue;
  assert(/^source = "registry\+https:\/\/github.com\/rust-lang\/crates.io-index"$/m.test(block), 'Unapproved package source');
  const name = /^name = "([^"]+)"$/m.exec(block)?.[1];
  const version = /^version = "([^"]+)"$/m.exec(block)?.[1];
  const hash = /^checksum = "([a-f0-9]{64})"$/m.exec(block)?.[1];
  assert(name && version && hash, 'Malformed locked registry package');
  actual.push(`${name} ${version} ${hash}`);
}
const expected = require('./approved-crates.json').map(p => `${p.name} ${p.version} ${p.checksum}`);
assert.equal(expected.length, 124);
assert.deepEqual(actual.sort(), expected.sort(), 'Lockfile differs from the approved source graph');
console.log('PASS exact approved registry graph: 124 versions and checksums');
