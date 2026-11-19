'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {createHash} = require('node:crypto');
const zip = require('./read-package-zip.cjs');
const {secretKinds, sensitivePath} = require('../windows/audit-payload.cjs');

function readFile(file, limit) {
  const stat = fs.lstatSync(file);
  assert(stat.isFile() && stat.size > 0 && stat.size <= limit, 'Unexpected release input');
  return fs.readFileSync(file);
}

function prepare(source, destination, version, arch) {
  assert(/^\d+\.\d+\.\d+$/.test(version) && ['arm64', 'x64'].includes(arch));
  const name = `Rocket-${version}-macos-${arch}-preview.zip`;
  const bytes = readFile(path.join(source, name), 512 * 1024 * 1024);
  const sha256 = createHash('sha256').update(bytes).digest('hex');
  const checksum = `${sha256}  ${name}`;
  assert.equal(readFile(path.join(source, name + '.sha256'), 512).toString().trim(), checksum);
  const report = JSON.parse(readFile(path.join(source, 'integration.json'), 1024 * 1024));
  const receipt = {platform: 'darwin', arch, profile: 'release', sha256};
  for (const [key, value] of Object.entries(receipt)) assert(report[key] === value, 'Native result differs from the preview');
  for (const entry of zip.inventory(bytes)) {
    assert(!sensitivePath(entry.name), 'Private or development path in preview');
    if (!entry.directory) assert(secretKinds(zip.content(bytes, entry)).length === 0, 'Credential pattern in preview');
  }
  fs.mkdirSync(destination, {mode: 0o700});
  const output = path.join(destination, 'prototype-results');
  fs.mkdirSync(output, {mode: 0o700});
  fs.writeFileSync(path.join(output, name), bytes, {flag: 'wx'});
  fs.writeFileSync(path.join(output, name + '.sha256'), checksum + '\n', {flag: 'wx'});
  fs.writeFileSync(path.join(output, 'integration.json'), JSON.stringify(receipt) + '\n', {flag: 'wx'});
}

if (require.main === module) prepare(...process.argv.slice(2));
module.exports = {prepare};
