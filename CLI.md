# Rocket CLI

This guide covers the commands shipped in [Rocket 0.92.2](docs/releases/0.92.2.md).
The portable packages include `rocket-cli` (`rocket-cli.exe` on Windows).
Platform requirements and provider-specific limits still apply.

## Hosting commands

`hosting account`, `hosting list`, `hosting pull NUMBER`, `hosting create`,
`hosting review NUMBER` and `hosting merge NUMBER` reuse Rocket's hosting API
adapters. They do not change the local checkout, Git credentials, SSH settings
or saved Rocket accounts. A local repository is not required for normal API use.

Every command needs `--provider` and `--token-env NAME`. Put the access token in
that process environment variable using your preferred secret manager; do not
put it in arguments, repository files or shell history. The variable name must
use uppercase letters/digits/underscores and end in `_TOKEN`. Rocket removes it
from its own environment before starting helpers; the calling shell is unchanged.
Credentials are sent over stdin to the existing HTTPS helper, never its argv.
Choose `--server HTTPS_URL` for enterprise/self-hosted instances, `--username`
only when your provider's token type requires it, and `--api-version` for Azure
DevOps Server. No automatic login fallback or token persistence is performed.

Examples, assuming `REVIEW_TOKEN` is already securely set in the invoking shell:

```powershell
rocket-cli --json hosting account --provider gitlab --token-env REVIEW_TOKEN
rocket-cli --json hosting list --provider gitlab --token-env REVIEW_TOKEN --repository group/project --kind pulls --state open --pages 2
rocket-cli --json hosting pull 42 --provider gitlab --token-env REVIEW_TOKEN --repository group/project
rocket-cli --json hosting review 42 --provider gitlab --token-env REVIEW_TOKEN --repository group/project --action comment --body "Reviewed the change" --preview
```

Writes require `--preview` first. Inspect `data.preview`, then repeat the same
command with `--expect-review` set to `data.preview.review_sha256`. Changing the
account, server, repository, revisions, text, comment locations or merge method
invalidates that fingerprint. The backend rechecks immediately before the one
write; a fingerprint is a stale-review guard, not proof that someone read it.

- **Create:** `--source BRANCH --target BRANCH --title TITLE [--body BODY]`;
  optional `--source-repository OWNER/REPO` is GitHub/GHES cross-fork only.
- **Review:** `--action comment|approve|request-changes [--body BODY]` and optional
  `--comments FILE`. Provider capabilities remain authoritative; GitLab approval
  cannot carry a message, and GitLab has no request-changes vote in this adapter.
- **Merge:** `--method merge|squash|rebase`. The preview explains provider-specific
  head conditions and remaining target-race limitations. No branch deletion,
  force/bypass, merge queue or automatic retry is requested.
- **List:** `--kind pulls|issues`, `--state open|closed|all`, and `--pages 1..20`.
  GitHub/GHES/GitLab accept `--search`, `--author`, `--assignee`; Azure issues
  accept `--work-item-state`. `limited` and per-collection errors identify partial
  results. A later-page failure keeps earlier rows but returns a failing exit.

The comments file is a bounded JSON array (32 comments and 128 KiB maximum).
Each item needs `path`, `start`, `line`, `side` (`LEFT`/`RIGHT`), `head`, `base`,
`body`, and optional `diff_revision`. Copy the exact path and available line
numbers from `hosting pull`'s `files`, plus its `inline_context`. GitLab requires
that diff revision and one single-line comment, with an empty separate review
message. Bitbucket Cloud/Server and Azure Services/Server likewise require one
single-line comment and an empty separate message. Server PR versions and Azure
iteration identities are included in `inline_context.diff_revision`; preserve
that value unchanged. Azure paths retain their leading `/` as API paths.
Gitea accepts up to 32 single-line comments in one review. GitHub/GHES
also accept ranges within one complete hunk. Binary, oversized, missing or
incomplete patches do not authorize comments. Editing or refreshing a draft does
not submit it. The file is user-managed; Rocket reads it without modifying it.

JSON writes report `remote_outcome`: `not_attempted` for preview/pin refusal,
`confirmed` for verified success, `not_confirmed` for a definite failure, or
`unknown` when an interrupted/unverifiable submission might have succeeded or
saved partial pending comments (Gitea). Gitea submissions also refuse an existing
pending review for the same account or an incomplete pending-review scan.
Inspect the same PR before retrying an unknown write. There are no automatic
retries. Token values and API response bodies are omitted from errors/logs;
successful JSON intentionally contains the requested private repository content.

For local acceptance only, `--fixture-api-port PORT` uses numeric loopback and
requires both `.git/rocket-ui-fixture` in `--repo` and the public synthetic token
`rocket-disposable-fixture`. Other token values are refused before any request.
This override is not a way to configure production HTTP hosts.

`rocket-cli` (`rocket-cli.exe` on Windows) is Rocket's standalone command-line client. It calls
`rocket-git` directly; repository commands do not require the desktop app, MCP,
Node.js or .NET. The explicit `review` launcher requires the desktop executable.
The CLI prefers its bundled Git when included; otherwise installed Git 2.46+ is
required. An explicit executable selection still takes priority. The focused
composed-workflow tests described below run on Windows; other platform runtime
acceptance is separate.
The original `inspect` example remains available for existing developer scripts.

