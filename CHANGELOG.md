# Changelog

Rocket is currently a development application. Versions describe local builds;
an entry here does not mean a public installer has been released.

The shared version in the root `Cargo.toml` is used by all workspace crates,
the About dialog, MCP server information and diagnostic session records. Update
that value and this file together. Record observable changes and migration notes;
keep detailed test evidence in `VALIDATION.md`.

## 0.91.3 — Structured counter scanner correction

- Secret protection distinguishes narrowly structured source constants made of
  a namespace, descriptive camel-case label and ordered decimal cycle from
  opaque credentials. The same pattern applies to current files and old history;
  no project names, fixture words, file paths or object IDs are allowlisted.
- Credential-specific fields, unordered digits, appended material and incomplete
  literals retain their checks. Known provider-key/private-key detection takes
  precedence. Scan limits, cancellation and redacted diagnostics are unchanged.

## 0.91.2 — Word-label scanner correction

- Secret protection recognizes complete, low-diversity word labels in explicit
  `token`/`secret` source constants, including labels that change case between
  words. This avoids false positives on synthetic diagnostics in current files
  and existing history, without recognizing project names or skipping test files.
- Credential-specific fields, opaque payloads, concatenated values and known
  provider key formats retain their checks. Commit/push history inspection,
  cancellation and redacted diagnostics remain enabled.

## 0.91.1 — History map security limit

- Git history maps refuse scopes exceeding 12,000 parent links, including Load
  more. The last completed snapshot stays visible; narrow the branch, base or
  literal path scope to read a smaller graph.
- History layout indexes parent connections and available lanes so dense valid
  graphs do not repeatedly scan every connection for every commit. First-parent
  lane ordering and stable positions across filters are preserved.

## 0.91.0 — Map exploration and Mermaid export

- Collapse Code folders/package directories while retaining connections between
  groups; focus on a selected node and its direct neighbors in all three maps.
  Search also finds files inside collapsed groups and keeps retained node positions.
- Scope history by revision/branches, excluded base and literal file/folder path.
  Load more grows the same immutable snapshot in 500-commit steps, up to 2,000;
  Refresh reads current tips. Boundary parent links and incomplete scopes are visible.
  Browse loaded nodes in 240-node sections; search spans the loaded snapshot.
  Stable control identities preserve search focus when paging or scope rows change.
- Review, copy and export the visible map as Mermaid text with pinned object IDs.
  No additional renderer or dependency. New-file publication preserves existing
  destinations, including racing external creators, with cancellation before publication.
- Added affected backend/UI tests and isolated localhost MCP coverage for scopes,
  groups, neighbors, Unicode exports, cancellation, external changes and compact layouts.

## 0.90.0 — Repository map

- Add native Code, Git history and People maps backed by pinned local Git
  objects. Browse committed folders, literal Cargo path dependencies, merge
  ancestry, canonical authors, changed areas and shared-file connections.
- Search, pan, zoom, focus, inspect nodes and open commit diffs. Compact windows
  use a details drawer. Stop or a failed read retains the previous snapshot;
  explicit refresh follows external changes without touching local edits.
- Bound history and graph size and label partial results. Record named map
  operations and read steps without repository contents in diagnostic logs.
  No new dependency or hosted service is required.

## 0.89.9 — Independent AI API-key guard

- Add **Guard AI API keys** in Preferences → Security, enabled by default and
  independent of general Secret protection. Saved profiles retain their general
  guard choice; the new guard starts on. Both settings apply to open repositories.
- Recognize supported OpenAI, Anthropic, Groq, xAI, OpenRouter, Hugging Face and
  standard Google API-key formats, plus DeepSeek and GLM/Z.ai shapes in credential
  assignments. Inspect messages, staged files, imported commits and outgoing
  history locally, with bounded scans, cancellation and redacted errors.
- Show both guard states in push review and MCP state. MCP cannot change either
  setting; the CLI enables both without a bypass flag. No provider validation
  requests or dependencies are added. Unknown formats can still be missed.

## 0.89.8 — Short marker constant recognition

- Recognize complete constant literals made from uppercase label words and a
  short hexadecimal ID in ambiguous `secret`/`token` fields. This corrects false
  refusals for diagnostic markers in staged content and older Git history.
- Match declaration, value and string boundaries without application names,
  fixture-file exclusions or a history bypass. Password/API-key fields, opaque
  credential literals, GitHub tokens and private keys retain their checks.

## 0.89.7 — Broader credential syntax matching

- Detect credential literals in typed assignments, Go shorthand declarations,
  Rust/Python raw and byte strings, and C# verbatim strings. Keep source/type
  references distinct from literal values and preserve complete string boundaries.
- Classify compound protocol labels independently of the generic credential
  threshold, fixing false refusals for varied event names without naming specific
  applications. Credential-specific fields and opaque payloads retain protection.
- Handle incomplete literals consistently under LF and Windows CRLF. The shared
  checks apply to staged content, commit messages and older push history.

## 0.89.6 — Protocol acknowledgment secret-scan correction

- Recognize a single escaped LF or CRLF at the end of a complete quoted protocol
  identifier in a generic `token` field. This fixes false credential refusals for
  line-oriented acknowledgment fixtures, including those in older push history.
- Keep credential-specific fields, opaque payloads, appended data and incomplete
  literals under the existing checks. No test-file or published-history exemption.

## 0.89.5 — Clearer folder scan review

- Search discovered repositories by name or path, hide saved entries and select
  only the visible results. Hidden selections are retained and counted explicitly.
- Keep search, destination and selection controls above a compact, scrolling list
  with relative paths. Empty, already-saved and stopped states have clearer actions.
- Show bounded explanations for skipped entries and validation progress. Unreadable
  scan roots fail visibly; all-saved results no longer show a checked Select all.

## 0.89.4 — Scan folders for saved repositories

- Repository manager can scan a parent folder in the background, review discovered
  repositories and add selected shortcuts to a saved folder in one operation.
- Existing shortcuts keep their names and groups; linked worktrees appear as
  separate repositories. Unicode paths, bare repositories and nested repositories
  are supported. Git metadata and folder links are not traversed.
- Stop cancels discovery or validation without partially saving the selection.
  Selected repositories are rechecked before saving. Large scans report explicit
  limits and skipped entries; compact windows keep the review actions visible.

## 0.89.3 — Portable Windows runtime dependencies

- Portable Windows x64 packages include the reviewed, Microsoft-signed Visual
  C++ runtime beside Rocket, avoiding a separate system runtime prerequisite.
- The isolated Lua worker links its runtime statically so its sandbox does not
  depend on the main application's folder or a system Visual C++ installation.
  Worker identity checks and sandbox restrictions remain intact.
- Release checks cover missing/replaced runtime files, ordinary startup, bundled
  Git, Unicode paths, external changes, terminal execution and compact windows.
  The HTTP fixture now stays alive through the bounded Git attempt; focused
  timing checks retain their existing limits and run sequentially.

## 0.89.2 — Complete privilege guards and npm scan coverage

- Rocket refuses elevated Windows startup and child-process launches across Git,
  custom commands, automatic bisect, external tools, terminals, MCP and update
  helpers. Reopen Rocket normally to use these features. Unsigned builds remain
  supported, with the existing Lua sandbox and embedded worker hash verification.
- The built-in Command Prompt starts with AutoRun disabled.
- Malformed or unsupported npm source metadata now reports incomplete coverage
  before a cached or fresh OSV result can be accepted. An enabled pre-push check
  stops that attempt before publishing refs; valid recognized sources still work.
- Secret checks distinguish structured protocol identifiers from credential
  assignments while retaining checks for other values and repository history.

## 0.89.1 — Clearer pre-push security popups

- The dependency check shows completed, scanning and waiting stages, with an
  explicit explanation of automatic continuation and warning review.
- The Review popup groups findings by package version, summarizes severity and
  keeps reported fixes visible. Package details and scan scope expand on demand.
- Cancel push is emphasized; Push anyway remains an explicit, single-attempt
  choice. Long findings scroll while the header and decision buttons stay visible.
- Scan policy, the opt-in preference and existing secret protection are unchanged.

## 0.89.0 — Optional dependency review before push

- Preferences > Security adds **Scan dependencies before pushing**, off by default
  and independent of background dependency warnings.
- When enabled, OSV checks the committed dependency files at every pushed tip.
  Clean, complete results continue automatically. Advisory matches or limited
  coverage pause for **Push anyway** or **Cancel push**; scanner/data failures
  stop the attempt and link to scanner setup.
- The same guard covers normal, force, atomic, tag, commit-and-push and GitHub
  publication pushes. Existing secret protection still runs first. Ref deletions
  need no dependency scan. Cancelled commit-and-push retains its local commit.
- Enabling this option writes profile format v24. Earlier profiles default off;
  older Rocket versions cannot read an enabled v24 profile.

## 0.88.1 — Clearer update controls

- A dismissible startup notice opens the available update. The status-bar entry
  keeps it accessible without another network check; Help can still check again.
- Update dialogs show installed/available versions, real release-note excerpts
  with full notes, download/verification phases and a separate restart choice.
- Compact windows scroll the content while keeping Close and the action footer
  reachable. Cancel check/download shows Stopping while the worker finishes;
  close and Escape also cancel active preparation. Errors keep their full details.

### Security audit fixes

- Refuse backend commands when Rocket is elevated or has a high-integrity Windows
  token, so repository hooks and helpers cannot inherit administrator rights.
  Reopen Rocket normally; configured hooks remain available for normal use.
- Treat placeholder names as complete values. Prefixes such as `example`, `dummy`
  and `process.env` no longer exempt an appended literal password from commit,
  message or push-history checks. Complete references remain supported.
- Report npm dependency coverage as incomplete when package-lock or shrinkwrap
  entries lack resolved sources. This guards an OSV 2.7 false-negative case before
  accepting either scanner output or a previously cached clean result.

## Unreleased — Dependency scanner setup

- Security now guides first-run users through choosing the reviewed OSV-Scanner
  and downloading public advisory data for the open repository. Scanner choices
  persist per profile; OSV remains an optional, separately obtained executable.
- Unreviewed, missing or changed scanners are refused. Failed/cancelled choices
  preserve the prior selection, and another window's change requires reloading.
- Fresh profiles show when scanner setup is required. Local caches remain separate
  for each user. With warnings enabled, an advisory download is followed by an
  offline scan.

## Unreleased — Password detection

- Secret protection now checks quoted passwords beginning with `$` or `%` and
  passwords containing punctuation such as `:`, brackets and escaped quotes.
  These literals could previously pass both commit and push checks.
- Complete environment/template placeholders remain accepted. Prefixes alone
  and placeholders with appended literal material no longer exempt a quoted
  password from the existing credential checks.

## Unreleased — Commit link dialog

- Clearer destination cards and a full, wrapped link preview. Copy link and Open
  in browser remain visible; custom remote/server settings are under Link options.
- Compact windows scroll the body while keeping Close and the footer reachable.
  Correcting settings clears stale errors and destinations before a fresh read.
- Local link generation, explicit remote selection and cancellation remain intact.

## Unreleased — Rebase stopped after its branch moved

- A rebase whose branch has moved from its saved tip now says so in the paused banner
  and asks you to finish it with Git. It used to say "stopped as planned" and offer
  Continue, which the review then refused. This covers Stop landing after the branch
  moved and another client moving the branch during a pause. Continue, Skip and Abort
  are still refused the same way, and history and the recovery reference are untouched.

## Unreleased — Pull preparation

- Pull preparation avoids duplicate checkout reads and combines upstream/remote
  configuration reads at each checkpoint. Reviews still recheck live state after
  calculation and again under the write lock before applying. Included config,
  hidden index flags, cancellation and stale targets remain guarded.
- Diagnostics separate ancestry calculation from the surrounding pull checks.

## Unreleased — Refresh timing diagnostics

- Activity details now separate bundled Git verification from process startup and
  execution, making remaining refresh delays easier to locate. Timing labels wrap
  inside narrow cards. Verification checks and fallback scans remain enabled.

## Unreleased — Revert, Cherry-pick and Abort import screens

- Revert and Cherry-pick are in a commit's right-click menu, for one commit or a
  selection.
- Their reviews show the commit, a small picture of what happens to your branch and a
  colored diff of what the new commit changes, instead of a raw patch and a full ID.
  Several commits show as a list beside each commit's own change.
- Reverting a merge opens already reviewed, on the side it merged into, and names both
  sides by commit. It used to open on a red error and two raw IDs.
- A stopped revert or cherry-pick gets the same banner as a rebase: which commit, what
  to do next, Continue and Abort, and Skip when commits come after it. Skip is gone
  from a single commit, where Abort does the same. It's marked paused, not failed, in
  Activity and opens its first conflict, and Continue revert or Continue cherry-pick
  takes over the main button in Changes once your fix is staged.
- Continue, Skip and Abort of a revert or cherry-pick show the same steps as an import,
  with your fix or the edits that go as a colored diff.
- Abort import lists every patch: what comes off, where it stopped and what never
  arrived. Skip patch and Abort import can show the edits that go.
- If you commit a fix by hand partway through several commits, Continue counts it as
  done and Abort says it will leave your branch where it is, which is what Git does.
- Reverting changes that are already gone says Nothing to undo instead of failing.
- These refusals read in plain words. Error codes stay in Activity details and the MCP
  dialog state. Activity says what finished, such as Reverted 3 commits on feature.

## Unreleased — Commit series and rebase screens

- Rebase's Onto is now On top of, beside a read-only From that names your branch, so
  the dialog says what moves and what stays. The same word replaces Commits after,
  Destination, base and rebasing onto in interactive rebase, the paused reviews, the
  sidebar menu, branch drops, Sync, Stash + rebase, Pull and the conflict editor.
  `rocket-cli`'s `--expect-onto` and its `onto` JSON key are unchanged.
- Export Commit Series has its own row in Repository → Patches, a palette entry and a
  right-click on commits. Its range is First to Last, both included, picked from
  lists; it used to be an excluded commit and a typed tip. It can't start at a
  repository's first commit, as before.
- Import Commit Series is in the palette. Both dialogs show each commit beside its
  colored diff instead of raw mail text, with a Browse button for the folder.
- A paused rebase or import says which commit or patch it stopped at and what to do
  next, opens its first conflict, and is marked paused, not failed, in Activity. While
  rebasing, the window names your branch instead of a detached commit. Once your fix
  is staged, Continue lights up and the main button in Changes becomes Continue import
  or Continue rebase.
- Continue, Skip and Abort show the series step by step with the changes that matter.
  Fixed: at patch 2 of 4 these dialogs opened on patch 1, so Skip previewed the wrong
  patch. Continue with nothing staged is refused in its review; Git refused it anyway.
- Series and rebase refusals read in plain words. Error codes stay in Activity details
  and the MCP dialog state. Finished work names what happened and opens Commits.
- Import, Continue, Skip and Abort are named the same everywhere: Continue import, Skip
  patch and Abort import.
- Interactive rebase opens at its final size, with the commit list and editor already in
  place, so it no longer jumps when the plan loads. On top of fills its row.

## Unreleased — Bundled Git starts faster

- A command on the bundled Git no longer starts by reading all 871 of its directories, which took
  about 190 ms each time. Rocket now checks the drive's change journal before each launch, and reads
  the directories after a change to them and at most every 2 minutes otherwise. A file added to the
  bundle still blocks the next Git command.
- A bundle on a drive with no active change journal reads its directories before every launch, as it
  did before. Second drives often have none, and turning one on takes an administrator.
- A junction set on an empty bundle folder is now refused at once. Before, it was only noticed after
  the program that set it closed its handle.

## Unreleased — New MCP test windows stay in the background

- A new `rocket-mcp` window, main or review, no longer takes the Windows foreground when it opens, so
  the user's typing can't land in it. It opened as an active window, and Windows hands the foreground to
  one of those once the desktop has seen no input for 200 s. The desktop app is unchanged: its review
  windows still open active.

## Unreleased — One reconciliation snapshot after fetch

- UI fetch and the fetch step of Pull now read one fresh repository snapshot for
  both reconciliation and display. They no longer discard status/branch/operation
  observations and immediately read them again, including after cancellation.
- A changed history view or automatic-refresh setting requests a new matching
  snapshot. Live repository watchers and fresh diff/config reads remain in place.
- Standard backend/CLI fetch and pull reports retain their independent results
  and refresh errors. Bundled Git verification is unchanged by this optimization.

## Unreleased — MCP scope check covers force, atomic and tag pushes

- Under MCP, force-with-lease, atomic and remote tag pushes skipped the fixture scope check when they
  ran, so a local receiver's hooks could run and a reviewed command's own `-c` options came after the
  hook and HTTPS policy. They now get the destination check and hook refusal `SECURITY.md` describes,
  the same as a plain push, and that policy comes last. A Git command with a global option Rocket
  doesn't recognize is refused under MCP. The desktop app and CLI don't use this check.

## Unreleased — Background network refresh

- Fetch, fetch-for-pull and push now reconcile in the background without blocking
  repository navigation. Full reads still refresh the selected diff.
- Reuse issue-link rules only when effective Git configuration and repository identity
  match, including included config files. Display reads reuse the verified repository
  instead of rediscovering Git. Explicit refresh still rereads the rules.
- Superseded rule reads are cancelled and completion wakes the viewport. No changes
  to bundled-Git verification, credentials or dependencies. Focused test evidence is
  recorded in `VALIDATION.md`.

## Unreleased — Public website

- `docs/` now holds a static site for GitHub Pages: Home, Get Rocket and a page-not-found, in light
  and dark, with a 29-second demo video. It has no build step, external fonts or analytics.
- The site offers no download, because there is no public release. Get Rocket gives the build steps.
- `tools/website/` previews the site the way Pages serves it, checks its links and content, and
  retakes its pictures and video from a disposable demo repository.

## Unreleased — Conflict tags on wrapped lines

- In the conflict editor, tags and buttons no longer sit on the code when a line wraps: the resolved
  tag and Undo in Result, and Use ours, Use theirs, Used and Not used in the side panes. A line with
  a button now wraps before it, and the words take a row with room for them. Used and Not used also
  show when only a later row has room. Other lines wrap as before, and short lines look the same.
- With Linked scrolling on, a side pane that wraps can scroll to its end in a narrow window.

## Unreleased — MCP test window stays in the background

- The MCP test window no longer takes the foreground on each MCP call, so typing in another app stays
  in that app. It was forced forward with a simulated Alt key press, which also reached the other app.
  A minimized test window is still restored, because it can't draw until it is.

## Unreleased — MCP review windows stay in the background

- MCP calls to a review window no longer take the foreground either, so typing in another app stays in
  that app. A minimized review window is still restored.

## Unreleased — All diagnostics, On origin and MCP input check

