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
      if (suite === 'gatekeeper-probe') clean[suite] += 'ROCKET_GATEKEEPER_BUNDLE_PROBE={"baseline_exit":42,"direct_execution":"blocked","bundle_only_quarantine":true,"rocket_refused":true,"metadata_retained":true}\n';
      fs.writeFileSync(path.join(root, suite + '.log'), clean[suite]);
    }
    assert.equal(verify(root).total, 42);
    const file = path.join(root, 'agents-runtime.log');
    for (const invalid of ['', clean['agents-runtime'].replace('11 passed', '0 passed'), clean['agents-runtime'].replace('quarantine_blocks_snapshot_and_later_launch_without_changing_the_source ... ok', 'quarantine_blocks_snapshot_and_later_launch_without_changing_the_source ... ignored'), clean['agents-runtime'].replace('enclosing_quarantine_blocks_review_recheck_and_launch_without_changing_metadata ... ok', 'enclosing_quarantine_blocks_review_recheck_and_launch_without_changing_metadata ... ignored'), clean['agents-runtime'] + 'test result: FAILED', clean['agents-runtime'] + clean['agents-runtime']]) {
      fs.writeFileSync(file, invalid);
      assert.throws(() => verify(root));
    }
    fs.writeFileSync(file, clean['agents-runtime']);
    const probe = path.join(root, 'gatekeeper-probe.log');
    for (const status of ['allowed', 'timed_out']) {
      fs.writeFileSync(probe, clean['gatekeeper-probe'].replace('"blocked"', JSON.stringify(status)));
      assert.equal(verify(root).os_block_reproduced, false, 'No OS block may be claimed without evidence');
    }
    for (const invalid of [clean['gatekeeper-probe'].replace('"rocket_refused":true', '"rocket_refused":false'), clean['gatekeeper-probe'].replace('"baseline_exit":42', '"baseline_exit":0'), clean['gatekeeper-probe'] + clean['gatekeeper-probe']]) {
      fs.writeFileSync(probe, invalid);
      assert.throws(() => verify(root));
    }
    fs.writeFileSync(probe, clean['gatekeeper-probe']);
    fs.unlinkSync(path.join(root, 'write-gate.log'));
    assert.throws(() => verify(root));
  } finally { fs.rmSync(root, { recursive: true }); }
});