On Windows, HTTP(S) Git operations require Git for Windows 2.56.0.windows.4 or
newer. Older or unrecognized builds retain local/SSH operations; HTTP and unknown
remote helpers are refused even after URL rewriting. This is the same runtime
policy as the desktop app. See [SECURITY.md](SECURITY.md#windows-git-http-runtime-gate-2026-10-29).

## Build and run

From the Rocket source directory:

```powershell
cargo build --release --offline --locked -p rocket-cli
.\target\release\rocket-cli.exe --help
$repo = Read-Host 'Full path to an existing Git repository'
.\target\release\rocket-cli.exe --repo "$repo" status
.\target\release\rocket-cli.exe --repo "$repo" --json status
```

Use `rocket-cli --version` and `rocket-cli --help` to check an installed portable
package's version and command set. Older packages may lack commands described here;
building the source does not replace an existing portable installation.

## Commit & Push and save/switch/restore

Both commands require a preview followed by its `review_sha256`. Preview does not
commit, stash, switch or push. Apply recreates the review and refuses changed
staging, tracked files (for save/switch), checkout, configuration, message or
destination before mutation. The backend still revalidates under repository write
ownership; the digest is an opaque pin, not permission to bypass protection.

```powershell
$review = rocket-cli --repo "$repo" --json commit-push origin main -m "Describe changes" --preview | ConvertFrom-Json
rocket-cli --repo "$repo" --json commit-push origin main -m "Describe changes" --expect-review $review.data.preview.review_sha256

$review = rocket-cli --repo "$repo" --json branch switch-saved feature/work --preview | ConvertFrom-Json
rocket-cli --repo "$repo" --json branch switch-saved feature/work --expect-review $review.data.preview.review_sha256
```

Commit & Push commits only the reviewed staging, then performs an ordinary guarded
push to the named destination. Unstaged and untracked files stay local. `data.commit`,
`reviewed_commit_confirmed`, `phase` and `push` distinguish a retained local commit
from a confirmed, rejected or unknown remote update. After a partial failure, inspect
the result and retry **Push separately**, rather than making another commit.

Save/switch/restore saves tracked changes, switches to an existing local branch and
reapplies staging and working changes. Untracked files stay in place and can block
the switch. The stash is retained even after success. JSON reports `phase`, the
recovery stash object and durable journal path. Conflicts or cancellation keep that
recovery data; inspect status before applying or dropping the stash. Neither command
automatically retries or rolls back writes. Cancellation retains exit code 130 and
fresh local observations; local status never proves what happened on a remote.

## Conflicts, line staging, bisect, remote tags and stash selection

These commands reuse the desktop's reviewed backend operations. A read command
prints the review; the write repeats that review in its own process and compares
the value you pass back (a SHA-256 or an object ID) before anything is written. A
mismatch is refused with exit 1 and nothing changes. The backend then checks again
under repository write ownership, as for every other CLI write. `--preview` shows a
write's plan with `"operation_started": false`.

```powershell
.\rocket-cli.exe --repo "$repo" --json hunks -- src/lib.rs
.\rocket-cli.exe --repo "$repo" --json stage --lines 7,9 --expect-patch $sha -- src/lib.rs
.\rocket-cli.exe --repo "$repo" --json conflict show src/lib.rs
.\rocket-cli.exe --repo "$repo" --json conflict resolve src/lib.rs --expect-sha256 $sha --choose 1=ours,2=both --stage
.\rocket-cli.exe --repo "$repo" --json bisect run $good $bad --test-timeout 300 --preview -- C:\Tools\check.exe --quick
.\rocket-cli.exe --repo "$repo" --json tag publish v1.2.0 --remote origin --source $tagId
.\rocket-cli.exe --repo "$repo" --json stash apply $stashId --lines 12 --expect-patch $sha -- src/lib.rs
```

**Line staging.** `hunks [--staged] -- PATH` returns the file's `patch`, its
`patch_sha256`, `hunks` (`header`, `line_ids`) and `changed_lines` (`id`, `sign`,
`text`). Line IDs are zero-based lines of that raw patch. `stage --lines` and
`unstage --lines` take IDs from that review plus `--expect-patch` and change only
the index; working files stay as they are. For an untracked file (`untracked: true`)
the selected lines become the staged new file. A changed patch returns
`selection_stale_preview`: run `hunks` again. A change reported in
`selection_unavailable` must be staged whole.

**Conflicts.** `conflict show PATH` lists `versions` (base, ours, theirs: blob `id`,
size, binary, executable, submodule, LFS), which version the working file matches,
`can_keep` and, for UTF-8 text up to 512 KiB, `text` with the working file's
`sha256` and its numbered `conflicts` (`ours`, `theirs`, line range).
`conflict resolve` needs that `sha256` and either `--choose N=ours|theirs|both,...`
(`all=SIDE` for every conflict; both means ours then theirs, as in the editor) or
`--result FILE` (relative to the current directory) with the complete UTF-8 result,
whose line endings and BOM follow the conflicted file. Unchosen conflicts keep their markers, and `--stage` refuses
remaining markers (`conflict_markers_remain`). A changed file returns
`conflict_file_changed`. JSON adds `saved`, `staged`, `recovery_directory` (copies of
the replaced file) and fresh `status` and `operation`.
`conflict keep PATH --side ours|theirs|deletion --expect BLOB_ID` keeps one whole
version, including binary, LFS, submodule and delete/modify conflicts; use
`--expect missing` with `--side deletion`. A different version returns
`conflict_review_stale`. `conflict mark-resolved -- PATH` stages a file you edited
yourself and refuses complete marker blocks. None of these finish the merge or
rebase: run `merge continue` or `rebase continue` afterwards.

**Bisect.** `bisect status` reports `active`, the `session` (`phase` testing,
checkout_pending, found, inconclusive or restore_only; `original`, `current`, `good`,
`bad`, `target`, `first_bad`, `candidates`, `decisions`, `recovery_reference`) and
the saved `tests`. `bisect start GOOD BAD` saves the return checkout and checks out
the first candidate; pass full IDs to keep the range fixed between invocations.
`bisect good|bad|skip --expect COMMIT_ID` records a decision for the checked-out
commit (`session.current.target`); a moved checkout returns `bisect_stale_preview`.
`bisect resume` finishes a pending checkout, and `bisect restore` returns to the
original branch without forcing.

`bisect run GOOD BAD --test-timeout SECS --preview -- PROGRAM [ARG...]` reviews an
automatic search. PROGRAM is an absolute local `.exe` outside the working folder,
started in the checkout with up to 64 literal arguments and no shell; run a script
through an explicit shell such as `pwsh.exe -NoProfile -File C:\tests\check.ps1`.
Its exit code classifies each revision: 0 good, 1–124 bad, 125 skip; any other exit
stops without classifying. `--test-timeout` is 1–3,600 seconds per revision. The
preview reports `program.sha256`; run again with `--expect-sha256 SHA256` in place of
`--preview`. A changed executable returns `bisect_program_changed` and nothing
starts; the backend also rechecks the digest before every candidate. `--continue`
replaces GOOD BAD to keep testing an active search, and GOOD BAD while a search is
active returns `bisect_active_operation`.

The run holds repository write ownership for the whole search, as the desktop's
automatic bisect does: other Rocket writes get `repository_busy` until it ends, and
reads continue. JSON `result` is `found`, `inconclusive` (warning
`bisect_inconclusive`; `session.candidate_ids` lists the remaining commits) or
`stopped`. `tests` lists each revision's `commit`, `outcome` (good, bad, skip,
cancelled, timeout, failed or changed), `exit_code`, `elapsed_ms`, `recorded` and up
to 8 KiB each of `stdout` and `stderr`; test output never reaches diagnostic logs. A
test past its limit returns `bisect_test_timed_out` with exit 124, one without a
usable exit code returns `bisect_test_failed`, and Ctrl+C returns 130 after the
test's process tree has stopped. In each case the search stays active: check
`bisect status`, then run `bisect run --continue` or `bisect restore`. Results (not
output) are saved in the repository, so the desktop's Bisect page shows a CLI run.
While a test runs, its output goes to private files under
`%LOCALAPPDATA%\Rocket\cli\bisect-output` (on Linux, `~/.local/state/rocket/cli/bisect-output`),
deleted after each test.

**Remote tags.** `tag remote REMOTE` reads the tags advertised by the remote's push
destination without downloading objects. Each entry has `name`, `remote_id`,
`peeled`, `local` (the local tag or null) and `relation` (published, different,
remote_only, local_only or symbolic); URLs are omitted. `tag publish NAME --remote
REMOTE --source TAG_OBJECT_ID` publishes a local tag the remote does not have yet
(`remote_tag_exists` otherwise); `--source` is the local tag's own object ID from
`tag inspect`. `tag delete-remote NAME --remote REMOTE --expect REMOTE_TAG_ID`
deletes the remote tag only while the remote still advertises that ID
(`remote_tag_changed` otherwise); the local tag stays. Both push one exact ref with
that lease, with secret protection and the HTTP runtime gate. JSON follows
`push-lease`: `preview`, `command_result`, `remote_outcome` (published, deleted,
unchanged, different or unknown), `observed` and `verification`.

**Stash selection.** `stash save [--message M] [--include-untracked] -- PATH...` saves
only those files; other changes stay, and every staged change must be among them.
`stash show FULL_ID [--part tracked|index|untracked]` lists its files with `patch`,
`patch_sha256`, numbered `hunks` and their `line_ids`. `stash apply FULL_ID --
PATH...` applies those whole files, and `--hunks N,...` or `--lines ID,...` with
`--expect-patch` apply part of one file. The index and the stash stay as they are,
executable-mode changes are refused, and replaced files get recovery copies
(`recovery`). `stash save --message M --lines ID,... --expect-patch SHA256 -- PATH`
stores the selected lines of one tracked file as a new stash, then removes them
from the file; JSON adds `stash` and `recovery_reference`. `stash pop INDEX --expect
FULL_ID` needs a clean working tree. It backs up every stash under
`recovery_namespace` first, applies with the index and drops only that entry; JSON
adds `removed` and `recovery_reference`.

