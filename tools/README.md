# Rocket development tools

## Apple Silicon macOS preparation

The manual-only `.github/workflows/macos-arm64.yml` prepares a development `Rocket.app`
with pinned Rust/actions and bounded runtime/artifact retention. Local native
builds use `bash tools/build-macos.sh [--offline]`. It runs only the affected core
platform, terminal/MCP and Lua refusal checks plus an isolated CLI smoke; it does not publish a release or enable
Lua. See [the build and acceptance plan](../docs/MacOS-port.md) and
[the sandbox investigation](../docs/MacOS-lua-sandbox.md). Native linking,
desktop/MCP acceptance and EC2 Mac testing remain separate verification gates.
The build now prepares two native localhost terminal runs and an Apple SDK ABI
check. `ROCKET_MCP_EXE` selects an already-built executable for
`test-mcp-terminal-interrupt.cjs` and `test-mcp-terminal-unicode.cjs`;
`ROCKET_MCP_RESULTS` selects their parent results folder. Each creates its own
synthetic repository/profile. No downloaded dependency, user key or hosted remote
is needed. Cross-compilation does not count as passing those native Mac runs.

Step 5 adds `test-mcp-desktop.cjs`, using those same executable/results overrides.
It covers primary/Control/mouse bindings, cancelled capture, persistence, Unicode
and compact repository browsing, and terminal Ctrl+C with an isolated profile.
External application handoffs are recorded by MCP; OS launcher argument/status
behavior and native locale/font handling have separate focused Rust/SDK checks.

`tools/macos/bundle.cjs` stages an exact bundle inventory, signs development code
with Apple's ad-hoc identity, and verifies architecture, system-only links,
permissions and hashes. `crates/rocket-ui/examples/macos_icon.rs` renders the
existing icon into Apple's ten-size iconset. Build that example, then set
`ROCKET_MACOS_ICON_TOOL` to its executable before running
`node --test tools/tests/test-macos-bundle.cjs`. These metadata/fixture tests run
on Windows and Linux too; native signatures, `iconutil` and ZIP roundtrip checks
remain part of the Mac workflow. No downloaded icon or packaging dependency is used.

## Direct SSH keys

`tools/mockups/ssh-direct-key/preview.html` preserves key selection and previews
the agent/direct choice, remembered passphrase and repository review. Focused
backend coverage: `cargo test --release --offline --locked -p rocket-git --test
ssh_direct --test ssh_keys`. Use the normal Windows user token: restricted tokens
cannot perform the ancestor-file protections/DPAPI checks. After Build-Local,
`node tools/tests/test-mcp-ssh-direct.cjs` tests the native page through localhost
MCP with disposable encrypted Unicode keys and local repos. It does not need or
modify the Windows SSH agent and does not connect to GitHub. The first-party
`rocket-ssh.exe` is included by the portable packager and updater manifest checks.

## Crash reports

The interactive design is `tools/mockups/crash-reports/preview.html`. Focused
checks select `rocket-git --lib crash_reports`, `rocket-git --test crash_reports`
and `rocket-ui --lib crash_reports`. Only the test executable's isolated child
mode deliberately panics or raises a Windows exception; no shipped app exposes
a crash trigger. After building, `node tools/tests/test-mcp-crash-reports.cjs`
checks the native viewer/copy/export, errors, Unicode and compact layout through
an isolated localhost MCP service. Its reports and repository are synthetic.

## Repository map

After building, run `node tools/tests/test-mcp-map-security.cjs <new-evidence-directory>`
for the history-link budget. It checks refusal above 12,000 links on initial reads
and Load more, retained snapshots and responsive filtering, exact-limit rendering
at wide/compact sizes, and repository preservation. Its synthetic fixture and
owned MCP service are removed after success; failures retain fixture evidence.

The focused exploration runner is `node tools/tests/test-mcp-map-exploration.cjs`.
It checks collapse/neighbor navigation, pinned scoped pagination and cancellation,
Mermaid review/new-file safety, external changes, Unicode and compact windows.
The design preview is `tools/mockups/map-exploration/preview.html`.