- Activity & diagnostics (the Activity drawer's All diagnostics) has the header of other pages,
  with Close at the right, and two cards. Operations read in words (Fetch, Review tag), times use
  this computer's clock, and a failure shows Rocket's explanation and next step with its code and
  where it stopped. It no longer says Git reported a problem when Rocket refused something.
- Reads Rocket does by itself are hidden unless they fail or are cancelled. Show reads lists them,
  and a search always covers them. Rows are shorter, and rows that need a look say why.
- Steps lists each step once, nested, with setup folded. Records is the full timeline it replaces.
- The page no longer jumps while Live reads, and it has its own reading, nothing to show, no match,
  failed, stopped and missing-records states. A narrow window shows Operations and Details as tabs.
  Request and session moved to the card's foot, and log status to the list's foot, in words.
- On origin (the Tags page's remote tab) has count buttons for what stands where. The first tag is
  selected after a read, the read stays when you change tabs, and every tag shows, with no Show more.
- Where it points gives the commit on both sides, named when this repository has it. The note no
  longer mixes tag object and commit IDs, and "Not in this repository" no longer shows for a commit
  that is here. An indirect tag no longer claims to be lightweight.
- Publish and Delete show only for tags they apply to. A tag that differs says how to replace it, and
  Show in history joins this tab. After a verified change the row updates with no further read.
- Failed reads have their own states, and the dropdown appears only with two or more remotes.
  `dialog-remote-tag-more` is gone. New ids: `dialog-remote-tags-all`, `-here`, `-there`,
  `-different`, `-same`, `-indirect`, `dialog-remote-tag-retry`, `-clear`, `-dismiss`,
  `-show-commit`, `activity-reads`, `activity-retry`, `activity-resume`, `activity-clear`,
  `activity-show-reads`, `activity-setup`, `activity-view-steps`, `activity-view-records`,
  `activity-group-N`, `activity-tab-list`, `activity-tab-details`, `dialog-remote-tags-list`,
  `dialog-remote-tags-details`.
- The MCP input check shows only after a second, in Rocket's dialog style, with its checks as a list
  and Stop instead of Cancel input. The status bar says briefly why a click was dropped. MCP clients
  keep the existing error text. Try again on On origin and Publish on This repository, which read
  the remote, now go through the check too. Once the dialog is up, a click on it no longer counts
  as using the window.

## Unreleased — Shell setting and known tools

- Preferences → Terminal chooses the shell the Terminal drawer runs: Windows PowerShell (the
  default, unchanged), Git Bash, Command Prompt, PowerShell 7 or a custom program with its
  arguments. Git Bash is the bash beside the Git Rocket uses, so the bundled Git supplies it.
  The saved format is v23, written only once the setting differs from its default.
- The drawer names the session's real shell, shows while it starts, and says why when it can't
  start, with Terminal settings and Try again. It never falls back to another shell. Command
  Prompt refuses a network folder, where it would run commands in the Windows folder.
- Git Bash saves history after each command, so End session, closing a tab and quitting keep it.
- The shell's program and arguments come only from this profile's settings. MCP can't change
  them: the page is locked in an MCP window, which always runs Windows PowerShell.
  `state.terminal.shell` names the running shell.
- Add on Preferences → External tools now lists known tools for that type: 11 in all, with the
  ones on this computer first. Choosing one only fills in the form, runs nothing and saves
  nothing. Runners open the list and choose `tools-known-empty` for the old blank profile.
- A saved tool profile whose program has gone says so on the form.
- The Preferences dialog is up to 40 pt taller, so its sidebar needs no scrolling at the usual size.

## 0.88.0 — Reviewed application updates (2026-11-01)

- Check public JasXTD/Rocket Releases once at desktop launch, with a manual
  **Help → Check for Updates** action. Review version/notes before downloading;
  installation and restart require a separate choice. No periodic checks or
  Git credentials are used.
- Verify the archive digest, file manifest, paths and sizes. Cancel preparation
  without replacing the app. Packaged installations recheck external changes,
  keep their complete previous directory, and attempt rollback if restart fails.
- Development builds offer a verified download rather than replacing their
  build directory. Changed or extra installation files block replacement.
- Move the unchanged release verifier into `tools/Verify-Release.ps1`. Packaging
  now creates a complete versioned update ZIP; no build tool publishes it.

## 0.87.3 — Codex and bundled Git security (2026-10-31)

- Require Codex to recognize all safety settings before an AI run starts, and
  refuse drafts or proposals when it reports unknown configuration or features.
  Commit messages, branch reviews and conflict proposals share these checks.
- Recheck bundled Git directory contents immediately before Git or LFS starts,
  including clients that already verified the runtime. Added files, directories
  or reparse points stop the operation; existing verified files remain locked.
- Use a new local package version for the current development source, including
  the changes described in the Unreleased entries. Earlier 0.87.2 packages stay
  unchanged. Rocket executables remain unsigned and require no certificate.
- Keep Unreleased sections out of the version list in the Release notes dialog,
  including sections between historical releases.

## Unreleased — Reliable Stop input

- Keep status-bar Stop in a stable position with a stable widget identity while
  progress text and progress bars change.
- Resolve localhost MCP Stop clicks against the latest rendered position after
  the existing visibility, fixture and expiry checks.

## Unreleased — Guide and README

- Help → Guide adds Files, Statistics, AI tools and AI review under Getting started, and
  Dependency advisories and Storage under Preferences and tools. It now says where to find
  Commit messages, Issue links and All diagnostics.
- The Guide no longer shows bold marks and code marks as typed, and Repository benchmark
  follows the Activity notes instead of splitting them from their topic.
- README and the wiki's SSH steps follow the current pages.

## Unreleased — Bundled Git

- Include a separate, relocatable Git for Windows 2.56.0.windows.4 runtime in new
  Windows x64 bundles. Automatic prefers it when included; explicit installed or
  custom selections still override it, and builds without a runtime use PATH.
- Preferences → Git adds Bundled Git, full-file verification progress and Stop.
  Stopped/failed checks and externally changed selections do not publish a choice.
  A damaged included runtime reports an error instead of silently switching Git.
- Build and packaging scripts verify the pinned inventory and preserve the vendor
  licenses. Source-only packaging remains available through `-WithoutBundledGit`.
  System Git, PATH and existing credential/SSH configuration are not changed.
- Recognize the bundled Git LFS helper in modern `ucrt64/bin` layouts while
  preserving older installations and the Git LFS security version gate.

## Unreleased — Extension commands, Jira and PlayGround

- Extensions can declare commands. Lua API 1 gains a `commands` list in the manifest: up to 8, each
  with an ID, a label and an optional target of commit or branch. A package says what a command
  acts on and Rocket decides where it shows: the palette, Repository → Extensions, or the
  right-click menu of commits and branches. Existing packages load and run as they did. There is
  no API 2; `INTEGRATIONS.md` now says API 1 can grow, with nothing removed or changed in meaning.
- A command opens its extension's page and runs there, in the same worker with the same limits as
  a button, with `ctx.command` and `ctx.target`. The target is the row you chose, and a commit or
  branch command needs the connection's Share switch. It ends in a view, or in one write that stops
  at the request review.
- Rows come from the approved copy, name included, so unreviewed text never reaches a menu. Off means
  no rows. With changed files, no server, or a saved draft that no longer fits, one row opens the
  review, the connection or the page with Clear draft, and the palette keeps the commands greyed with
  their reason. Reviews and extension details list commands; a review marks them New, Changed or
  Removed.
- Turning a connection's Share switch on keeps its saved draft, which used to be blocked until
  cleared. Turning it off still blocks a draft saved while sharing. Going back to an extension
  whose request review is open keeps the review.
- The panel API gains clickable table rows (`id`, `keys`, `selected`), pills and monospace in table
  cells, `ui.split`, `ui.segments`, `ui.code`, `placeholder` and `submit` on `ui.input`, `tone` and
  `small` on `ui.text`, and `ctx.confirmed`, which tells the next callback that a reviewed request
  went through. `ui.column` takes `small` to keep its children close. Every package gets three fixes:
  work in progress shows in the Refresh panel row, so the panel no longer jumps after every result;
  tables fill their card, with lines between rows in place of stripes that stopped at the text; and
  progress is a thin bar under its label.
- Jira 1.2.0 is redesigned: one search box for a key, words or JQL, issues beside the chosen issue,
  filters that apply at once, readable comments, and Edit and Create issue only when asked for.
  Create lists the project's issue types, so Issue type ID leaves Settings. It adds four commands.
- UI playground is now PlayGround 1.1.0: sections of live controls beside the Lua that draws them,
  and two commands that show what a command receives. It ships with Rocket, off, so an installed
  copy can update. The Lua editor's page is now titled Lua playground.

## Unreleased — Extensions

- Integrations are now Extensions, with their own group in Preferences after Connections. The
  group has All extensions and a row for each enabled extension, and a puzzle piece replaces the
  second plug. Connections keeps Accounts, SSH keys and MCP server.
- The list shows each extension with its version, what it can reach and an Enable switch.
  Install from file takes a package's integration.json, with main.lua beside it. A click opens
  the details: Enable, permissions, Update and Remove first, then the connection as one form.
  Save and Discard show only after an edit. An extension with no network has Settings instead.
- Jira is a bundled extension, listed from the first run and off. Enable reviews it and copies it
  into the extensions folder. Add Jira example is gone. Add repository example moved to Developer
  tools as Add sample, beside the folder and Open playground.
- Enable, Install, Update and Review again share one review: where the extension came from, its
  code, its three permissions and what it can never do. A file says Rocket can't confirm who made
  it. An update marks new permissions, counts changed lines against the approved copy, shows
  them, lists what stays, and leaves the extension on or off as it was. No review runs a script.
- Each enabled extension has its own entry in the sidebar, the icon rail, View → Go to and the
  palette, and a page headed with its name. An amber dot marks a review or an unchecked write. The
  shared Integrations page and its row of package names are gone. The palette adds Extensions,
  Install extension from file and Lua playground.
- Folders that can't load are rows with their reason, and one copy of a duplicate ID can be
  removed from the list. Errors show their words without the code.
- A file can't use bundled Jira's ID, so it can't inherit saved tokens. Update and Remove wait
  while a request may have reached the service.
- Stopping a Lua run just after it started could report clean Lua sandbox instead of the stop, and
  left a rocket-lua folder in Temp. Rocket now waits up to 5 seconds for the file, and how the run
  ended comes first. A successful run that still can't clean up reports it, as before.
- Migration: none needed. Folders stay in integrations, connections stay in
  integration-accounts.dpapi in the same format, and packages enabled today stay enabled with no
  new review. The Enable switch reads the connections' saved state, and one review now covers all
  of an extension's connections; Allow writes and Share stay per connection. A draft stays bound
  to the exact files and connection, so after an update it stays saved but blocked, and the Update
  review says so first.
- Automation: int-install-jira goes with its button. int-enable moves to the Enable switch, with
  int-disable as its name while on, and Save is int-save. goto-integrations opens the list, and
  each extension has goto-ext-{id} and view-ext-{id} in place of view-integrations. int-open-panel
  goes; other int-* ids stay. Lua API 1, its manifest and error codes are unchanged.

## Unreleased — General settings

- Preferences → General adds three cards, and every new setting starts at Rocket's behaviour
  before it existed. Window and Updates keep their rows and take the header bar.
- Repositories. Source code folder: where Clone's Parent folder and New repository's Project folder
  start, with Browse and ×; a missing folder or a file at that path turns the row amber and Clone
  starts next to the latest repository. Tags in fetch: where the Fetch dialog starts and what
  automatic fetch brings. Space in branch names: spaces become - or _ as you type or paste, in New
  branch, Rename, New worktree and branch recovery. Update submodules on checkout: after a branch
  switch, initialized and clean submodules move to the recorded commit with no review; others are
  left alone and named in Activity. Push after commit: Commit & Push takes the main button and
  Ctrl+Enter in Changes, and its review still opens first.
- Diff view, with a live preview: show or hide the − and + marks, font size 11 to 16, and Syntax
  highlighting for every tab. These apply to Changes, Commits, Compare and review windows. The
  Syntax checkbox in a diff's toolbar still flips one tab.
- Commit list, with a live preview: Compact branch labels folds a branch and the remote branch it
  tracks into one chip, drops the remote's name when there is only one remote, shortens folders to
  their first letter, and counts what doesn't fit as +2. Hover shows the full names.
- The Changes button reads Commit & Push, without the ellipsis.
- Migration: saved state gains format v22, written only once one of these settings differs from
  its default. Older Rocket builds then refuse the state file instead of losing data.

## Unreleased — Repository benchmark page

- View → Repository Benchmark uses the shared page header: an icon tile, "Local read timings · 3
  passes · read only", Run benchmark as the main button and a quiet Close. Stop takes Run's place
  while it runs and reads Stopping until the Git processes exit.
- The results are one Timings card. Its bar shows the run's status (Ready, Waiting for other
  operations, Pass 2 of 3 · History with a spinner, Stopping, Complete, Stopped, Failed) and the
  reads and time so far. The table aligns First, Repeat median (bold) and Range on the right;
  while it runs the read in progress has a tint and a spinner and reads still to come show
  placeholders, so the table keeps its shape. A repository with no commits shows History and
  Commit diff as "Skipped · no commits yet".
- Errors and the changed-during-the-run, unavailable-observation and limit warnings are one line
  each inside the card instead of full-width boxes above it. Copy summary moved to the card's
  foot and reads Copied after a click. The explanations moved to the Guide.
- Automation ids and the measurement itself are unchanged (benchmark-open/run/stop/close/copy/
  scroll, `state.benchmark`, the report and its JSON).

## Unreleased — Repository manager

- File → Repository manager has its own sidebar in the app sidebar's place: Library (All saved,
  Ungrouped), Folders and Workspaces with their counts, a + to make a folder or a workspace, and
  nested workspaces indented under their chevron.
- The header shows the counts with Save open tabs, Add repository and a quiet Close. The
  repositories sit in a card whose bar names the selection and holds Filter (a folder adds Rename
  and Remove); rows show the name, the path on one line and Open, Unavailable, Opening or Waiting.
  A workspace lists what it opens, marking its active tab and the nested workspace each repository
  comes from.
- The selected repository or workspace has a details card with Open, Edit and a red Remove (and
  New child) in its foot. While repositories open, Stop takes Open's place and the foot says which
  one is opening of how many; the result ("Opened 2 · 1 unavailable") stays there, and an
  unavailable repository shows its error in its own card. The save notices are gone.
- Forms use a label column and full-height fields: Add repository gets Browse (the folder picker
  in repository mode) and a Folder dropdown, and a workspace's repositories are one list with an
  Active tab column. Removal reviews say what goes and what stays, with a red Remove. A location
  without a repository reads "No Git repository at this location".
- Automation: every manager-* id is kept; manager-stop now sits in the details card's foot. New
  ids manager-browse, manager-folder (manager-folder-choice-* are inside it), manager-empty-add,
  manager-empty-save and manager-filter-clear; rocket-mcp allows manager-browse and manager-folder
  while a form is open. No library model, persistence or settings-format change.

## Unreleased — CLI maintenance, identity, Git LFS and SSH keys

