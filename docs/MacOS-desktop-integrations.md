# macOS updates, spelling and menus

Implementation is for Apple Silicon/macOS 15+. Native validation results are
recorded below after the focused batch. Developer ID/notarization acceptance
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
accelerators; mouse-only bindings do not become fake keyboard shortcuts.
Edit targets the current text field. Quit uses Rocket's normal shutdown handling.
More Commands opens the Command Palette for the additional repository workflows.
Rows update in place so a repository refresh does not replace an open menu. If
native initialization fails, the existing in-window menu remains available.

## Validation

Pending the combined focused Windows/Linux/native Mac batch. No Developer ID,
notarization, physical keyboard/IME or bare-metal acceptance is claimed here.

Sources: [Apple NSSpellChecker](https://developer.apple.com/documentation/appkit/nsspellchecker),
[Apple signing requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements),
[Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