After `Build-Local.ps1 -Offline`, run `node tools/tests/test-mcp-repository-map.cjs`.
It tests the native Code, Git history and People pages through an isolated
localhost MCP profile using synthetic local repositories. It covers Unicode,
Cargo path links, merge parents, canonical authors, compact detail drawers,
external changes, bounded large trees, cancellation, superseded reads, errors
and diagnostic privacy. No hosted remotes or personal credentials are used.
Focused Rust checks select `rocket-git --test repository_map` and
`rocket-ui --lib repository_map`; parser tests select `rocket-git --lib repository_map`.

## Repository folder scan

Run `node tools/tests/test-mcp-repository-scan.cjs` after a matching
`Build-Local.ps1 -Offline` build. It creates an isolated localhost MCP profile and
32 local repositories/worktrees, then checks scan/add cancellation, duplicates,
Unicode paths, filtered selection, skipped-entry details, external removal,
pinned compact-window controls, persistence and log privacy.
Only disposable fixture repositories are used; no remote operation runs.
The refined mockup is `tools/mockups/repository-scan/preview-v2.html`; native
results and captures are kept under `tools/maintenance/repository-scan-ux-20261103`.
Focused Rust checks: `cargo test --offline --locked --release -p rocket-ui --lib repository_scan`
and `cargo test --offline --locked --release -p rocket-ui --lib repository_manager`.

## Windows package acceptance

Use Test-ReleaseChecks.ps1 -Offline for the five exact sequential release gates,
and Test-WindowsPackage.ps1 with a reviewed bundle and new system-temp output
directory for the packaged CLI, ordinary desktop and localhost MCP smoke checks.
Run them one at a time, with the shared test slot reserved. All original timing
criteria remain enforced; no full workspace test or automatic retry is implied.
See [Windows release testing](../docs/Windows-release-testing.md) for commands,
isolation, evidence, clean-machine limits and Windows 11 handoff instructions.

Reusable tools live here. `work/` is only generated test data, logs and temporary
output. Run commands below from the Rocket project root. Run only the checks for
the feature you changed; reserve the build/MCP slot in `COORDINATION.md` first.

Create a new unsigned local release with `.\tools\Package-Release.ps1 -Offline`
after updating the workspace version. It builds through `Build-Local.ps1`, includes
all six Rocket executables, ConPTY and the reviewed app-local Visual C++ runtime,
regenerates `SHA256SUMS.txt`,
then runs the bundled verifier before naming the versioned directory. Existing
release versions are never overwritten. A failed candidate stays in a `.partial`
directory for inspection. No certificate is required.

The Lua worker links its runtime statically because it runs from a private sandbox
directory. Packaging checks actual PE imports using the existing MSVC toolchain;
the worker's compiler flags do not alter the ordinary host builds.

On Linux x86_64, `tools/build-local.sh [release|debug] [--offline]` builds the matching
bundle the same way: the worker first, then the hosts with its SHA-256. The Linux worker
links its libraries as usual, because it locks itself down only after the dynamic loader
has opened them. It is built without debug info, since Rocket copies and hashes the
whole worker for every run, and for an explicit target, so its cache stays apart from
the ordinary host builds. Its sandbox tests are `cargo test --offline --locked -p
rocket-git --lib integration_worker::linux::tests`; they need `rustc` to build their
probe. `cargo test --offline --locked -p rocket-integrations --features runtime
--test linux_worker` builds and tests the real worker automatically, without an
opt-in path, digest or ignored test. Both require Linux x86_64 with Landlock enabled.

`Copy-VcRuntime.ps1` reuses the pinned Microsoft DLL from an existing Visual Studio
installation; `Package-Release.ps1 -VcRuntimeDirectory <directory>` can select
that reviewed directory explicitly. It downloads/installs nothing and refuses an
unknown hash or an existing destination. See [runtime provenance](../runtime/vc/README.md).
Run `tests/test-vc-runtime.ps1 -OutputDirectory <new directory>` for its focused
missing/tampered-runtime and overwrite-protection checks. Historical bundles
retain their own verifier; use the new verifier for newly assembled packages.