- `remote rename`, `remote prune` (obsolete tracking refs only, with a recovery
  ref), `branch delete-remote --expect ID` (never the server's default branch) and
  `branch fast-forward` (to already-fetched upstreams, with a recovery ref).
- `worktree move`, `repair` and `prune` for one linked worktree, pinned to its HEAD;
  `fetch --tags`, `tag list --order newest` and signed tags with `tag create --sign`.
- `identity show` and `identity set` for the repository-local user.name and
  user.email, refused when the configuration changed after it was read.
- Git LFS: `lfs status`, `track` (`--literal`), `untrack`, `fetch`, `push`,
  `checkout`, `locks`, `lock` and `unlock` (your own locks only).
- SSH: `ssh keys` and `ssh check` read public keys and Rocket's profiles; `ssh use`
  and `ssh restore` bind this repository's `core.sshCommand` to a profile or put
  back the earlier configuration, pinned to the configuration's SHA-256.
- Writes repeat the `--preview` review and call the desktop's backend operations
  under its write ownership. New refusal codes include `remote_branch_changed`,
  `ssh_config_changed` and the backend's LFS lock codes. Forced LFS unlocks,
  pruning, and SSH key creation, import and unlocking stay in the desktop. JSON
  stays schema 1. No rocket-git, desktop, MCP, dependency or settings-format changes.

## Unreleased — Slider clicks in the MCP

- `rocket_click` now moves the Preferences → Appearance Visibility slider (`theme-visibility`):
  its release arrives one frame after the press, so egui's slider, which senses drags only,
  moves to the clicked point, the control's centre (35%). Every other control keeps its
  one-frame click, and the click guard and fixture checks are unchanged. This supersedes
  "Visibility can be read but not moved" under Themes in the MCP.
- `tools/tests/test-mcp-themes.cjs` moves Visibility from 30% to 35%, checks it in
  `state.appearance` through later clicks and reopening, and checks that a click is refused
  while Visibility is off.

## Unreleased — CLI undo, commit operations, rebase start, reset and history reads

- `commit` JSON adds `undo` (branch, commit, previous, index SHA-256). `undo-commit`
  with those values undoes that commit when nothing was staged since and no
  remote-tracking branch has it, keeping its changes staged and a recovery ref.
- `cherry-pick` and `revert` (1–100 full IDs, `--mainline`, optional HEAD pin) and
  `amend --expect HEAD_ID`, with `continue`, `skip` and `abort`.
- `rebase start TARGET`, applied with the reviewed original and destination IDs;
  `--interactive` lists the range as a plan, and `--plan FILE` applies a data-only
  plan (pick, reword, edit, squash, fixup, drop). Plans with `exec` are refused.
- `reset --soft|--mixed|--hard ID --expect HEAD_ID` keeps the old tip as a recovery
  ref and refuses work it would discard; `reflog` lists all five scopes.
- Reads: `verify commit|tag` (`--require-valid`), `signing show`, `blame`,
  `file-history`, `search` (text, path or content) and `compare`.
- Backend: `UndoCommit::index_digest`, `Repository::pinned_undo_commit` and the
  public `undo_failure_code`. Rebase reports gain `stop`. Desktop behavior is
  unchanged; no dependencies or settings formats changed.

## Unreleased — Repository benchmark

- View → Repository Benchmark measures three passes of full refresh, HEAD
  history and commit-diff preparation using the desktop's real read backend.
  Shows first-pass/repeat timings, skips empty history, flags bounded results
  and observed repository changes, and keeps partial results after stopping.
- Stop gives immediate feedback while Git exits. Closing waits for cleanup;
  normal refresh resumes afterward. Scheduled fetches wait during a benchmark.
- Copy summary exports timings/counts and fixed outcome codes without repository
  paths, names, refs, identity or content. Activity records each benchmark phase.

## Unreleased — Themes in the MCP

- `rocket_state` gains an `appearance` section: the chosen theme, the palette on screen
  (System follows Windows' app mode, reported as `windows_mode`) and the accent; while Custom
  is chosen, its base, accent, picture, size, position and visibility, and whether its
  picture is showing yet. Your own picture is named by the file name Rocket keeps it under,
  never by a path.
- Automation can now change the theme: the Preferences → Appearance controls (the MCP refused
  them while Preferences was open), the #RRGGBB field, and View → Appearance, whose row is now
  published as `view-appearance` (before, its view-theme-* items could not be reached). The
  Visibility slider can be read but not moved: an MCP click presses and releases in one
  frame, which an egui slider ignores.
- New runner `tools/tests/test-mcp-themes.cjs`: all five themes from both places, the Custom
  options, the four built-in pictures, a picture chosen in the fixture-only picker (a file
  that is not a picture is refused) and reopening the window.

## Unreleased — CLI conflicts, line staging, bisect, remote tags and stash selection

- `rocket-cli` gains the reviewed desktop operations it lacked since 0.84. Each read
  prints a review; the write repeats it and refuses a mismatch with the SHA-256 or
  object ID passed back, before the backend's own checks under write ownership.
- Line staging: `hunks -- PATH` lists changed-line IDs and the patch SHA-256;
  `stage`/`unstage --lines ... --expect-patch ...` change only the index.
- Conflicts: `conflict show`, `conflict resolve` (choices per conflict, or a whole
  result file; `--stage` refuses remaining markers), `conflict keep` (one whole
  version, pinned by blob ID) and `conflict mark-resolved`, with recovery copies.
- Bisect: `bisect status/start/good/bad/skip/resume/restore`, decisions pinned to the
  tested commit, and `bisect run` with a reviewed `.exe` pinned by SHA-256. A run
  holds write ownership for the whole search; a test timeout exits 124 and Ctrl+C
  130, both keeping the search. Results appear on the desktop's Bisect page.
- Remote tags: `tag remote`, `tag publish` (new remote tags only) and
  `tag delete-remote`, each pinned to the reviewed objects.
- Stash: `stash save -- PATH...`, `stash save --lines`, `stash show`, applying part
  of a stash (`stash apply ... -- PATH...` with `--hunks` or `--lines`) and
  `stash pop` with a backup ref. JSON schema 1 only gains commands and fields.
- Backend: public `sha256_bytes`, `BisectAutoPreview::program_digest()` and
  `ConflictPreview::original()`; five new diagnostic operation labels. Desktop
  behavior is unchanged; no dependencies or settings formats changed.

## Unreleased — Nested workspaces

- Organize saved workspaces into a tree with empty groups, child creation, parent
  selection, breadcrumbs and collapse/expand controls. Moves keep membership;
  removal promotes children and refuses duplicate names under the same parent.
- Optionally open descendant repositories with deduplication and a 64-repository
  limit. Existing tabs, terminal sessions and drafts remain open; Stop cancels the
  remaining queue. Hierarchies are bounded to eight levels and 128 workspaces.
- Existing flat settings retain their format. Nested settings use v21 inside the
  existing draft envelope; older builds refuse this format instead of losing parents.

## 0.87.2 — Security bundle (2026-10-30)

- Package the current Git HTTP safety gate and hash-bound, Windows-isolated Lua
  worker together with the application, MCP and CLI. All six executables are
  required; unsigned releases remain supported without a certificate.
- Release verification rejects missing or mismatched Lua workers, including a
  regenerated manifest. `tools/Package-Release.ps1` builds and verifies a fresh
  versioned directory and refuses to overwrite existing release versions.
- Bound conflict-rail painting and control registration independently of the
  number of conflicts; cached indexed hit testing preserves overlap selection.
  Existing choice, undo, scrolling and reviewed-save paths remain in use.
- This local package contains the current development changes described below.
  Earlier 0.87.1 bundles remain unchanged and predate these security controls.

## Unreleased — Details card scrolling

- A page's details card (Git LFS, Tags, Worktrees, Submodules and the other
  list-and-details pages) sizes its scroll area to the part that is on screen.
  With the Activity drawer open or a long notice in a short window, its last
  action (for example Git LFS Review checkout) can still be scrolled into view;
  before, it could stay hidden under the drawer.

## Unreleased — Diff overview

- Text diffs show added/removed-line markers and the current viewport in an
  overview strip. Click or drag it to scroll; when focused, arrows, Page Up/Down
  and Home/End navigate. It follows unified and split layouts and hides when the
  loaded diff fits. Markers describe the loaded diff, including its preview limit.
- The existing conflict marker rail gains a viewport indicator and scroll
  navigation, respecting linked and independent panes. Conflict selection,
  resolution choices, save reviews and recovery copies keep their existing paths.
- Summaries are cached when text/layout changes and use at most 1,024 paint
  buckets. No additional Git commands, background scans or dependencies.

## Unreleased — Menus

- Repository goes from 39 rows to 13, so it no longer scrolls: Fetch, Pull, Push and Push
  Branches Together, then submenus by task: Commit, Branch, Git Flow, Stash, Patches,
  History & Recovery, Custom Commands, Remotes & Hosting and Settings. Items keep their
  shortcuts; a few have new names: Workflows & Cleanup, Ignored Files (.gitignore), File
  Backups & Restore, Signing & Verification, GitHub Repositories & Notifications.
- Tags, Worktrees, Submodules, Git LFS, Statistics, Integrations and AI review opened the
  same pages as the sidebar; they move to a new View → Go to, which lists every sidebar page
  in the sidebar's order and ticks the current one. View's Compare commits and branches is
  now Go to → Compare.
- Edit's Undo, Redo, Cut, Copy, Paste and Select All work: they act on the text field you
  were typing in, with its selection, and are grey when no field has focus (Cut and Copy
  also need selected text). Undo Last Commit stays a separate item. Outside a text field,
  Ctrl+V still applies a patch from the clipboard.
- The command palette, hints and the Guide use the new names and menu paths; the Guide
  has a short Menus section under Getting started.
- Automation: item ids are unchanged. Submenu rows are repository-commit-menu,
  -branch-menu, -stash-menu, -patches-menu, -history-menu, -remotes-menu and
  -settings-menu (Git Flow and Custom Commands keep theirs); Go to is view-goto with goto-*
  rows; the text commands are edit-undo, edit-redo, edit-cut, edit-copy, edit-paste and
  edit-select-all. Runners open a moved item's submenu first (tools/tests/menu-paths.cjs);
  test-mcp-menus.cjs covers the menus.

## Unreleased — Shortcuts page name

- Preferences → Keyboard & mouse is now called Shortcuts, and so is its command palette
  entry (formerly Keyboard shortcuts). The page still sets keyboard and mouse bindings, and
  Preferences search still finds it by keyboard, mouse or hotkeys.

## Unreleased — SSH keys, keys first

- Preferences → SSH keys lists every key once, beside the chosen key's location,
  fingerprint and public key with Copy public key: the key pairs found in the SSH folder
  and the keys Rocket made or set up. A key found in the folder no longer needs a profile
  to be copied, unlocked or removed from ssh-agent.
- A tick before each key shows whether it is unlocked in ssh-agent. Tick to unlock it
  (ssh-add asks for the passphrase); untick to remove only that key. A found key is used
  only while its files are the ones the scan read.
- New key and Add key file sit under the list, and an empty page offers both. Use only
  this key reviews the open repository's SSH setting; for a found key it first asks for
  the host, user and port, taken from the repository's SSH remote when it has one.
  Restore previous setting stays available, also when there are no keys.
- Rescan also reads ssh-agent again. Check profile is now Check settings: it reads the
  key's SSH config with `ssh -G` and does not connect. The Discovered keys card and its
  Set up profile buttons are gone.

## Unreleased — SSH key discovery

- Preferences → SSH keys automatically lists public/private filename pairs in the
  user's SSH folder, with Rescan, Unicode filenames, fingerprints and a reviewed
  profile form. Discovery is bounded, cancellable and does not activate keys or
  modify SSH config. Import rechecks the selected files and verifies the key pair.
- Cancellation preserves the previous list; diagnostics report scan phases,
  counts, skipped entries and typed failures without key contents or filenames.

## Unreleased — Themes

- Preferences → Appearance, View → Appearance and the command palette choose a theme:
  Default (Rocket's navy, unchanged), Dark (neutral graphite), Light, System (Light or
  Dark, following Windows as it changes) and Custom. A theme applies at once; the window
  title bar keeps following Windows.
- Custom starts from Default, Dark or Light and takes an accent: a swatch, a colour code or
  From picture (a built-in picture's own colour, or your picture's most common vivid
  colour). It can show a picture behind the main window's panes, which turn see-through by
  the Visibility amount (10–60%): Astronaut, Space cat, Planet, Stars, or your own PNG, JPG
  or WebP (up to 32 MB). Fill covers the
  window; Fit shows the whole picture, its inner edge fading into the theme. Rocket dims (on
  Light, lightens) parts of the picture that would make text hard to read. Dialogs, menus
  and cards stay solid. A chosen picture is copied into Rocket's storage, at most 2,560
  pixels on its long side, and replaces the one chosen before.
- Every colour now comes from the active theme's palette (about 90 roles). Diff rows,
  syntax colours, terminal colours and graph lanes follow the theme; the diff row cache
  rebuilds when the palette changes. The Guide has a Themes topic, which "?" on the
  Appearance page opens.
- Local settings with a non-default theme use inner format v20; older binaries refuse it.
  The Default theme keeps writing the earlier format. No dependencies were added; the four
  built-in pictures ship as JPEGs with their SVG sources in `crates/rocket-ui/assets/themes`.
- Automation: new ids theme-default/-dark/-light/-system/-custom, theme-base-*,
  theme-accent-picture/-<swatch>/-code, theme-picture-none/-astronaut/-space-cat/-planet/
  -stars/-choose/-own, theme-size-*, theme-position-*, theme-visibility and View →
  Appearance's view-theme-*. rows-* are kept.

## Unreleased — Keyboard and mouse shortcuts

- Preferences → Keyboard & mouse now edits 21 common commands. Record up to two
  keyboard/mouse bindings per command, review duplicates/reserved inputs, remove
  a binding, or restore defaults. Back/forward buttons work alone; other clicks
  and wheel directions require Ctrl, Alt or Shift.
- Bound gestures are consumed before widget input, preventing a second click,
  scroll or zoom. Terminal focus, modal reviews, ordinary editing and existing
  command guards remain respected. Menu/palette hints follow saved bindings.
- Local settings with shortcut overrides use inner format v19; older binaries
  refuse it instead of silently dropping bindings. Streamed draft storage is
  retained. No dependencies were added.

## Unreleased — Shortcut table, conflict editor, review windows, Hosting lists and Git preferences

- Keyboard & mouse is a table of Command, Shortcut and Alternate. Click a cell and press the
  keys or a mouse gesture; it is saved at once. Esc cancels; Delete or Backspace clears the
  cell. A clash shows under its row with Use for <command> or Keep; an input that cannot be
  used shows why, with Record again and Cancel. Changed commands carry a dot and a reset
  button; mouse buttons and the wheel read as chips (Mouse forward, Ctrl + Wheel up).
- Conflict editor: the result has line numbers, tinted ours and theirs and a dimmed base and
  markers. Use ours, Use theirs and Use both sit on each conflict's first line instead of the
  toolbar (shorter words in narrow columns); an amber bar marks the selected conflict. Save
  and stage no longer ends in an ellipsis.
- Review windows open with a page header: the commit title, author, date, its ID (click to
  copy) and Pinned · read only. Commit reviews list changed files beside the diff (a menu
  below 760 px); file and blame reviews no longer repeat the history header. The window's
  own close button closes it, so the in-window Close is gone; Retry appears after a stopped
  or failed read.
- Hosting: New pull request and Publish repository move from the tabs to the header; the tabs
  are Pull requests, Issues and Saved drafts. GitHub search is one row: words, author:,
  assignee:, recent searches behind a clock button (Clear recent searches ends that list) and
  Search; Enter also searches. Each pull request row has Details beside Open in browser.
- Preferences → Git uses two cards: Git program (the Git in use, Find Git automatically,
  installed copies as choices, a path with Browse, Check and Use this Git) and Commit
  identity (Name, Email, Reload or Discard changes, Save identity). Its notes moved to the
  Guide.
- Preferences no longer shows the Rocket version under its page list; Help → About shows it.
- Automation: existing ids are kept (pref-shortcut-<command>-edit-N/-add/-reset,
  pref-shortcut-cancel/-again, git-*, review-*, hosting-tab-create/-publish, hosting-search-*;
  conflict-ours/-theirs/-both name the selected conflict's choices). New: pref-shortcut-move,
  review-copy-id and per-conflict conflict-ours-N/-theirs-N/-both-N; pref-shortcut-save is
  gone. MCP rocket_review_click on review-close closes the window as its close button does.

## Unreleased — Compare page

- One compare bar: Revisions, Staging area or Working tree as a segmented switch; Base and
  Target as single fields with the branch list inside; Swap as an icon between them; and
  Compare as the main button, which Stop replaces only while a read runs. In Staging area
  and Working tree, Target is fixed. The always-disabled Stop and the standing hints are gone.
- One result line: both commit IDs (click to copy), the files by kind (added, modified,
  deleted, renamed) and when it was compared. When the fields change it says Not compared yet.
- Files are rows like Changes: a coloured letter, the name, then its folder or where it came
  from. A file that was only renamed or copied shows a card saying where it moved, instead of
  Git's raw headers with escaped paths.
- Before comparing, while reading, no differences and errors show a centered message; Retry
  for a file's diff sits in the diff area. A narrow window wraps the bar, and Files and Diff
  take turns as two tabs. The diff view itself is unchanged.
- Automation: ids keep their meaning (compare-load and compare-cancel take turns; compare-swap
  and compare-target exist in Revisions; compare-toggle-files is on the tab that switches).
  New: compare-copy-base and compare-copy-target.

## Unreleased — Apply part of a stash, Undo last commit and loading states

- Apply part of a stash (Stashes → Apply selected) lists the stash's files with a checkbox
  each (all, some or none of it) and their added and removed lines. Beside the list, the
  picked file's changes are numbered cards with line numbers and a checkbox per changed line.
  Each choice is checked at once, so Review selection is gone: Apply selected changes applies
  a checked selection, and Check again appears only after a refused or stopped check. The
  dialog's rules moved to the Guide.
- Undo last commit shows the commit being undone and the commit the branch moves back to as
  two cards, then what was checked, Copy recovery name and one Undo commit button. An open
  review checks again by itself before Git's two-minute limit; Check again appears only
  after a refusal. The dialog's notes moved to the Guide.
- A pane, page or dialog with nothing to show yet centers its spinner and text (Loading diff
  keeps Stop under them) instead of starting in the top-left corner. Status lines under
  content stay in place.
- Automation: the stash dialog adds dialog-stash-show-N (shows a file's changes) and
  dialog-stash-whole; change and line checkboxes exist for the shown file.
  dialog-preview-refresh (stash) and dialog-undo-refresh appear only after a refused or
  stopped check, and Undo's dialog-submit only while there is a review to undo.

## Unreleased — Conflict editor: three clear panes

- Ours, Result and Theirs are matching boxes with line numbers, each under a header saying
  what it is: Ours (HEAD, your branch), Result (saved to the file) and Theirs (the incoming
  branch); rebase, cherry-pick and revert use their own words. After a choice, a file that fits
  no longer scrolls past its end, leaving the panes mostly empty.
- Each conflict has the same number in all three panes (amber while open, green once
  resolved), and a band joins its lines across the gaps. Use ours sits on the Ours lines and
  Use theirs on the Theirs lines; Result keeps Use both, and all three in a narrow window. A
  resolved conflict says Used or Not used on each side. Text in Ours and Theirs can be selected.
- Result's rail marks sit beside their lines on a file that fits, and the conflict counter
  numbers conflicts as the badges do.
- Automation: conflict-ours(-N) and conflict-theirs(-N) name the side panes' choices in the
  three-column view; no ids were removed.

## Unreleased — Find a shortcut by its keys; undo a resolved conflict

- Keyboard & mouse: a keyboard button at the end of the search box waits for one chord or mouse
  gesture and lists only the command that uses it; the box shows the input, such as Ctrl+K. It
  never changes a binding. Esc or a plain click cancels; plain wheel scrolling still scrolls.
- Conflict editor: a conflict resolved with Use ours, Use theirs or Use both stays tinted green,
  with "Conflict N · resolved · used both" (shorter on a long line) and Undo on its first line
  and a green mark on the scroll rail. Undo puts the original block back and selects it. Typing
  elsewhere keeps Undo; typing on the resolved lines, Reload, a merge-tool or AI result, or a
  failed save ends it. The header reads "N of M left" while resolved conflicts can be undone.
- Automation: new ids pref-shortcuts-find and conflict-undo-N. state.shortcuts adds finding (pass
  it as the rocket_shortcut recording_id) and found.

## Unreleased — Git, Custom commands and MCP server in Preferences

- Preferences → Git lists the ways to run Git in one card: Automatic (the first git.exe on
  PATH), each installed copy with its version, and Custom path with Browse. Choosing one
  runs git --version and saves it when it passes, so Check and Use this Git are gone; a copy
  that is too old or does not run cannot be chosen, and a failed Custom path keeps the saved
  choice. The card says when a restart is needed. Commit identity has full-height Name and
  Email fields; Save and Discard appear only after an edit, and the config file is named on
  the info icon. The page footer is gone.
- Custom commands are edited in Preferences → Custom commands instead of a separate window.
  Commands are grouped by the menu they show in and edited under their row; Find, Import,
  Export and New command sit above the list, Save commands and Discard in the footer, and
  Preferences stays open while changes are unsaved. New: Duplicate. Repository → Custom
  Commands → Edit Custom Commands and the palette open the page.
- Preferences → MCP server uses the same cards. While Rocket checks the server, the card
  says Checking and the rows the answer fills hold placeholders, so nothing jumps when it
  arrives. Start server or Stop server sits at the server card's foot. Port and folder are
  full-height fields; Save and Discard appear only after an edit, and Start server waits
  for them. Errors show on the card they concern; the protocol line moved to the guide.
- Automation: the custom-commands dialog kind is gone; state.preferences.page is commands. New ids
  dialog-commands-save, -discard, -duplicate and -filter, git-custom; git-reload is Discard
  or Try again; git-check and pref-manage-commands are gone. Under MCP,
  dialog-commands-save keeps the fixed-test-command rule, and a Git row may be chosen only
  for the Git the window runs. MCP server: mcp-save shows only after an edit; new
  mcp-discard; mcp-guide is gone.

## Unreleased — Setup Guide and the Help menu

- Guided setup is now called Setup Guide: in the Help menu, on the welcome screen and the
  Summary page, in the command palette and in the guide.
- Help no longer lists Keyboard Shortcuts. They live in Preferences → Keyboard & mouse,
  which the command palette's Keyboard shortcuts entry also opens.
- Automation: help-shortcuts is gone; help-setup, summary-setup and welcome-setup keep
  their ids.

## Unreleased — Dialog footers

- Dialogs no longer repeat their header × with a footer Close or Cancel: the ×, or
  Escape, closes them as that button did, with the same checks while work runs.
  Among them: Push, Pull, Merge, Rebase, Git Flow, tag, worktree, submodule and Git
  LFS reviews, Commit link, Undo, Bisect, recovery, Open and Clone.
- While work runs, the button that ends it reads Stop. File restore and patch
  reviews show it only then; Preferences (Git, SSH keys, Accounts, Dependencies), the
  commit link remote read and custom command transfers say Stop instead of Cancel.
- Pages say Stop for running work too: Hosting, GitHub, Playground, Integrations,
  AI review, Compare, Path history and Files. Review windows say Stop reading, the
  repository manager Stop opening, the commit message box Stop drafting and the
  conflict editor Stop saving. Git LFS and Submodules keep Cancel read: that button
  also closes the page. Control names are unchanged.
- The remote tag review's × now returns to the remote tab; it did nothing before.
- Automation: the × also answers to `dialog-cancel` and to a removed button's own
  name (`dialog-tag-back`, `manager-cancel`, `picker-cancel`, `commit-link-close` and
  others); a footer Stop keeps `dialog-cancel` while work runs.

## Unreleased — AI conflict proposal page

- AI conflict proposal has one layout from start to finish: the code on the left, a column on
  the right, and the main button in one place at the top right: Send, then Stop, then Apply.
  The page shows an AI mark, and so does AI resolve, which also sits beside Open conflict editor
  in Changes (`conflict-open-ai`).
- Who answers is Claude Code, Codex or Connected agent, starting on the first program found.
  Codex is new here (`conflict-ai-source-codex`, `conflict-ai-codex`, diagnostics
  `ai-conflict-codex`). Goes to says who gets the code, with Found or Not found checked for real:
  the page used to show claude.exe even with nothing installed, and fail after Send. Not found
  offers Browse on the page (`conflict-ai-browse`), and a stopped MCP server offers its
  Preferences page (`conflict-ai-server`).
- Prepare snapshot is gone (`conflict-ai-prepare`): the page prepares by itself, and each send
  checks the file again. Show what will be sent became Show exact data, in the code pane
  (`conflict-ai-exact`). Sends now says the whole file goes, which is what always happened.
- Review: Conflict and Proposed result use the editor's colours and numbers, changed lines are
  green, and each conflict names its lines. Clicking one moves both panes to it
  (`conflict-ai-section-N`). Discard reads Ask again; its id stays. Errors say what happened and
  what to do; the code and the program's own message move under Details.
- Apply lands each conflict in the editor as resolved, marked AI, with its own Undo. An AI result
  used to end Undo. A narrow window puts the card first and shows the conflicts as chips above
  the code.
- MCP state: `conflict_editor.ai.source` can be `codex`, `error` is now always the code alone, and
  `refused` and `selected` are new.

## Unreleased — Claude Code conflict proposals

- AI resolve can ask Claude Code on this computer as well as a connected MCP agent:
  choose Claude Code in Ask for a proposal, prepare the snapshot, check what will be
  sent, then Send to Claude Code. Rocket starts the Claude program from Preferences →
  Commit messages once, from an empty folder, with no tools or project settings, and
  sends only the prepared snapshot with fixed instructions. The reply goes through the
  same review as an agent's proposal; nothing changes until Apply. Your Claude plan or
  API usage applies; no API key is stored.
- While Claude Code works, a status card shows the elapsed time and Stop, which ends
  its process tree; a request stops after 5 minutes. Other running AI work on the page
  shows Stop instead of Cancel. The page subtitle names the conflicted file.
- Automation: `conflict-ai-source-agent`, `conflict-ai-source-claude` and
  `conflict-ai-claude`; MCP state reports `conflict_editor.ai.source`; under MCP only
  the fixture's stub program may run, as for commit drafts. Diagnostics name the
  request `ai-conflict-claude`.

## Unreleased — UI revamp: Preferences connections and newer screens

- Edit → Preferences gains Accounts and SSH keys under a new Connections group;
  its sidebar is grouped (Workspace, Git, Connections, Security & data) and scrolls
  in short windows. File → Configure SSH Keys, File → Accounts and Repository →
  SSH Keys and Accounts are removed; Repository → Hosting replaces Hosting and
  Accounts. The command palette lists Accounts and SSH keys.
- Hosting has no Accounts tab. Accounts beside the sign-in status, and Open
  Accounts when signed out, open Preferences; closing it, or connecting an account,
  returns to Hosting with any error message kept.
- The account form names the account it edits, groups providers as Cloud and
  Self-hosted, and gives each provider one line about its token; details moved to
  the Hosting guide. SSH keys shows profiles as rows with a details card, a
  passphrase card for OpenSSH prompts and a review card for Use for <repository>,
  which needs an open repository. Closing Preferences waits while OpenSSH runs.
- Automation: control names of both pages are kept (`hosting-tab-accounts` reaches
  the Hosting link); MCP state reports `preferences.page`. No new dependencies,
  credential or SSH configuration changes.
- Integrations: Preferences lists packages as rows and edits a connection in Service,
  Package settings and Permissions cards (permissions are switches). The page has a
  header and toolbar; the request review shows the method, address and numbered
  body. Lua panels get Rocket's spacing, padded cards, accent primary buttons and
  pills without package changes.
- Hosting pull request details: breadcrumb, a header card with state, branches, head
  and base, Write a review as the main action, and tabs with counts. Commit link is a
  review-style dialog; Open in browser appears once a remote is chosen.
- Bisect keeps its dialog with short IDs, a First bad commit card, decisions as
  pills and Restore original as the main action once the search ends. Summary shows
  count tiles, a Next step card and the branch's upstream state; Guided setup is a
  stepper with progress. The playground has Run in its header, numbered lines and a
  framed preview; AI conflict proposals show numbered, tinted conflict and result
  panes beside the sections and their confidence.

## Unreleased — Lua write recovery security fixes

- Keep redirected POST/PUT/PATCH/DELETE results uncertain: the service may have
  applied the write before returning a redirect. Restart retains the recovery
  guard until the user checks the remote result and acknowledges it.
- Serialize protected integration settings saves across Rocket processes with a
  persistent OS file lock. A competing save preserves the existing file and
  draft, reports a retryable busy error, and still rejects stale snapshots.
  Close older Rocket processes before using the updated build.

## Unreleased — Lua coding playground and MCP conflict proposals

- Replace the component-demo install entry with a Lua editor, Run/Cancel and an
  interactive native preview, including compact code/preview tabs. Preserve the
  previous successful preview and callback state after failures/cancellation.
- Save scratch source in separate protected local storage with external-change
  checks and a normal-exit save guard. Restart never runs scratch code. Existing
  installed component demos remain intact.
- Add user-approved conflict snapshots for connected MCP agents, with validated
  per-section proposals, explanations and agent confidence. Apply updates only
  the editor draft; file saves retain existing backups and external-edit checks.
- Add authenticated localhost proposal tools that work without a development
  window. Cancellation/expiry revoke shared context; general MCP state and logs
  omit source and proposals. No AI API key, model invocation or automatic merge
  commit is introduced.

## Unreleased — Lua developer tools and Jira workflows

- Add native Lua syntax validation and bounded local action diagnostics with source
  locations. Validation never executes module initialization or the callback.
- Extend the Jira Cloud example with JQL pagination, issue details/edit metadata,
  reviewed edits/transitions and create-field inspection. Preserve rich descriptions
  unless replacement is selected; reject checked writes when the update marker
  changes. This preflight is not an atomic service-side lock.
- Add explicit per-connection permission for read-only branch, commit IDs and
  change counts, plus an offline example. Paths, remotes, identities and file
  contents remain outside the context. Existing connections default to no access.
- Drop obsolete field values when a callback changes a control's type or choices.
  Existing example folders are never replaced. Live Jira remains unverified.

## Unreleased — Integration connections and saved drafts

- Report invalid packages independently so healthy integrations remain usable;
  refuse all copies of a duplicate ID and keep their files unchanged.
- Add up to 16 named connections per integration, with separate credentials,
  permissions, settings and uncertain-write recovery. Existing connections migrate
  to Default without changing the old protected file until a successful save.
- Save bounded forms, script state and their native panel locally in protected
  storage. Switching/restarting restores the same connection's draft without
  running Lua or sending requests. Writes and previews never replay.
- Bind saved drafts to exact source and connection identity. Preserve incompatible
  drafts for repair or explicit clearing; refuse external-file overwrites. Drain
  pending draft saves before exit and keep the window open on save failure.

## Unreleased — Lua integrations and native panels

- Add an Integrations page and Preferences section for explicitly enabled local
  Lua packages. Expose versioned native layouts, forms, tables and action buttons.
- Bundle a bounded Lua 5.5.1 worker; keep HTTP, credentials, request review and
  cancellation in Rocket. No system Lua or Node runtime is needed.
- Add an offline UI playground and Jira Cloud example for reading issues and
  reviewing issue/comment creation. Existing folders are preserved.
- Bind approval to exact source/manifest contents. Protect optional saved tokens
  separately and persist uncertain write outcomes before sending; restart never
  retries a write. Real Jira acceptance remains unverified.

## Unreleased — PR collaboration and saved drafts

- Protect PR review/line/reply drafts locally and persist uncertain review,
  discussion and merge outcomes before sending. Restore requires the same account
  and fresh reads; previous previews and writes never replay automatically.
- Add GitHub/GHES thread replies, resolve/reopen previews and multiline comments
  within a complete diff hunk. Preserve stale text and honor thread permissions.
- Add PR details, provider-supported comments/approvals/change votes and reviewed
  merges to GitLab, Bitbucket Cloud/Server, Azure Services/Server and Gitea.
  Show bounded/unsupported data and provider-specific concurrency limitations.
- Keep local files, existing credentials and provider policy protections intact;
  no new dependencies or embedded scripting runtime are included.

## Unreleased — Parallel-load reliability

- Speed up secret scanning of large files with bounded candidate searches while
  retaining cancellation, token detection and existing scan limits.
- Check an object batch's complete size metadata before reading its payloads,
  so a known over-budget scan reports the size limit promptly.
- Avoid repeated waits once a diagnostic log lock is known to be busy. Batch
  recovery flushes across retained files and keep all overflow copies until
  every destination flush succeeds, including after rotation or a failed flush.

## Unreleased — Azure DevOps Server

- Add a separate on-premises hosting provider with HTTPS collection URLs,
  server-issued PATs and API 6.0/7.0/7.1 choices, persisted in protected profiles.
- Verify an active authenticated account; browse paged PRs and project work items,
  and review both branch tips before creating a same-repository PR. Scope pages
  and previews to the account, collection, repository, filters and API version.
- Preserve local files and existing credentials, TLS checks, immediate stopping
  feedback and uncertain-create recovery. Show fixed authentication, API, redirect,
  certificate and service errors without logging tokens or response payloads.
- Include a mockup and focused temporary-repository/mock-API coverage. Windows
  authentication/SSO and live Azure DevOps Server compatibility remain unverified.
- Clear the previous successful PR link when reviewing another creation, so an
  interrupted attempt cannot appear to refer to the earlier pull request.

## Unreleased — Re-audit fixes

- Warn before closing with unsent pull request review text or inline comments.
  Cancel keeps the review open. The newer PR collaboration storage preserves
  these drafts on confirmed exit instead of discarding them.
- Finish image loading in separate commit review windows and
  release their preview state on close.
- Share unchanged retained draft text between autosave snapshots, including tab
  close and reverted edits, to avoid scanning/copying all retained text each poll.
  The state v18 format, complete draft content and save retry behavior stay intact.
- Report a changed interactive-rebase journal as a plan mismatch before decoding
  it; keep refusing Abort, Continue and Skip until the reviewed plan is restored.

## Unreleased — GitHub Enterprise Server

- Add a separate Enterprise Server provider with configurable HTTPS origin,
  server-issued tokens and optional protected account persistence.
- Reuse GitHub PR/issue search, pagination, reviewed creation, details, inline
  reviews and merging against the enterprise `/api/v3` endpoint. Match remotes
  only to the explicitly selected server; preserve public-only OAuth/publishing.
- Show reported server version and distinct certificate, redirect, unavailable
  service and retired-API errors. Keep credentials/private response data out of
  diagnostics, certificate checks enabled and cancellation/uncertain-write guards.
- Add an interactive mockup, focused backend and native localhost MCP coverage
  with disposable repos/accounts. Real GHES deployments remain unverified.

## Unreleased — GitHub PR workflows

- Queue, edit and remove inline comments from validated old/new diff lines;
  submit them together in a review tied to the original PR revision. Preserve
  Unicode rename paths and refuse missing, unsafe or incomplete locations.
- Review merge, squash or rebase on GitHub with current permissions/method checks,
  a head-SHA condition and explicit uncertain-outcome recovery. No local checkout
  writes, branch deletion, automatic retry or merge-queue operation.
- Create a PR from an existing fork after reviewing both repositories and branch
  IDs, rechecking their fork network and tips. No automatic fork or branch push.
- Extend the isolated localhost MCP acceptance runner and user documentation.
  GitHub live policy/scope acceptance remains separate from synthetic API tests.

## Unreleased — Guided setup, repository summary and Safe Undo

- Add Help → Guided setup with existing Git preferences, Open/New/Clone,
  repository identity and Changes entry points. No automatic writes or sign-in.
- Add Summary in the sidebar and command palette: staged/unstaged/untracked/
  conflict counts, branch state, local upstream comparison and a next-step hint.
  It reuses existing refresh data and marks missing or stale information.
- Add reviewed Edit → Undo last commit for the latest ordinary commit created
  in this UI session, including a first commit. Retain a recovery ref before
  moving the branch; preserve index and working files. Refuse later staging,
  moved tips, active operations, known publication and stale external edits.
- Reconcile cancellation against the actual branch/index/recovery result, with
  fixed diagnostic failure codes. No general Undo/Redo or CLI command is added.
  Existing release bundles are unchanged; this is a local development build.

## Unreleased — Draft storage and responsiveness

- Save more than 64 retained drafts and messages above 128 KiB in a streamed
  archive. Atomic state v18 migration preserves prior saves on failure. Close
  Rocket before backing up the whole state folder; older builds cannot read
  state v18. The composer now
  explicitly rejects edits above the existing 1 MiB commit-message limit.
- Share unchanged terminal rows between snapshots and skip output snapshots for
  hidden terminals while continuing to observe process shutdown.
- Generate image difference textures on demand and cap retained image textures
  across tabs and review windows at 192 MiB. Least recently viewed previews can
  be released and offer Reload. Decoding buffers and driver overhead are separate.
- Correct the mail-import test's startup preference setup and add phase-specific
  timeout diagnostics without increasing its deadline. The earlier intermittent
  timeout's precise cause is not established by this fixture correction.

## Unreleased — Audit corrections

- Bound retained worktree presentation settings so opening more than 256
  repositories over time does not stop draft/settings saves. Add Retry save for
  failed writes while preserving unread or unowned state files.
- Cancel obsolete queued spelling checks, bound pending text, and keep disabling
  or closing responsive. Accepted dictionary changes finish asynchronously.
- Repair the reflog, privacy and contextual-workflow test expectations, plus
  hook-timeout and asynchronous terminal test fixtures. Clean up workspace
  Clippy warnings. The release version and packaged binaries are unchanged.

## 0.87.1 — Large-file secret protection

- Fix an 8 MiB batching threshold being treated as a per-object safety-scan
  limit. Larger files, including unchanged files already in the index, can now
  pass the same default-on checks before commit or push.
- Bound object reads by their declared size and lower the cumulative scan budget
  from 256 MiB to 128 MiB. Oversized, missing or incomplete objects still block
  the operation, with a specific explanation for the total byte limit.
- Check cancellation and the shared deadline during content inspection, including
  binary data and UTF-16 normalization. Keep raw contents out of diagnostic logs.
  No dependencies, credential settings or repository policies changed.

## 0.87.0 — GitHub PR details and reviews

- Open PR descriptions, commits, file patches/renames, conversation, inline
  comments, review history, check runs and commit statuses inside Hosting.
  Missing, truncated, partial and permission-limited results are explicit.
- Preview Comment, Approve and Request changes with the connected account,
  repository and exact commit. Recheck head/base before a single-use submission;
  keep the draft on cancellation, rejection or changed remote commits.
- Distinguish an uncertain submission from a rejected review. Require refreshed
  review history and explicit acknowledgment before another attempt; never retry
  automatically. Preserve confirmed success even after late cancellation.
- Add fixture-scoped localhost MCP controls and focused synthetic acceptance.
  No new dependencies or credential-store changes. Review drafts are in memory;
  inline comment publishing, merging, cross-fork creation and CLI review commands
  remain outside this release. Real GitHub review writes remain unverified.

## 0.86.0 — Automated bisect

- Review a test executable, literal arguments and timeout to find the first bad
  commit automatically. Manual Good/Bad/Skip and recovery remain available.
- Show per-revision outcomes, exit codes, durations and bounded local output.
  Persist metadata for reopening without saving command arguments or raw output.
- Stop without classifying on command errors, timeout, cancellation or tracked
  edits. Contain and settle test descendants before another checkout. Keep the
  original recovery ref, generated files and non-forced restoration safeguards.
- Add localhost MCP command/review controls for isolated native validation.
  No dependency changes; automatic bisect is not yet a CLI command.

## 0.85.0 — Finer stash selection, partial staging and syntax

- Apply individual changed lines from a saved text stash. Whole-file and hunk
  controls remain available; changing a selection invalidates its preview.
  Existing backups, HEAD/file checks, cancellation and partial-save reporting
  remain in use. The stash and index are retained.
- Partial stage/unstage supports regular text additions and deletions. New-file
  reviews do not modify the index; filters/encoding conversions are refused.
  Partial unstaging of a rename's text preserves the staged rename. Working files
  remain unchanged. Unsupported data and executable-mode changes use whole-file
  actions; partial discard/stash-save still require ordinary modifications.
- Added Dart, Elixir, Erlang, MATLAB and free/fixed-form Fortran highlighting,
  C++ raw delimiters, nested Dart comments and multiline/sigil handling. Highlighting
  remains bounded lexical analysis, with independent diff-side state.
- Backend `StashApplySelection` adds `lines`; callers use either hunks or raw
  changed-line IDs. Both empty still means whole file. MCP adds bounded line-ID
  metadata and visible line controls. No new dependency or settings migration.

## 0.84.0 — Reviewed CLI pushes, tags and worktrees

- CLI force-with-lease pins both the reviewed local and remote object IDs. Atomic
  push reviews up to 32 branches and publishes all or none; unsupported servers
  are refused without falling back to separate pushes. JSON reports remote
  observations independently of interruption or command success.
- CLI local tags: bounded list, exact inspection, lightweight/annotated creation
  and expected-object deletion with recovery references. Linked worktrees: list,
  preview/create and expected-HEAD removal, preserving dirty/locked worktrees,
  existing destinations and recovery information.
- Native close regression runner targets only actual Rocket/review windows.
  The previous stall report was caused by closing Winit's invisible event target;
  app shutdown behavior is unchanged. Strict checks reject second-close retries.
- JSON schema 1 gains additive command/results; no settings migrations or new
  dependencies. The Tokio 1.53.3 security refresh remains in the lockfile.

## 0.83.0 — Independent reviews, commit links and history ordering

- Open commit details, file history and blame in separate native windows. Each
  stays pinned to its repository/revision with independent selection, cancellation
  and retry; closing a review leaves the main window and working files alone.
- Commit link previews choose a remote, copy its full commit URL or open it in
  the browser. GitHub, GitLab, Bitbucket Cloud/Server, Azure DevOps and Gitea are
  supported; custom servers require an explicit provider/address. No login or
  remote-presence check occurs when generating a link.
- History offers topological, commit-date and author-date order, all preserving
  child-before-parent traversal. Pagination/search retain the selected order and
  pinned starting commits; the order is saved per checkout. Older settings default
  to topological order. Settings containing date ordering use state format 17.
- CLI: `history --order`, `commit-url`, `review commit` and `review file --blame`.
  Native review launch needs `rocket.exe` beside the CLI; other CLI commands keep
  working independently. JSON schema 1 gains additive order/link/launch fields.
- Localhost MCP exposes review-window state, native clicks, resize and captures.
- No new dependencies. The separately approved Tokio 1.53.3 security refresh is
  retained in the shared lockfile.

## 0.82.0 — Branch actions and visible history clicks

- The Branch toolbar button now opens New Branch, Sync Branch and Finish Branch
  in full and compact windows. Sync/Finish open their existing reviews, including
  save/restore and fast-forward choices; detached or paused checkouts disable them.
- MCP commit rows and merge-expansion buttons publish only their visible areas.
  Hidden rows are refused with guidance, and clicks use the current visible target
  at delivery. `history-scroll` exposes the commit list for mouse-wheel navigation.
- MCP can submit New Branch inside the fixture's marked branch namespace;
  cancellation and normal desktop branch creation keep their existing behavior.
- No new dependencies or repository data migrations.

## 0.81.1 — Safe imported command review

- Imported and saved commands are never probed automatically. Check explicitly
  inspects the selected local executable; changing its path clears the result.
- UNC/WebDAV paths, device namespaces, alternate streams and hidden formatting
  characters are refused before executable access. Import errors explain why.
- Reviews and menus escape hidden characters and backslashes while preserving
  the original execution values. Expanded defaults obey the existing argument limit.

## 0.81.0 — Saved branch Sync and complete command contexts

- Sync optionally saves tracked edits, rebases and restores staging. Recovery
  stashes, original branch tips and phase journals survive success, conflicts
  and cancellation; untracked/ignored files are not stashed or discarded.
- Finish offers merge-commit or allow-fast-forward behavior with an explicit
  preview; the default remains merge commit and both branches remain.
- File custom commands capture the exact revision/path in commit details,
  rename-aware file history and repository trees. `{file}` remains the current
  checkout path; the review explains historical selections. `{path}` is relative.
- Slash-separated command groups become nested menus. Dedicated HTTPS URL actions
  encode selected values, keep a literal host and require full-address review.
- URL definitions use JSON v3 and saved state v16. Program-only exports keep the
  v1/v2 formats. Older builds do not understand `{path}` or URL actions and must
  not edit state containing URL actions.
- Localhost MCP exposes the new choices and historical menus. URL tests capture
  only a fixed synthetic destination without opening a browser or making requests.
  No new dependencies or CLI commands.

## 0.80.0 — Git Preferences and contextual custom commands

- Preferences now selects Automatic/PATH, detected installed Git, or a custom
  executable checked for Git 2.46+. Selection is saved for the next startup;
  invalid selections report an error and can be repaired through Preferences.
- Explicit global name/email saves show the target and effective global sources,
  preserve included/unrelated config and reject stale edits under Git's config lock.
- Custom commands have Repository, Commit, File, Branch and Submodule types.
  Matching context menus capture the clicked item for the run preview. Added
  literal `{submodule}` expansion for initialized submodule directories.
- Existing command definitions import as Repository commands. Scoped definitions
  use JSON v2 and state v15; older Rocket builds must not edit those new files.
- Localhost MCP can right-click fixture rows; it still executes only fixed test
  commands with the installed Git and writes identity only inside the fixture root.

## 0.79.0 — Bitbucket browser sessions and private servers

- Bitbucket Cloud browser sign-in through installed Git Credential Manager, with
  credential storage disabled. Temporary sessions use Bearer authentication;
  expiry requires another sign-in. Remember remains available for API tokens.
- Separate Bitbucket Server/Data Center adapter: HTTPS base/context URLs,
  authenticated identity verification, project/personal repository paths,
  paged PR lists, and reviewed same-repository PR creation.
- Returned pagination offsets, exact branch matching, stale-review rejection,
  read cancellation and uncertain submitted writes preserve existing safeguards.
  Missing/anonymous identity, permissions, redirects and invalid pages have clear
  errors. No personal credentials, Git/SSH config or dependencies are changed.
- Account provider choices wrap without expanding narrow forms beyond the window.
- Verified against local contracts and localhost MCP fixtures. Real Bitbucket
  OAuth grants, expiry/permissions and installed server compatibility remain
  unverified; no claim of Fork's own test coverage or live provider acceptance.

## 0.78.1 — File menu account shortcuts

- Add File > Configure SSH Keys and File > Accounts with no trailing ellipses.
  Accounts opens the account settings directly, including when another hosting
  section was last viewed. Both managers are available without an open repository.
- Wrap Accounts form actions at narrow widths so Forget and Disconnect do not
  overlap the sign-in controls.
- Reuse existing hosting and Windows OpenSSH managers and preserve repository
  ownership checks. No account, credential or key migration and no new dependency.

## 0.78.0 — Drag files to stage and unstage

- Drag whole files between Changes groups, with Ctrl/Shift multi-selection,
  visible drop targets, edge scrolling and Escape cancellation. Staging uses
  current working bytes and includes both paths of a rename; working files and
  unrelated index entries are preserved by the existing guarded write backend.
- Reject foreign-tab, stale/refreshed, busy and conflict-row drags. Same-group
  and outside drops do not write. Keep drop targets clipped above other panels.
- Extend fixture-scoped MCP pointer drags and modifier clicks to changed files,
  with typed cancellation input and selection counts for verification.
- Fix the 0.77 in-app guide section's missing topic-index entry and help routing.
  No new dependency, CLI command, network operation or persistent setting.

## 0.77.0 — Commit & Push and saved branch switching

- Review Commit & Push from Changes or repository workflows: commit the reviewed
  staging, then push one explicit branch normally. Preserve unstaged/untracked
  files, hooks, signing, secret protection and existing upstream settings.
- Report local commit and remote push separately. A failed/cancelled push retains
  the commit; only a confirmed submitted draft is cleared. Recheck staging,
  checkout, hook-modified commits and effective destinations before publishing.
- Add explicit Save, switch & restore for existing local branches. Preserve staged
  and unstaged tracked versions, leave untracked/ignored files protected in place,
  and always retain the recovery stash and a phase journal. Conflicts, external
  edits and cancellation stop for inspection without automatic rollback or drop.
- Add phase diagnostics, reviewed native controls and local-fixture MCP boundaries.
  These composed actions are desktop/backend workflows; CLI commands are unchanged.
  See VALIDATION.md for the completed acceptance evidence and limitations.
## 0.76.1 — Safer mail and hosting review text

- Bound and cache commit-mail review text, make invisible characters visible, and
  require explicit acknowledgement before importing content omitted by a preview.
  Preserve original commit data and refuse source changes after approval.
- Bound and escape provider-controlled hosting titles, authors, states and branches
  across all supported providers, including Azure work items.

## 0.76.0 — Hosting search and reviewed recovery workflows

- Add GitHub PR/issue text, author and assignee search across server pages, with
  explicit partial/capped results and twelve bounded, clearable recent searches.
  Migrate saved settings without losing repository groups, drafts or preferences.
- Import numbered text patch series as authored commits. Review source and local
  state, preserve metadata and original tips, expose Continue/Skip/Abort, and
  retain resolution backups before destructive recovery. Refuse stale mail or
  Git recovery metadata. Cover letters and binary mail are currently unsupported.
- Add merge-preserving regular rebase with a pinned data-only topology plan and
  guarded Continue/Skip/Abort. Recreated merges can conflict; interactive editing,
  prediction and dependent-branch updates remain separate linear workflows.
- Enable reviewed tag and linked-worktree management in bare repositories while
  retaining working-tree requirements for checkout/file operations. Extend MCP's
  explicit fixture ownership checks to bare roots and their linked worktrees.
- Record import phases, fixed error codes, pause state and backup/record outcomes
  without source patches, messages, paths or credentials in diagnostic events.
  No new dependency, installation or personal credential/config migration.

## 0.75.1 — Scoped live GitHub publishing acceptance

- Add a separate local-operator MCP marker for one randomly named private GitHub
  repository and a pinned temporary SSH branch. Bound the hosting API session and
  Git transport independently; keep saved credentials, public creation and other
  repository actions unavailable. Existing fixture/account-only modes stay narrow.
- Extend the explicit Windows SSH agent profile to the selected disposable
  publishing destination, retaining its pinned public key and GitHub host key.
  Live acceptance results and cleanup status are recorded in VALIDATION.md.
- Expose only the scoped disposable publishing form/result to MCP, and permit
  its exact SSH remote for a reviewed recovery push. Verify real private creation,
  first push, errors, cancellation and recovery without changing the saved login.

## 0.75.0 — Publish local repositories to GitHub

- Add a reviewed publishing page under Hosting and Accounts. Create a personal
  or organization repository, default private, attach an unused remote and push
  one pinned branch over HTTPS or system SSH. Public publishing requires an
  explicit acknowledgement; staged, unstaged and untracked files remain local.
- Scan outgoing history before repository creation when secret protection is
  enabled. Refuse stale branch/config reviews, existing names, URL rewrites and
  config locks; preserve existing upstream settings unless none are configured.
- Show immediate stopping feedback and separate creation, local remote and push
  outcomes. Lost creation responses remain unknown; never automatically retry or
  delete a repository. A first push cannot replace a competing remote branch.
- Add payload-free publish diagnostics and confined localhost MCP fixture support.
  No new dependencies or login migration. Tests use synthetic GitHub HTTP APIs
  and local bare repositories; live GitHub repository creation is not established
  by those tests. This publishing workflow is desktop-only in this release.

## 0.74.0 — Submodule moves and selected stash application

- Add reviewed submodule Move / rename. Move a clean checkout and initialized
  descendants within the immediate parent repository, update Git links, and
  stage the gitlink path and .gitmodules. Keep module names, URLs, history,
  unrelated staging and local files. Retain metadata backups with move guidance.
- Refuse occupied destinations, dirty descendants, metadata drafts and stale
  previews. Cancellation stops preparation; after the folder moves, finish the
  bounded prepared filesystem updates. Report partial failures without rollback.
- Add Apply selected to Stashes: choose whole files or individual text hunks from
  All tracked, Saved index or Untracked. Preview against current files in a
  sandbox, keep the stash/index, and retain recovery copies for applied files.
- Reject overlapping edits and stale files/HEAD. Whole-file selection supports
  the existing bounded patch formats; executable-mode changes require the full
  stash workflow. New file and submodule paths require existing parent folders.
- Fix false submodule metadata refusals after identical contents are restored;
  validate actual diff content instead of only Git's read-only stat-cache result.
- Correct patch and submodule outcome logs to keep operation names separate from
  info/warning severity, including partial or unverified results.
- No new dependencies, settings migration or CLI commands. Local builds remain
  unsigned; existing security and operation-indicator work is preserved.

## 0.73.0 — Commit-series patch export

- Add Commit series to Export patch: choose an excluded base and included tip,
  review commits oldest first, inspect their author/date, paths and patch text,
  then save numbered Git mail patches in a new folder. Unicode paths, renames,
  binary changes and empty commits are included.
- Recheck range references and prepared bytes before publication. Never replace
  existing output. Retain preparation files after interruption or external edits,
  and report whether publication occurred. Working files and staging stay intact.
- Record explicit export steps and fixed refusal codes without logging private
  payloads. Support scrolling longer commit lists through localhost MCP.
- First scope: contiguous non-merge ranges with an existing base, at most 64
  commits, 1,000 paths per commit, 16 MiB per patch and 64 MiB total. No series
  importer, new CLI command, dependency or settings migration. Preserve 0.72.1
  secret protection; local Rocket executables remain unsigned.

## 0.72.1 — Optional default-on secret protection

- Refuse explicit commits and pushes that contain recognized GitHub token
  families, private-key material, likely credential literals or environment files.
  Check the staged index and raw outgoing history, including deleted secrets,
  annotated tags and force/atomic pushes. Pin pushed source objects to the scan.
- Show actionable, redacted refusal messages in the desktop, CLI and MCP activity.
  Agents are told to correct the content instead of bypassing protection.
- Add a saved General preference, on by default and user-disableable with an
  amber warning. MCP cannot change it. Existing state migrates with protection on;
  state version 12 preserves explicit choices. CLI runs keep their own default-on
  protection and do not read the desktop profile or offer a bypass flag.
- Fail closed on incomplete or oversized scans; document detection and client
  boundaries. No new dependencies. Local Rocket binaries remain unsigned.

## 0.72.0 — Ignore tracked files and stash snapshots

- Review exact files/folders, append root ignore rules and stop tracking while
  keeping local copies. Selected staged changes are refused; unrelated staging
  stays. Back up the index and ignore rules before publication. Never stage the
  root .gitignore automatically; report partial rule saves explicitly.
- Save a stash snapshot without cleaning working files or changing staging.
  Include untracked files explicitly. Publish its stash entry and recovery ref in
  one guarded ref transaction, and observe the result after cancellation.
- Add entry points in Changes, Manage .gitignore, Stashes and reviewed workflows.
  Keep previews and file checks on workers, with immediate stopping feedback.
- Preserve all 0.71.1 security changes. No new dependencies or setting migration.

## 0.71.1 — SSH acceptance, history and push reporting

- Add an explicit MCP Windows-agent option for the approved GitHub test repository,
  with fixed system OpenSSH, pinned host trust and immutable public identity/config.
  Repository command overrides and all existing scope restrictions remain blocked.
- Follow renamed files across merges in ancestry order, including paginated history.
- Report pushes refused before process start as not attempted. Preserve uncertain
  outcomes after possible remote updates; never infer rollback from an error message.
- Include the completed custom-command, Hosting and submodule UI work missing from
  the previous release snapshot. Refresh the source manifest and normal release.
- Keep local Rocket builds unsigned, as requested.

## 0.71.0 — Reviewed branch, submodule, patch and stash workflows

- Review and fast-forward selected local branches together to locally available
  upstream tips. Checked-out and diverged branches are blocked; recovery refs
  are created in the same ref transaction. Fetch remains a separate action.
- Review multiple submodule updates, stop remaining updates after failure or
  cancellation, and deinitialize a clean checkout into retained recovery storage
  without deleting the parent gitlink. A session option opens submodule review
  after branch switching; updates still require review and confirmation.
- Merge independent histories with an explicit opt-in, preserving existing
  conflict, Continue and Abort handling. Shallow, replaced and grafted histories
  cannot use this override.
- Import/export bounded literal binary and rename patches, executable-mode changes and a single
  format-patch email. Mail patches apply changes without making commits. Existing
  staged blobs are kept for mode changes; new executable files are staged, with
  an index backup and an explicit preview notice.
  Delta-encoded binary patches remain unsupported pending bounded size validation.
- Rename remotes with config/tracking backups; choose no tags, remote defaults
  or all tags when fetching. Existing tag replacement is refused.
- Reviewed stash pop keeps entries on conflicts or interruption. Clear retains
  recovery refs. Selected working-file hunks can be saved before cleanup while
  unrelated hunks and existing staging stay intact.
- No new dependencies or installations. All 0.70.1 scoped transport controls remain.

## 0.70.1 — Scoped hooks and HTTPS policy

- Scoped Git operations disable hooks per invocation. Local pushes refuse a
  receiver with active default or configured hooks, because Git drops the client's
  configuration overrides before starting a local receive-pack process.
- Scoped Git/LFS transports reject proxy, TLS trust, certificate-verification and
  routing overrides, including URL-specific configuration and inherited variables.
  They require direct HTTPS with certificate verification and no redirects.
- LFS and child Git receive the same invocation policy. Normal desktop/CLI hooks
  and authentication retain their existing behavior. Repository config and hooks
  are not rewritten. Local Rocket executables remain unsigned as requested.

## 0.70.0 — Submodule safety, worktree views and workflow tools

- Submodules can review a URL change, stage only `.gitmodules`, and synchronize
  the affected local configuration. Metadata backups and stale-review checks keep
  unrelated staging and child edits intact. All branch push modes check for
  unpublished submodule commits using Git's locally available remote information.
- Worktree views persist independently: branch/history filters, hidden sidebar
  tags/stashes, collapsed sections, diff layout and graph folding. New worktrees
  inherit the creating worktree's view; drafts stay independent. Existing settings
  migrate to version 11.
- Custom commands support groups and text, branch, file and checkbox inputs.
  Inputs expand into literal arguments after review. Versioned JSON import is
  reviewed before saving; export refuses existing filenames. Imports never run.
- Branch Sync rebases onto a reviewed local base. Finish switches to the reviewed
  base and merges while keeping the feature branch. Clean-checkout checks, pinned
  tips, recovery references and a phase journal support cancellation and conflicts.
  Neither helper fetches, pushes or deletes branches automatically.
- Long folder breadcrumbs stay within the file picker at small window sizes;
  full paths remain available through the address field and hover text.

## 0.69.2 — Scoped transport, browsing and release verification

- MCP transports reject configured or inherited SSH, askpass, proxy and credential
  helper commands before running Git or LFS. Ordinary desktop/CLI authentication
  keeps its existing behavior.
- Scoped folder browsing refuses junctions and outside Git metadata, rechecks on
  the reader worker and pins directories during reads. Safe New/Clone destinations
  remain supported.
- The canonical release verifier rejects unlisted or unexpected payloads and
  verifies the bundled Microsoft runtime signatures and pinned architecture pair.
  Local Rocket builds remain unsigned by the user's explicit choice.
- Security candidate built from the 0.69.1 checkpoint plus these fixes; concurrent
  unfinished 0.70 development is preserved separately in the working source.

## 0.69.1 — Hosting pagination and token compatibility

- Bitbucket Cloud follows the server's opaque continuation instead of guessing a
  page number. GitHub, GitLab and Gitea honor next-page Link headers even when a
  server returns fewer items than requested.
- Pagination links must stay on the configured API origin, base path and exact
  collection. Stale account/repository/filter cursors and invalid links produce
  `hosting_invalid_pagination`; existing results survive a failed or cancelled
  load. Continuation values are excluded from cursor debug output and diagnostics.
- Bitbucket API tokens can connect without an email using Bearer authentication.
  Existing profiles with an email retain their Basic authentication behavior.
- Rechecked hosting contracts against official provider docs and the earlier
  installed-Fork study. Validation uses isolated HTTP fixtures and localhost MCP;
  this does not claim live GitLab, Bitbucket, Azure or Gitea account acceptance.

## 0.69.0 — Cleanup, saved work and browser sign-in

- Review and prune selected obsolete tracking refs in an atomic transaction that
  retains their tips in recovery refs. Missing-upstream local branches are listed
  for inspection and kept. Delete one reviewed remote branch with an exact-ID
  lease, default-branch refusal and explicit remote outcome observation.
- Prepare squash merges or merges without committing. The UI and CLI distinguish
  ordinary squash commits from merge Continue/Abort recovery.
- Rename file-backed stash entries without changing their IDs/order, with a
  reflog backup. Explicit stash/pull-or-rebase/reapply retains the stash and a
  durable phase journal on success, conflicts or cancellation.
- Review LFS cache candidates and byte counts. Verify the selected remote, pin
  local remote objects, and hash/delete only the exact reviewed Windows file
  handles. Dirty tracked files, linked worktrees and unsupported caches are refused.
- GitHub browser sign-in through installed GCM, with its credential store disabled
  for that invocation. Tokens remain in memory unless Remember is selected;
  other hosting providers keep their existing token flow.
- Claude's UI revamp: Open includes a folder browser; Activity, diagnostics and
  Preferences are redesigned; External tools and MCP settings move to Preferences.
  External tool launches have a separate dialog; finished merge results can be
  hidden and reopened; quiet buttons no longer shift on hover.
- Clone and New Repository use dedicated forms with background destination checks.
  External tool settings are reached through Preferences. Their separate native
  validation is tracked in COORDINATION.md.
- README boundaries corrected. No new third-party dependencies or installations.

## 0.68.0 — Local Compare, LFS/gitlink conflicts and submodule diffs

- Compare a committed base against the staging area or tracked working files,
  preserving Unicode paths and rename pairs. Local snapshot checks reject stale
  lazy text/image/external diff reads; cancellation retains previous results.
- Resolve LFS conflicts using verified local objects, exact staged pointers and
  recovery copies. Missing/corrupt objects are identified without downloading.
- Resolve submodule conflicts by staging a reviewed gitlink without changing the
  child checkout. Original stages/index are backed up; external edits, existing
  locks and cancellation remain explicit outcomes.
- Submodule diffs list bounded added/removed commit history from local objects,
  retaining original IDs and guidance when a checkout or commit is unavailable.
- No new dependencies or changes to personal Git/SSH configuration.

## 0.67.0 — Linked conflict panes and AI branch reviews

- Full-height Ours / editable Result / Theirs columns with linked vertical
  scrolling anchored to common lines, a compact single-pane fallback, conflict
  markers and Alt+Up/Down navigation. Text is never padded or rewritten for layout.
- Existing save reviews, recovery copies, external-edit checks and cancelled-save
  behavior remain in use. Binary/delete conflicts retain whole-version choices.
- Read-only AI branch reviews through the installed Claude Code or Codex CLI.
  Prepare a pinned merge-base preview locally, inspect the exact bounded payload,
  then explicitly send it. Moving a selected ref requires a new preview.
- Cancellation stops the owned CLI process tree and retains earlier findings.
  Findings stay separate from the working tree; no suggestions are applied.
  Diagnostics exclude payloads and findings; default MCP state redacts them.
- No new dependencies. Includes the 0.66.1 security fixes; local builds remain unsigned.

## 0.66.1 — Security remediation

- Require Git LFS 3.8.1+ and probe its version outside repository configuration.
- Enforce MCP transport destinations immediately before execution, including
  background fetch; reject URL rewrites, custom transport overrides and redirects.
- Hide terminal, conflict-editor and external-tool private text from default MCP
  state; synthetic fixtures can opt in explicitly and revoke access immediately.
- Bound commit ancestry and rendered graph lanes; explain simplified graphs.
- Escape terminal controls in all developer inspect output, including errors.
- Preserve contended diagnostic records in bounded durable overflow and include
  them in Activity History/MCP reads, with crash recovery and sequence deduplication.
- Local builds remain unsigned by explicit user choice. A separate verification
  gate refuses unsigned distribution artifacts. See SECURITY.md for boundaries.

## 0.66.0 — Compare commits and branches

- New read-only **Compare** view with Base/Target revisions, filtered branch
  pickers, Swap, file filtering and per-file text/image/external diffs. Open it
  from the sidebar, View menu, two selected commits or branch details.
- Direct committed-tree comparisons pin both full commit IDs. Repository refresh
  and externally moved branches do not change an existing comparison; Compare
  again to load newer tips. The checkout, index and local edits stay untouched.
- Raw literal paths preserve Unicode and rename pairs; filtering matches old and
  new names. Metadata loads first, the file list is virtualized, and only the
  selected diff is prepared. Compact windows switch between files and diff panes.
- Cancel reports Stopping and retains the previous comparison. File reads offer
  Retry; navigation cancels owned reads and discards late results. Errors and
  diagnostic steps distinguish revision resolution, listing and file loading
  without recording paths, revisions or file contents.
- Includes the completed executable-icon work: Rocket's icon is embedded in
  `rocket.exe` and `rocket-mcp.exe` for Explorer and shortcuts. No new dependencies.

## 0.65.0 — Help guide, shorter dialogs, Rocket window title and icon

- **Help → Guide** (F1) explains how each operation works, by topic: reviews, recovery
  copies and cancelling; push, force push with lease and pushing branches together; merge,
  rebase and conflicts; stashes, reflog, tags, worktrees, submodules, Git LFS, patches and
  more. The **?** in a review dialog's header opens the matching topic on top of the dialog,
  which keeps its state. While the guide is open, Esc closes only the guide and Ctrl+Enter
  cannot submit the dialog underneath.
- Dialogs are shorter. They keep the preview, errors, progress, results and a one-line
  warning for destructive or irreversible actions; background explanations (what is checked
  again, what is kept for recovery, what cancelling means, syntax references, provider
  limits) moved into the guide, subtitles are one sentence, and confirmation messages lost
  their boilerplate paragraphs. The Claude Code/Codex drafting consent text is unchanged.
  Git behaviour, control keys and MCP state are unchanged; the new Help menu item is
  `help-guide`.
- The localhost MCP test window is titled **Rocket** (was "Rocket — MCP test session"), and
  both windows show Rocket's own rocket icon instead of eframe's default logo. The
  executable files' icon in File Explorer is unchanged.

## 0.64.1 — Button labels without a trailing ellipsis

- Buttons in dialogs, pages, banners and panes drop the trailing "…" as menus did in 0.64.0
  (**Abort merge**, **Review track**, **Verify signature**, **Rename**). The shared page buttons
  apply the same label rule, so new ones follow it; the Cancel button's transient labels read
  **Cancelling**, **Stopping** and **Checking result**. Placeholders, progress and status text
  and truncated text keep their ellipsis. Control keys and behaviour are unchanged.

## 0.64.0 — UI polish: plain menu labels, review marker, steadier loading

- Menus, submenus, right-click menus and the command palette list commands without the
  trailing "…" (**Fetch**, not **Fetch…**). Commands that need input still open their dialog.
  Buttons inside dialogs and pages are unchanged.
- The Activity review marker clears once the Activity drawer has shown the failed or
  recovery-pending result: the toolbar dot and the **Needs review** chip no longer stay on
  while the result is on screen. The drawer's Activity tab shows the dot, with a tooltip,
  only while the Terminal tab is in front.
- Steadier loading: the status-bar spinner appears after 0.25 s of work and stays through
  gaps shorter than 0.4 s, so a refresh and its diff, highlights and issue-link reads show as
  one load; a drafts/settings save waiting out its debounce is no longer shown as work.
- After a background refresh, the diff of the same selection stays on screen (read-only)
  until its reload arrives instead of flashing a spinner on every file change. A foreground
  refresh shows one **Loading diff…** state, and the commit details' file list no longer adds
  a second spinner. Control keys and Git behaviour are unchanged.

## 0.63.0 — Tool profiles and richer syntax colors

- Save several named external editor/diff/merge configurations, choose a default
  per type, and select a different saved profile for one launch. Add, duplicate,
  rename, confirm removal or discard edits; existing settings migrate to state v10.
- Language-specific lexical colors for types, functions, properties, attributes,
  operators and literals, with multiline handling and independent diff-side state.
  Bounded background preparation keeps search/word overlays and raw patch identity.
- Existing safe merge import, external-edit checks and owned-process cancellation
  remain in use. No new packages or installed external tools.

## 0.62.0 — 2026-10-23 (development build)

- Codex as a second drafting CLI: Preferences > AI commit messages chooses Claude Code or
  Codex (OpenAI's Codex CLI), and the composer button reads Draft with Claude or Draft with
  Codex. Drafting stays off by default; staged changes only, the 128 KiB and 16 MiB limits,
  binary files named only, styles, languages and Restore work the same for both.
- Codex is found without settings: the standalone installer's
  `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin\codex.exe` (or `CODEX_INSTALL_DIR`), npm's global
  install, then the newest copy bundled with the Codex app
  (`%LOCALAPPDATA%\OpenAI\Codex\bin\<build>\codex.exe`); another absolute `codex.exe` can be
  chosen. Nothing is installed and PATH is not searched.
- The first Codex draft explains that the staged diff goes to OpenAI through Codex and asks,
  separately from Claude Code's consent; nothing is sent without that click.
- Codex runs once as `codex exec --json` from Rocket's empty folder: read-only sandbox, never
  asking for approval, no session files, history, analytics or update check, without the
  user's `config.toml` (no MCP servers, profiles or hooks from it) and rules, with web search,
  the shell and every other tool, apps, plugins, skills, memories and sub-agents switched off,
  and Rocket's instructions in place of Codex's coding-agent prompt. Codex still adds the
  user's own global AGENTS.md, if any; the explanation says so. Same process-tree containment,
  2-minute timeout and Cancel as Claude Code.
- New typed failures `draft_codex_missing`, `draft_codex_unsupported`, `draft_codex_outdated`
  and `draft_codex_failed`; `draft_sign_in_required`, `draft_timed_out` and
  `draft_invalid_reply` name the CLI that ran. Diagnostics record the contained `codex`
  process by label, sizes, timings and codes only.
- Saved state v9 adds the CLI choice and Codex's program and consent; older state keeps Claude
  Code and its settings. Includes 0.61.1; no dependency additions.

## 0.61.1 — 2026-10-23 (development build)

- Adds a separate localhost MCP opt-in for real atomic-push acceptance on the
  explicitly approved GitHub test repository. Review and submission require the
  exact fixture namespace and transport URL; other endpoints remain refused.
- Includes a focused HTTPS/SSH runner with temporary branches, concurrent-writer
  rejection, cancellation before/after acceptance, independent remote observation,
  diagnostic checks and guarded cleanup. Results are recorded in VALIDATION.md.
- Atomic backend and desktop behavior are unchanged from 0.61.0. No new dependency,
  CLI command, account setting, or saved-state migration.

## 0.61.0 — 2026-10-23 (development build)

- Repository > Push branches together reviews 1–32 explicit local/remote branch
  mappings on one remote. Sources and remote tips are pinned; only creations,
  fast-forwards and already-current tips are allowed. Git must support atomic
  push: no force, sequential fallback, extra tags or implicit upstream changes.
- Changed tips/configuration require a fresh review. Server rejection and missing
  atomic support leave the transaction unpublished. Cancellation shows stopping,
  then independently observes every selected remote tip; it is not a rollback.
- Repository > Manage .gitignore edits the root rules and appends exact selected
  file/folder rules or built-in Rust, Node.js, Python and .NET starter templates.
  The Changes panel also offers Ignore for an untracked selected file.
- Existing comments, rule order, BOM and line endings survive append operations.
  Saves recheck file bytes/time and index, keep recovery copies under Git metadata,
  and refuse redirected/read-only/unsupported targets. Nothing is automatically
  staged, untracked, deleted or committed. Closing/cancelling leaves drafts unsaved.
- Native localhost MCP exposes both reviews, stopping and results. Logs record
  bounded steps, failure codes and branch counts; never rule text or transport URLs.
- Includes 0.60.0. No new dependencies or saved-state format changes. These new
  commands are desktop/backend features; standalone CLI syntax is unchanged.

## 0.60.0 — 2026-10-23 (development build)

- Ordinary rebase and pull-with-rebase reviews now predict content conflicts against
  the pinned destination in disposable Git storage. Clean, conflict, newly empty
  and unavailable results are distinct; the first stopping commit and Unicode
  paths are shown, with later commits explicitly unchecked.
- Originally empty commits are kept, matching Rocket's actual rebase flags.
  Interactive plans and recovery actions are not predicted in this pass. A
  fast-forward or already-current branch shows that no replay is needed.
- Stop check immediately shows stopping, waits for cleanup, discards the result
  and allows a fresh review. Pull retains its fetched ticket; checking again or
  changing strategy never fetches automatically.
- The checkout, index, named destination, merge settings and attributes are checked
  again before accepting/applying the review. Pull also rechecks upstream/tracking
  state. Unsupported attributes, custom drivers, submodules and limits are explicit;
  simulations run no hooks, external filters, signing or network operations.
- Localhost MCP exposes prediction and stopping state. Private diagnostics record
  rebase-prediction steps and fixed outcomes/counts. CLI pull previews gain an
  additive prediction field in schema 1 and a readable text summary.
- Includes 0.59.0. No new dependencies or state-format changes.

## 0.59.0 — 2026-10-23 (development build)

- Git Flow (Repository > Git Flow and the command palette), implemented with plain Git: Git
  for Windows has no `git flow` command and none is needed. Initialize writes git-flow's own
  `gitflow.*` settings (production and development branches; feature, release, hotfix and
  support prefixes; version tag prefix) to the repository's config, where the git-flow command
  line and Fork read them too, and creates develop from `origin/<develop>` (tracking it) or the
  production branch when missing; nothing is switched. Start creates a feature, release, hotfix
  or support branch at its reviewed base and switches to it; like git-flow it refuses a second
  release or hotfix, an existing version tag and a base behind its origin counterpart.
- Finish lists every step before anything runs: each switch; each merge and its mode (a
  one-commit feature may fast-forward develop as in git-flow, or always `--no-ff`; releases and
  hotfixes merge `--no-ff` into the production branch, get an annotated version tag and merge
  `--no-ff` back into develop); whether the branch is deleted with Git's safe `branch -d`; and
  that nothing is pushed. The steps run in order under one write gate through Rocket's existing
  branch, merge and tag transactions. A conflict stops the run and leaves the merge in Changes;
  after Continue merge, Finish again skips the completed steps.
- AI commit messages, off by default: Preferences > AI commit messages adds **Draft with
  Claude** to the Changes composer. It runs the user's installed Claude Code
  (`%USERPROFILE%\.local\bin\claude.exe` or a chosen `claude.exe`) once, with no tools, MCP
  servers, hooks, CLAUDE.md or saved session, from an empty folder, inside Rocket's process-tree
  containment with a 2-minute timeout and Cancel. The first draft explains what is sent and
  asks; nothing is sent without an explicit click. A draft only fills the editable message
  fields (Restore brings back a replaced message); committing stays the user's click.
- A draft sends the staged diff (up to 128 KiB: whole files that fit, the rest named; binary
  files named only; over 16 MiB nothing is sent), Rocket's fixed instructions, the style (plain,
  or Conventional Commits) and the language. Failures have `draft_*` codes (for example not
  signed in, or an outdated Claude Code); diagnostics record sizes, timings and codes only,
  never the diff, prompt or message.
- Saved state v8 adds the drafting preferences; older state loads with drafting off. Includes
  0.58.0; no dependency additions.

## 0.58.0 — 2026-10-23 (development build)

- Cherry-pick and revert reviews predict content conflicts in disposable Git
  storage, including reviewed sequences and a single merge's explicit mainline.
  Clean, conflicting, empty and unavailable outcomes are distinct; conflicts show
  paths and the stopping commit, and later commits remain explicitly unchecked.
- Stop check cancels background simulation and cleans its temporary storage.
  Checkout, index and merge-setting/attribute changes are rechecked before apply.
  Existing explicit apply and Continue/Skip/Abort recovery remain available.
- The isolated simulation runs no hooks, filters, custom merge drivers, signing
  or network calls.
  Unsupported merge settings/attributes, submodules, replaced objects and bounded
  failures show unavailable rather than clean. Private diagnostics record fixed
  steps/outcomes/counts; localhost MCP exposes the result and Stop check.
- Includes 0.57.1; no dependency additions. CLI commands/schema are unchanged.

## 0.57.1 — 2026-10-23 (development build)

- UI and CLI backend writes now share repository ownership across processes,
  including linked worktrees. A competing process receives a clear repository-busy
  error before mutation; CLI JSON uses `repository_busy` (exit 1). Reads and
  unrelated repositories remain available.
- Ownership covers preflight, mutation and existing reconciliation, releases on
  errors/cancellation/owner exit, and permits safe retained-submodule moves. The
  existing cancellable in-process write queue is preserved. No Git lock deletion
  or automatic retry is introduced.
- Diagnostics identify busy ownership failures; LFS paths use the same observed
  gate. No dependency additions. Includes 0.57.0; see CLI.md and VALIDATION.md.
- Localhost MCP can use the visible status-bar Cancel button while an operation
  runs, with the same cancellation permission as the Console button.

## 0.57.0 — 2026-10-23 (development build)

- WebP image comparisons: `.webp` files, recognized by their signature, open in the image
  comparison like PNG, JPEG, BMP and TGA, in every mode and with transparency. An animated WebP
  shows its first frame.
- One new package, `image-webp` 0.2.4, through the `image` crate's `webp` feature. The user
  approved it after scanning the download with Malwarebytes (Microsoft Defender is off on this
  computer), following a supply-chain review; Cargo.lock gains only its entry. See
  DEPENDENCIES.md.
- Includes 0.56.0. See VALIDATION.md.

## 0.56.0 — 2026-10-23 (development build)

- TGA image comparisons: files named `.tga` (any case), uncompressed or run-length encoded and
  with transparency, open in the image comparison like PNG, JPEG and BMP, in every mode. TGA has
  no file signature, so only the `.tga` name makes bytes decode as TGA; a `.tga` file that is
  really a PNG, JPEG or BMP shows as that format.
- WebP is not included: its one download (`image-webp` 0.2.4) could not get the required
  Microsoft Defender scan on this computer, so it waits for the user's decision (see
  DEPENDENCIES.md).
- No packages added (TGA is a feature of the already-locked `image` 0.25.10). Includes 0.55.0.
  See VALIDATION.md.

## 0.55.0 — 2026-10-23 (development build)

- **Repository → Statistics…** (also in the sidebar and command palette): a read-only page with
  **Commit activity** (commits, merges, authors, first and last date; authors as a pie and list;
  commits per month or year, by weekday and by hour in each author's time zone) for 30 days,
  90 days, 1 year or all time, on the current branch or all branches; and a **Treemap** of the
  files at HEAD sized by bytes and coloured by the number of commits that changed them in the
  chosen period, with folders to open and a breadcrumb back. Reads are bounded (100,000 commits
  for activity, 20,000 for the treemap), run in the background and can be stopped.
- MCP: `dialog.statistics` state and the page's controls; diagnostics `commit-activity` and
  `activity-treemap`.
- Includes 0.54.0. No dependencies added. See VALIDATION.md.

## 0.54.0 — 2026-10-23 (development build)

- Collapse merge commits in the commit list: a collapsed merge hides the commits it brought
  in, shows a **+N** badge with how many it hides, and the graph follows its first parent.
  Use **←** / **→** on a focused merge row, its right-click menu, the button in its details,
  the badge (expands), or **View → Collapse All Merges / Expand All Merges** (also in the
  command palette). A hidden selected commit hands the selection to its merge. It only changes
  the view of the open tab.
- File history timeline: the loaded commits of a file or folder on a time axis, oldest at the
  left; hover for date, author and subject, click to select, Ctrl+click to compare.
- MCP: `collapsed_merges` and `all_merges_collapsed` in the tab state; controls
  `commit-expand-<id>`, `detail-merge-fold`, `view-collapse-merges`, `view-expand-merges`,
  `file-timeline-<id>`.
- Includes 0.53.0. No dependencies added. See VALIDATION.md.

## 0.53.0 — 2026-10-23 (development build)

- Stacked branches in rebases: when other local branches point at commits that a rebase or
  interactive rebase replays, the review lists them with their commits, and **Move them with
  the rebase (--update-refs)** moves each to its rewritten commit when the rebase finishes
  (after a squash or fixup to the combined commit; a dropped commit's branch to the commit
  before it). It is off unless ticked; `rebase.updateRefs` in Git configuration is still
  ignored. Branches checked out in another worktree stay and are shown as staying. The listed
  branches are checked again before Git starts, so a branch moved after the review makes the
  review stale. After a conflict, the **Continue** review lists the branches the rebase will
  move; **Abort** leaves them where they were. The activity reports each branch's move.
- MCP: checkbox `dialog-rebase-update-refs`; `dialog.rebase_update_refs` reports the choice,
  whether the reviewed rebase uses it, and the listed branches.
- Includes 0.52.0. No dependencies added. See VALIDATION.md.

## 0.52.0 — 2026-10-23 (development build)

- Apply a patch from the clipboard: **Repository → Apply patch from clipboard…**, the
  command palette, or **Ctrl+V** while no text field, terminal or dialog has the keyboard,
  opens the reviewed patch import with the copied Git diff instead of a file. It is checked
  in a disposable repository, revalidated and applied exactly like a patch file (16 MiB
  limit); the reviewed text is what gets applied. Copied text that is not a diff only shows
  a notice, and nothing opens.
- Drop a patch file on the window to open the same review for it (the first file if several
  are dropped); dropped folders still open as repositories. Nothing changes until **Apply**.
- MCP: `rocket_drop_files` (1–4 existing paths inside the fixture root) delivers real
  dropped files with the scope, active-tab, blocking-dialog and busy checks of other input;
  `dialog.patch_clipboard_bytes` reports the size of a clipboard source.
- Saving issue link rules now reports "1 issue link rule saved" and "Issue link rules
  removed" correctly.
- Includes 0.51.0. No dependencies added. See VALIDATION.md.

## 0.51.0 — 2026-10-23 (development build)

- **File → Repository Manager…** (`Ctrl+Shift+M`) saves named repository shortcuts
  in folders and workspaces. Rename, relocate or remove shortcuts, edit workspace
  membership and choose its active repository. Opening adds tabs without closing
  existing repositories, drafts or terminal sessions. Folder removal moves shortcuts
  to Ungrouped; no manager removal deletes repository files.
- Repository discovery runs in cancellable workers; unavailable paths report errors
  and can be corrected. Workspace opening can stop between repositories, preserving
  those already opened. Settings v7 appends the saved library, migrates v1–v6 and
  retains atomic background saves and the exclusive settings lock. Older binaries
  cannot read v7 settings; back up existing settings before downgrading.
- `rocket-cli pull` keeps fast-forward-only by default and adds explicit `--merge`,
  `--rebase` and `--preview`. Preview fetches remote refs/objects, while leaving
  HEAD/index/working files unchanged. Integration uses the desktop backend guards.
- `rocket-cli rebase continue|skip|abort [--preview]` recovers paused rebases; merge
  conflicts use existing `merge continue|abort`. JSON includes strategy, fetch,
  preview, integration, recovery references and fresh state/error observations.
  Cancellation preserves the result and uses fresh tokens for reconciliation.
- No new libraries, downloads, credential changes or network services are required.

## 0.50.0 — 2026-10-23 (development build)

- Issue links: issue IDs in commit messages (subject and body, in the details pane and the
  Space popup) become links to your tracker, such as `PROJ-12` → Jira or `#7` → Redmine.
  **Repository Settings → Issue links** edits this repository's rules: a name, an ID
  pattern (a documented regular-expression subset, matched in linear time) and an https
  link where `{id}` is the whole match and `{1}`…`{9}` its groups, with a **Try it** field.
  Rules are saved in the repository's Git config (`rocket.issuelink.<name>.pattern` and
  `.url`); rules in global or system Git config apply too, and a repository rule with the
  same name replaces one. Only plain https links with a host name are opened, and only
  after the URL is checked again; the matched ID is percent-encoded into the link.
- Test connection in **Add remote**, **Edit remote URL** and **Clone**: lists the remote's
  branches with `git ls-remote` and reports the branch count and default branch, or a
  classified failure (host, network, TLS, SSH host key, key or agent, authentication,
  repository) with the next step. It never asks for credentials (no askpass; Git
  Credential Manager and SSH are told not to prompt), stops after 20 seconds, can be
  stopped, and saves or downloads nothing. A clone source follows the clone's rules.
- Includes the 0.49.1 push refresh change. No dependencies added. See VALIDATION.md.

## 0.49.1 — 2026-10-23 (standalone CLI development build)

- Ordinary push now refreshes branches and working-tree status concurrently after
  the write stops. Both observations remain fresh, retain their own errors, and
  finish before releasing the repository write gate. Cancellation still reports
  an unknown remote outcome when acceptance was not confirmed by Git output.
- The CLI JSON schema and log fields are unchanged. Branch/status refresh events
  can interleave; pair spans by session and ID rather than line adjacency.
- Timing investigation isolated the earlier 5.094 s outlier to process startup.
  Local replay did not reproduce it. This removes measured serial refresh cost;
  it does not claim to eliminate occasional Windows process-start stalls.
- No dependencies added. Desktop bundle remains the separately verified 0.49.0;
  focused CLI and backend evidence is in VALIDATION.md.

## 0.49.0 — 2026-10-23 (development build)

- Line history: in File history → Blame, click a line's commit, Shift-click another
  line to select a range, then **Line … history** (or the new **Lines** view) lists the
  commits that changed those lines, newest first, each with its changes to them. Git's
  `log -L` follows the lines through edits and the file through renames; up to 200
  commits are listed.
- Compare two versions of a file: in File history, Ctrl-click a second commit to see
  the file's changes from the older to the newer one (**Compare** view), including a
  rename between them. A plain click returns to a single commit.
- No dependencies added. See VALIDATION.md for the focused checks.

## 0.48.2 — 2026-10-23 (standalone CLI development build)

- Add `rocket-cli.exe`, reusing rocket-git for repository reads, guarded local
  writes, branches, stashes, merge/recovery, remote configuration, clone, fetch,
  fast-forward pull and explicit single-branch push. The old inspect example remains.
- Consistent grouped commands, strict argument validation, versioned JSON, stable
  exit codes and lossless byte fields. Primary writes, remote outcomes, refresh
  warnings and retained clone/init workspaces are represented separately.
- Windows Ctrl+C/Ctrl+Break and timed cancellation wait for Git cleanup and fresh
  observations; cancelled writes are not reported as rolled back. CLI limits and
  cancellation now include the initial Git version probe, with no duplicate probe.
- Escape terminal controls in readable output, omit raw hook/helper output and URL
  credentials, and retain the existing private bounded diagnostic logging.
- Standalone CLI bundle only; the existing desktop 0.48.1 binary is retained.
  No new third-party package or installation. See CLI.md and VALIDATION.md.

## 0.48.1 — 2026-10-23 (development build)

- Faster Windows refresh and diff reads: find the suspended Git child’s thread using
  a process-specific OS snapshot, removing repeated system-wide thread scans. Git
  still starts suspended, joins its cancellation job before running, and cleans up
  owned helpers. Repository reads remain fresh, with the same external-change rules.
- Diagnostics and Activity History show separate prepare, spawn, containment,
  execution and pipe-drain timings in microseconds. Cancellation still reports its
  actual stop/cleanup time separately; no arguments, paths or command output are added.
- Focused before/after and localhost MCP evidence is recorded in VALIDATION.md.
  No new dependencies, helper executables or runtime payloads.

## 0.48.0 — 2026-10-23 (development build)

- Right-click a repository tab for **Open in File Explorer**, **Open in Terminal** (Rocket's
  terminal for that repository) and **Close repository**. **Ctrl+Shift+T**, or **+ →
  Reopen closed repository**, brings back the most recently closed repository (up to 20
  per session).
- **View → Repositories in Sidebar** (also in Preferences) lists open repositories as rows
  at the top of the sidebar instead of the tab strip, with the same menu and close button.
  The tabs return while the sidebar is hidden or narrowed to icons.
- Commit list keyboard: a clicked commit keeps keyboard focus; Up and Down select the
  previous or next commit, Space opens a details popup (Space or Esc closes it), and
  Tab or Shift+Tab leave the list for the next or previous pane instead of stepping
  through every row.
- Branch lists with a filter field: Push offers the selected remote's branches, as last
  fetched, for the destination; the interactive rebase base and the worktree remote and
  existing-branch lists filter the same way. Pull uses the configured upstream and has no
  branch list.
- Spell check for commit messages on Windows, off by default: **Preferences → Commit
  messages**. Misspelled words in the summary and description are underlined; right-click
  one for suggestions, **Ignore** (this session) or **Add to dictionary** (the Windows user
  dictionary). It uses the Windows language, or en-US when that has no checker; words in
  Han, kana or Hangul are not flagged. A new first-party crate, `rocket-spell`, calls the
  Windows Spell Checking API; no third-party dependency. Not offered on other platforms.
- Saved state format 6 adds the two preferences; older files load with both off.

## 0.47.0 — 2026-10-22 (development build)

- Interactive rebase can rewrite the whole branch from its root commit: tick **Rewrite
  from the root commit** in the review, or drop a commit on or below the first commit.
  Every commit is in the plan and the first kept commit becomes the new root, so the
  first commit can be reworded, edited, squashed into, moved or dropped like any other.
  If the first replayed commit conflicts, Continue is refused, because Git would keep
  its temporary empty base commit in the history; Skip or Abort instead.
- New worktree can start from a remote branch: **Remote branch** creates a new local
  branch that tracks it (Git's usual upstream settings) at the reviewed remote commit,
  with the local name suggested. A remote branch that moves before creation makes the
  review stale; symbolic names such as `origin/HEAD` are refused.
- No dependencies added. See VALIDATION.md for the focused checks.

## 0.46.2 — 2026-10-22 (development build)

- History pages reuse repository discovery, combined history reuses the snapshot's
  observed HEAD and branch list, and exact operation-marker probes run concurrently.
- Text search reads up to 4,096 commits per worker batch with buffered object output.
  Existing byte limits remain active; oversized batches retry at the former 1,000
  size. History ordering, pinned cursors and literal Unicode matching are preserved.
- No Git-content cache or dependency changes. Cancellation and external-change
  refresh remain active; focused tests and native localhost MCP measurements use
  disposable local repositories. See VALIDATION.md for results and limits.
- Correct README boundaries for reviewed pull, force-with-lease, remote tags,
  selected-file stashes, hosting accounts and the SSH key manager.

## 0.46.1 — 2026-10-22 (development build)

- Ctrl+C on the focused terminal screen now handles the native Windows Copy event,
  interrupting the foreground command while keeping the same PowerShell session.
  Copy in the separate command field and other text inputs remains unchanged.
- The Interrupt button and keyboard interruption also work when Rocket's launcher
  ignores Ctrl+C. A small first-party Rust `rocket-terminal-host.exe` resets that
  inherited setting inside the existing terminal job before starting PowerShell.
  Include this helper beside the app; Cargo builds it automatically. Existing
  sessions need End session / New Terminal after the update.
- Typing, history, completion, foreground-child cleanup and shell state are covered
  by focused real PowerShell and localhost MCP checks. Diagnostics retain lifecycle
  labels without command or terminal-output payloads. No new dependencies; the
  reviewed Microsoft ConPTY runtime and the 0.45.1 job-list startup are preserved.

## 0.46.0 — 2026-10-22 (development build)

- Worktrees adds reviewed Move, Repair and Prune registration actions. Move refuses dirty or open linked worktrees and existing destinations; repair reconnects an intact manually moved checkout while preserving edits and staging. Prune affects only one missing registration, retains its metadata and committed tip, and never deletes working files.
- Maintenance previews are invalidated by external metadata changes. Errors report retained recovery data and observed results; cancellation before publication leaves the checkout unchanged. A started move finishes its minimal backlink update before reporting cancellation.
- Localhost MCP receipts and Activity & diagnostics now separate queue, validation and input-delivery timings, with an internal numeric correlation ID and no request payloads. Expired/cancelled input is never replayed. These diagnostics do not establish the cause of an earlier intermittent stall.
- Includes the 0.45.1 process-start fixes. No dependencies added or upgraded. See VALIDATION.md for executed checks and limits.

## 0.45.1 — 2026-10-22 (development build)

- The terminal panel's PowerShell and the SSH manager's `ssh-keygen`/`ssh-add` prompts
  are now created directly inside their own job, instead of suspended, then added to
  the job and resumed. If Rocket ended between those steps, such a process stayed
  suspended for ever; now it always ends with Rocket.
- No dependencies added or upgraded. Built on the 0.45.0 source.

## 0.45.0 — 2026-10-22 (development build)

- Rocket starts each Git process, external tool and custom command suspended, puts
  it in its own job and only then lets it run. If Rocket (or a test process) exited
  in between, that process stayed suspended for ever. Exit now waits for starts in
  progress, up to 10 seconds, so every child runs in its job, which ends it with
  Rocket. This covers normal exits; a crash or a forced end can still leave one.
- Rocket's own standard input, output and error are no longer inherited by every
  child; each child still gets the handles it is given. A leftover child can no
  longer keep a caller's pipe open after Rocket exits, which made `cargo test` wait
  for ever.
- No dependencies added or upgraded. Built on the 0.44.0 source.

## 0.44.0 — 2026-10-22 (development build)

- **Drag and drop.** Drop a sidebar branch on the checked-out branch, or the
  checked-out branch on another local branch, and choose **Merge … into …** or
  **Rebase … onto …**: the existing merge or rebase review opens prefilled. Drop a
  commit of the checked-out branch on another commit and choose **Above**,
  **Below**, **Squash into** or **Fixup into**: the interactive rebase review opens
  with that plan. Nothing changes until the review is applied; Escape, Cancel or a
  click elsewhere discards a drop. Commits can be rearranged in the branch's
  first-parent history after its latest merge, up to 200 commits. The branch menu
  also gains **Rebase current branch onto this…**.
- **Automatic fetch** (Preferences, off by default) fetches every remote of every
  open repository every 5, 10, 15, 30 or 60 minutes, with the checks of a manual
  fetch. It never prunes, fetches tags, merges or asks for credentials: a remote
  that needs a sign-in shows a quiet failure in the status bar and is retried at
  the next interval. It does not start while an operation runs, a repository's
  fetch stops when an operation starts there, and **Stop** ends a running fetch.
- **Custom commands** (Repository → Custom Commands, command palette): up to 20
  programs, each an absolute .exe path with one argument per line and `{repo}`,
  `{branch}`, `{commit}` and `{file}` placeholders. Every run first shows the exact
  program, arguments and folder and starts only after **Run**. No shell parses
  anything; choose cmd.exe or powershell.exe to run a script. Output goes to
  Activity (the first 64 KiB of each stream) and Cancel stops the program and
  everything it started; programs it opens, such as an editor, keep running after
  it finishes.
- Rocket state moves to version 5 (fetch interval and custom commands) and still
  reads earlier versions. Local MCP automation gains `rocket_drag`; automatic fetch
  runs there only with a runner marker and local fixture remotes, and custom
  commands are limited to fixed Git test commands with Rocket's own Git.
- No dependencies added or upgraded. Built on the 0.43.0 source.

## 0.43.0 — 2026-10-22 (development build)

- Windows OpenSSH key/account profiles with encrypted Ed25519 or RSA 4096
  generation, existing matching key registration, public-key copy and selected-key
  agent unlock/unload. Dedicated ConPTY prompts keep passphrases out of shell
  history, process arguments, saved settings and diagnostics.
- Reviewed repository-local SSH selection uses a separate profile config.
  Existing SSH config and key files are preserved. Exact config and key rechecks,
  Git config locks, atomic displaced-file backups and reviewed restoration protect
  external edits. Working files, staging and remote URLs are unchanged.
- Immediate Stopping feedback, reconciled agent state, typed errors and numeric
  OpenSSH exit/stop diagnostics. Local MCP controls require disposable-key fixtures.
- No new dependencies, downloads or installed tools. Uses installed Windows
  OpenSSH and Rocket's already reviewed ConPTY runtime.
- Built on the completed 0.40 source. Reserved 0.41/0.42 work from the other agent
  remains separate and is not included in this build.

## 0.40.0 — 2026-10-22 (development build)

- **Save changes to stash** can save **only selected files**: tick unstaged files
  (and untracked ones when they are included); every other change stays in the
  working tree. Staged files are always saved, because Git records the whole index
  in a stash. A staged deletion or rename can only be stashed with every change, so
  the dialog asks to unstage it first or save all changes.
- **Save as patch…** on a stash reviews exporting its tracked changes (against the
  commit it was saved on) to a new patch file. Untracked files saved in the stash
  are not included; the stash list and working tree are unchanged.
- **Repository → Delete Branches…** deletes several local branches from a filtered
  checklist. The current branch and branches checked out in other worktrees are not
  offered. Git deletes only merged branches and keeps the others; the result lists
  each deleted branch's last commit and each kept branch with Git's reason.
- **Tags → Delete several…** and **Remote tags → Publish several…** choose tags from
  a checklist, then review them. A review names every tag it refuses (changed,
  symbolic, already on the remote) and must cover exactly the selection before the
  button enables. Tags then change one at a time with the single-tag checks;
  deleted tag objects are kept at recovery references and a remote tag is never
  replaced. A batch stops at the first tag that is not verified and leaves the
  rest unchanged. At most 200 branches or tags at once.
- No dependencies added or upgraded.

## 0.39.0 — 2026-10-22 (development build)

- **Repository → Hosting and Accounts…** adds GitHub, GitLab, Bitbucket Cloud,
  Azure DevOps Services and Gitea accounts; paged PR/MR and issue lists with state
  filters and a text filter over loaded results. Azure shows project work items;
  Bitbucket Cloud's retired issue tracker is explicitly unavailable.
- Create a pull/merge request between two pushed branches in the same repository.
  Review the account, server, repository and branch commits first. Submission
  rechecks both commits and refuses stale or reused reviews. Interrupted writes
  report an unknown remote outcome and require checking that repository before retry.
- Optional remembered tokens use Windows current-user DPAPI, separate from Git
  Credential Manager and SSH. Tokens, API bodies and private repository identifiers
  stay out of diagnostic logs and MCP state. No browser OAuth in this pass.
- Existing GitHub repository/notification browsing remains under its own renamed
  menu entry. New provider acceptance uses isolated API fixtures; real account,
  SSO and self-hosted deployment acceptance remains outstanding. See HOSTING.md.
- No dependencies added or upgraded; no tools installed. System SSH remains in use.

## 0.38.0 — 2026-10-22 (development build)

- Commit search has a **Text / Path / Content** selector. **Path** finds commits
  that changed a file whose path contains the text (ignoring case; `\` matches `/`;
  renames count as both paths). **Content** finds commits that add or remove the
  exact text, ignoring case, like `git log -S`; it checks 200 commits per step.
  Searches stay cancellable and continue through the whole selected history.
- The Changes composer starts an empty message from `commit.template`. The
  template's comment lines become the description's guidance and are never
  committed; after each commit the next message starts from the template again.
- **Messages** in the composer reuses one of the last 20 messages committed in
  Rocket for this repository, or the template. Recent messages are saved with
  drafts and can be cleared. Rocket state moves to version 4 and still reads
  earlier versions. No spell checking.
- No dependencies added or upgraded.

## 0.37.3 — 2026-10-22 (development build)

- Native MCP input validation for force-with-lease, remote tags and explicit pull
  fetch runs on a cancellable worker. State, captures and diagnostic reads remain
  available while the check runs; progress identifies the current check.
- Escape, Cancel input and `rocket_cancel_input` cancel undelivered validation.
  The existing 20-second deadline is retained. Expired, cancelled, changed or
  stale inputs are refused without replay; cleanup retains the single worker slot.
- Effective Git configuration (including includes) is compared before/after the
  check. UI selection is rechecked and the same enabled control is resolved again
  after the progress overlay closes, before input delivery.
  Validation step timings omit paths, configuration values and helper output.
- No dependencies added or upgraded.

## 0.37.2 — 2026-10-21 (development build)

- Force-with-lease Cancel immediately acknowledges the stop request, then shows
  when the push has stopped and Rocket is checking the remote result. The dialog
  and status bar share these phases; repeated cancellation is disabled.
- Diagnostics distinguish cancellation-to-Git-stop/cleanup time from remote
  verification and refresh. Activity details retain the numeric timing field.
- Remote verification still runs after cancellation; it never implies rollback.
  No dependencies added or upgraded.

## 0.37.1 — 2026-10-21 (development build)

- Localhost force-with-lease testing can use the exact approved HTTPS/SSH test
  repository only when a separate fixture marker matches its owned branch and URL.
  Remote editing remains local-only; no general network allowance was added.
- Verified force-with-lease through localhost MCP over live HTTPS and SSH:
  reviewed replacement, concurrent remote changes, GitHub rejection, and
  cancellation before/after acceptance. Temporary branches were safely cleaned up;
  local changes stayed intact and diagnostic privacy checks passed. Detailed
  results are in VALIDATION.md.
- No dependencies added or upgraded.

## 0.37.0 — 2026-10-21 (development build)

- Push offers a separate **Review force-with-lease…** flow with the actual push
  endpoint, expected remote tip, pinned local tip and removed/added commit lists.
  The exact destination branch name must be entered before applying.
- Apply revalidates the source, configuration and remote tip and sends one explicit
  expected-object lease. Ordinary push remains non-forcing. There is no plain-force
  switch, automatic retry, branch deletion or implicit fetch/upstream change.
- Cancellation and rejection receive a fresh remote observation with separate
  replaced, unchanged, different, missing and unknown outcomes. Staged/unstaged and
  untracked files are preserved. Typed failures and diagnostic steps omit payloads.
- Local-only MCP controls and focused fixture tests cover stale reviews, concurrent
  server updates, rejection and cancellation. No dependencies added or upgraded.

## 0.36.0 — 2026-10-21 (development build)

- **Repository → GitHub…** (also in the command palette) opens a read-only GitHub
  page. It signs in with the github.com credential Git's credential helper (Git
  Credential Manager) already stores; the token stays in memory and is never shown,
  logged, saved or put on a command line. **Sign in…** lets Git Credential Manager
  prompt when no usable sign-in is stored.
- **This repository** shows the branch's pushed state, fork parent and any open pull
  request. **Create pull request…** opens GitHub's comparison page in the browser;
  **View pull request**, **View repository**, **View branch** and **Pull requests**
  open the matching github.com pages. Rocket opens only validated github.com links.
- **Repositories** lists the account's repositories with paging and a filter, plus
  **Open on GitHub** and **Clone…** (fills the Clone dialog). **Notifications** lists
  unread or recent notifications and opens their pages without marking them read.
- Requests are cancellable read-only GETs through Windows' curl.exe with fixed
  failure codes and private `github-*` diagnostics. Rocket writes nothing to GitHub
  or to Git's credential store. No dependencies added.

## 0.35.0 — 2026-10-21 (development build)

- Pull separates **Fetch and review** from **Apply pull**, with explicit
  fast-forward-only, merge and linear rebase choices. Reviews show actual incoming
  and local-only history; changing strategy reuses a pinned fetched commit.
- Apply rechecks checkout, upstream and tracking state, protects local files and
  reuses existing merge/rebase conflict recovery. Rebase retains the original tip.
  Cancellation reconciles state without automatic retries or rollback; fetch and
  integration results are reported separately in Activity and diagnostic steps.
- Image comparisons add adjustable overlay, swipe and exact RGBA pixel differences
  with counts/bounds. Unequal sizes keep a shared top-left origin without scaling
  one version to match the other. Work runs off the UI thread with cancellation,
  bounded allocation, external-edit reloads and no image-file writes.
- No dependencies added or upgraded. Existing PNG/JPEG/BMP and decoder limits
  remain; combined pixel masks above 8 megapixels show a specific limit message.

## 0.34.0 — 2026-10-21 (development build)

- **Repository → Signing and Verification…** reads local and effective settings
  and reviews changes to the signing format, key, commit signing and SSH allowed
  signers file. Saving uses a config lock, stale-settings checks and atomic
  publication. Global settings, keys and trust files are not edited.
- Commit details and local tag details offer **Verify signature…**. Results
  distinguish unsigned, valid/trusted, valid/untrusted, invalid, expired, revoked
  and unavailable verification. Reads run on demand in cancellable workers;
  repository refresh does not start signing or verification helpers.
- Annotated tags offer **Sign tag using repository settings** for SSH/OpenPGP.
  Git signs in an owned temporary bare repository, then Rocket validates and
  publishes the resulting object through the existing create-only transaction.
  Existing tags, HEAD, staging and working files remain protected. The final
  signed object ID is available after signing. Unsigned creation remains explicit.
- Signing failures have fixed recovery guidance and diagnostic codes. Logs record
  request/step outcomes without key paths, fingerprints, identities, messages or
  raw signer output. Git continues to handle commit signing and key unlock prompts.
- No dependencies added or upgraded. No key generation/import/trust management UI,
  X.509 signing editor, remote signature download or installer signing is included.

## 0.33.0 — 2026-10-21 (development build)

- **Tags → Remote tags…** inspects one configured push destination and compares
  advertised objects with local tags. Explicit refresh, filtering, Unicode names,
  cancellation and stale-result notices keep network reads under user control.
- Review publication of one absent remote tag or type the exact name to delete
  its reviewed object. Publication preserves the local tag object, including its
  annotation/signature bytes; existing remote tags are never overwritten. Deletion
  leaves the local tag in place and does not create a remote backup or undo.
- Recheck destination/configuration, the local source and the advertised remote
  object before pushing one explicit reference with an expected-object lease.
  Ambiguous destinations, same-repository targets and symbolic remote tags are
  refused. Inspection requires Git protocol v2 with server-option support.
- Observe the remote after completion or interruption. Errors distinguish verified
  effects, rejection and unknown remote state, with guidance against blind retries.
  Diagnostic records include fixed operation/step/error codes without private
  tag names, endpoint URLs, annotations or raw server output.
- Added localhost MCP state and destination guards for remote-tag controls, plus
  focused native acceptance using disposable local repositories. See
  `VALIDATION.md` for the actual checks and remaining limits. No dependencies added.
- Backend API: `remote_tags`, `preview_remote_tag`, `apply_remote_tag` and
  `remote_tag_destination`, with remote-tag snapshot/preview/report types and
  new `TagFailure` variants. Exhaustive consumers of that enum need new arms.

## 0.32.0 — 2026-10-21 (development build)

### Added

- The conflict editor resolves binary and delete/modify conflicts during merge,
  cherry-pick, revert and rebase. **Keep ours…**, **Keep theirs…** or, when one side
  deleted the file, **Keep deletion…** keeps one whole version. Binary or unreadable
  files show Ours/Theirs version cards (binary or text, size, mode, object ID and
  whether the working file already holds that version); a text modify/delete
  conflict keeps its editor and adds a **Whole file** strip. The status pill reads
  **Binary** or **One side deleted** instead of **Resolved**.
- Each choice opens a confirmation that states its effect: replace the working file,
  create it, move it into recovery, or change only the index entry. Applying it
  rechecks the stages, HEAD, Git operation, conversion settings and working bytes
  under the write gate, publishes with the text editor's checked `ReplaceFileW` path
  and recovery copies in `<git-dir>/rocket-conflict-recovery/`, then stages exactly
  the reviewed index entry with `git update-index --index-info` and verifies it.
  Nothing is committed, and the operation is not continued.
- Line endings follow checkout rules without running filters. Content filters such
  as Git LFS, working-tree encodings, ident expansion, symbolic links, submodules and
  versions over 16 MiB are refused with `unsupported_conflict_version`.
- Backend: `Repository::review_whole_conflict` and `keep_conflict_side` with
  `WholeConflictPreview`, `ConflictKeep`, `ConflictVersion`, `WholeConflictReport`,
  `WholeFileChange` and `CONFLICT_FILE_LIMIT`. New fixed failure codes:
  `unsupported_conflict_version`, `conflict_version_unavailable`,
  `conflict_review_stale` and `conflict_resolution_unverified`.
- MCP: `conflict-keep-ours/theirs/deletion` and `conflict-whole-scroll` controls,
  the `conflict-keep` confirmation (also inside owned rebase fixtures),
  `conflict_editor.whole` and `conflict_editor.kept` state, and each activity's
  `whole_conflict` outcome. New native runner `work/test-mcp-binary-conflicts.cjs`.
  No dependencies changed.

### Changed

- The text editor's unsupported-file guidance now points binary and deleted files
  to keeping a whole version.

### Migration

- `ConflictFailure` has four new variants; exhaustive matches need new arms.
- `rocket_ui::backend::Action::KeepConflictSide` and `Outcome::whole_conflict` are new.

## 0.31.0 — 2026-10-21 (development build)

- **Reflog & Recovery → Lost commits** extends the former Unreachable scope to
  include unreachable ancestors, with scan stages, elapsed time, explicit read
  cancellation and reuse of completed results when loading more rows.
- Recovery reviews show the pinned commit, author, parents and its own change
  (first parent for merges). Recovery creates a new local branch without switching
  HEAD or writing the index or working files. Reset is unavailable in this scope.
- Recheck the selected object, reviewed diff and local tracked state before
  creating the branch; refuse collisions, stale previews and missing content.
  Failed/cancelled scans retain previous complete results without presenting them
  as a fresh scan. Cancelled writes reconcile actual refs and local state.
- Added typed scan/recovery errors and private diagnostic operation/step records.
  Scans reject shallow/replacement/grafted histories and more than 10,000 lost
  commits; already-pruned content cannot be reconstructed. No dependencies added.

## 0.30.0 — 2026-10-21 (development build)

### Added

- Interactive rebase adds **Edit** (stop to amend a replayed commit), **Stop after
  this commit** and **Run a command after this commit**. The review lists stops
  and commands and warns when commands run; planned stops are reported as pauses,
  and Skip at a planned stop becomes **Discard changes and continue**.
- Ctrl-click and Shift-click select several history rows; **Cherry-pick N
  Selected Commits…** and **Revert N Selected Commits…** apply 2–100 non-merge
  commits as one reviewed sequence. A paused sequence shows the commits left and
  reviewed Continue/Skip/Abort in the banner; Abort keeps the commits it created
  under `refs/rocket/recovery/sequence/`.
- Reflog & Recovery adds **Deleted branches** (from this worktree's HEAD log,
  offered back under their old names), **Kept by Rocket** (`refs/rocket/`) and
  **Unreachable** (dangling commits such as dropped stashes) next to HEAD and All
  local refs. The recovery-branch and reset reviews are unchanged.
- Backend: `commit_sequence`, three new `ReflogScope` variants, and the rebase plan
  fields and stops described below.
- MCP: `rocket_click` takes optional `ctrl`/`shift` modifiers for commit rows; new
  rebase stop/command, sequence, selection and recovery-reference state and
  controls. Automation can submit only the planned rebase commands `true` and
  `false`. No dependencies changed.

### Migration

- `CommitActionRequest.target: Option<ObjectId>` is now `targets: Vec<ObjectId>`
  (empty for amend and Continue/Skip/Abort; one or more commits to start).
- `RebasePlanEntry` gains `exec: Option<String>` and `stop_after: bool`; use
  `RebasePlanEntry::new(commit, instruction)` for plain rows.
  `RebaseInstruction::Edit` and `RebaseWriteReport.stop` are new, so exhaustive
  matches need a new arm. Plans without edit rows, stops or commands are still
  saved in format version 1, and version 1 plans are still read for recovery.
- `ReflogEntry.reference` is a new field, and `ReflogScope` has three new variants.

## 0.29.0 — 2026-10-20 (development build)

- Added reviewed manual bisect from Repository → Bisect and the command palette.
  Choose known good/bad commits, test each revision, then review Good, Bad or Skip.
  Found and inconclusive results are distinct; skipped candidates stay visible.
- Save the original branch or detached commit and a local recovery ref before
  starting. Restore verifies the checkout before clearing the search. A moved
  original branch or new detached commit is protected, never forcibly reset.
- Use Git's no-checkout bisect engine with separate non-forced checkout. Pending
  checkouts can resume without submitting a decision twice. Interrupted setup,
  checkout or restoration keeps recovery state across app restarts.
- Recheck previews, index locks and tracked edits; protect untracked/ignored file
  collisions. No automatic stash, forced checkout, reset-hard or test commands.
  Support complete local ranges up to 10,000 commits; shallow/replaced/grafted
  history and ranges containing submodules require Git.
- Expose native controls/state through localhost MCP and record bounded private
  diagnostic steps, refusal codes and observed results. No dependencies changed.

### Fixed

- The Repository menu now uses the window height (up to the window height minus 80
  points) and scrolls only when its rows do not fit; it used to stay about 386
  points tall at every window size.
- Submenu rows (File → Open Recent, View → Diff Layout and Appearance,
  Repository → Stash) line up with the other menu rows.
- In a short terminal drawer the screen shows whole rows only; the top row used to
  be cut through its glyphs below the command field. The shell keeps at least four
  rows; extra rows scroll.

## 0.28.0 — 2026-10-20 (development build)

- Extended Reflog & recovery with an All local refs scope. Retained branch,
  remote-tracking and other ref logs can supply a commit for the existing reviewed
  recovery-branch/reset workflows. Deleted or expired logs are not reconstructed.
- Added fixed-path folder history, including descendants and deleted folders,
  with pinned pagination and commit diffs. Open it from the tree browser's Folder
  history button or enable Folder in File & folder history. Individual file
  history still follows renames; folder renames are not inferred.
- Kept history reads cancellable and off the UI thread. Localhost MCP exposes
  the scope, retained selections, folder controls and read state. Diagnostic
  records identify directory-history operations without including paths or content.

- Finished the branch merge workflow: the sidebar and Branches context menus
  retain the selected branch, and the details card now offers Merge into current.
  A background preview shows exact current/incoming commits, ancestry counts and
  the selected merge mode before applying.
- Recheck checkout, index and selected target under the write gate; reject stale
  reviews, dirty tracked files, hidden index flags and existing index locks.
  Changing the target or mode requires another preview. No dependencies changed.
- Classify merge conflicts separately and open Changes with existing Continue /
  Abort recovery. Incomplete refreshes and cancelled writes remain visible as
  uncertain outcomes; cancellation never implies rollback.
- Detect cancellation after a merge commit exists but before Git clears its
  markers. Block duplicate Continue/Abort and offer reviewed Clear merge state,
  preserving the completed commit, index and working files.
- Added localhost MCP controls for merge reviews and Abort, branch row selection,
  and recovery status in Activity results. See `VALIDATION.md` for local evidence.

## 0.27.0 — 2026-10-20 (development build)

### Changed

- Redesigned window layout (see the [user guide](crates/rocket-ui/src/guide.md)). Repository
  tabs have their own row; the toolbar holds Git actions only, with behind/ahead
  counts on Pull and Push and Refresh beside the branch picker. Appearance, row
  density and workspace toggles moved to the View menu; MCP settings moved to a
  status-bar chip and **Window → MCP Server Settings…**. The toolbar no longer
  wraps to a second row in narrow windows.
- One sidebar without duplicates: Changes, Commits and Files, then collapsible
  Branches, Remotes and Stashes sections and a Repository section. Branch rows
  have a right-click menu (switch, merge, rename, delete).
- Changes lists Conflicts, Staged and Unstaged files with M/A/D/R/U/C badges and
  hover actions. The commit form has Summary and Description fields over the same
  saved draft, a 72-character counter, a primary Commit button and
  **Amend last commit…**.
- Commit details show message, metadata, changed files and diff together; narrow
  panes switch between Details and Files.
- Flat panes, hairline dividers, refreshed navy tokens and a single cyan accent.
  Menus are text-only, with shortcuts aligned on the right.
- Operation progress and Cancel appear in the status bar; a **Needs review** chip
  opens Activity. Terminal and Activity share one bottom drawer with tabs.
- Tags, Worktrees, Submodules and Git LFS are pages in the content area instead of
  floating windows: header actions, LFS tabs, a list and a details card. Every
  change opens a shared review dialog over the page (what will happen, risk note,
  red button for destructive actions). In a review, **Cancel**/Esc returns to the
  page; **Close** leaves the page. Navigation, shortcuts and Ctrl+K work while a
  page is browsing. Owner-bound safety, write completion, MCP `dialog.*` state and
  automation keys are unchanged; `dialog-cancel` in a review now returns to the page
  (also published as the review's back key).
- All other modal dialogs use the same review layout (icon badge, title and
  subtitle, a "what will happen" body with fields and callouts, and a footer with
  the decision on the right): Open, Clone, New repository, Repository settings,
  Fetch/Pull/Push, New branch, Rename, Merge, Add/Edit remote, Save stash, the
  confirmations, Close repository, Exit, Preferences, Keyboard shortcuts, About and
  Release notes, plus the amend/cherry-pick/revert, rebase, interactive rebase,
  reflog and recovery, file restore and patch dialogs. Destructive decisions —
  delete branch, remove remote, abort merge, drop stash, skip/abort a rebase,
  cherry-pick or revert, hard reset, discard — have a red badge and button;
  everything else uses the accent primary. The interactive rebase plan has one row
  per commit with its action, ID, subject and Up/Down. File backups/restore and the
  patch dialogs are now modal: Esc or a click outside closes them only when nothing
  is running. Behaviour, validation, owner checks, cancellation and automation keys
  are unchanged, and `dialog-cancel` still dismisses these dialogs; `dialog-scroll`
  (generic dialog body) is new. Keyboard shortcuts now lists Ctrl+K and Ctrl+N.
- Branches, Remotes, Stashes, Repository files (files at a commit), File history
  and the conflict editor use the same full-bleed layout: page headers with the
  primary action, list and details card (Branches, Remotes) or a list pane beside
  the viewer, path strips with metadata, segmented switches and line-number
  gutters. Branches and Remotes select a row and act on it from the card; remote
  URLs stay behind a per-remote disclosure. The conflict editor shows the conflict
  count and unsaved state, and **Both** is now labelled **Use both**. Reads,
  cancellation and existing automation keys are unchanged
  (see the [user guide](crates/rocket-ui/src/guide.md)).
- The no-repository screen has Open folder, Clone and New repository cards with
  their shortcuts, a drop target that highlights while a folder is dragged over
  the window, recent repositories with folder name, path and current branch, and
  Git discovery status with retry. The branches are read in the background with
  read-only Git queries while that screen is shown (the status bar says **Reading
  repository…** meanwhile, and MCP waits for them); a folder that can no longer be
  read shows **Unavailable**. Opening a repository or quitting cancels the reads.
- Activity drawer rows show a status mark, a one-line summary, details on demand
  and the duration.

### Added

- Command palette (**Ctrl+K**, the toolbar search field or **View → Command
  Palette…**) for Git actions, branch switching, views and layout toggles.
- **Hide labels** / **View → Show Sidebar Labels** turns the sidebar into an icon
  rail. Windows narrower than 980 points start as a rail.
- Branch details offer **Show history**, and right-clicking a branch on the
  Branches page opens the sidebar's branch menu (including merge).
- Automation keys `repository-clone-welcome`, `welcome-recent-N`,
  `welcome-git-retry`, `remote-row-N`, `remote-fetch-N` and `console-details-N`.
- Automation key `page-details-scroll`: the visible scroll area of a Tags,
  Worktrees, Submodules or Git LFS page's details card, so localhost MCP can scroll
  to card actions (Delete, Checkout…) when a short window clips the card.

### Fixed

- A review dialog shrinks again when its content does, for example after an error
  clears or when a shorter dialog follows a taller one. Its footer used to keep the
  earlier height, leaving an empty band above the buttons.
- The sidebar's Tags, Worktrees, Submodules and Git LFS entries no longer turn
  disabled while a background refresh runs; only a write or a foreground load
  disables them.
- Localhost MCP treats a management page that is only browsing like the UI does:
  menus, sidebar, toolbar and `rocket_open` stay available instead of failing with
  "Dismiss or complete the active dialog first". `rocket_type` accepts
  `commit-description`, so automation can write and clear a commit body again.

### Removed

- The always-empty Checks tab and the disabled View on GitHub button, until
  hosting integration exists. Automation key `detail-checks` no longer exists.

### Migration

- Saved settings use format version 3, which adds the sidebar-labels choice.
  Versions 1 and 2 are still read. Earlier Rocket builds cannot read a version 3
  settings file.

## 0.26.0 — 2026-10-19 (development build)

- Added **Git LFS → Locks** with verified server ownership, Unicode paths,
  reviewed lock/unlock and separate force-unlock confirmation. Normal unlock
  addresses the exact reviewed ID; stale locks, remote changes and external
  file edits require a new review. Read-only file permission changes are disclosed.
- Added cancellable background lock reads/writes and distinct not-started,
  helper-confirmed and unknown outcomes. Interrupted requests are not retried
  automatically. Local content and staging verification are separate from
  refreshing and confirming the server result. Post-check errors do not claim
  that no request was sent; a fresh read clears the previous dialog notice.
- Protected existing LFS lock caches with operation-owned temporary storage,
  and prevented server-returned unlock paths from changing local permissions.
  Malformed, inconsistent, excessive and unsupported responses produce errors;
  helper JSON success alone cannot imply an empty lock list.
- Added private diagnostic steps and remote outcomes to Activity & diagnostics,
  plus an isolated localhost MCP locking fixture. No dependencies were added
  or downloaded. See `VALIDATION.md` for executed checks and remaining limits.

## 0.25.0 — 2026-10-19 (development build)

- Added **Activity & diagnostics** with retained sessions, operation/error search,
  outcome filters, per-request steps, durations and structured summary copying.
  Reads run in cancellable workers; lists and timelines render visible rows only.
- Treat inner failures/cancellation separately from outer request completion and
  neutral Git probe exits. Show incomplete traces, retention gaps, dropped records,
  read errors and stale retained results explicitly. Existing Console behavior and
  raw-output opt-in remain separate.
- Added narrowly scoped localhost MCP LFS upload acceptance on the approved GitHub
  test repository: exact temporary commit/object markers, one object under 1 KiB,
  isolated authentication, independent download verification and guarded cleanup.
  See `VALIDATION.md` for actual runs and limits.
- The UI reuses the already pinned and reviewed `serde_json 1.0.151`. No external
  package version changed and no package was downloaded.

## 0.24.0 — 2026-10-19 (development build)

- Added reviewed LFS snapshot uploads: explicit push destination, pinned HEAD,
  unique object count and maximum bytes. Validate SHA-256/size and recheck local
  state before offering only those object IDs to the installed helper.
- Added numeric progress, owned-process cancellation, typed upload errors and
  private phase/outcome logs. Late failure/cancellation reports remote availability
  as unknown; uploaded objects are retained and Git refs are never published.
- Added native localhost MCP upload controls and isolated local acceptance tests.
  Network uploads through MCP remain disabled pending separate test approval.
  No dependency was added or upgraded.

## 0.23.0 — 2026-10-19 (development build)

- Added root `.gitattributes` tracking rules to Git LFS: review and append a
  pattern or literal path, or untrack one selected rule. Previews show exact
  removed/added lines and explain separate staging and existing-pointer limits.
- Preserve unrelated attributes, comments, line endings, staged/unstaged edits
  and file contents. Guard saves against external edits, retain recovery copies,
  support cancellation and report completed phases with private diagnostics.
- Exposed tracking controls/state through localhost MCP for isolated native
  acceptance tests. No dependencies were added or upgraded.

## 0.22.1 — 2026-10-19 (development build)

- Fixed LFS inspection rejecting the normal authentication-mode metadata written
  by Git LFS after GitHub authentication. Fetch verification permits documented
  mode changes only for the reviewed endpoint, plus the initial format marker.
  Other endpoint/storage overrides and unrelated configuration edits stay guarded.
- LFS previews explain these metadata writes. Localhost MCP can exercise the
  already approved `JasXTD/rocket-auth-test` HTTPS/SSH sources on marked temporary
  branches; other network sources remain refused before execution.
- Added isolated real-GitHub LFS acceptance tooling with get-only credential access,
  agent-only SSH, a shared 72-byte synthetic object and guarded branch cleanup.
  See `VALIDATION.md` for executed checks. No dependency was added or upgraded.

## 0.22.0 — 2026-10-19 (development build)

- Added a native Git LFS dialog for tracked pointer files, cache/working state,
  reviewed fetch of the pinned HEAD commit and single-file checkout.
- Checkout checks SHA-256/size, preserves staging, rejects changed reviews and
  retains pointer/recovery files before publishing content without replacement.
  Existing content and local edits are never automatically overwritten.
- LFS reads/writes support cancellation, owner-bound previews, refreshed results
  and typed diagnostic phases/errors. Localhost MCP restricts downloads to local
  fixture sources. No Rust dependency was added or upgraded.
- Development/test builds retain full Rocket symbols and reduce third-party debug
  information to line tables; assertions and incremental compilation stay enabled.
- The Repository menu scrolls in small windows; localhost MCP exposes only visible
  menu targets, including the new LFS entry.

## 0.21.0 — 2026-10-18 (development build)

- Added reviewed submodule add, remove and URL sync in the native Submodules dialog,
  including explicitly selected nested parents and Unicode paths.
- Add pins the source commit, prepares its checkout separately and stages only
  the new gitlink and `.gitmodules` entry using a private index. Existing destination
  files and unrelated staged/working changes are protected.
- Removal rejects dirty, untracked/ignored or hidden files and active operations;
  it stages the metadata removal and retains the entire checkout and local commits
  in recovery storage. Existing Git-file admin data stays in place.
- URL sync updates repository-local parent/child fetch URLs from `.gitmodules`,
  with old/new previews. It preserves file edits, staging and push URLs and performs
  no fetch. Ambiguous/included settings are refused.
- Recheck previews and metadata before publication, keep recovery copies, and report
  completed steps and partial/cancelled results. Local MCP tests constrain resolved
  add sources to the fixture root before source access, including URL rewrites.
- Fixed Changes refresh for untracked embedded repositories and ignored directories:
  Git's trailing directory marker is normalized while unsafe paths stay rejected.
- No dependency added or upgraded; settings need no migration.

## 0.20.0 — 2026-10-18 (development build)

- Added Submodules in the sidebar and Repository menu: nested list, source and
  recorded/current commit inspection, opening initialized repositories, and
  reviewed initialization/update of one selected checkout.
- Updates use the parent index's gitlink commit, with detached checkout and a
  recovery reference for the previous tip. Parent staging and commits are not changed.
- Refuse dirty, untracked/ignored files, hidden index flags, active operations,
  occupied destinations, redirected layouts and stale reviews. An initialized
  nested checkout cannot be removed by updating its parent.
- Prepare new submodule checkouts separately, then publish without replacing a
  destination. Cancellation stops owned Git processes; interrupted preparations
  are retained and observed results are reported without automatic rollback.
- Added owner-bound background reads, responsive cancellation, typed diagnostic
  failures and authenticated localhost MCP controls restricted to local fixtures.
- No third-party dependency added or upgraded. Existing settings need no migration.

## 0.19.0 — 2026-10-18 (development build)

- Added local external editor, diff and merge settings with separate executable
  paths and one-argument-per-line templates. Launches use no shell expansion and
  never select executables from repository configuration.
- Added external comparisons for selected staged/unstaged/untracked files and
  committed changes, using bounded read-only copies of exact Git versions.
  The editor action explicitly opens the actual working file.
- Added merge-tool result review through the native conflict editor. Temporary
  results do not overwrite working files or stage automatically. Import checks
  the original file, Git context and Rocket draft; confirmed saves retain the
  existing recovery and external-edit protections.
- Added owned-process-tree cancellation, exit/error outcomes and private diagnostic
  spans. Temporary workspaces remain available for recovery after cancellation,
  failure or closing a repository/window.
- Added localhost MCP controls/state and restricted automated executable launches
  to the configured fixture root. Existing settings migrate to state format 2,
  preserving drafts, preferences and recent repositories. Older builds reject
  format 2 rather than silently dropping external-tool settings.
- Kept compact diff actions in the existing file menu so the terminal layout
  still has room for usable diff rows. Completed import failures clear progress
  messages; ending an editor session explicitly preserves saves already made.
  No external dependencies were added.

## 0.18.0 — 2026-10-18 (development build)

- Added optional word-level highlights to unified and split text diffs in Changes
  and Commits → Files Changed. Unicode word boundaries retain emoji/combining
  sequences; bounded matching falls back explicitly to normal line highlights.
  Search and syntax colors coexist, and staging/discard still use original bytes.
- Added lazy PNG/JPEG/BMP comparisons for staged, unstaged, untracked and committed
  images, including renames, additions and deletions. Previews show dimensions,
  encoded sizes, transparency, Fit/100%/zoom controls and JPEG EXIF orientation.
- Image reads and decoding use background workers, compressed/pixel/memory limits,
  cancellable reads and stale-result rejection. No filters, checkout or automatic
  object downloads are needed. Invalid/large files have explicit error messages.
- Added localhost MCP image controls and diagnostic read/decode/highlight spans.
  Compact text/image previews keep controls and changed lines above the terminal.
  Commit image Reload refreshes sources and retains the selected file.
- Enabled audited JPEG support through already-locked `zune-core` 0.5.3 and
  `zune-jpeg` 0.5.15; `image` and Unicode segmentation are now direct UI dependencies.
  No external package versions/checksums changed, and no packages, fonts or
  runtimes were downloaded. The Git backend remains dependency-free.
  See DEPENDENCIES.md and VALIDATION.md.

## 0.17.0 — 2026-10-18 (development build)

- Added reviewed patch import and export under Repository. Export unstaged,
  staged or single-commit diffs to a new file; an existing destination is never
  replaced. Commit export contains a diff, not author metadata or a new commit.
- Imported text patches are parsed, checked and applied in a disposable Git
  repository first. Real working files use guarded publication with recovery
  copies, unchanged staging, source checks and revalidated external state.
- Added **Discard selected…** for changed lines or whole hunks in unstaged
  unified/split diffs. Selections retain original patch identities; unselected
  edits and staged contents remain intact.
- Cancellable workers, bounded before/after previews, compact dialogs and
  localhost MCP controls cover import/export and selective discard. Interrupted
  batches report changed files and retained recovery copies; there is no unsafe
  automatic rollback.
- Added fixed diagnostic phases, typed errors, conflicting-path guidance and
  aggregate completion/partial-result counts without recording patch contents
  or paths in diagnostic logs.
- Fixed read-only Git diffs refreshing the index stat cache despite optional
  locking being disabled. Read commands now explicitly disable that refresh.
- Short windows cap terminal height to leave selectable diff rows visible.
  Stale-preview guidance accounts for files already changed in a partial batch.
- Side-by-side code previews preserve indentation; export identifies its combined
  patch without presenting inactive file-selection controls.
- No new dependencies. Initial patch scope is regular UTF-8 text with existing
  parent folders; see UI.md for size, format and platform limits.

## 0.16.0 — 2026-10-18 (development build)

- Added whole-file **Discard unstaged**, **Restore from commit**, repository-tree
  **Restore this file**, and **Repository → File backups / restore**. Reviewed
  source contents are pinned; only the working file changes, never the index.
- File replacements/removals retain recovery copies. Recovering a copy reviews
  the current file and retains another backup. Deleted and untracked files,
  binary bytes and Unicode paths are supported within explicit limits.
- Revalidate working bytes, index entry/flags, HEAD and conversion settings.
  Refuse stale reviews, active operations, special paths, links/junctions,
  submodules and unsupported filters/encodings. Failed/interrupted publication
  keeps recovery artifacts and reports verification separately.
- Added cancellable literal search in the main diff viewer, match navigation and
  basic lexical syntax colors for common source/config files. Work stays on
  background tasks; rendering keeps virtualized rows and original staging IDs.
- Compact Changes panes group file actions in a menu and use shorter diff controls
  so the terminal leaves usable review space. Dense syntax/search overlays use an
  ordered span walk rather than repeated scans.
- Added fixed diagnostic phases/error codes and native localhost MCP coverage.
  No new third-party dependency, remote operation or automatic backup cleanup.

## 0.15.0 — 2026-10-18 (development build)

- Added **Repository Files**, **Browse files** on the commit list and
  **Repository → Browse files at commit**. Browse a commit, branch or tag with
  lazy folder reads, root/up navigation and a pinned commit ID.
- Preview committed UTF-8 files, including Unicode names and contents, without
  checking out files. Symlinks show target text only; submodules show their
  recorded commit. Binary, oversized and locally missing objects have clear states.
- Reads run on cancellable workers. New selections cancel superseded reads;
  refresh and external branch movement leave the viewed snapshot pinned.
  Narrow windows switch between folders and contents, including with the terminal open.
- Added bounded localhost MCP state and controls, private diagnostic request/step
  records, and temporary-repository tests. No new dependencies or network reads.

## 0.14.0 — 2026-10-18 (development build)

- Added **Worktrees** in the sidebar and **Repository → Worktrees**: list paths,
  branches, commit IDs, main/linked status and locked/missing registrations.
  Open a selected worktree in a tab, or switch to its existing tab.
- Reviewed creation supports a new branch at a pinned commit or an available
  existing local branch. Preparation happens in a sibling folder; publication
  never replaces an existing destination. Interrupted preparation is retained.
- Reviewed removal leaves branches intact and retains the committed tip at
  `refs/rocket/removed-worktrees/<commit-id>`. Main/current/open tabs, locked or
  missing worktrees, dirty or ignored files, active operations, submodules and
  sparse/hidden index flags are refused. There is no force-removal option.
- Added background reads, cancellation reconciliation, fixed worktree error codes,
  named diagnostic phases and guarded localhost MCP support for linked fixtures.
  No new third-party dependencies or remote operations.
- Partial Git cleanup reports remaining administrative metadata separately from
  the working folder. The installed Git for Windows reproduces this case with
  reftable worktrees; Rocket preserves the error and does not force cleanup.

## 0.13.0 — 2026-10-18 (development build)

- Added **Tags** in the sidebar and **Repository → Tags**: browse local tags,
  inspect their exact objects and targets, and read annotation metadata/messages.
  Tags pointing to the same commit remain separate selections.
- Create lightweight or explicitly unsigned annotated tags after a preview. The
  commit, tagger identity/time and annotation are pinned; editing invalidates the
  preview. Existing tags are never overwritten and no editor or signer is opened.
- Reviewed deletion compares the exact tag object and retains it under
  `refs/rocket/deleted-tags/<object-id>` in the same checked ref transaction.
  External replacements, conflicting recovery refs and existing locks are refused.
- Added cancellable background reads, fresh observation after attempted writes,
  clear stale/invalid/locked errors, named diagnostic steps and localhost MCP
  controls/state. Tag names, identities, annotations and raw output stay out of logs.
- No remote-tag operations, signing/verification UI or new third-party packages.

## 0.12.0 — 2026-10-18 (development build)

- Added **Repository → Reflog & Recovery**: browse the current worktree's HEAD
  reflog, load a larger window of older entries, and recover a selected commit on
  a new local branch without switching or overwriting an existing branch.
- Added reviewed soft, mixed and clean-worktree hard reset. Previews pin the full
  commit ID, show branch/index/file effects and a committed-tree comparison, and
  retain the original tip at `refs/rocket/recovery/reset/<original-id>`.
- Soft reset keeps staging and files. Mixed reset requires no staged changes;
  hard reset also requires clean tracked files and refuses hidden/sparse index
  flags, submodules and untracked/ignored collisions. Commit or stash edits first.
- Revalidate previews under the write gate, preserve Git locks, and reconcile
  cancellation and failure independently from the observed result. Clear guidance
  includes the observed HEAD and the committed-history recovery reference.
- Added localhost MCP controls/state for reflog selection and recovery reviews,
  with named diagnostic phases and fixed error codes. No new third-party packages.

## 0.11.0 — 2026-10-17 (development build)

### Added

- **Repository → Interactive Rebase** with oldest-first reorder controls, pick,
  reword, squash, fixup and explicit drop. Replacement/combined messages are
  editable, and a separate final review shows the exact plan before execution.
- Linear plans retain every original commit exactly once, with up to 200 entries.
  Leading squash/fixup and blank/oversized replacement messages are refused.
  Choosing an ancestor as the base supports rewriting commits on the current branch.
- Saved plan journals support conflict recovery after reopening Rocket. Continue,
  Skip and Abort recheck the original branch, recovery reference and reviewed
  sequence. Changed/missing journals and a skip that would fold into the destination
  produce specific errors. Cancellation reconciles actual Git state.
- A rejected reword hook can be retried with Continue even with a clean index.
  Recovery distinguishes an unsaved message from an amendment that completed
  before cancellation; it never silently skips or repeats that amendment.
- Bundled first-party `rocket-rebase-editor` passes only validated todo instructions
  and reviewed message data to Git. Include it beside Rocket and Rocket MCP.
  No third-party dependencies added or upgraded; hooks and signing remain enabled.
- Native localhost MCP plan controls and diagnostic action counts, named steps,
  fixed failure codes and payload-free logs. Merge-preserving/root rebases, arbitrary
  todo commands and force pushing are outside this release.

## 0.10.0 — 2026-10-17 (development build)

### Added

- Reviewed local rebase onto a branch or commit, with a pinned destination,
  oldest-first replay list, fast-forward/no-change previews and original-tip
  recovery references. Linear history is supported up to 200 commits.
- Continue, Skip and Abort previews for supported rebases; staged resolutions,
  discarded tracked edits, external changes and empty results have explicit states.
  The conflict editor works while Git temporarily detaches HEAD during a rebase.
- Branch, destination, index, changed files and rebase metadata are revalidated
  before execution. Untracked/ignored collisions, index flags that hide edits and
  externally reordered/replaced replay picks are refused. Cancellation is
  followed by independent status, operation and branch refreshes.
- Late interruptions that leave an updated branch and unfinished rebase metadata
  have a separate recovery error and Git guidance; Rocket does not reset that branch.
- Native localhost MCP rebase controls with bounded previews, restricted fixture
  recovery scope, named diagnostic steps and fixed failure codes. No dependencies
  added or upgraded. Interactive rewriting, merge-preserving rebases and taking
  over arbitrary external rebase sequences are outside this release.

## 0.9.0 — 2026-10-16 (development build)

### Added

- Native conflict editor with Base/Ours/Theirs previews, an editable result,
  per-block Ours/Theirs/Both choices and conflict navigation. Wide layouts show
  both sides above the result; compact layouts use source tabs with the terminal open.
- Separate reviewed Save file and Save and stage actions. Exact working bytes,
  unmerged index entries, HEAD and operation state are rechecked before writing.
  Stale or cancelled saves retain the draft; reload requires confirmation.
- Windows replacement keeps original/result recovery copies in the Git directory.
  A saved file, confirmed staging and later failures/cancellation are reported
  separately. Saving never commits or continues a Git operation.
- UTF-8 Unicode paths/content, LF/CRLF and BOM preservation; bounded text reads,
  diff3/custom-width marker handling and explicit unsupported-file errors.
- Localhost MCP controls and payload-free diagnostics for conflict reads,
  revalidation, file replacement, staging and cancellation. No dependencies added
  or upgraded. Conflict drafts live in the window; close/exit warns before discard.

## 0.8.0 — 2026-10-16 (development build)

### Added

- Stash inspection shows all tracked changes, the saved index and untracked files
  separately, including Unicode paths and binary-file markers. Reads run on workers,
  can be cancelled, and stay pinned to the selected object when the list changes.
- Save shows the current changed-path counts. Apply can restore the staging split
  and always retains the stash. Drop confirms the exact observed entry and rejects
  a changed list. Conflicted apply explains recovery in Changes without offering
  an unsupported stash abort command.
- Localhost MCP exposes stash controls and fixed pending-input resize probes.

### Fixed

- Terminal resizing reflows wrapped lines and complete Unicode clusters instead
  of truncating them, and answers ConPTY cursor-position queries.
- The owned PowerShell prompt places the full location above the editable line,
  avoiding Windows PSReadLine 2.0's lost input column when widening a long prompt.
  Resize never injects shell keystrokes. Existing sessions use the new prompt after
  End session / New Terminal; normal history remains unchanged.
- No dependencies or bundled runtimes added or upgraded.
- Diff filenames containing spaces and Unicode decode Git's trailing header tab
  correctly; literal escaped tabs in filenames remain visible.

## 0.7.1 — 2026-10-16 (development build)

### Fixed

- Windows PowerShell no longer inherits PowerShell 7's incompatible module search
  paths through Rocket or MCP. PSReadLine and standard commands load correctly;
  execution policies, user profiles and the parent process environment stay intact.
- Shift+Tab sends reverse completion to the focused terminal screen.
- Enter immediately after clicking the terminal stays with the shell instead of
  being consumed by the separate command field's stale focus state.
- Localhost MCP adds fixed keyboard probes for typing/editing, history and
  completion. Test terminals use an owned history file instead of personal history.
  Normal terminals retain PowerShell's default history behavior.
- MCP keyboard frames work while its fixture window is in the background without
  changing desktop focus; missing widget focus is reported before key delivery.
- No dependencies added or upgraded.

## 0.7.0 — 2026-10-16 (development build)

### Added

- File History follows a single path across renames, including Unicode names,
  with paged commit history, committed contents, a first-parent diff and blame.
- Blame shows the original commit, author, path and line number. Inspect its
  history and return with Back. Background refresh keeps the inspected revision
  pinned; Load HEAD explicitly starts a new read.
- Cancellable background reads, bounded previews, virtualized rows and a compact
  commit-list/inspector toggle when space is limited. Binary, empty, deleted and
  oversized files have explicit states.
- Localhost MCP exposes the native controls and bounded fixture state for testing.
  No dependencies added or upgraded.

## 0.6.0 — 2026-10-16 (development build)

### Added

- Amend the latest commit, cherry-pick a selected commit and revert a selected
  commit through repository-bound previews. Merge commits require an explicit
  mainline parent for cherry-pick/revert.
- Review and Continue, Skip or Abort a standalone cherry-pick/revert after a
  conflict or empty result. Externally started multi-commit sequences/rebases
  remain managed by Git.
- Amend retains the original commit in a recovery reference. Failed amendments
  keep their replacement message in the dialog and preserve the regular draft.
- Stale preview checks, local-file collision protection, bounded noninteractive
  execution and independently refreshed operation state. Diagnostics identify
  preflight, write and recovery outcomes without storing commit messages or patches.
- Native localhost MCP can inspect the new operation state and interact with the
  preview dialogs. No dependencies were added or upgraded.

## 0.5.3 — 2026-10-16 (development build)

### Fixed

- Short Changes panes put file groups and the commit form in one scrolling
  column, keeping them clear of the terminal. Larger panes retain a pinned form.
- Long commit drafts scroll inside a bounded editor instead of expanding beyond
  the panel. The same draft and editor identity survive compact/large transitions.
- Localhost MCP publishes only visible Changes control bounds and can send
  bounded mouse-wheel input to its list/form and message viewports. Scrolled-off
  controls cannot be clicked through neighboring panels.
- No dependencies were added or upgraded.

## 0.5.2 — 2026-10-16 (development build)

### Fixed

- Terminal CJK, emoji and combining accents now occupy display columns rather
  than one cell per Unicode character. Joined emoji, variation selectors and
  split UTF-8 input retain complete clusters; wrapping, erase, insertion, deletion
  and resize preserve wide-cell boundaries.
- Text, backgrounds and the cursor share a measured terminal grid. Installed
  Segoe UI Emoji takes priority for wide terminal clusters, and scroll row spacing
  no longer clips the first output rows.
- The owned PowerShell session starts with UTF-8 input/output encodings. These
  changes apply to that session only; shell profiles and machine settings stay intact.
- Localhost MCP exposes bounded terminal cell positions and cursor state, with
  exact Unicode probes for native alignment and margin-wrap checks.

### Compatibility

- Windows builds bundle the reviewed Microsoft ConPTY 1.25.260930003 runtime
  to provide grapheme-aware cursor positions on Windows 10. Distribute the
  `conpty/1.25.260930003` directory beside the executable; Cargo copies it during
  builds. No .NET runtime or Windows installation is needed.
- Added exactly pinned `unicode-width` 0.2.2 and reused the existing locked
  `unicode-segmentation` 1.13.3. Audit details and native package signatures/hashes
  are recorded in `DEPENDENCIES.md` and `runtime/conpty/1.25.260930003`.
- Glyph artwork still depends on installed fonts. Ambiguous-width characters use
  one column; individual clusters are bounded to 128 UTF-8 bytes. Full-screen
  terminal application support remains limited as described in `UI.md`.

## 0.5.1 — 2026-10-16 (development build)

### Fixed

- Windows CJK fallback fonts now cover labels, inputs, headings and monospace
  diffs. Installed Chinese, Japanese, Korean, symbol and emoji fonts supplement
  the existing typefaces; font bytes are cached across window reopenings.
- Unicode diff paths decode Git's quoted byte escapes for display. Original
  path/patch bytes remain unchanged for staging and other Git operations.
- Bounded text previews stop at complete UTF-8 characters. Diff horizontal
  scrolling uses measured text width, including wider fallback glyphs. Split-diff
  column labels follow the actual scrolling columns.
- Localhost MCP accepts longer file control IDs, including UTF-8 paths encoded
  by the UI. Requests remain bounded and fixture ownership checks still apply.
- No external dependencies or font downloads were added. Coverage depends on
  installed Windows fonts; advanced terminal cell-width handling is unchanged.

## 0.5.0 — 2026-10-16 (development build)

### Added

- File → New Repository (Ctrl+N) initializes a new or existing project folder,
  opens its unborn branch, and preserves files without staging them. Existing/nested
  repositories, redirected folders and publication collisions are rejected.
- Repository → Repository Settings edits local name/email together. Blank fields
  remove local overrides. Effective Git user settings are shown; global settings,
  included files, signing configuration and working files stay unchanged.
- Loading, creation and saves run on workers. Stale edits, invalid inputs, config
  locks and preparation/publication failures have explicit errors. Diagnostics
  record operation phases without identity values or file contents.
- Localhost MCP controls for both dialogs and unborn fixture branches.

### Improved

- Refresh now sits beside the current-branch selector instead of the main toolbar.
  The branch control uses one row; the repository name remains above it.
- MCP opens from the main toolbar beside Workspace, with a selected state and a
  return action that preserves the repository view and draft. Repository panels
  stay out of the settings view; existing Console and terminal sessions are retained.
- Connection and token settings share a clearer responsive layout. The toolbar
  keeps all controls available at the minimum window size, and uses an icon for
  the app brand. Routine messages use the status bar instead of a full-width banner.
- History gives more space to search and displays topological order as a label.
  Remotes separates connection information from edit actions and explains push
  URL fallback. Dialogs name the operation being confirmed.
- New Repository is available from the welcome screen and Open dropdown. Creation
  and local identity forms explain the scope and preservation of existing settings.

### Compatibility

- Creation requires Windows and an existing parent folder. New repositories use
  SHA-1 and an explicit initial branch (default `main`). Configured templates are
  not copied. Failed preparation workspaces are retained for inspection.
- Local identity settings are shared by linked worktrees. Worktree/include and
  author/committer overrides can take precedence; they are not rewritten.
- Corrected outdated README boundaries for remote editing and all-branch history.
- No external dependencies were added or upgraded.

## 0.4.0 — 2026-10-16 (development build)

### Added

- Local HTTP control endpoint, default `http://127.0.0.1:47631/mcp`, with a
  required random access token, loopback binding and Host/Origin validation.
- Application-wide **MCP** settings: port, fixture folder, connection status,
  start/stop, copy endpoint/token, and confirmed token rotation. File and service
  work runs in the background. Changes are saved independently of repository tabs.
- Clear errors for occupied ports, invalid settings, token mismatch and attempts
  to stop the service while its test window remains open.

### Fixed

- Closing the owned test window no longer requires restarting the MCP connection.
  `rocket_open` can reopen it in the same process; stale tab IDs are rejected.
- Client disconnects no longer terminate the HTTP service. A fresh client can
  initialize and inspect the existing window without replaying operations.
- Windows unused-port timeouts no longer make a stopped server appear broken.
  Service ownership and port availability are checked before reporting it stopped.
- Accepted HTTP connections now wait for request bytes on Windows, preventing
  intermittent resets when a client sends its headers after connecting.

### Compatibility

- Use direct localhost HTTP instead of the former Codex `rocket-ui` registration.
  Stdio remains available for isolated regression runners. No new external crates.
- One controller and one owned fixture window per service. Start/stop is manual;
  existing fixture restrictions and Git operation confirmations still apply.
- Keep `rocket.exe` and `rocket-mcp.exe` from the same build in the same folder.

## 0.3.0 — 2026-10-16 (development build)

### Added

- Add and remove remotes from the Remotes view. Removal confirms the deletion of
  local remote-tracking branches; it does not delete anything on the server.
- Edit a remote's fetch or push URL, or restore push's fetch-URL fallback.
  These operations save local configuration without connecting to a server.
- All branches history combines local and remote-tracking branches plus HEAD,
  with shared commits deduplicated and pagination pinned to the observed tips.
- Background history search across messages, authors, emails and commit ID
  prefixes, including commits beyond the loaded page. Search has progress,
  cancellation, resumption and explicit incomplete/error states.

### Fixed

- Deferred refreshes no longer keep a modal dialog falsely busy; pending changes
  are refreshed after dismissal.
- Automatic refresh compares every starting history tip, so a change on another
  branch is detected even when the first displayed commit remains the same.
- Late pages and search results from a previous traversal cannot replace or append
  to the current history after its scope, query or branch tips change.

### Compatibility

- Remote editing preserves other settings and refuses stale observations, inherited
  remote settings and multiple URLs in the selected direction. Use Git for those
  advanced configurations. Remote names are set when adding; renaming is not yet
  exposed. Git's normal URL rewrites and transport settings still apply when fetching.
- All branches refers to branches available locally, including fetched tracking
  branches; it does not query servers or include tag-only/stash-only histories.
- Search uses literal case-insensitive text matching, not regular expressions.
  It scans bounded batches and pauses after a page of matches; Load more continues.
  Deep scans currently retraverse skipped history and can take longer on very large
  repositories. Failures and stopped scans are never presented as complete results.
- No external dependencies were added or upgraded.

## 0.2.0 — 2026-10-16 (development build)

### Added

- Automatic local repository refresh using Windows directory notifications.
- Saved commit drafts, recent repositories and display/refresh preferences.
- Atomic state replacement, single-writer ownership and visible persistence errors.
- A shared application version and this changelog.
- Help → About shows the build version; Help → View Release Notes opens this changelog.
- Bounded untracked-file previews, unified/side-by-side diffs, and selection of
  individual changed lines or hunks for staging and unstaging.
- Conflict base/ours/theirs/working-file previews and a Mark resolved action that
  rejects remaining standard conflict-marker blocks.
- An interactive PowerShell terminal per repository, with persistent shell state,
  direct keyboard input, a command field, Ctrl+C, resizing and bounded scrollback.
- Terminal lifecycle and partial-review operation names in structured logs;
  commands, terminal output and file contents are excluded.

### Improved

- External changes refresh the selected working diff while preserving drafts and
  commit selections. Background work leaves the displayed repository usable.
- Failed state saves keep the application open instead of silently losing a draft.
- Full refresh batches repository discovery, reuses the observed HEAD and reads
  independent sections concurrently. It still rereads all sections and observes
  changes that arrive during the refresh.
- Diff row layouts are prepared once on the worker instead of rescanning the patch
  on every frame. Selected patches are checked for staleness before index writes.

### Compatibility

- State is stored in `%LOCALAPPDATA%\Rocket\state`. Reopen a repository to restore
  its draft. Open tabs and panel sizes are not restored.
- MCP test state stays beside the initial owned fixture, separate from normal
  application state.
- Partial selection currently supports regular text modifications with unchanged
  file mode and a final newline. New/deleted files, renames, binary data, links,
  submodules and oversized patches require whole-file staging or Git.
- Conflict editing happens in an editor or terminal; Rocket previews the sides and
  stages the reviewed working file. This is not a three-way merge editor.
- Terminal sessions use Windows PowerShell with no profile, require Windows 10
  1809 or newer, and end with their repository tab or app. Commands and sessions
  are not restored. Advanced full-screen terminal applications, wide-character
  cell layouts and extended-color rendering are not yet supported.

## 0.1.0 — Development baseline

- Rust Git-helper backend: repository reads, staging, unstaging, commits, branches,
  remotes, fetch, fast-forward pull, explicit branch push, clone, stash and merge.
- Native navy desktop UI with repository tabs, commit graph, changes, unified
  patches, operation progress, cancellation and a diagnostic console.
- Local MCP controls for actual native widgets, state, captures and bounded logs.
- Structured diagnostic logging with raw private helper output excluded.
- One Git process per diff selection, with stale-result cancellation.

This baseline records existing functionality, not a claim of production readiness
or feature parity with other Git clients.
