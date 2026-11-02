# Bundled Git for Windows

Rocket can carry a separate, verified Git runtime beside its executables at
`git/2.56.0.windows.4/cmd/git.exe`. **Automatic** prefers this copy when included,
otherwise it finds Git on PATH. Explicit installed/custom choices still override
Automatic. Select **Preferences → Git → Bundled Git** to require this copy, wait
for verification, then restart. The saved bundled choice travels with the app.
System Git, PATH, global Git configuration, credentials and SSH keys are unchanged.
If the runtime is changed or an included bundle is incomplete, Rocket reports it
and does not silently switch to another Git. A developer build without any bundled
runtime still uses installed Git. No runtime files are downloaded on startup.

The reviewed package is official **PortableGit 2.56.0.windows.4, Windows x64**:
[release and source information](https://github.com/git-for-windows/git/releases/tag/v2.56.0.windows.4).
Archive: `PortableGit-2.56.0.4-64-bit.7z.exe` (60,087,472 bytes).
SHA-256: `b1f3e52216e2ed6882d1bebfd4d3fdf319bec40d2edbbf9fd26ec4fd3fce7ed1`.
Its Authenticode signature was valid for Johannes Schindelin, with a Microsoft
timestamp. The user reported a clean Malwarebytes scan on 2026-10-30 before use.

The initialized payload contains **9,612 files, 411,449,437 bytes**. The complete
inventory is compiled into Rocket from `crates/rocket-git/src/managed_git.manifest`;
its SHA-256 is `94395c3c041d75fd66b5d768844e8545fc35010e368537aaede718eaf4e7ee96`.
Keep the vendor `LICENSE.txt`, `README.portable`, and all included notices. The
runtime's licenses and corresponding-source information accompany the official
release; Git itself is GPL-2.0. Rocket remains Rust; Git's bundled credential helper
and other vendor tools are separate executables. The vendor's Windows credential
helper targets .NET Framework 4.7.2; this task installs no system .NET software.
Git LFS 3.8.1 is included under the current `ucrt64` layout.

Preparation uses the official portable initialization in an isolated profile.
Machine-specific Windows hosts/services copies are excluded. The bundle-local
`etc/gitconfig` selects `manager` directly, avoiding the portable helper-selector's
prompt to change a global preference. No user configuration is copied into it.
Its MSYS virtual `mtab` is a regular encoded link file, not a Windows reparse point.

For developers, the prepared payload belongs in `runtime/git/2.56.0.windows.4/`,
which is ignored by Git. Do not copy an existing personal Git installation into
this directory. `tools/Assemble-GitRuntime.ps1 -Destination <bundle-folder>` verifies
every source file, creates a new staged copy and verifies it before publication.
It refuses to overwrite a changed or unrelated existing runtime. A matching
runtime is verified and reused. It never downloads software or installs system Git.

`Build-Local.ps1` includes this runtime when present. `Package-Release.ps1` requires
it by default; an explicit `-WithoutBundledGit` produces an installation that uses
separately installed Git. `Verify-Release.ps1` checks the pinned inventory as well
as ordinary release hashes. There is no runtime updater in this feature.