The canonical verifier is `tools/Verify-Release.ps1`; keep it in source control.
Packaging also creates `Rocket-<version>-windows-x64.zip` with update metadata and
complete file hashes. `tools/New-UpdateArchive.ps1 -Bundle <folder> -Version <version>`
can create that ZIP from an already verified local bundle. Neither tool uploads it.
Packages include the user wiki; UI.md, COORDINATION.md, VALIDATION.md and generated
test/build folders stay outside the release.
See [app updates and publishing requirements](../docs/wiki/App-updates.md).
Affected checks: `test-mcp-app-updates.cjs` and `test-app-update-installer.cjs` in
`tools/tests`, using only temporary installations, local Git and loopback HTTP.

On Linux x86_64, `tools/package-release.sh [--offline]` builds the release folder
`tools/releases/Rocket-<version>-linux-x64` and its update archive
`Rocket-<version>-linux-x64.tar.gz`: POSIX ustar in name order with fixed owners and
times, compressed by gzip. The folder's `SHA256SUMS.txt` checks with
`sha256sum -c SHA256SUMS.txt`, so Linux needs no verifier script. The ignored test
`app_updates::package::tests::a_packaged_linux_release_passes_the_update_checks`, given
`ROCKET_TEST_UPDATE_ARCHIVE` and `ROCKET_TEST_UPDATE_VERSION`, runs Rocket's own update
checks on such an archive. `test-app-update-installer.cjs` runs on Linux too, without
its Windows-only locked-folder case. `sh tools/tests/test-package-release.sh` checks
archive creation, extracted checksums and propagation of archive-tool failures with
tiny synthetic build outputs; it does not compile Rocket or contact a server.

New packages also include the reviewed Git runtime by default. `Build-Local.ps1`
adds it when the local prepared payload is present; `Package-Release.ps1` requires
it unless `-WithoutBundledGit` is explicitly selected. Neither script downloads
software. See [the runtime preparation and provenance notes](../runtime/git/README.md).
`Assemble-GitRuntime.ps1 -Destination <existing-folder>` verifies both the source
and copied inventory and refuses to overwrite a changed existing runtime.

`node tools/tests/test-mcp-managed-git.cjs` exercises bundled Git selection,
verification, Stop, external settings changes, Unicode paths, relocation and
native local repository operations using one disposable runtime copy and isolated
profiles. Reserve the shared MCP slot before running it. It does not contact a
Git hosting service or change normal credentials/configuration.
It also adds an inert file to its disposable, already-verified runtime and checks
that the cached client refuses the next operation until that file is removed.

Then run `node tools/tests/test-release-managed-git.cjs` before removing that
disposable app copy. It reuses the closed fixture to check pinned files, inventory,
verifier code, mixed builds and missing/extra files. It never executes damaged
binaries or changes an existing versioned release.

`node tools/tests/test-release-bundle.cjs tools/releases/0.92.0` checks missing
workers, mismatched host/worker pairs, manifest tampering and unsigned policy on
a disposable copy. `node tools/tests/test-release-git-guard.cjs tools/releases/0.92.0/rocket-cli.exe`
checks vulnerable/fixed/unknown Git versions using an owned compiled fixture that
never starts Git or opens a network connection.

`node tools/tests/test-mcp-shortcuts.cjs` checks customizable keyboard/mouse
bindings through localhost MCP, with isolated preferences, dirty temporary
repositories, actual native input, restart/reset checks and terminal probes.
The `rocket_shortcut` tool accepts recording IDs or safe navigation probes only;
it cannot execute arbitrary Git writes or shell commands.

Terminal runners: `node tools/tests/test-mcp-terminal.cjs`, `test-mcp-terminal-unicode.cjs`,
`test-mcp-terminal-interrupt.cjs` and `smoke-mcp-terminal.cjs` drive the drawer's shell
through MCP, on Windows and on Linux. MCP types only fixed synthetic probes into the
terminal, each matched exactly: `SHELL_PROBES` and the fixture write probe in
`crates/rocket-mcp/src/ui.rs`, and the Unicode tables and keyboard probes in
`crates/rocket-mcp/src/terminal_probes.rs`. On Windows they are Windows PowerShell
commands. On Linux, where an MCP window runs bash, they are bash commands that print the
same markers. `tools/tests/platform.cjs` holds what each runner sends on each platform.