## Undo, commit operations, rebase start, reset and history reads

These commands follow the same review-and-pin pattern. Every write here holds
repository write ownership through its revalidation, the write and the fresh
observations, as the desktop does; the reads take none.

```powershell
$u = (.\rocket-cli.exe --repo "$repo" --json commit -m 'Draft' | ConvertFrom-Json).data.undo
.\rocket-cli.exe --repo "$repo" --json undo-commit --branch $u.branch.text --expect $u.commit --index-sha256 $u.index_sha256
.\rocket-cli.exe --repo "$repo" --json cherry-pick $commit --expect $head
.\rocket-cli.exe --repo "$repo" --json rebase start main --interactive --preview
.\rocket-cli.exe --repo "$repo" --json rebase start main --plan plan.json --expect-original $tip --expect-onto $main
.\rocket-cli.exe --repo "$repo" --json reset --soft $commit --expect $head
.\rocket-cli.exe --repo "$repo" --json blame HEAD --path src/lib.rs
```

**Undo last commit.** `commit` JSON now includes `undo`: `branch`, `commit`,
`previous` and `index_sha256`, the digest of the index right after the commit. It
is null, with warning `undo_unavailable` when Undo could not be prepared, for a
commit that cannot be undone this way, such as a merge or a commit on a detached
HEAD. `undo-commit --branch NAME --expect COMMIT_ID --index-sha256 SHA256` saves
the commit as `recovery_reference`, then moves the branch back to the previous
commit and keeps the index and working files. It refuses with an `undo_*` code when
the branch moved (`undo_head_changed`), anything was staged since
(`undo_index_changed`), a remote-tracking branch contains the commit
(`undo_shared_commit`), an operation is active or the commit is not ordinary. This
is the desktop's Undo last commit, with the values in place of the session.

**Cherry-pick, revert and amend.** `cherry-pick ID...` and `revert ID...` take 1–100
full commit IDs in the order to apply them; more than one is a reviewed sequence,
and `--mainline N` chooses the parent for a single merge commit. `--expect HEAD_ID`
refuses a moved HEAD (`stale_preview`). `--preview` reports `head`, `commits`,
`prediction` (as for `pull`) and `recovery_reference`. A conflict stops with
`commit_failure` and an active `operation`: resolve and stage, then run
`cherry-pick continue` or `revert continue`, or `skip` or `abort`.
`amend --message MESSAGE --expect HEAD_ID` replaces the HEAD commit with the staged
changes and a new message, keeping the old commit as `recovery_reference`. JSON
adds `branches`, `operation`, `recovery_reference` and `journal_cleanup` to
`command_result` and `status`.

**Rebase start.** `rebase start TARGET --preview` reports `original`, `onto`,
`commits`, `fast_forward`, `up_to_date`, `dependents` and `prediction`; start it
with `--expect-original` and `--expect-onto` set to those IDs (`stale_preview`
otherwise). `--update-refs` also moves other branches that point into the range,
and `--preserve-merges` keeps merge topology. `--interactive --preview` lists the
range as a `plan` of picks, even when the branch is up to date. Save it as a JSON
array with one row per reviewed commit, `{"commit": ID, "instruction": "pick",
"message": "", "stop_after": false}`, where the instruction is pick, reword, edit,
squash, fixup or drop, and pass it with `--plan FILE` (relative to the current
directory; implies `--interactive`). Reword and squash need a message. Rows with
`exec` are refused, because commands are reviewed in the desktop UI. Plans run
through `rocket-rebase-editor.exe`, which must sit beside `rocket-cli.exe`, as it
does in the bundle. A conflict or a planned stop (`stop`: edit or break) continues
with `rebase continue`, `skip` or `abort`. The original tip stays as
`recovery_reference`.

**Reset and reflog.** `reflog [--scope head|all|deleted-branches|retained|unreachable]
[--limit N]` lists `id`, `selector`, `subject`, `commit_subject`, `actor` and
`reference` (default 100, maximum 2,000, with `has_more`); keep a listed commit with
`branch create NAME --start ID`. `reset --soft|--mixed|--hard ID --expect HEAD_ID`
moves the current branch to that full commit ID. Mixed and hard refuse staged
changes and hard refuses tracked edits (`staged_changes`, `dirty_worktree`), so no
uncommitted work is discarded. The previous tip stays as `recovery_reference`.

**Signatures.** `verify commit REV` and `verify tag NAME` report `status` (unsigned,
valid, untrusted, invalid, expired, revoked or unavailable), `format`, `signer`,
`fingerprint`, `trust` and `guidance`. With `--require-valid`, any status other
than valid exits 1 with `signature_not_valid`. `signing show` reports the `local`
and `effective` settings, `format` and the verifying `program`. Signing setup stays
in the desktop.

**History reads.** `blame REV --path PATH [--first N] [--size N]` returns up to 500
lines per call with `next_line`. `file-history PATH [--revision REV] [--limit N]`
follows renames (`previous_path`) for up to 200 entries, with `has_more`.
`search TEXT [--kind text|path|content] [--all | --revision REV]` scans one bounded
window: 4,096 commits for text, 1,000 for paths, 200 for content; `complete` is
false when the window ended first. `compare BASE TARGET`, or `compare BASE` with
`--staged` or `--worktree`, lists changed files with `change`, `original_path` and
`similarity`; `target` is null for a local comparison.

## Maintenance, identity, Git LFS and SSH keys

The same review-and-pin pattern applies; writes hold repository write ownership as
the desktop does.

```powershell
.\rocket-cli.exe --repo "$repo" --json remote prune origin --preview
.\rocket-cli.exe --repo "$repo" --json branch delete-remote origin topic --expect $id
.\rocket-cli.exe --repo "$repo" --json identity set --name 'Ada Example' --email ada@example.invalid
.\rocket-cli.exe --repo "$repo" --json lfs track '*.psd'
.\rocket-cli.exe --repo "$repo" --json ssh use rocket-work --preview
```

**Remotes and branches.** `remote rename OLD NEW` renames the remote's settings and
tracking refs; JSON adds `config_backup` and `recovery_reference`.
`remote prune REMOTE --preview` reads the server and lists `obsolete` tracking refs,
whose branch the server no longer has, and `stale_local_branches`, which are
candidates only and never deleted. `remote prune REMOTE -- REF...` removes the
chosen tracking refs (full names), keeping a recovery ref; the server is unchanged.
`branch delete-remote REMOTE BRANCH --expect ID` deletes a server branch only while
it still points at ID, the remote branch's `target` in `branch list` after a
`fetch` (`remote_branch_changed` otherwise). It refuses the server's default branch;
`remote_outcome` is deleted, unchanged or unknown.
`branch fast-forward --preview` lists local branches with their upstream and whether
each can fast-forward (`blocked` says why not, such as for the checked-out branch);
`branch fast-forward -- BRANCH...` moves the chosen ones to their already-fetched
upstream, keeping a recovery ref; it does not fetch. `fetch REMOTE --tags` also
fetches all remote tags, never replacing a local tag of the same name; without
`--tags`, fetch is unchanged and fetches no tags.

**Worktrees.** `worktree move PATH DESTINATION --expect HEAD_ID` moves a clean,
unlocked linked worktree to a new folder on the same drive. After a linked folder
was moved outside Git, `worktree repair PATH --at FOLDER --expect HEAD_ID` points
its registration at the new folder. `worktree prune PATH --expect HEAD_ID` removes
the registration of a folder that no longer exists while its parent folder still
does, keeping its HEAD as `recovery_reference` and the registration in
`metadata_backup`. Previews and JSON follow `worktree remove`.

