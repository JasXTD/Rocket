'use strict';
const test = require('node:test'), assert = require('node:assert/strict');
const cp = require('node:child_process'), path = require('node:path');
const {once} = require('node:events');
const {gone} = require('./platform.cjs');

test('process inspection distinguishes a live owned child, another image and exit', async () => {
  const child = cp.spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], {stdio:'ignore', windowsHide:true});
  await once(child, 'spawn');
  const ended = once(child, 'exit');
  try {
    assert.equal(gone(child.pid, path.basename(process.execPath)), false);
    assert.equal(gone(child.pid, 'rocket-absent-process-image'), true);
  } finally { child.kill(); await ended; }
  assert.equal(gone(child.pid, path.basename(process.execPath)), true);
});