On Linux the runners need Node 24 and a release build, `target/release/rocket-mcp`. Under
WSL, set `WAYLAND_DISPLAY` to WSLg's socket, `/run/user/<uid>/wayland-0`, when the shell
has no Wayland session. `test-mcp-psreadline.cjs` and `test-mcp-terminal-settings.cjs`
test Windows-only behavior and stop on other platforms.

`smoke-mcp-terminal.cjs` blocks its commit with the Git relay, `tools/tests/git-relay.cjs`,
not a `pre-commit` hook: MCP's transport scope disables hooks for every Git command
(`core.hooksPath=/dev/null`). The relay holds `before:commit`, and Stop must end it before
real Git starts.

`node tools/tests/test-mcp-playground-ai.cjs` exercises the Lua editor/preview and
the connected-agent conflict workflow with synthetic proposals. It uses only
temporary local conflicts, protected scratch storage and localhost MCP. It does
not contact an AI provider; model quality is outside this deterministic test.

Lua integration acceptance uses `node tools/tests/test-mcp-lua-integrations.cjs`
for the bounded runtime/transport and `node tools/tests/test-mcp-integration-profiles.cjs`
for package errors, multiple connections, protected drafts, restarts and save-failure
recovery. `node tools/tests/test-mcp-integration-workflows.cjs` checks developer
validation, explicit repository context and Jira search/edit/transition workflows.
Run these sequentially. All create marked dirty repositories and isolated
profiles under `work/mcp-fixtures/`, use loopback mocks and preserve normal logins.

## Source control and local files

The source repository keeps reusable scripts, tests, test-helper Rust
sources, and the signed ConPTY runtime with its manifest and licenses. `Cargo.lock`
is part of the source. `.gitattributes` preserves the pinned runtime bytes.

Build output (`target/`, `tools/helpers/bin/`), generated test data (`work/`), local release bundles
(`tools/releases/`), archived history (`tools/archive/`), design mockups (`tools/mockups/`)
and dated maintenance subfolders are excluded by `.gitignore`. The two reusable maintenance checks
directly in `tools/maintenance/` remain included. Existing local copies are kept;
these folders are not restored by a fresh Git checkout.

The optional downloaded `runtime/osv-scanner/` is also local-only, as documented
in `DEPENDENCIES.md`. It is separate from the required `runtime/conpty/` bundle.
Assistant settings, credentials, logs, databases and debug symbols stay local.
The internal `UI.md`, `COORDINATION.md` and `VALIDATION.md` records also stay
local. The included [user guide](../crates/rocket-ui/src/guide.md) documents the
desktop workflows; fresh checkouts do not include the internal agent/test history.

For a fresh checkout, build the current MCP executable and pass it explicitly
when running the maintenance checks:

```powershell
.\tools\Build-Local.ps1 -Offline
pwsh -NoProfile -File tools/helpers/build.ps1
node tools/maintenance/check-tooling.cjs target/release/rocket-mcp.exe
node tools/maintenance/check-launcher.cjs target/release/rocket-mcp.exe
```

Use an online locked build if the declared Cargo dependencies are not cached yet.
Only clear generated data after its processes have exited. Keep needed failure
evidence and recovery archives; ignoring a directory does not delete its contents.

## Folder layout

PR collaboration acceptance: `node tools/tests/test-mcp-pr-collaboration.cjs`
(or `--provider=github`, `gitlab`, `bitbucket`, `bitbucket-server`, `azure`,
`azure-server`, `gitea`, `github-enterprise`). It uses only loopback mock APIs,
synthetic credentials and disposable dirty repositories. It checks protected
draft/recovery persistence, thread and review writes, merge guards, cancellation,
redacted logs and unchanged local files. The matching mockup is
`tools/mockups/pr-collaboration/preview.html`. No live account is required.

