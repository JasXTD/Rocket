'use strict';
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const suites = {
  'write-gate': [4, ['dropping_owner_releases_lock_while_forked_child_still_holds_descriptor', 'process_ownership_survives_directory_move_and_is_released_after_exit_or_crash']],
  'history-units': [6, ['oversized_chain_is_rejected_before_parsing', 'chain_read_stops_at_the_bound_even_if_the_file_grows', 'chain_refuses_links_and_non_regular_files']],
  'statistics-units': [5, ['activity_rejects_extreme_dates_and_invalid_timezones_without_panicking']],
  'history-integration': [6, ['another_writers_lock_refuses_the_change_and_is_left_alone', 'stopped_write_preserves_locks_created_by_an_external_writer_after_preflight']],
  'statistics-integration': [4, ['real_git_extreme_author_date_returns_an_error_and_preserves_local_work']],
  'agents-runtime': [10, ['quarantine_blocks_snapshot_and_later_launch_without_changing_the_source', 'revoked_execute_permission_blocks_inspection_and_recheck', 'policy_revoked_during_worktree_preparation_prevents_terminal_start', 'executable_swap_after_recheck_never_runs_replacement', 'setup_handoff_rechecks_original_but_launches_reviewed_snapshots', 'native_snapshot_preserves_arguments_and_worktree_and_stop_releases_it']],
};
function verify(directory) {
  const counts = {};
  for (const [suite, [expected, names]] of Object.entries(suites)) {
    const log = fs.readFileSync(path.join(directory, suite + '.log'), 'utf8');
    assert(!/test result: FAILED|panicked at/.test(log), `${suite} includes a failure`);
    const results = [...log.matchAll(/^test result: ok\. (\d+) passed; 0 failed;/gm)];
    assert.equal(results.length, 1, `${suite} must run once`);
    assert.equal(Number(results[0][1]), expected, `${suite} test count changed`);
    for (const name of names) assert(log.includes(name + ' ... ok'), `Missing regression: ${name}`);
    counts[suite] = expected;
  }
  return { passed: true, suites: counts, total: Object.values(counts).reduce((a, b) => a + b, 0) };
}
if (require.main === module) {
  const directory = process.argv[2];
  fs.writeFileSync(path.join(directory, 'verified.json'), JSON.stringify(verify(directory), null, 2));
}
module.exports = { verify, suites };
