# macOS updates, spelling and menus

Implementation is for Apple Silicon/macOS 15+. Native validation results are
recorded below. Developer ID/notarization acceptance
still requires the owner's future signing identity and a real release package.

## In-app updates and recovery

Startup checks and Help → Check for Updates use the existing public GitHub
release metadata, bounded download, SHA-256 verification and cancellation flow.
The Mac update asset is `Rocket-VERSION-macos-arm64.tar.gz`. It contains a
`Rocket-VERSION-macos-arm64` folder with exactly `Rocket.app` and
`rocket-update.json`. The metadata is outside the signed app, avoiding a
signature/manifest hash cycle. The portable development ZIP remains separate.

The existing bounded ustar parser extracts only regular files/directories:
links, devices, traversal, duplicate names and oversized input are refused.
Developer ID identity is anchored in the running Rocket signature. The incoming
app must have the same Apple-issued Team ID, Rocket identifier, reviewed version,
valid sealed resources and notarization. Ad-hoc builds refuse public installation;
there is no production environment variable or command-line trust bypass.

Prepare the final asset after signing and stapling:

```sh
node tools/macos/update-package.cjs /path/to/Rocket.app /path/to/new-output TEAM_ID
```

The packager validates the original and copied app, including the stapled ticket,
and emits a checksum. It does not sign, notarize, publish or alter the input app.
The owner's future distribution workflow must invoke it on the final signed app.

Installation requires a writable local installation parent. A protected system
folder or filesystem without exclusive rename support is refused; Rocket never
asks for administrator access or falls back to overwriting existing paths.
The helper observes its actual parent's exit with kqueue, then rechecks inventory,
signatures and approval before moving anything. Cancellation before the first
move leaves the installed app untouched. Once a move starts it finishes the
transaction or recovery. The entire previous app is retained beside the new app.
Replacement or process-launch failure restores the previous directory; failed
recovery retains both sets of files and a `recovery_required` receipt. Externally
modified old apps are preserved but not automatically restarted. This is recovery
for installation/launch failure, not automatic detection of later application bugs.

## System spelling

Preferences → Commit enables macOS spelling (off by default). Rocket uses
`NSSpellChecker`, converting UTF-16 ranges to valid UTF-8 text boundaries.
Suggestions, session Ignore and explicit Add to Dictionary use the existing UI.
Added words belong to the user's shared macOS dictionary. No dictionary changes
occur simply by enabling the option. Startup, results and shutdown use Rocket's
bounded worker queue; Cocoa calls execute on its main queue. Discarded queued
checks are cancelled, and stale results cannot underline newer text. An OS call
already in progress completes normally; it is never killed mid-call.

## Native menus

The Mac system menu contains Rocket, File, Edit, View, Repository, Terminal,
Window and Help. Common commands reuse Rocket's keyboard-command dispatch and
recheck availability when selected. User keyboard bindings appear as native
accelerators for letters, digits, punctuation, function and navigation keys.
Mouse-only bindings do not become fake keyboard shortcuts.
Edit targets the current text field. Quit uses Rocket's normal shutdown handling.
Repository submenus expose reviewed commit, merge/rebase, stash, patch and recovery
workflows, with settings, tags, worktrees, submodules, LFS, hosting and Lua playground
entries. More Commands retains the Command Palette for other actions.
Rows update in place so a repository refresh does not replace an open menu. If
native initialization fails, the existing in-window menu remains available.

The development MCP host reports actual installed rows in `native_menus`.
`rocket_menu` invokes Cocoa's enabled item and queues its normal action; it does
not invent screen rectangles or bypass dialog/backend validation. The automation
allowlist permits only reviewed navigation/dialog opening and terminal interruption,
requires the current marked fixture, and rejects stale tabs and raw native IDs.

## Complete package acceptance

The manual Lua-helper workflow's `complete-package` scope builds the fixed guest,
signs the isolated helper, binds its exact hash into freshly compiled app/MCP hosts,
then packages all five native executables. After a ZIP round trip, the tests launch
that extracted `Rocket.app/Contents/MacOS/rocket-mcp` through authenticated localhost
MCP. The bundle is verified again after testing. Results and screenshots contain
synthetic fixture data; profiles, access tokens and repository contents are excluded
from artifacts. The complete-package jobs are capped at 5/10 minutes; other Mac
Lua test scopes retain their 15-minute Mac cap.

The combined acceptance covers native actions and compact reviews, updates through
a credential-free loopback API, Cocoa spelling on commit text, the bound Lua consumer,
and zsh typing/history/completion/resize/Control+C. The local test runner opts into
`/bin/zsh -dfi` with a fixed marker; MCP cannot choose an executable. Personal startup
files are disabled and history stays in the fixture. Normal shell settings and user
history are unchanged. Production update signature requirements remain enforced.

The hosted disposable Mac user uses an isolated temporary Keychain for encrypted
Lua source save/reload, then restores its original default and search list. This
setup refuses to run in a personal desktop account. App state, Git configuration,
zsh startup files and history are separately scoped to the synthetic fixture.

This combined run is pending until its results are recorded below; the prior native
unit/adapter results do not establish full-package MCP acceptance.

The security follow-up runs before that acceptance: Unix MCP marker reads refuse
symlinked leaf files at open time and validate the opened regular file and size.
Private-state opt-in and fixture ownership still require exact marker contents.
Git tests use canonical native Unix temporary roots by default, including when
WSL builds Windows-hosted source; `ROCKET_TEST_REPOSITORIES` remains an explicit
test override. Production private-directory checks are unchanged. The two new
regressions failed before these fixes and passed in WSL afterward, together with
the affected policy, code-search and image-read cases.

## Validation

The [combined native job](https://github.com/JasXTD/Rocket/actions/runs/42579693442)
passed on implementation commit `bccf851` on 2026-11-09:

- Six actual ad-hoc signed app transactions: install/restart, cancel while the
  parent stays open, missing approval, changed prepared files, changed installed
  files and launch failure with rollback. Inventories and restarted executable
  versions were checked independently. The test-only signing policy is absent
  from production builds; production rejects ad-hoc hosts.
- Four loopback HTTP scenarios: download/extraction, checksum rejection, server
  rejection and cancellation. The cancellation case stopped in about 21 ms.
- Real Cocoa spelling: Unicode ranges, suggestions, session Ignore, discarded
  checks, shutdown/restart and unavailable language. No persistent dictionary
  changes were made.
- Real Cocoa menus: eight groups, Unicode, modifiers, checked/disabled rows,
  single action dispatch and in-place state refresh; six shortcut/input tests.
- Eleven update/backend tests, seven spelling/queue tests, two UI installer
  entries (one is the child harness invoked by the six transaction scenarios),
  and the exact package-inventory test.

Windows passed 29 affected tests plus eight real installer transactions. Linux
passed 38 affected tests and built the app with its existing WSL cache; the
separate Node installer driver could not start because Node is absent in WSL.
One existing Linux package-archive test requires a supplied release archive and
was not run. No extra software or dependencies were installed on the workstation.

The job took 78 seconds. The conservative cumulative task compute estimate was
$4.402 against the user's $8 included-allowance cap, not a GitHub invoice.
No Developer ID/notarized distribution acceptance, visual/physical keyboard/IME
or bare-metal acceptance is claimed. No release package was published by this run.

Sources: [Apple NSSpellChecker](https://developer.apple.com/documentation/appkit/nsspellchecker),
[Apple signing requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements),
[Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