| Folder | Contents |
| --- | --- |
| `mcp/` | Localhost MCP launcher, health check, request client and test transport clients |
| `tests/` | Reusable backend smoke tests and focused native acceptance runners |
| `tests/features/` | Newer feature-specific native runners |
| `helpers/` | Test-only Git relays and small Rust helpers; never put them on your normal PATH |
| `diagnostics/` | Diagnostic log viewer |
| `website/` | Website preview, checks, screenshots and demo video |
| `releases/0.92.0/` | Latest locally packaged release and hash manifest; matching built executables are in `target/release/` |
| `releases/0.81.1/`, `releases/0.82.0/` | Small frozen bundles retained for existing baseline/test tools |
| `mockups/branch-dropdown/` | Interactive Branch dropdown preview with compact and recovery states |
| `mockups/review-windows/` | Independent review windows, history order and commit browser link preview |
| `archive/` | Archive cleanup notes; obsolete local history/test ZIPs were removed on 2026-11-04 |
| `maintenance/` | Cleanup inventory, path mapping, archive hashes and verification records |

## Website

The public site is plain HTML and CSS in `docs/`, with no build step: `index.html`, `get.html`,
`404.html`, `site.css`, `site.js` and `media/`. Configure GitHub Pages to publish
`main` / `docs` when repository visibility and the account plan permit it.
`.nojekyll` keeps these static files unchanged. Source files alone do not enable
the hosted site. Edit the pages directly; the download page links to verified releases.

```powershell
node tools/website/serve.cjs   # Preview at http://127.0.0.1:8792/Rocket/, the way Pages serves it
node tools/website/check.cjs   # Links, anchors, alt text, nothing personal or external
node tools/website/media.cjs --social   # Only the 1280x640 GitHub card; keep other media
```

To refresh the pictures and video after the screens change, build `target/release`, reserve the
build/MCP slot and leave the test window alone:

```powershell
node tools/website/capture.cjs                                    # About 10 minutes, in a disposable demo repository
node tools/website/capture.cjs work/website-shots stage conflict  # Retakes only the steps named
node tools/website/media.cjs                                      # Writes docs/media through Edge; nothing to install
```

Look at every new picture and play both videos before publishing, because they go public as they
are. Every new video stays in Git history, so re-record only when the screens have really changed.
Crops, scenes and the pointer's paths are at the top of `tools/website/media.html`. The GitHub and
site addresses are in the three pages, `docs/sitemap.xml` and `check.cjs`, the `<base>` in
`404.html` names the project path, and `get.html` names the current version.

### GitHub Wiki

Keep the authoritative Markdown in `docs/wiki`. Export a new directory before
publishing; the exporter fixes page links, source references and image URLs and
adds a sidebar/footer without changing the source pages:

```powershell
node tools/website/export-wiki.cjs tools/maintenance/wiki-export
```

Enable Wiki in the repository settings and create its first Home page on GitHub.
Then clone `https://github.com/JasXTD/Rocket.wiki.git` into a separate directory,
review the export against existing pages, copy only those reviewed files and
commit/push that Wiki repository. Never replace unrelated Wiki pages or history.
Pages and Wiki availability depends on the GitHub plan and repository visibility;
changing Rocket from private to public requires the owner's explicit approval.

## Localhost MCP

Azure DevOps Server (unreleased): `mockups/azure-server/preview.html` and
`node tools/tests/test-mcp-azure-server.cjs`. Uses a new protected profile, dirty
temporary repo and loopback APIs for auth/errors, all three API choices, profiles,
pagination/work items, reviewed creation and cancellation. Evidence is under
`work/mcp-fixtures/azure-server-*`. Backend:
`cargo test --offline --locked -p rocket-git --test forge azure_server_tests`.
The TLS case reuses installed Git for Windows OpenSSL with a disposable local
certificate. No real accounts, new packages, trust-store changes or hosted writes.

GitHub Enterprise Server (unreleased): `mockups/github-enterprise/preview.html`.
Run `node tools/tests/test-mcp-pr-reviews.cjs --enterprise` after building the
current app/MCP executables. This reuses the PR workflow runner against an exact
enterprise-shaped `/api/v3` mock, with auth/errors/cancellation, separate saved
profiles, search/issues/pagination and diagnostic privacy. All requests stay on
loopback, using disposable tokens and a dirty temporary repo. Evidence is under
`work/mcp-fixtures/pr-ghes-*`. Backend:
`cargo test --offline --locked -p rocket-git --test forge enterprise_tests`.
The TLS case uses the OpenSSL already shipped with Git for Windows to create a
disposable loopback certificate; it never changes trust stores or installs tools.

