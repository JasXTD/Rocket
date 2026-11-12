'use strict';
const test = require('node:test'), assert = require('node:assert/strict');
const fs = require('node:fs'), os = require('node:os'), path = require('node:path');
const { verify, suites } = require('../macos/verify-security-followups.cjs');
test('Native security evidence requires every affected suite and named regression', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rocket-security-evidence-'));
  const clean = {};
  try {
    for (const [suite, [count, names]] of Object.entries(suites)) {
      clean[suite] = names.map(name => `test module::${name} ... ok\n`).join('') + `test result: ok. ${count} passed; 0 failed; 0 ignored;\n`;
      fs.writeFileSync(path.join(root, suite + '.log'), clean[suite]);
    }
    assert.equal(verify(root).total, 35);
    const file = path.join(root, 'agents-runtime.log');
    for (const invalid of ['', clean['agents-runtime'].replace('10 passed', '0 passed'), clean['agents-runtime'].replace('quarantine_blocks_snapshot_and_later_launch_without_changing_the_source ... ok', 'quarantine_blocks_snapshot_and_later_launch_without_changing_the_source ... ignored'), clean['agents-runtime'] + 'test result: FAILED', clean['agents-runtime'] + clean['agents-runtime']]) {
      fs.writeFileSync(file, invalid);
      assert.throws(() => verify(root));
    }
    fs.writeFileSync(file, clean['agents-runtime']);
    fs.unlinkSync(path.join(root, 'write-gate.log'));
    assert.throws(() => verify(root));
  } finally { fs.rmSync(root, { recursive: true }); }
});
