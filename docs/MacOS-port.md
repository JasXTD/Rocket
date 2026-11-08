# Apple Silicon macOS preparation

Status: steps 1–5 and the native watcher/emoji follow-up passed, including the
development app bundle. Bare-metal acceptance remains pending; no supported
macOS release is published by this workflow.

## Native result (2026-11-08)

[Run 42495686349](https://github.com/JasXTD/Rocket/actions/runs/42495686349) passed
on source commit `261105a14bd7e86a2802c35f7e186cfa3a0415af`, with no local changes.
The runner used macOS **15.8.2 (24H232)**, the **15.5 SDK**, Rust **1.99.1** and
Git **2.56.0**, all native arm64.

- All-target compilation, watcher SDK ABI checks and **24 affected Rust tests**
  passed: callback invalidation, fonts/licenses, real recursive notifications,
  linked worktrees, root replacement, selective refresh and repeated edits.
- **20 localhost MCP checks** passed: notifications (6), Unicode terminal (7)
  and desktop controls (7). They cover compact windows, external stage/commit,
  repeated selected-diff updates, disable/re-enable, close/reopen, cancellation,
  local-change preservation and diagnostic privacy in temporary repositories.
- Closing an active automatic refresh took **262 ms**; Git's measured stop time
  was **27.155 ms**. The watcher suite recorded 329 diagnostic events with zero
  drops and no file payloads. These are fixture timings, not universal guarantees.
- All six packaged CLI checks passed, plus ten package tests (one test explicitly
  for refusing non-Mac signing is skipped on Mac), three preflight cases and
  native icon/signature/archive checks. Normal user credentials are untouched.
- The five arm64 executables linked and passed package validation; the packaged
  CLI and native UI through MCP ran successfully. Bundle inventory, per-file hashes,
  ad-hoc signatures and ZIP extraction/verification passed on macOS. The downloaded
  artifact digest, archive checksum and manifest were independently rechecked.

`Rocket-macos-arm64-development.zip` is **92,611,883 bytes**. SHA-256:
`66ab2528e12e93f098d2b625d589ae87a7eaf52f15c0bc67ffe44655e4c9e37d`.
This is a development build for macOS 15+, not a notarized distribution.
Its embedded build record identifies the tested source; subsequent documentation
changes do not change these binaries.

Native screenshots confirm CJK and monochrome emoji in the diff/terminal,
aligned emoji clusters and usable compact dialogs. The older missing-crab and
automatic-refresh warning are resolved in this build. This does not establish
all Unicode sequences or real IME/Retina behavior.

The earlier [platform gate](https://github.com/JasXTD/Rocket/actions/runs/42491021304)
on `424db2e` passed 198 core-platform Rust test executions and 33 MCP checks,
including disposable Keychain/SSH, process containment and Lua refusal. Those
unchanged areas were not rerun in this focused follow-up.

## Notifications and emoji

Rocket uses a recursive FSEvents watcher on a private serial dispatch
queue. Worktree changes refresh selected diffs; Git/common-directory changes
invalidate metadata. Lost events force a full refresh, and directory identity
checks reject replaced roots. Closing detaches and drains callbacks before freeing
their context. No polling Git commands or directory scans run while idle.

Mac/Linux now include Noto Emoji **3.002** as an outline fallback, ahead of egui's
older emoji faces but after ordinary interface/code faces. Terminal wide clusters
use it first. The 1.9 MiB asset was approved, matched Google's Git blob hash and
passed the owner's Malwarebytes scan before loading. The exact source, hashes and
license are in `runtime/fonts/noto-emoji/3.002`; nothing is installed system-wide.
It supplies monochrome outlines, not Apple's color emoji or a replacement IME.

Windows font/license/MCP watcher tests, Linux font/license/watcher/package tests
and native Mac checks passed. Lua remains refused by agreement; its sandbox is the
next separate task. [Signing preparation](MacOS-signing.md) is a future runbook,
not an executed notarization or a release-ready signing pipeline.

## Build and evidence

`.github/workflows/macos-arm64.yml` is **manual-only**. It has no push, pull-request,
schedule, release or deployment trigger. It uses a standard Apple Silicon
`macos-15` runner, one concurrent job, a 15-minute timeout and three-day artifact
retention. A newer manual run cancels the previous run. It requests read-only
repository access and checkout does not retain credentials. It uses no personal
tokens, signing keys, repository credentials, third-party setup action, or
uploaded Cargo cache. The runner label fixes the OS family, not the exact image;
record the image version shown by GitHub with each validation result.

The official GitHub actions are pinned to complete commits, verified against
their publisher's release metadata on 2026-11-08:

- [checkout v7.0.1](https://github.com/actions/checkout/releases/tag/v7.0.1):
  `3d3c42e5aac5ba805825da76410c181273ba90b1`.
- [upload-artifact v7.0.2](https://github.com/actions/upload-artifact/releases/tag/v7.0.2):
  `cf430e030ddbb5b0abf93d22962f4752f3646cd9`.

On an authorized run, the workflow selects Rust **1.99.1** using the
runner's rustup and downloads the existing locked Cargo dependencies. It does not
install tools on the developer's PC. Local native builds require that toolchain,
Xcode command-line tools, Node.js and compatible Git to be installed already:

```sh
bash tools/build-macos.sh --offline
```

Omit `--offline` to allow Cargo to obtain the locked dependencies. The script
refuses other operating systems, Intel/Rosetta, root and existing output folders.
The default output is `target/macos-stage1`; `ROCKET_MACOS_OUTPUT` may select a new
absolute folder. Its build cache is `target/macos-stage1-cargo`. Windows/Linux
build scripts, profiles and release directories are not changed. This first
development build targets macOS **15.0**, without promising older-OS support.

The build checks all workspace targets, runs only the affected core platform,
credentials/SSH, desktop, terminal/MCP and Lua preflight/unsupported-worker tests,
builds five binaries, packages them in `Rocket.app`, verifies their arm64
architecture, and runs a bounded CLI smoke test from the packaged helpers. That smoke uses a new repository
with a Unicode path; it verifies JSON status, pre-cancellation, missing-repository
errors, diagnostic JSON and preservation of the fixture. It uses no hosted remote
or personal Git configuration. The backend suites also test mid-operation Git
cancellation and descendant-held pipes, locking across processes/crashes, safe
init/clone publication, local identity and file/conflict recovery. Terminal tests
exercise Ctrl+C, editing/history/completion, resize, Unicode and session cleanup.
The localhost MCP runners exercise actual windows, token/Host/Origin refusal,
SSH discovery, desktop shortcuts, terminal diagnostics and compact layouts.
They fail the build if native window
creation is unavailable; no headless skip counts as acceptance. Successful unit fixtures are removed;
failed ones are retained locally. `ROCKET_TEST_REPOSITORIES` selects a new absolute
test-only root outside the source checkout, on the native filesystem. This avoids
mistaking fixtures for nested repositories or testing Unix semantics on WSL DrvFS.

The manual `checks` input defaults to `followups`: watcher SDK/runtime and UI
refresh tests, fonts/licenses, packaging, CLI smoke, and HTTP MCP watcher,
Unicode-terminal and desktop captures. `platform` includes the original broader
platform boundary checks as well. Local builds default to `platform`; set
`ROCKET_MACOS_CHECK_SCOPE=followups` to select the focused pass. Both scopes keep
all-target compilation and package/signature/archive verification mandatory.

The artifact is `Rocket-macos-arm64-development.zip`, with per-file hashes and an
archive SHA-256 sidecar. It contains `Rocket.app`, notices and this document,
not source snapshots, test repositories, logs with credentials, or a Lua worker.
Native link information and smoke results are separate evidence.

The app uses Rocket's existing artwork, rendered at all ten Apple iconset sizes
and converted with Apple's `iconutil`. `Info.plist` declares Apple Silicon's
minimum macOS version and Rocket's workspace version. All five executables remain
siblings in `Contents/MacOS`, preserving rebase, SSH and credential-helper lookup.
Licenses and the icon live in `Contents/Resources`. Packaging refuses existing
output folders, links, unexpected files, wrong architectures and non-system
dynamic dependencies. The workflow verifies the packaged helpers, hashes,
permissions and an extracted ZIP roundtrip.

Helpers and then the application receive **ad-hoc development signatures** using
Apple's `codesign`; signature verification is required before publication of the
development artifact. These signatures do not authenticate a publisher. There is
no Developer ID certificate, notarization, entitlement or security-policy change.
The bundle is not an update package or a replacement for Windows/Linux releases.
Finder launch and Gatekeeper acceptance still need an actual Mac test; do not
bypass Gatekeeper to present this artifact as a supported release.

## Running Actions later

After an authorized commit/push puts the workflow on the default branch, select
Actions → macOS Apple Silicon preparation → Run workflow and review the selected
revision. Merely preparing these files consumes no Actions credit. A failed
compile/check fails the job; no `continue-on-error` hides failures. Artifacts from
a failed build are diagnostic evidence, not successful builds. A timeout or
cancellation may prevent evidence upload.

Standard runners in public repositories have free compute; private repositories
use the owner's shared included compute allowance, then applicable overage.
Artifact storage has separate limits. The timeout and retention reduce exposure
but do not impose an account-wide spending cap. Check the account's billing
budget before running. See [GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)
and [runner availability](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
Rocket remains private; this preparation does not change its visibility.

## Known platform gaps, not completed by a successful build

| Area | Current boundary / follow-up |
| --- | --- |
| Lua | Execution explicitly refused. See [sandbox investigation](MacOS-lua-sandbox.md). |
| Terminal | Native isolated Bash job control, resize, Ctrl+C, same-shell recovery, history and completion passed. Normal interactive zsh, user profiles and physical keyboard acceptance remain pending. |
| Localhost MCP | Native windows, secure tokens, Host/Origin refusal and the four affected suites passed. This does not certify every Rocket workflow on Mac. |
| Credentials / SSH | Disposable Keychain roundtrip/tamper/locked checks, disposable-agent tests and SSH discovery/import passed. Normal-account access prompts, signed-helper upgrades and live hosted authentication still need Mac acceptance. |
| Filesystem / hashing | Native ACL, storage, locks, atomic file/repository operations and recovery checks passed. Additional volumes and Unicode-normalization edge cases need acceptance. Unsupported atomic operations fail without an overwriting fallback. |
| Git | Step 2 filesystem-only installed-Git discovery and launcher refusal implemented; Finder environment and installed Git versions still need native acceptance. Windows' bundle is never selected. |
| Automatic repository refresh | Native FSEvents and focused Mac runtime checks passed, including linked/common directories, repeated edits, root replacement, shutdown and idle behavior. Older development artifacts still require F5. Windows and Linux retain their own watchers. |
| Desktop | Native Command/Control bindings, CJK, outline emoji, compact dialogs and persistence passed. Real IME, clipboard, Retina and Finder launch remain pending. A native macOS menu bar is not implemented. |
| Spell checking / updates | macOS adapters and signed `.app` update/rollback remain absent. |

The next runtime gate is an isolated ordinary-user account on an **Apple Silicon EC2 Mac**
for bare-metal acceptance: Finder/Gatekeeper, Keychain, SSH agent, terminal,
Unicode files, cancellation and recovery. An Intel EC2 Mac needs a separate build.
Do not use a headless build or a remote desktop session to claim physical-display,
keyboard-layout or real multi-monitor coverage that was not exercised.

## Desktop adapters (step 5)

Mac defaults use Command for desktop actions, with Control+Tab for repository
navigation, Control+backtick for Terminal and Command+Shift+J for Activity.
Saved Control bindings remain literal Control bindings. Command can be recorded
with keyboard, modified clicks and wheel gestures; editing/OS shortcuts stay
reserved. Menu hints use the actual binding. Existing dialog primary actions and
diff search use the platform's primary modifier. Terminal Ctrl+C stays Control.
The interface still uses Rocket's in-window menus, not a native macOS menu bar.

Mac font discovery reads bounded trees under `/System/Library/Fonts`,
`/Library/Fonts` and `~/Library/Fonts`. Installed outline fonts supplement egui;
the existing approved Noto Sans CJK 2.004 bytes supply Chinese/Japanese/Korean
fallbacks, including monospace editor/terminal faces. No font is downloaded or
installed. The development archive includes its license and notice. Color emoji,
IME composition and physical keyboard layouts still need real Mac inspection.
The WSL and native Mac captures confirmed CJK and combining accents, but still
showed a missing crab emoji in the diff's existing outline-font fallback. The
follow-up's Noto Emoji outlines render the crab, rocket and tested clusters in
native captures. This does not promise every Unicode sequence or color emoji.
Preferred language and Region come from read-only CoreFoundation APIs, so a
Finder launch need not inherit a shell's LANG. Rocket's explicit language setting
continues to override automatic selection. See Apple's [locale declarations](https://github.com/apple-oss-distributions/CF/blob/main/CFLocale.h)
and [bounded UTF-8 conversion](https://github.com/apple-oss-distributions/CF/blob/main/CFString.h).

Validated HTTPS links and existing absolute folders use `/usr/bin/open` on Mac
(folders use `-R` to reveal them in Finder, never launch a `.app` directory);
Linux uses installed `/usr/bin/xdg-open`, and Windows retains Explorer. Arguments
are passed separately without a shell. At most four background handoffs wait for
ten seconds after spawning; errors are surfaced and logs omit targets. Cancellation/closing
stops and reaps only the directly launched process, without a process-tree kill
or targeting an existing desktop application. An already completed handoff
cannot be undone. Explorer's exit status is not a reliable acceptance signal,
so it is ignored; spawn, waiting and timeout failures remain visible.
The MCP fixture records external handoffs without opening personal applications.

The existing Rust folder browser now lists Unix executable files (including
installed symlinks during normal browsing), while automation keeps its strict
fixture/link boundary. It offers the Unix filesystem root; scoped test windows
never expose it. Application bundles must still be navigated to an executable;
native macOS dialogs and `.app` tool selection are separate work.

The manual native gate checks SDK CoreFoundation types, locale reads, shortcut
dispatch/persistence, CJK glyphs and fallback metrics, folder traversal and
cancellation. `test-mcp-desktop.cjs` exercises the real window at wide/compact sizes
with Unicode data, shortcuts, cancelled capture, save/reopen, terminal interrupt
and unchanged repository hashes. Windows/WSL results are regression evidence,
not proof of any macOS runtime behavior.

## Core platform services (step 2)

No new dependency version or software installation is required. The already
locked RustCrypto `sha2 0.11.1` now covers macOS as well as Linux. Windows keeps
BCrypt; Linux credential encryption and Windows DPAPI are unchanged. Hashing keeps
the same byte limits, error propagation and per-chunk cancellation checks.

- **Storage:** `~/Library/Application Support/Rocket` for settings, logs, crash
  reports and previews. Linux retains XDG; Windows retains LOCALAPPDATA. Existing
  data is not migrated or chmodded. Missing private directories use 0700. Checks
  reject unsafe ancestors, user links, traversal and extended ACL access grants;
  deny-only ACLs are accepted. Only the exact root-owned OS aliases `/tmp` and
  `/var` are resolved, and their targets are checked normally. Tests include ACL
  preservation and an attempted user-link redirect. This is not a Lua sandbox.
- **Atomic filesystem operations:** Darwin `renamex_np(RENAME_EXCL)` publishes
  init/clone and newly restored files without overwriting a destination that
  appeared concurrently. `RENAME_SWAP` replaces a file while retaining the actual
  displaced bytes, followed by an exclusive move to its backup. If that second
  move fails, the error states where the displaced file remains. Cancellation,
  external-edit checks and recovery copies stay in the existing workflows.
  Native volumes must support these flags; no copy/delete fallback is used.
- **Opened files:** `F_GETPATH` checks the actual opened vnode for reviewed file
  contents; device/inode checks reject redirected or replaced entries. Darwin's
  own `O_NOFOLLOW` protects ownership/preview locks. Git executable bits are
  preserved during restore/conflict saves.
- **Ownership and cancellation:** retain the standard-library file locks and
  existing Unix process groups, including helper termination, pipe draining,
  reaping, timeouts and request timing. This coordinates Rocket clients; it does
  not replace Git's locks or prevent unrelated applications from writing.
- **Git discovery:** search absolute PATH entries, an absolute `DEVELOPER_DIR`
  (developer folder or `.app`), `/opt/homebrew/bin`, `/usr/local/bin`, installed
  Command Line Tools and the default Xcode application. Resolve normal executable
  symlinks, but refuse `/usr/bin/git` and its aliases, including for an explicit
  preference. Nothing executes during discovery. The actual version probe still
  requires **Git 2.46+** and uses the caller's limits/cancellation. A non-default
  Xcode installation can be selected explicitly; Rocket does not change the
  active developer directory or start an installer. Credentials/SSH are covered
  by step 4 below; live hosted authentication from a normal Mac remains pending.

Implementation references: Apple's [Application Support directory](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/MacOSXDirectories/MacOSXDirectories.html),
[rename flags/ABI](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/stdio.h),
[atomic rename semantics](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/rename.2),
[file descriptor flags](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/fcntl.h),
[ACL declarations](https://github.com/apple-oss-distributions/Libc/blob/main/include/sys/acl.h),
[descriptor ACL lookup](https://github.com/apple-oss-distributions/Libc/blob/main/posix1e/acl_file.c),
[absent ACL property behavior](https://github.com/apple-oss-distributions/Libc/blob/main/gen/filesec.c),
and [developer-tool installation behavior](https://developer.apple.com/documentation/xcode/installing-the-command-line-tools).
These support the source design; actual Darwin behavior must pass the native gates.

## Terminal and localhost MCP (step 3)

No new dependencies or tools are installed. The screen model and bounded worker
queues are shared; Darwin uses its own terminal ABI, PTY device lookup, signal
numbers, UTF-8 locale and libproc session enumeration. The installed Apple SDK
independently checks the handwritten ABI before native tests run. Ctrl+C goes
through the controlling terminal to the foreground job; resizing sends the
kernel's window-change notification. Command+C does not send a terminal interrupt.

Normal macOS windows default to installed `/bin/zsh`, so a Finder launch does not
require `$SHELL`. Login shell, Bash, Fish and custom selection remain available.
Acceptance windows deliberately use `/bin/bash --noprofile --norc -i` with owned
history and an isolated home; they cannot execute arbitrary commands through MCP.
No shell profile, SSH configuration, agent or existing key is edited. The native
test's ssh-keygen creates only a disposable key in its fixture.

Closing a terminal hangs up and continues its session jobs, then kills survivors
after a one-second grace period. On Darwin the leader is observed without reaping
until cleanup finishes, protecting its session ID from reuse. Output draining is
bounded even if a deliberately detached process keeps writing. A PTY is not a
sandbox: processes that deliberately start a new session are not followed, and
privileged descendants may refuse signals. Cleanup failures remain visible.

MCP tokens use 32 bytes from Apple's Security framework random generator. The
service still binds only numeric loopback and requires its bearer token, exact
Host/Origin rules and fixture policy. Unix token/settings/lock opens reject leaf
symlinks. macOS checks extended ACL grants on the profile and its opened private
files using the same read-only verifier as core storage; existing permissions
are never repaired automatically. The separate Keychain boundary is described below.

For an already-built native test executable:

```sh
ROCKET_MCP_EXE=/absolute/path/rocket-mcp ROCKET_MCP_RESULTS=/absolute/new/results node tools/tests/test-mcp-terminal-interrupt.cjs
ROCKET_MCP_EXE=/absolute/path/rocket-mcp ROCKET_MCP_RESULTS=/absolute/new/results node tools/tests/test-mcp-terminal-unicode.cjs
```

Each runner creates a unique child fixture. Results/screenshots may be retained,
but the workflow does **not** upload token profiles, repositories, keys or raw
terminal histories. Windows and WSL execution validates their own adapters, not
Darwin. Mac font selection/IME, Finder behavior and physical keyboard/display
acceptance still belong to the native desktop/EC2 gate.

Implementation references: Apple's [terminal layout](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/termios.h),
[PTY requests](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/ttycom.h),
[PTY allocation and names](https://github.com/apple-oss-distributions/Libc/blob/main/stdlib/grantpt.c),
[wait flags](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/wait.h),
[process inspection](https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.h),
and [secure random bytes](https://developer.apple.com/documentation/security/secrandomcopybytes(_:_:_:)).

## Credentials and system SSH (step 4)

The existing locked XChaCha20-Poly1305 dependency now also builds on macOS. A
random 32-byte wrapping key is stored as a generic-password item in the default
file-based Keychain (`io.github.JasXTD.Rocket.CredentialKey`, account `v1`).
Apple's random generator supplies keys and nonces. Encrypted files use a separate
Mac format/AAD; Windows DPAPI and Linux envelopes retain their existing formats.
The historical `.dpapi` filenames are retained for compatibility, not as a claim
that macOS uses DPAPI. Copying those files between operating systems cannot
migrate credentials. There is no plaintext fallback.

Keychain access uses Security.framework directly, without credentials in shell
arguments. It suppresses prompts within the Rocket process for each serialized
call and restores the previous process setting. A locked Keychain, denied access,
missing key or malformed record fails visibly without replacing anything. No key
is created during decryption, and simultaneous creation reads back the winning
add-only item. The wrapping key is not cached on Mac, so a later lock/access change
is honored. The synchronous OS call cannot be interrupted internally; cancellation
is checked around it and before credential-file publication. The native fixture
passed its locked-Keychain refusal check; a normal user's access prompts and
permissions still need separate acceptance.

This first implementation uses the legacy/file-based Keychain because the
development binaries do not have a Developer ID/access-group identity. Ad-hoc
bundle signatures do not establish a stable publisher identity across builds.
On first creation, access is limited to the creating binary and verified sibling
Rocket helpers; existing item ACLs are never broadened on reload or upgrade. Their
paths must have safe ownership, permissions and no ACL grants or redirected
components. A changed code identity may need review in Keychain Access. Signed
application/helper upgrades, access prompts and distribution identity remain an
EC2 Mac acceptance/design gate; cross-compilation does not establish those behaviors.

SSH profiles retain key selection and multiple accounts. Only `/usr/bin/ssh`,
`ssh-keygen` and `ssh-add` are selected; there is no PATH fallback or installer.
Agent mode uses the inherited `SSH_AUTH_SOCK` owned by the current user. Rocket
does not start an agent, change launchd, or edit the user's SSH config. Direct-key
mode explicitly disables the agent and Apple's separate `UseKeychain` passphrase
mechanism, uses Rocket's protected credential, and requires host verification.
Private keys with ACL grants are refused without changing permissions. Saved
credentials publish with an exclusive move; external files and symlink locks
are refused. Existing keys remain in place.

The native build checks the Security/CoreFoundation ABI against the Apple SDK,
runs policy tests and the focused SSH suites with disposable keys/agent, then runs
the same SSH discovery/import/cancellation localhost MCP runner as Windows/WSL.
Tests requiring the normal default keyring are ignored by default. One separate
explicit-Keychain fixture is opt-in via `ROCKET_MACOS_DISPOSABLE_KEYCHAIN_TEST=1`,
enabled only on the ephemeral Actions runner. It creates/deletes its own Keychain,
restores the search list (creation can register it), verifies the default/search
list remain unchanged, and covers roundtrip, duplicate adds, tampering and a
locked Keychain. Do not enable this fixture in a personal macOS login. It does not
read/write ordinary Keychain items, unlock the login Keychain or reset any Keychain.
This disposable-Keychain fixture passed on the ephemeral native Apple Silicon
runner. That does not establish access prompts, upgrades or helper interoperability
with a normal user's existing Keychain.

Sources: Apple's [SecItem declarations](https://github.com/apple-oss-distributions/Security/blob/main/keychain/headers/SecItem.h),
[Keychain API](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/SecKeychain.h),
[application access lists](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/SecAccess.h),
and [OpenSSH Keychain/agent behavior](https://developer.apple.com/library/archive/technotes/tn2449/_index.html).

## Focused local checks

With the already installed Apple Silicon target, this checks Rust source and
metadata on Windows without installing an SDK or trying to run a Mach-O binary:

```sh
cargo check --offline --locked --workspace --all-targets --target aarch64-apple-darwin
```

It does not compile the optional C Lua runtime, link against Apple's SDK, or
execute the macOS-only tests. The native workflow provides that separate evidence.
For supported-platform preflight regressions:

```sh
cargo test --offline --locked --release -p rocket-git --lib integration_worker::preflight_tests
```

Run the existing worker hash-rejection test on Windows/Linux when changing this
preflight. Native macOS refusal tests are `rocket-integrations --features runtime
--test unsupported_worker`; an empty test suite on another OS is not macOS proof.