GitHub inline reviews / merging / cross-fork creation (unreleased):
`mockups/pr-workflows/preview.html` and `node tools/tests/test-mcp-pr-reviews.cjs`.
The runner now covers the existing PR review flow plus these additions, using
only a synthetic loopback API and temporary dirty Git repository. No real account
or hosted mutation. Backend additions: `cargo test --offline --locked -p rocket-git --test forge pr_workflow_tests -- --test-threads=1`.
Evidence: `work/mcp-fixtures/pr-reviews-*`; source snapshots and report:
`maintenance/pr-workflows-20261027/`. Build current app/MCP executables first.
Add `--fork-only` to the native runner for changes confined to the PR creation
form; it checks fork preview/create/rejection/cancellation, fixture guards and logs.

Guided setup / repository summary / Safe Undo (unreleased):
`mockups/guided-setup/preview.html` and `node tools/tests/test-mcp-guided-undo.cjs`.
Build the current `rocket-ui` and `rocket-mcp` release-profile executables first.
The focused runner uses temporary local repositories, isolated identity/settings,
compact terminal-open layouts, external edits and cancellation before/after Undo's
branch movement. Results and screenshots stay in `work/mcp-fixtures/guided-undo-*`.
No hosted remotes, real login, source staging or push is involved.

GitHub PR details/reviews (0.87): `mockups/pr-reviews/preview.html` and
`node tools/tests/test-mcp-pr-reviews.cjs`. Uses synthetic loopback HTTP responses,
disposable credentials and a temporary repository/profile. No real account is
contacted. Backend: `cargo test --offline --locked -p rocket-git --test forge pull_review_tests:: -- --test-threads=1`.
Generated evidence: `work/results/pr-reviews-0.87/` and
`work/mcp-fixtures/pr-reviews-*`; source evidence: `maintenance/pr-reviews-0.87/`.

Automated bisect (0.86): `mockups/auto-bisect/preview.html` and
`node tools/tests/test-mcp-auto-bisect.cjs`. First build the test-only helper with
the installed Rust compiler: `rustc --edition=2024 -C debuginfo=0 -o tools/helpers/bin/auto-bisect/test-runner.exe tools/helpers/auto-bisect/test-runner.rs`
(create its output folder first; on Linux, name the output `test-runner`, without
`.exe`). The same helper serves the focused
`cargo test --offline --locked -p rocket-git --test automatic_bisect -- --test-threads=1`
suite. Outputs live in `work/mcp-fixtures/auto-bisect-*` and
`work/results/auto-bisect-0.86/`; source evidence is under `maintenance/auto-bisect-0.86/`.

Fine stash/diff selection and syntax (0.85): interactive mockup in
`mockups/fine-diff/preview.html`; focused native runner
`node tools/tests/test-mcp-fine-diff.cjs`. It uses isolated local repositories,
synthetic text, disposable HTTP profiles and the existing Git cancellation relay.
Generated evidence goes to `work/mcp-fixtures/fine-diff-*` and
`work/results/fine-diff-0.85/`. Source snapshot/report: `maintenance/fine-diff-0.85/`.

```powershell
pwsh -NoProfile -File .\tools\mcp\start-local-mcp.ps1
node tools/mcp/check-local-mcp.cjs
```

The launcher uses `target/release/rocket-mcp.exe` when present, then the preserved
bundle matching `Cargo.toml`. `-ExecutablePath` selects another verified local
build. Its normal service profile remains under `%LOCALAPPDATA%\Rocket\state\mcp`.
Acceptance tests use separate profiles inside their generated fixture folders.

## Focused tests

```powershell
cargo build --offline --locked -p rocket-git --example inspect
pwsh -NoProfile -File .\tools\tests\smoke-write-backend.ps1

# Most native feature runners require the current developer build:
.\tools\Build-Local.ps1 -Offline
node tools/tests/test-mcp-stashes.cjs
node tools/tests/features/workflow-commands-0.81/native.cjs
node tools/tests/test-mcp-branch-toolbar.cjs
node tools/tests/test-mcp-reviews.cjs
pwsh -NoProfile -File .\tools\tests\test-cli-reviews.ps1 -StrictClose
node tools/tests/test-mcp-shutdown.cjs
```