**Tags and identity.** `tag list --order newest` sorts by tagger date (or commit
date) and reports `order`. `tag create NAME REV --message M --sign` creates a
signed annotated tag with the repository's SSH or OpenPGP signing settings. A key
that needs a passphrase must be available through its agent, because no passphrase
passes through the CLI; check the result with `verify tag`. `identity show` reports
the `local`, `effective` and `inherited` user.name and user.email. `identity set
--name NAME --email EMAIL` writes the repository-local values (a value left out
stays as it is) and refuses a configuration that changed after it was read.

**Git LFS.** These use the Git LFS installed with the Git that Rocket runs
(`lfs_helper_missing` without one). `lfs status` lists LFS `files` (`path`,
`oid`, `size`, `cached`, and `state`: pointer, content, missing or unsupported),
`remotes`, and the `.gitattributes` `tracking` rules with their `line`.
`lfs track PATTERN` adds a rule; `--literal` makes the pattern one exact path from
the repository root. `lfs untrack --line N --expect TEXT` removes exactly that rule
as `lfs status` showed it. `lfs fetch REMOTE` downloads the objects HEAD needs,
`lfs push REMOTE` uploads HEAD's objects (`upload_outcome`: not_started,
confirmed_by_helper or unknown), and `lfs checkout -- PATH` replaces a pointer file
with its cached content. `--preview` reports `objects`, `bytes`, `files` and any
`attributes` change. `lfs locks REMOTE`, `lfs lock PATH --remote REMOTE` and
`lfs unlock PATH --remote REMOTE --expect LOCK_ID` use the server's locking API
(`remote_outcome`: not_started, confirmed_by_helper or unknown); a local remote has
none (`lfs_lock_unsupported`). Unlocking only works on your own locks; forcing an
unlock and pruning stay in the desktop. Server addresses are left out of the JSON.