Large-repository checks first need `node tools/tests/prepare-large-repo.cjs`;
it creates fresh synthetic data and the fixture-path record in `work/`.

Check each runner's header and arguments before running it. Tests containing
`github`, `private`, or `live` can contact real services and require explicit
authorization; the cleanup does not authorize another live run. Some older test
fixtures still assume this Windows machine's Git/OpenSSH installation paths.

The six Git relays and the automated-bisect helper are generated local files,
excluded from Git and never shipped with Rocket. Rebuild them from the included
Rust sources with `pwsh -NoProfile -File tools/helpers/build.ps1` before running
tests that use `helpers/bin/`. Existing local copies are preserved by cleanup.
The diff-review relay shares the file-history read-pause implementation; the
workflows relay shares productivity's before/after gate implementation. To build
an isolated set, pass `-OutputDirectory work/helpers/my-check`.

A native runner that needs to hold a Git command uses a relay, not a hook: MCP runs
Git with `core.hooksPath=/dev/null`. After `Build-Local.ps1` a relay on PATH alone is
skipped too, because the bundle beside `rocket-mcp.exe` comes first. Use
`tools/tests/git-relay.cjs`: it compiles a relay into the run's folder, puts it first
on PATH and saves it as that run's Git choice, which MCP accepts as the installed Git.
It needs no product change, and the bundle stays where it is. A runner that restarts
the service without the relay calls `direct()` first, or MCP refuses the saved choice.
A server hook goes in while the relay holds `before:push`: MCP refuses a local receiver
with active hooks, and that check runs before Git starts. Every runner that holds Git
works this way now, so `helpers/bin/` only serves `check-tooling.cjs` and
`test-mcp-auto-bisect`. The `productivity` relay takes `before:<argument>` and
`after:<argument>`, and after a `before:` hold it writes the real Git's PID to
`<marker>.child`. The `file-history-preview` relay holds a read whose argument equals
the gate, and its marker carries both PIDs. The helper runs on Linux too, which has no
bundle: there the relay is named `git`, and real Git is the first `git` on PATH.

## Logs and cleanup

```powershell
pwsh -NoProfile -File .\tools\diagnostics\show-logs.ps1 -Detailed
```

After test processes and temporary Rocket windows have exited, generated `work/`
contents may be cleared. Preserve any failure evidence you still need first.
Never store reusable scripts, source backups or release bundles in `work/` again.
Keep `tools/`, `crates/`, `runtime/`, Cargo manifests and root documentation.

Old `work/...` evidence paths in changelogs, validation notes and coordination
history describe past runs and may no longer exist. Their historical ZIPs were
removed on 2026-11-04 after confirming the current source, reusable tools and tests
remain outside them. Archive-only experiments and patches are no longer retained.
See `maintenance/release-archive-cleanup-20261104/REPORT.md` for the local inventory.
Do not rerun historical one-time migration, archive or deletion scripts.
`node tools/maintenance/check-tooling.cjs target/release/rocket-mcp.exe` is the
small offline localhost/relay smoke check for this layout after a current build.
# Complete macOS package acceptance

Use the manual `macos-wasm-prototype.yml` workflow with `checks: complete-package`
after reserving its full 5 + 15 minute job caps plus cleanup within the agreed
compute budget. It builds and binds Lua before compiling the app, then tests the
round-tripped complete bundle via localhost MCP. See
[Mac desktop integration](../docs/MacOS-desktop-integrations.md).

On Mac, `rocket_state.native_menus` describes installed native items without fake
rectangles. `rocket_menu {target, expected_tab}` invokes an enabled Cocoa item from
the limited navigation/review allowlist; a receipt means queued, not completed.
Inspect the resulting state. Numeric IDs, direct Git mutation and external launches
are not accepted. Non-Mac hosts refuse this tool and retain existing pointer input.

The local Mac runner alone may write `rocket-terminal-shell` containing exactly
`zsh-isolated-v1\n` under its fixture root. It selects fixed `/bin/zsh -dfi` with
private history; it never enables arbitrary shell selection through MCP. The fixed
`zsh-initialize` terminal probe configures only that disposable session.