**SSH keys.** `ssh keys` lists the key pairs in your `.ssh` folder (`folder`: HOME's
when set, else USERPROFILE's) with `path`, `kind`, `fingerprint`, `public_key` and
whether the Windows ssh-agent service has it `unlocked`, and Rocket's profiles in
`%USERPROFILE%\.ssh\rocket-keys` (`alias`, `host`, `user`, `port`, `key`,
`fingerprint`, `encrypted`, `unlocked`). Only public key material is reported.
`ssh check ALIAS` shows the profile's `effective_config` from `ssh -G` without
connecting. `ssh use ALIAS --preview` reviews setting this repository's
`core.sshCommand` to that profile and reports `config_sha256`; apply it with
`--expect-config SHA256` (`ssh_config_changed` otherwise). It refuses when
environment variables or included settings would override the binding
(`ssh_config_overrides`). `ssh restore` reviews and applies the same way: while the
configuration is exactly as one of Rocket's bindings left it, it puts back the
configuration from before that binding (`ssh_no_matching_recovery` otherwise).
Each save keeps a backup in `recovery_directory`. Creating, importing, unlocking or
removing keys stays in the desktop, because those involve passphrases and ssh-agent.

## Code search

These reads use the desktop's code search. They never write and take no repository
write ownership.

`search TEXT --kind content` lists commits that change how often TEXT appears, as
`git log -S` does. Three options change how it matches and are accepted only with
`--kind content` (a `usage` error otherwise): `--regex` reads TEXT as a POSIX
extended regular expression and matches added or removed lines, as `git log -G`
does; `--match-case` makes letters match exactly; `--path PATH` keeps changes to one
file or folder, exact and case-sensitive. The result adds `content` with `regex`,
`match_case` and `path`; it is null for text and path searches. Without
`--match-case`, this search ignores case for the letters A to Z only.

`grep PATTERN [--revision REV] [--regex] [--match-case] [--path PATH]` lists the
lines that match in files. Without `--revision` it searches the files on disk:
tracked ones and untracked ones that aren't ignored. With `--revision` it searches
that commit's files and reports the commit's full ID as `revision`. Binary files are
skipped, and without `--match-case` case is ignored for other letters too. Each entry
in `files` has `path`, `size`, `lfs_pointer` (a Git LFS pointer, so only the pointer
was searched), `capped` (100 lines matched, the most kept for one file) and `lines`.
Each line has `line` (1-based), `preview`, `marks`, `columns`, `cut_start`, `cut_end`
and `not_utf8`. `preview` is display text around the first match: at most 400
characters, tabs as four spaces and control characters escaped, so it is not the
exact line. `marks` are character ranges in `preview` and `columns` byte ranges in
the whole line, each `[start, end]` with the end excluded. `matches` counts the
matching lines. `end` is `complete`, `too_many` (2,000 matching lines, or lines too
long to read them all) or `timed_out` (30 seconds); `complete` is false when the
search stopped early. Finding nothing is a success with an empty `files`, unlike
`git grep`, which exits 1. Plain output prints one `path:line: preview` line per match.

An expression Git can't read fails with `invalid_pattern` (exit 1) and Git's
`reason`; nothing is searched. A repository without a working tree needs `--revision`.

## Reviewed push, tags and worktrees (0.84)

These commands reuse the desktop backend. Start with a preview and review its
objects before applying. Preview commands do not change remote refs or working
files, but push previews contact the remote and can fetch objects for comparison.
There is no unconditional force flag or automatic fallback/retry.

```powershell
.\rocket-cli.exe --repo "$repo" --json push-lease origin topic topic --preview
.\rocket-cli.exe --repo "$repo" --json push-atomic origin --preview -- topic:topic release:release
.\rocket-cli.exe --repo "$repo" --json tag list --limit 200
.\rocket-cli.exe --repo "$repo" --json tag inspect '版本/一'
.\rocket-cli.exe --repo "$repo" --json tag create '版本/一' HEAD --message 'Release notes' --preview
.\rocket-cli.exe --repo "$repo" --json worktree list
.\rocket-cli.exe --repo "$repo" --json worktree create '..\linked checkout' --branch feature --start HEAD --preview
```

`push-lease REMOTE LOCAL REMOTE_BRANCH --source LOCAL_ID --expect REMOTE_ID`
requires both full object IDs from the preview. A moved source or remote is
refused with `force_push_stale_preview`. The remote must already exist and the
backend's reviewed replacement rules apply. This does not configure an upstream.

For atomic apply, each mapping is
`LOCAL:REMOTE_BRANCH:SOURCE_ID:EXPECTED_ID`, replacing the last field with the
literal `missing` only when the preview reported a missing remote branch. Use
`--` before the mappings. Between 1 and 32 distinct destinations are accepted.
This is a normal atomic push, not a multi-branch force push. A server without
atomic support returns `atomic_push_unsupported`; no branches are published
individually. Changed reviewed objects return `atomic_push_stale_preview`.

Each invocation makes a fresh preview using the current remote configuration;
the IDs pin objects across invocations, not the remote URL/configuration from a
previous CLI process. The backend then rechecks refs/configuration under write
ownership before execution. Rerun the preview after changing remote settings.

Push JSON contains `preview`, `command_result`, `remote_outcome`, `observed` and
`verification`. Lease outcomes are `unchanged`, `replaced`, `different`, `missing`
or `unknown`; atomic outcomes are `unchanged`, `published`, `different` or
`unknown`. Different/missing report externally changed state. A cancelled command can already
have changed the remote. Check these fields before retrying; unknown is never
reported as rollback. Actual stop timing remains separate from reconciliation.

Omit `--preview` to create a tag or worktree after review. Tag creation makes a
lightweight tag without `--message`, or an annotated tag with it. For a target
that must remain pinned between invocations, pass the full commit ID instead of
`HEAD`. `tag list` defaults to 200 entries (maximum 2,000), with `has_more`;
`tag inspect NAME` reads the exact tag even beyond that page.

`tag delete NAME --expect TAG_OBJECT_ID [--preview]` uses the tag's own object ID
from inspection, not an annotated tag's peeled commit. Deletion retains a
`recovery_reference`. These are local tags; no tag publishing/deletion or signing
options are added to the CLI in this release.

Worktree creation defaults to a new branch. `--existing` selects an existing
local branch and cannot be combined with `--start`. Relative destinations are
relative to the CLI's current directory, not `--repo`. Existing directories are
refused. `worktree remove PATH --expect HEAD_ID [--preview]` requires a registered,
clean, unlocked linked worktree at that exact HEAD. It cannot remove the main
checkout or discard local files. Reports include observed worktrees, verification,
recovery refs and any retained workspace/metadata paths after an incomplete step.
The branch itself remains. Tag/worktree verification failures return a nonzero
result even if their command succeeded; inspect both fields rather than repeating
the mutation automatically.

## History order, browser links and native reviews (0.83)

```powershell
.\rocket-cli.exe --repo "$repo" --json history --all --order author-date --limit 100
.\rocket-cli.exe --repo "$repo" --json commit-url HEAD --remote origin
.\rocket-cli.exe --repo "$repo" commit-url HEAD --remote upstream --provider gitlab --server https://git.example.test/gitlab
.\rocket-cli.exe --repo "$repo" --json review commit HEAD
.\rocket-cli.exe --repo "$repo" review file HEAD --path 'src/表示.rs' --blame
```

`--order` accepts `topo` (default), `commit-date` and `author-date`. The latter
two use Git's ancestry-safe date orders, not a strict global timestamp sort.
History JSON adds `data.order`; its existing `commits` and `has_more` stay unchanged.

`commit-url` resolves the revision locally and prints one URL without opening a
browser or contacting the host. With multiple matching remotes, choose `--remote`.
Custom hosts require `--provider` (`github`, `github-enterprise`, `gitlab`,
`bitbucket`, `bitbucket-server`, `azure`, `azure-server`, `gitea`) and `--server`
together. Azure DevOps Server uses its HTTPS collection URL, including any
virtual directory. The validated server
must match the remote; credentials, arbitrary schemes and unknown-host guesses
are rejected. JSON includes the full `commit`, `remote`, `provider`, `url` and
`remote_presence: "not_checked"`. The commit must already exist on that host for
its web page to work. SSH aliases not matching a configured host are unsupported.

`review commit REV` or `review file REV --path PATH [--blame]` resolves the commit
before launching the adjacent `rocket.exe` with literal arguments. The resulting
native review is pinned, read only and independently cancellable. No shell command
is constructed. A successful launch returns `launched`, `pid`, `commit`, `path`
and `mode`; loading happens asynchronously and errors appear in the window.
The desktop process does not hold the CLI's output pipe open, so JSON returns
without waiting for the review window to close.
Cancellation before launch prevents it; cancellation after launch does not close
an independent application. No extra CLI capabilities are implied by the launcher.
Version 0.76.0 adds the `mail_import` boolean to operation-state JSON so CLI
consumers can distinguish a paused commit-series import from a rebase. The new
mail-import and merge-preserving rebase controls are desktop workflows; existing
CLI command behavior is unchanged.
Version 0.60.0 adds `data.preview.prediction` to pull reviews: outcome
(clean/conflicts/empty/unavailable), checked/total counts, zero-based stopped_at,
lossless text/hex paths, reason and guidance. The field is null for merge/FF,
no-op or fast-forward cases. Schema 1 remains additive-compatible. Text output
shows the prediction too. --preview still fetches but never integrates; without
--preview a prediction is advisory and existing rebase recovery still applies.
Interactive-plan prediction is not included. Standalone `rebase start` is described
in the commit operations section above.
Copy the directory where desired; no installer or PATH change is performed.
Version 0.51.0 adds merge/rebase pull and recovery, verified with temporary local
repositories. The live HTTPS/SSH acceptance below used 0.48.2; it does not claim
live network coverage for these new integration choices. Version 0.49.1 added
parallel post-push observations and was also verified locally.

Without `--repo`, repository commands use the current directory. `init` and
`clone` use their explicit destination and ignore `--repo`. Relative destination
and local clone-source paths use the process's current directory. In contrast,
stage/unstage/diff paths are **literal paths relative to the repository root**,
even when `--repo` names a subdirectory. Use `--` before paths beginning with `-`.

## Repository write coordination

### Secret protection

CLI commits and pushes enable both local Secret protection and Guard AI API keys.
Recognized AI-provider key formats use the fixed code `ai_api_key_detected`.
Recognized GitHub tokens, private keys, likely credential assignments and `.env` paths cause exit
1 before the write process starts. The JSON error has a fixed detection `code`,
sanitized `location`, optional `line`, `operation_started: false` and
`retry_requires_changes: true`, plus corrective `guidance`. Normal push results
report `remote_outcome: "not_attempted"`. The matching secret is never echoed.

Remove the content from the staged snapshot and all history being published
before retrying; a later deletion is insufficient. Do not retry through another
tool to bypass the refusal. The optional desktop setting applies to its own
profile; the CLI has no bypass flag. See [AI key coverage](SECURITY.md#ai-api-key-guard)
and [general coverage and scan limits](SECURITY.md#local-secret-protection).

### Concurrent writes

Starting with 0.57.1, UI and CLI backend writes acquire shared ownership of the
repository before mutation and hold it through the existing reconciliation reads.
Linked worktrees share ownership because their refs and configuration are shared.
Another Rocket process receives exit 1 with JSON error code `repository_busy`;
the rejected operation does not start and is not automatically retried. Wait for
the owning operation to finish, refresh/review the current state, then retry.
In-process writes retain their cancellable FIFO queue. Reads and writes in
independent repositories remain available.

On Windows, a non-inheritable OS mutex is identified by the common metadata
directory's volume/file identity. Path spelling and linked-worktree locations do
not create separate locks. Ownership ends after completion, error or cancellation
reconciliation, and Windows releases it when the owner exits. No Rocket lock file
is created, no PID is used to guess stale ownership, and no Git lock is removed.
An unavailable ownership mechanism fails the write with an I/O error. Diagnostics
record `write_queue` / `repository_busy` without paths or command payloads.

Both programs must use the new build. This coordinates cooperating Rocket
processes on this machine, not Git/Fork commands, terminal commands, older Rocket
builds or clients on another machine using a network share. Existing Git locks,
external-change checks and review validation still apply. A sequence of separately
reviewed actions (such as fetch, then pull integration) revalidates between actions;
this is not a transaction across arbitrary CLI invocations. Clone/init retain their
existing destination-publication guards.

## Command reference

| Command | Purpose |
|---|---|
| `info`, `status`, `operation` | Repository details, current changes, recovery markers |
| `history [--all \| --revision REV] [--order topo\|commit-date\|author-date] [--limit N]` | Bounded history, default 25, maximum 1,000 commits; topological order by default |
| `commit-url REV [--remote NAME] [--provider PROVIDER --server HTTPS_URL]` | Print the commit's web page on its host, without opening a browser or contacting the host |
| `review commit REV`, `review file REV --path PATH [--blame]` | Open a pinned, read-only review in the adjacent `rocket.exe` |
| `diff [--staged \| --base REV --target REV] [--context N] [-- PATH...]` | Worktree, staged or commit-to-commit patch |
| `hunks [--staged] -- PATH` | Changed-line IDs and patch SHA-256 of one file's change |
| `blame REV --path PATH [--first N] [--size N]` | Line authorship at a commit |
| `file-history PATH [--revision REV] [--limit N]` | Commits that changed a file, following renames |
| `search TEXT [--kind text\|path\|content] [--all \| --revision REV]` `[--regex] [--match-case] [--path PATH]` | One bounded history search window; the last three need `--kind content` |
| `grep PATTERN [--revision REV] [--regex] [--match-case] [--path PATH]` | Matching lines in the checkout's files or a commit's |
| `compare BASE (TARGET \| --staged \| --worktree)` | Changed files between commits or against local changes |
| `reflog [--scope head\|all\|deleted-branches\|retained\|unreachable] [--limit N]` | Reflog, deleted-branch, retained or unreachable commits |
| `verify commit REV`, `verify tag NAME` `[--require-valid]`, `signing show` | Signature status and signing settings |
| `identity show`, `identity set [--name NAME] [--email EMAIL] [--preview]` | Repository-local user.name and user.email |
| `lfs status`, `lfs locks REMOTE` | LFS files, tracking rules and server locks |
| `lfs track PATTERN [--literal]`, `lfs untrack --line N --expect TEXT` `[--preview]` | Add or remove one `.gitattributes` LFS rule |
| `lfs fetch REMOTE`, `lfs push REMOTE`, `lfs checkout -- PATH` `[--preview]` | LFS object transfers and pointer checkout |
| `lfs lock PATH --remote REMOTE`, `lfs unlock PATH --remote REMOTE --expect LOCK_ID` `[--preview]` | Server file locks; your own locks only |
| `ssh keys`, `ssh check ALIAS` | Public keys, Rocket profiles and their effective SSH settings |
| `ssh use ALIAS`, `ssh restore` `(--preview \| --expect-config SHA256)` | This repository's `core.sshCommand` |
| `stage \| unstage --lines ID,... --expect-patch SHA256 -- PATH` | Stage or unstage reviewed lines; index only |
| `init DESTINATION [--branch NAME]` | Initialize a new or existing project folder without replacing its files |
| `stage (--all \| -- PATH...)` | Stage selected paths or explicitly all changes |
| `unstage (--all \| -- PATH...)` | Unstage selected paths or explicitly all changes |
| `commit --message MESSAGE` | Commit staged changes; `-m` is an alias |
| `undo-commit --branch NAME --expect COMMIT_ID --index-sha256 SHA256 [--preview]` | Undo a commit made by `commit`, keeping its changes staged |
| `amend --message MESSAGE --expect HEAD_ID [--preview]` | Replace HEAD with the staged changes and a new message |
| `cherry-pick \| revert ID... [--mainline N] [--expect HEAD_ID] [--preview]` | Reviewed commit operations; `continue`, `skip` or `abort` recover them |
| `reset --soft \| --mixed \| --hard ID --expect HEAD_ID [--preview]` | Move the branch, keeping a recovery ref; refuses work it would discard |
| `branch list` | Local and remote-tracking branches |
| `branch create NAME [--start REV]` | Create a branch at the selected commit |
| `branch switch NAME` | Switch to an existing local branch, protecting local edits |
| `branch rename OLD NEW` | Rename without replacing an existing branch |
| `branch delete NAME` | Delete a merged local branch; no force flag |
| `branch delete-remote REMOTE BRANCH --expect ID [--preview]` | Delete a server branch at the reviewed object |
| `branch fast-forward [--preview] [-- BRANCH...]` | Fast-forward branches to their upstream |
| `stash list` | List indices, immutable IDs and subjects |
| `stash save [--message MESSAGE] [--include-untracked]` | Save a stash; `-m`/`-u` are aliases |
| `stash apply FULL_ID [--index]` | Apply the selected stash, optionally restoring its index |
| `stash drop INDEX --expect FULL_ID` | Delete only if the index still names that exact stash |
| `stash save [--message M] [--include-untracked] -- PATH...` | Save only these files |
| `stash save --message M --lines ID,... --expect-patch SHA256 [--preview] -- PATH` | Save reviewed lines as a stash, then remove them from the file |
| `stash show FULL_ID [--part tracked\|index\|untracked]` | Files, patches, hunks and line IDs of a stash |
| `stash apply FULL_ID [--part P] [--hunks N,... \| --lines ID,... --expect-patch SHA256] [--preview] -- PATH...` | Apply selected files or changes; index and stash kept |
| `stash pop INDEX --expect FULL_ID [--preview]` | Apply with index and drop, after a backup ref |
| `conflict show PATH` | Versions, conflict blocks and working-file SHA-256 |
| `conflict resolve PATH --expect-sha256 SHA256 (--choose N=SIDE,... \| --result FILE) [--stage] [--preview]` | Save a checked resolution with recovery copies |
| `conflict keep PATH --side ours\|theirs\|deletion --expect BLOB_ID\|missing [--preview]` | Keep one whole version |
| `conflict mark-resolved -- PATH` | Stage an edited file without conflict markers |
| `bisect status`, `bisect start GOOD BAD`, `bisect good\|bad\|skip --expect COMMIT_ID`, `bisect resume\|restore` | Manual bisect with the desktop's state and recovery |
| `bisect run (GOOD BAD \| --continue) --test-timeout SECS (--preview \| --expect-sha256 SHA256) -- PROGRAM [ARG...]` | Automatic bisect with a reviewed executable |
| `merge start REV [--ff-only \| --no-ff \| --squash \| --no-commit]` | Guarded merge; squash and no-commit prepare changes without advancing HEAD |
| `merge continue --message MESSAGE`, `merge abort` | Recover an existing merge |
| `remote list` | Names, fetch mappings and URL counts; URLs are omitted |
| `remote add NAME URL` | Add local remote configuration; does not fetch |
| `remote set-url NAME URL [--push]` | Replace one local URL after checking its current configuration |
| `remote remove NAME` | Remove local remote settings/tracking refs; never the server repository |
| `remote rename OLD NEW [--preview]` | Rename a remote and its tracking refs |
| `remote prune REMOTE [--preview] [-- REF...]` | Remove tracking refs the server no longer has |
| `fetch REMOTE` | Explicit fetch with refreshed status and branches; `--tags` adds all remote tags |
| `pull [--ff-only \| --merge \| --rebase] [--preview]` | Fetch configured upstream; default fast-forward-only, explicit merge/rebase integration or preview |
| `rebase continue \| skip \| abort [--preview]` | Recover a paused pull rebase through a fresh guarded review |
| `rebase start TARGET [--interactive] [--plan FILE] (--preview \| --expect-original ID --expect-onto ID)` | Reviewed rebase, optionally with a data-only plan |
| `push REMOTE LOCAL_BRANCH REMOTE_BRANCH [--set-upstream]` | Explicit single-branch push with an authoritative remote-outcome field |
| `push-lease REMOTE LOCAL_BRANCH REMOTE_BRANCH --preview` | Review one lease-protected replacement; apply with `--source` and `--expect` full IDs |
| `push-atomic REMOTE --preview -- LOCAL:REMOTE...` | Review atomic publication; apply with four-field mappings described above |
| `tag list [--limit N]`, `tag inspect NAME` | Bounded local tag list and exact tag details; `--order name\|newest` |
| `tag create NAME REV [--message MESSAGE] [--preview]` | Lightweight or annotated local tag; `--sign` signs it |
| `tag delete NAME --expect TAG_OBJECT_ID [--preview]` | Guarded local deletion with a recovery ref |
| `tag remote REMOTE` | Tags advertised by the push destination, compared with local tags |
| `tag publish NAME --remote REMOTE --source TAG_OBJECT_ID [--preview]` | Publish a tag the remote does not have |
| `tag delete-remote NAME --remote REMOTE --expect REMOTE_TAG_ID [--preview]` | Delete a remote tag only at the reviewed ID |
| `worktree list` | Registered checkout paths, branches, lock state and IDs |
| `worktree create PATH --branch NAME [--start REV] [--existing] [--preview]` | Protected linked checkout creation |
| `worktree remove PATH --expect HEAD_ID [--preview]` | Clean linked checkout removal with recovery details |
| `worktree move PATH DESTINATION`, `worktree repair PATH --at FOLDER`, `worktree prune PATH` with `--expect HEAD_ID [--preview]` | Move, re-link or unregister a linked worktree |
| `clone (--local PATH \| --url URL) DESTINATION` | Clone and publish without replacing an existing destination |

All command flags accept separate values, for example `--limit 50`; `--limit=50`
and combined short options are not supported. Unknown flags, duplicate options,
missing required arguments and conflicting modes are rejected before Git starts.
Global options may appear before or after a command, but not after `--`.
A message literally equal to `--json` is a message, not a global option.

`--all` history includes local and remote-tracking branches plus detached HEAD,
not tags or stashes. `has_more` reports that the selected limit was reached with
more commits available. No cross-process history cursor is exposed in schema 1.
Normal diffs do not include untracked file contents; binary markers are preserved.

The CLI covers the commands above. Rebase plans that run commands, signing setup,
LFS pruning and forced unlocks, SSH key creation and unlocking, hosting CI/thread
management and other advanced desktop features are not
exposed as CLI commands yet. No backend behavior is replaced.

## JSON contract (schema 1)

`--json` writes **one complete UTF-8 JSON document followed by a newline to
stdout**, on both success and handled failure. It contains no banners or progress.
Cancellation feedback goes to stderr. Human-readable mode escapes terminal control
characters; use JSON to retain exact patch content instead of redirecting a human diff.

```json
{
  "schema_version": 1,
  "version": "0.84.0",
  "command": "status",
  "ok": true,
  "data": {
    "head": {"kind": "branch", "name": {"text": "main", "hex": null}, "target": "FULL_OBJECT_ID"},
    "upstream": null,
    "ahead_behind": null,
    "entries": []
  },
  "error": null,
  "warnings": [],
  "diagnostics": null
}
```

`command` is a fixed canonical label: grouped branch commands use `branch-create`,
`branch-switch`, `branch-rename`, `branch-delete` or `branches`; stash commands use
`stash-push`, `stash-apply`, `stash-drop` or `stashes`; remote commands use
`remote-add`, `remote-edit`, `remote-remove` or `remotes`. Stage/unstage with `--all`
use `stage-all`/`unstage-all`. Other labels match their verbs, with `merge-continue`
and `merge-abort` for recovery. Reviewed pushes use `force-push`/`force-push-preview`
and `atomic-push`/`atomic-push-preview`. Tags and worktrees use `tags`,
`tag-inspect`, `tag-preview`, `tag-create`, `tag-delete`, `worktrees`,
`worktree-preview`, `worktree-create` and `worktree-remove`. Syntax failures use `usage`.
Conflict, selection, bisect and remote-tag commands use `hunks`, `stage-selection`, `unstage-selection`,
`conflict-show`, `conflict-preview`, `conflict-save`, `conflict-keep`,
`mark-resolved`, `bisect-status`, `bisect-preview`, `bisect-start`, `bisect-good`,
`bisect-bad`, `bisect-skip`, `bisect-resume`, `bisect-restore`,
`bisect-auto-preview`, `bisect-auto`, `remote-tags`, `remote-tag-preview`,
`remote-tag-publish`, `remote-tag-delete`, `stash-show`, `stash-apply-preview`,
`stash-apply-selection`, `stash-selection-preview`, `stash-selection`,
`stash-control-preview` and `stash-pop`; `stash save -- PATH...` keeps `stash-push`.
Undo, commit operations, rebase start and reset use `undo-preview`, `undo-commit`,
`commit-preview`, `amend`, `cherry-pick`, `revert`, their `-continue`, `-skip` and
`-abort` forms, `rebase-preview`, `rebase`, `interactive-rebase`,
`recovery-preview`, `reset-soft`, `reset-mixed` and `reset-hard`; the reads use
`reflog`, `signature-verify`, `signing-read`, `file-blame`, `file-history`,
`history-search`, `file-search` and `compare`. Maintenance, LFS and SSH use `remote-rename`,
`remote-prune`, `remote-branch-delete`, `branch-fast-forward` (each with a
`-preview` form; prune's is `remote-cleanup-preview`), `worktree-move`,
`worktree-repair`, `worktree-prune`, `tag-sign`, `identity-read`, `identity-save`,
`lfs-status`, `lfs-preview`, `lfs-track`, `lfs-untrack`, `lfs-fetch`, `lfs-push`,
`lfs-checkout`, `lfs-locks`, `lfs-lock-preview`, `lfs-lock`, `lfs-unlock`,
`ssh-discover`, `ssh-check`, `ssh-binding-review` and `ssh-binding-save`.

Fields may be added within schema 1; consumers should ignore unknown fields.
Changing field meanings or types requires a new schema version. Repository-derived
bytes (names, paths, messages, patches) use `{ "text": "...", "hex": null }` when
UTF-8 is valid, otherwise `{ "text": null, "hex": "lowercase hexadecimal bytes" }`.
Never interpret display text as losslessly decoded bytes when `text` is null.
Non-Unicode OS paths on Windows use `text: null` and an exact `utf16` unit array.
Object IDs are full strings; absent values are `null`; empty lists are `[]`.

Examples of data fields:

- Status: `head`, `upstream`, `ahead_behind`, and `entries` with index/worktree
  change names, kind, literal path, optional original path and submodule state.
- History: `commits` with IDs, parents, author/committer, encoding and message;
  `has_more`. Diff: `patch` plus `byte_count`.
- Writes: `command_result` and fresh observations such as `status`, `branches`,
  `stashes` or `operation`. Each observation is `{ok, data, error}`. An unattempted
  optional phase/observation is `null`.
- Push: `remote_outcome` is `not_attempted`, `confirmed_success`, `rejected` or
  `unknown`, independently of command success. `not_attempted` means the push
  process never started. A cancellation or failure after process start remains
  uncertain unless Git returned a conclusive ref status. Parsing, upstream
  verification and local observations are separate.
- Pull: `fetch`, `update`, `upstream`, `status`, `branches`, `operation`; an update
  has its own verification result. A completed fetch can precede a refused update.
- Init/clone: `published`, destination, staging directory and individual phase results.
  A published repository remains successful if reopening/empty-directory cleanup fails.

`ok` means the primary operation succeeded, not that every observation succeeded.
Warnings and failed observation fields do not turn confirmed success into a failed
mutation. Conversely, cancellation/error does not establish rollback. If a commit
completed before cancellation, its ID appears in the refreshed status. Do not retry
commits or pushes automatically just because an exit code is nonzero.

Errors contain `code`, `message` and `guidance`, plus applicable numeric OS/Git
details. Git stderr, credential-bearing URLs, helper messages and hook output are
never echoed by this CLI. Unknown failures get an explicit generic code and guidance;
the backend's typed auth/transport/push/merge errors retain their specific codes.
JSON results intentionally include repository metadata and file/patch content;
diagnostic logs do not.

## Exit codes and scripting

| Exit | Meaning |
|---:|---|
| 0 | Primary operation succeeded; inspect warnings and observations |
| 1 | Operation failed, was refused, or its result could not be written |
| 2 | Invalid command syntax; Git was not started |
| 124 | A Git command, or a `bisect run` test, exceeded its limit |
| 130 | Cancelled; inspect final observations before retrying |

```powershell
$cli = Join-Path $PWD 'target\release\rocket-cli.exe'
$repo = Read-Host 'Full path to an existing Git repository'
$text = & $cli --repo "$repo" --json status
$exitCode = $LASTEXITCODE
$result = $text | ConvertFrom-Json
if ($exitCode -ne 0) { throw $result.error.message }
$result.data.entries | ForEach-Object { $_.path.text }
```

If the receiving process closes stdout, the CLI reports delivery failure on stderr
and exits nonzero without rerunning the operation. An abruptly terminated process
or broken output pipe cannot guarantee delivery of a JSON document.

## Cancellation and diagnostics

Ctrl+C/Ctrl+Break requests cancellation through the backend token, terminates owned
Git/helpers and waits for final write observations. Keep the process running until
it returns. Repeated Ctrl+C does not bypass cleanup. `--cancel-after-ms N` provides
the same mechanism for automation (0 means pre-cancelled, maximum 86,400,000).
`--timeout-secs N` sets each Git command's limit (1..86,400, default 600), including
the version probe. It is not a deadline for the whole operation: fresh reconciliation
reads can run after interruption. A timed-out or cancelled clone retains its sibling
staging directory for inspection; existing files are not recursively removed.

By default, diagnostics use `%LOCALAPPDATA%\Rocket\logs\cli\rocket.jsonl` with
the existing bounded backend rotation. On Linux the folder is `logs/cli` under
`$XDG_STATE_HOME/rocket`, by default `~/.local/state/rocket`. `--log-dir ABSOLUTE_PATH` or `ROCKET_LOG_DIR`
selects a dedicated directory. `--no-log` or `ROCKET_LOG=off` disables logging.
`diagnostics` gives the actual log path, session ID and dropped-event count.
Logging failure is a warning, never a reason to repeat a successful write.

Crash metadata is separate from operation logs. Operational CLI commands capture
Rust panics and best-effort unhandled Windows exceptions locally, without panic
payloads, repository data, credentials or dumps. The default directory is
`%LOCALAPPDATA%\Rocket\crashes` (on Linux, `~/.local/state/rocket/crashes`); explicit `--log-dir` uses its `crashes` subfolder
for isolated runs. `--no-log` controls operation logging, not crash capture.
Initialization failure adds `crash_capture_unavailable` without changing the
requested operation. Reports are never uploaded. See [capture limits](SECURITY.md#local-crash-reports-0920).

Request spans describe report delivery; for write results inspect the backend's
write/phase/outcome records and the JSON `command_result`. Numeric process timings
and actual cancellation cleanup timing remain available. Arguments, paths, commit
messages, patches, URLs and raw output are excluded from diagnostic records.

Git Credential Manager and system SSH retain responsibility for authentication;
the CLI neither installs helpers nor changes global Git/SSH settings. `--git PATH`
selects an existing Git executable explicitly. Initial acceptance used temporary
local repositories, isolated configuration and synthetic hook contents. Follow-up
HTTPS and SSH fetch/pull/push checks on `JasXTD/rocket-auth-test` passed 44 CLI
invocations using two temporary branches, both removed with guarded cleanup.
See `work/cli-github-results.json` and the live CLI section in `VALIDATION.md`.

Those checks used the existing GCM login and loaded Windows OpenSSH key through
fixture-only configuration. No stored credential was erased, token/key revoked,
or global Git/SSH setting changed. Authentication failure cases disabled access
only in the disposable client. Cancellation used the production token with
controlled real-Git response/pre-push gates; the 6/12-second test deadlines are
not cancellation latency. Actual Git stop time was 4.741–12.661 ms. CLI completion
followed 228–669 ms after stopping feedback in seven cases; one SSH case took
5.518 seconds, with 5.094 seconds measured in launching the follow-up branch
refresh. A completed cancellation can still need time to collect fresh state.

### Post-push refresh in 0.49.1

Ordinary push now starts its fresh branch and working-tree status reads together
after the write stops, retaining the write gate until both finish. Each observation
has its own result and warning. A refresh error cannot turn a cancelled push into
success or hide the possibility that the remote already accepted it. Bare sources
still return `status: null`. There is no cached branch list or automatic retry.

The JSON schema and diagnostic fields are unchanged. Concurrent `refresh` and
`git_start`/`git_end` records can interleave: pair by session, span kind and `id`.
`cancel_to_stop_us` measures owned-process cleanup, while the later branch/status
events measure reconciliation. `spawn_us` includes synchronous OS process startup;
its intermittent stalls are distinct from branch enumeration and Git stop time.

A matched local benchmark (2,000 refs, eight cancellations per version) reduced
median stopping-feedback-to-exit time from 586 ms to 326 ms. Direct Git alone
improved from 415 ms to 299 ms; the response-holding fixture improved from 653 ms
to 347 ms. The original 5.094-second spawn outlier did not repeat, so this is not
a guarantee that Windows process startup cannot stall. See
`work/cancel-refresh-comparison.json` and the 0.49.1 section in `VALIDATION.md`.


## Squash and no-commit merges

`merge start REV --squash` prepares the combined changes without creating a merge
commit or `MERGE_HEAD`. Inspect the resulting index/files, resolve conflicts if
needed, then use an ordinary `commit`. Continue/abort merge does not apply to a
squash; reviewed discard/restore in the UI can undo prepared files.

`merge start REV --no-commit` prevents fast-forwarding and prepares a merge for
inspection. Finish with `merge continue --message MESSAGE` or restore the original
checkout with `merge abort`. Both choices protect existing local changes and leave
HEAD unchanged while preparing. The four merge-mode flags are mutually exclusive.
JSON includes `data.mode` and `data.completion`; always inspect the reported
operation state and errors after cancellation or conflicts.

The other 0.69 cleanup/saved-work commands are currently available in the desktop
review dialogs and backend API, not new standalone CLI commands.

## Pull strategies and recovery

`pull` and `pull --ff-only` keep the original fast-forward-only behavior and JSON
fields. Divergence requires an explicit `pull --merge` or `pull --rebase` choice.
These use the desktop backend's fetch ticket, exact fetched commit, local review
and revalidation immediately before integration. No automatic stash, second fetch,
retry, strategy fallback or Git-config change is performed.

`pull --merge --preview`, `pull --rebase --preview` and `pull --preview` fetch and
report the proposed integration without applying it. Fetch can update objects and
remote-tracking refs; HEAD/index/working files remain unchanged by the preview.
A later invocation fetches and prepares a new review. This is not an executable
cross-process approval ticket. Read `preview.can_apply` before deciding to apply.

Explicit-strategy JSON includes `strategy`, `preview_only`, `fetch`, `preview`,
`integration`, and current `status`, `branches` and `operation` observations.
The preview names full original/target IDs, incoming/local-only counts and rebase
replay IDs. Integration includes the typed merge/rebase failure, fresh observations
and, for a rebase, the original-tip recovery reference. A failure after fetch retains
its fetch report. Cancellation is not rollback; inspect the final observations.

For merge conflicts, resolve files, `stage -- PATH...`, then
`merge continue --message MESSAGE`; `merge abort` uses the existing protected
recovery path. For rebase conflicts, resolve and stage, then `rebase continue`.
The `conflict resolve`, `conflict keep` and `conflict mark-resolved`
commands resolve and stage with the desktop editor's checks and recovery copies.
`rebase skip` discards the stopped commit's tracked changes and skips that commit;
`rebase abort` restores the original checkout under the backend's recovery checks.
Each rebase recovery command accepts `--preview` to inspect its fresh review
without applying. Untracked/ignored collisions and changed recovery state are refused.
Interactive plans that run commands must be completed in the desktop UI; this CLI
recovery path does not run those commands.

All modes require the backend's supported checkout/index state. Dirty tracked
files, an ongoing operation, missing upstream configuration and unsupported replay
ranges return typed errors with guidance. Diagnostic records retain fixed operation
and step labels without messages, paths, remote URLs or raw hook output.
