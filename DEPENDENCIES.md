# Dependency review

## Linux window backends — 2026-11-06

`crates/rocket-ui/Cargo.toml` turns on winit's X11 and Wayland backends for
`target_os = "linux"` only:
- `x11` and `wayland` on egui-winit 0.36.2;
- `x11`, `glx` and `wayland` on glutin 0.32.3 and glutin-winit 0.5.0.

These three crates are the ones that read eframe's own `x11` and `wayland` features.
eframe's features were not used, because their weak `egui-wgpu?/…` entries make Cargo
lock the unused wgpu stack (18 more packages). The pins match the versions eframe 0.36.2
already locks and must move together with it. rocket-mcp gets them through rocket-ui.

Cargo.lock adds ten packages and no locked version changes. Existing entries only gain
dependency edges, or versioned names where a second version now exists:

| Package | License | Build script | Built for |
| --- | --- | --- | --- |
| x11-dl 2.21.0 | MIT | yes, optional pkg-config library paths | Linux |
| glutin_glx_sys 0.6.1 | Apache-2.0 | yes, generated GL bindings | Linux |
| wayland-protocols-plasma 0.3.12 | MIT | no | Linux |
| smithay-client-toolkit 0.19.2 | MIT | yes | Linux, for winit; 0.20.0 stays for the clipboard |
| calloop-wayland-source 0.3.0 | MIT | no | Linux; 0.4.1 stays |
| as-raw-xcb-connection 1.0.1 | MIT OR Apache-2.0 | no | Linux |
| getrandom 0.3.4 | MIT OR Apache-2.0 | yes | Linux, for ahash's runtime RNG through winit; 0.4.3 stays |
| r-efi 5.3.0, wasip2 1.0.4+wasi-0.2.12, wit-bindgen 0.57.1 | — | — | getrandom 0.3's UEFI and WASI targets only; never downloaded or built here |

Wayland is loaded at run time through smithay-clipboard's default `dlopen`. So building
needs neither libwayland-dev nor pkg-config. Windows and macOS compile the same crates as
before, because the features are target-specific.

In WSL, Cargo downloaded the seven Linux crates from crates.io and checked each archive
against its Cargo.lock SHA-256. `cargo audit --no-fetch --stale` (cargo-audit 0.22.2)
reported no advisories for the 295 locked packages, against the local RustSec snapshot of
2026-11-05 (1,409 advisories).

THIRD-PARTY-NOTICES.txt still describes the Windows release. A Linux package will need
notices generated for the Linux target.

The check used Ubuntu 26.04's build-essential 12.12ubuntu2.26.04.2 (gcc 15.2), rustup
1.27.1-8 and Rust 1.99.1 in WSL. None of these ship with Rocket.

This is a bounded integrity and advisory review, not proof against an undisclosed
compromise.

## Portable Windows Visual C++ runtime — 2026-11-03

The package now carries Microsoft x64 `vcruntime140.dll` 14.51.36247.0 beside
the executables. Five Rocket binaries import it; the older ZIP
silently depended on an installed system copy. The reviewed DLL is reused from
the existing Visual Studio redistributable directory, with its exact hash,
Microsoft signature and timestamp checked before copying. No download, system
installation or Cargo dependency is involved. The Lua worker links the runtime
statically because its sandbox stages only the EXE; the 0.89.3 hosts bind that
rebuilt worker's hash. Provenance,
redistribution references and servicing responsibility are recorded in
[runtime/vc/README.md](runtime/vc/README.md). Existing versioned bundles remain
unchanged; Windows 11 testing uses the corrected 0.89.3 package.

## Optional pre-push OSV check — 0.89.0

Preferences > Security > **Scan dependencies before pushing** reuses the same
selected, hash-verified OSV-Scanner 2.7.0 and per-profile advisory data as background
checks. It adds no dependency, download or installation. The option defaults off.
Set up the scanner and advisory data through **Scanner & data** before enabling
it. Each enabled push scans committed lockfiles offline; no automatic advisory
download occurs. Warnings require review, and scanner/data failures stop the push.
See SECURITY.md for coverage, lifecycle and persistence limits.

## Bundled Git for Windows — 2026-10-30

The new Windows x64 bundle includes official PortableGit **2.56.0.windows.4**,
the latest stable release verified on 2026-10-30. It is a separate helper runtime,
not a Cargo dependency or system-wide installation. The official archive SHA-256
is `b1f3e52216e2ed6882d1bebfd4d3fdf319bec40d2edbbf9fd26ec4fd3fce7ed1`;
its Johannes Schindelin Authenticode signature and Microsoft timestamp verified,
and the user reported a clean Malwarebytes scan before its original extraction.
The resumed implementation reused only a preserved copy matching all 9,612
prepared-file hashes and rechecked the Git executable's publisher signature.

The complete inventory is pinned in `crates/rocket-git/src/managed_git.manifest`.
Verification runs on a cancellable worker before use; verified files stay open
without write/delete sharing while their Git client lives. Repository requests
reuse that verified client. Explicit installed/custom Git retains its existing
trust model and Windows HTTP version gate. No automatic runtime updater is added.

Vendor licenses and notices remain in the complete runtime. Preparation details,
sizes and source links are in [runtime/git/README.md](runtime/git/README.md).
Publisher release: https://github.com/git-for-windows/git/releases/tag/v2.56.0.windows.4.
Hashes, signatures and a malware scan are bounded checks, not proof against an
undisclosed vulnerability. Existing versioned Rocket releases remain unchanged.

## clipboard-win yanked-resolution refresh — 2026-10-29

The transitive Windows clipboard package is resolved from yanked
`clipboard-win` 5.4.1 to non-yanked 5.4.3 under arboard 3.6.1's existing
compatible requirement. No manifest, feature, API, package count or application
version changed. Cargo used the archive already present in the official crates.io
cache, so this task downloaded and installed nothing. Its SHA-256,
`052af7878466feccda12ba7a6a67d019d712f296809559b47c12c8cc196d3a9a`,
matches the crates.io index checksum. The package declares BSL-1.0, has no build
script (`build = false`) and retains only `error-code` 3 as Rocket's normal
dependency; the optional monitor feature is not enabled.

The reviewed 5.4.1-to-5.4.3 source delta replaces an unchecked raw-pointer copy
in HTML-fragment clipboard parsing with a bounds-checked slice and adds a
malformed-offset regression test. It also changes `Clipboard::new_attempts`
retries from a zero-duration yield to a 5 ms sleep. Rocket reaches this package
through eframe -> egui-winit -> arboard; the current egui clipboard adapter reads
and writes text rather than calling arboard's HTML getter, so the faulty HTML path
was linked but no Rocket call site to it was found. Removing the yanked package
still eliminates the latent unsafe implementation.

Offline locked reverse-tree and feature checks resolve only 5.4.3 with its
expected default/std features. `cargo audit --no-fetch --stale` passed with zero
warnings against the local 1,381-advisory RustSec snapshot dated 2026-10-28, and
offline locked all-target `cargo check` passed for rocket-ui and rocket-mcp. This
is a bounded integrity/source/advisory review, not proof against an undisclosed
supply-chain compromise. No release binary was rebuilt because another owner
retained the shared release/MCP slot.

## Lua integration worker — 2026-10-28

The user approved exact official crates.io packages and Malwarebytes instead of
disabled Defender. The user reported the prepared archives and extracted source
folder clean, then all archive/source hashes were rechecked before compilation.
Five registry entries were added: mlua 0.12.2, mlua-sys 0.13.0, lua-src 551.1.0,
bstr 1.13.1 and rustc-hash 2.1.3. No system Lua or new tool was installed.

Official registry checks covered exact names, owners, creation/publication dates,
download counts, yanked state and source repositories. mlua/mlua-sys/lua-src share
maintainer khvzak; bstr is maintained by BurntSushi and rustc-hash by rust-lang-owner.
The five archives match registry SHA-256 checksums. All 624 archive source files
were hashed; cached files used for building match the reviewed archives. OSV's
exact-version responses listed no advisories. This is bounded supply-chain review,
not proof against an undisclosed compromise. Detailed evidence and hashes are in
`tools/maintenance/lua-integrations-20261028/dependency-audit.json`.

mlua uses lua55 with default features disabled. mlua-sys uses external linking;
Rocket's first-party build script compiles only the approved lua-src 551.1.0 C
sources. No LuaJIT sources, executable download or build-time network operation is
needed. Source archives have no applicable Authenticode signature; no third-party
Windows binary was downloaded. Generated notices include all five packages and
the Lua source license.

Lua's [official bug list](https://www.lua.org/bugs.html) has four known 5.5.1 issues.
The exposed API omits coroutine/metatable/collectgarbage controls used by two
documented reproductions. NaN loop and negative-zero arithmetic semantics remain
upstream limitations; the runtime is not claimed fully patched. Instructions,
Lua memory, native data conversion, output and process time are independently
bounded. Since 2026-10-29, the native host additionally launches a hash-bound worker
inside a Windows LPAC with no capabilities and a bounded job. This uses installed
Windows APIs and adds no dependency. See [runtime limits](INTEGRATIONS.md#runtime-and-safety-limits).

Sources: [mlua](https://crates.io/crates/mlua/0.12.2),
[mlua-sys](https://crates.io/crates/mlua-sys/0.13.0),
[lua-src](https://crates.io/crates/lua-src/551.1.0),
[bstr](https://crates.io/crates/bstr/1.13.1),
[rustc-hash](https://crates.io/crates/rustc-hash/2.1.3).

## License files and third-party notices — 2026-10-25

No package, feature, tool or download was added, and `Cargo.lock` is unchanged. Each
crate's manifest now declares `license = "MIT OR Apache-2.0"` through the workspace.
`LICENSE-APACHE` is byte-identical to apache.org's Apache License 2.0 text (SHA-256
`cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30`), copied from the
`kurbo` 0.13.1 package already in the local Cargo registry. `THIRD-PARTY-NOTICES.txt`
is written by `crates/rocket-ui/examples/third_party_notices.rs`, which runs
`cargo tree` and `cargo metadata` with `--locked --offline` for x86_64-pc-windows-msvc
and reads the license files of the packages already on disk; it uses only the existing
`serde_json` dependency. Crates that ship no license file get the standard text of
their license with their declared authors. OSV-Scanner is not listed: it is not shipped.

## Security minimum update — 2026-10-24

The parallel Git-update chat installed Git for Windows **2.56.0.windows.3** with
Git LFS **3.8.1**; both versions were rechecked live after the update completed.
Rocket source now requires **3.8.1**, whose [official security release](https://github.com/git-lfs/git-lfs/releases/tag/v3.8.1)
fixes CVE-2026-69260, CVE-2026-69261, CVE-2026-105802 and CVE-2026-105803.
The older 3.7.1 minimum and advisory review below are historical. This security
remediation adds no Rust dependency and performs no installation. The selected
`cmd/git-lfs.exe` launcher has a valid Authenticode signature; the bundled
`ucrt64/bin/git-lfs.exe` runtime reports NotSigned. Exact versions, paths and
hashes are recorded in `work/security-remediation-20261023/git-lfs-evidence.json`.

Review date: 2026-10-19.

## Executable file icon (0.66.0) — 2026-10-23

No package, feature, tool or download was added, and `Cargo.toml` and `Cargo.lock` are
unchanged. rocket-ui has a new first-party build script, which rocket-mcp's build script
includes. It uses only the standard library, starts no process and uses no network: it
reads the committed `crates/rocket-ui/src/rocket.ico`, writes a Windows resource file into
Cargo's `OUT_DIR` and passes it to the MSVC linker the toolchain already uses
(`cargo:rustc-link-arg-bins`), so no Windows SDK tool such as rc.exe runs during the build.
Its output was checked once, byte for byte, against what the Windows SDK's rc.exe
10.0.26100.0 compiles from the same file. The `.ico` is written by
`crates/rocket-ui/examples/file_icon.rs` with the existing `image` PNG encoder.

## 0.57.0 WebP image comparisons — 2026-10-23

Adds one package, `image-webp` 0.2.4, through the existing exact `image = 0.25.10`
dependency's `webp` feature (`webp = ["dep:image-webp"]`). Cargo.lock gains only its entry
(checksum 525e9ff3e1a4be2fbea1fdf0e98686a6d98b4d8f937e1bf7402245af1909e8c3) and its line in
image's dependency list, besides the workspace versions. It was resolved offline from the copy
fetched for review (see 0.56.0), so nothing else was downloaded, added or upgraded. Its normal
dependencies, byteorder-lite 0.1.0 and quick-error 2.0.1, were already locked.

Approval and checks. Microsoft Defender cannot scan on this computer (it is turned off and
Malwarebytes is the active antivirus), so the user scanned the downloaded archive and its
unpacked copy with Malwarebytes (clean) and then approved using the package. The coordinating
session's supply-chain review reported: the archive's SHA-256 matches crates.io and the
unpacked copy matches the archive; the publisher, fintelia, is also an owner of `image`, whose
locked 0.25.10 declares `image-webp` 0.2 for this feature; all 18 non-generated files are
byte-identical to the Git blobs of GitHub tag v0.2.4 (commit
08b57d473c90148dd4b37a075df365afc3dfcc2b); OSV lists no advisories for image-webp,
byteorder-lite or quick-error. My own read-only check of the unpacked package agrees on the
build: `build = false` and no build script, `#![forbid(unsafe_code)]` and no `unsafe`, and no
process, network, file, environment or FFI use; its dev-dependencies (paste, png, rand, webp)
are not part of Rocket's build. WebP input is untrusted like other images: the viewer's limits
(16 MiB file, 4,096 pixels per side, 8 megapixels, 64 MiB decoder allocation) apply, and
decoding stays serialized off the UI thread and cancellable. To remove it, drop `"webp"` from
the image features in `crates/rocket-ui/Cargo.toml` and the image-webp entry from Cargo.lock.

## 0.56.0 TGA image comparisons — 2026-10-23

The UI enables the `tga` feature of its existing exact `image = 0.25.10` dependency. That
feature has no dependencies of its own (`tga = []` in the package's manifest), so no package
was added, upgraded or installed for it, and `Cargo.lock` changes only the workspace versions
(`--locked` builds pass). The TGA decoder is part of the image source reviewed for 0.18.0
below; the viewer's limits apply to it unchanged (16 MiB file, 4,096 pixels per side,
8 megapixels, 64 MiB decoder allocation), and decoding stays serialized off the UI thread and
cancellable.

WebP would need one new package, `image-webp` 0.2.4 (crates.io, published 2025-08-27; sole
owner fintelia; repository github.com/image-rs/image-webp; MIT OR Apache-2.0; normal
dependencies byteorder-lite and quick-error, both already locked; no RustSec page). With the
user's approval it was downloaded once by `cargo fetch` into Cargo's cache, and the archive's
SHA-256 matches crates.io's (525e9ff3e1a4be2fbea1fdf0e98686a6d98b4d8f937e1bf7402245af1909e8c3).
It is not used: the required Microsoft Defender scan cannot run on this computer (Defender is
turned off; Malwarebytes is the active antivirus), so Rocket's manifests and lockfile do not
reference it and nothing has built it. WebP waits for the user's decision. To remove the
download, delete `%USERPROFILE%\.cargo\registry\cache\index.crates.io-1949cf8c6b5b557f\image-webp-0.2.4.crate`
and the folder `%USERPROFILE%\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\image-webp-0.2.4`.

## 0.48.2 standalone CLI — 2026-10-23

The new first-party rocket-cli uses rocket-git plus the existing exact
`serde_json = 1.0.151` resolution. No package was added to the registry dependency
set, upgraded, downloaded or installed. All 271 external Cargo.lock records match
the pre-change snapshot. CLI's transitive runtime packages remain serde_core
1.0.229, itoa 1.0.18, memchr 2.8.3 and zmij 1.0.23.

`work/cli-dependency-audit.json` verifies all five cached archives against their
locked SHA-256 checksums and all 206 extracted source files against those archives.
This is an integrity recheck of the previously reviewed pinned cache, not a fresh
online advisory audit or proof of no undisclosed compromise. The CLI adds no
argument-parser or signal library. Ctrl+C/Ctrl+Break use Windows kernel32 and
the backend's existing process ownership/cancellation. No runtime DLL is bundled.

## 0.48.0 spell checking — 2026-10-23

No third-party package was added, upgraded, downloaded or installed. The lockfile diff
adds only the local `rocket-spell` package and its `rocket-ui` dependency line, besides
the workspace version changes. `rocket-spell` is first-party and uses only the standard
library and Windows system DLLs: `ole32` (`CoInitializeEx`, `CoCreateInstance`,
`CoTaskMemFree`, `CoUninitialize`) and `kernel32` (`GetUserDefaultLocaleName`). It calls
the Windows Spell Checking API (Windows 8 and later) through hand-declared COM
interfaces; their GUIDs and method order were checked against the installed Windows SDK
header `um/spellcheck.h` (10.0.26100.0). Every COM object lives on one worker thread
that initializes COM for itself, and every returned string is freed with
`CoTaskMemFree`. Spell checking is off by default and not offered on other platforms.

## 0.46.1 terminal interrupt packaging — 2026-10-22

No third-party package was added, upgraded, downloaded or installed. The lockfile
diff contains only the five local workspace version changes. The existing
`rocket-terminal` build script now also compiles `src/host.rs` with Cargo's Rust
compiler and packages `rocket-terminal-host.exe` beside app/test/example binaries.
This first-party helper uses only the standard library and Windows APIs; its
release binary is 141,312 bytes on this x64 build. It clears the inherited
Ctrl+C-ignore attribute in its own process, registers a non-inherited handler to
keep itself alive, and starts the fixed System32 PowerShell executable in the
existing terminal job. It does not alter the parent console policy or search PATH.

The helper is part of the trusted application directory, as are Rocket's existing
executables. Missing helper files produce a startup error. The original ConPTY
source/package payloads are unchanged; the release hash inventory now also names
the first-party helper. This is a scoped dependency-change check, not a fresh
audit of every existing package. See `VALIDATION.md` for the reproduced Windows
inheritance behavior, native keyboard routing and focused runtime evidence.

## 0.26.0 LFS locking — 2026-10-19

No Rust package, executable or runtime was downloaded or added. The backend remains
standard-library-only; existing Git LFS discovery, minimum version and bounded
process supervision are reused. A bounded JSON reader handles only lock responses;
duplicate keys, invalid UTF-8/surrogates, excessive nesting and inconsistent lock
identities are rejected. A second successful ownership query catches the helper's
JSON-only error-path limitation. Windows uses the native file-information API solely
to reject read-only hard links before a possible permission change.

Reviewed the official Git LFS 3.7.1 [lock-list implementation](https://github.com/git-lfs/git-lfs/blob/v3.7.1/commands/command_locks.go),
[locking/cache behavior](https://github.com/git-lfs/git-lfs/blob/v3.7.1/locking/locks.go),
and [unlock command](https://github.com/git-lfs/git-lfs/blob/v3.7.1/commands/command_unlock.go)
as protocol/lifecycle references. External source was read as data, not executed.
No Fork code was used. The first-party loopback fixture uses Node built-ins only.

## 0.25.0 Activity viewer — 2026-10-19

The UI now directly uses the existing `serde_json = 1.0.151` resolution that MCP
already used. Its numeric/search dependencies remain `itoa 1.0.18`, `zmij 1.0.23`
and `memchr 2.8.3`. All 271 external lockfile package records are unchanged; the
only dependency-graph change is the new local UI-to-existing-parser edge.

Rechecked the four cached archives against their pinned checksums and all 179
extracted source files against those archives. Fresh official crates.io metadata
reports matching checksums and non-yanked versions. The current
[RustSec tree](https://github.com/RustSec/advisory-db/tree/999ec89d5d3f10f20be14750ec8c2d8b44e0ac9b)
contains no matching package advisory paths. This is a bounded provenance/advisory
check, not proof of no undisclosed compromise. No package was downloaded or installed.
See `work/activity-dependency-audit.json` and `work/audit-activity-dependencies.cjs`.

The reader accepts only the existing five bounded JSONL files, bounds each record,
validates schema/session/sequence fields and projects known scalar diagnostic
fields. It excludes raw payload fields; it is not a general-purpose log-file opener
or a sanitizer for arbitrarily edited messages. Reads run off the render thread.

## 0.24.0 LFS uploads — 2026-10-19

Uses the existing standard-library backend, bounded process runner and installed
Git LFS helper. No dependency was added, upgraded or downloaded. The runtime and
launcher hashes/signatures match the prior audit, both Authenticode Valid; evidence
is in `work/lfs-push-helper-audit.json`. External lockfile records are compared
against the pre-change checkpoint by `work/record-lfs-push-review.cjs`.

The installed version's official [push contract](https://github.com/git-lfs/git-lfs/blob/v3.7.1/docs/man/git-lfs-push.adoc)
and [command implementation](https://github.com/git-lfs/git-lfs/blob/v3.7.1/commands/command_push.go)
support `--object-id --stdin` with a named remote. Rocket sends only the reviewed
unique IDs. It does not invoke a ref/history scan or a Git ref push. Forced stdout
progress is documented in the [configuration reference](https://github.com/git-lfs/git-lfs/blob/v3.7.1/docs/man/git-lfs-config.adoc).
Only recognized numeric progress reaches the UI; raw output stays out of ordinary
diagnostics. Existing custom endpoint/storage/transfer restrictions still apply.
This is an unchanged-helper and contract review, not a claim of no vulnerabilities.

## 0.23.0 LFS tracking rules — 2026-10-19

No new dependency is needed. Attribute parsing, bounded reads, cancellation and
guarded recovery/publication use Rust's standard library and existing Rocket
filesystem primitives. Git LFS is used for its existing availability check; track
and untrack do not launch its mutating commands or install filters.

All **271** external lockfile package records match the 0.22.1 checkpoint.
The installed Git LFS runtime and launcher retain the recorded hashes and valid
signatures (`work/lfs-tracking-helper-audit.json`). No package was downloaded.

Contracts reviewed in the official [track documentation](https://github.com/git-lfs/git-lfs/blob/v3.7.1/docs/man/git-lfs-track.adoc),
[untrack documentation](https://github.com/git-lfs/git-lfs/blob/v3.7.1/docs/man/git-lfs-untrack.adoc)
and [Git attributes reference](https://git-scm.com/docs/gitattributes).
Git LFS track can touch matching files; Rocket confines this workflow to the
reviewed root attributes edit. Nested and higher-precedence attributes still apply.

## 0.22.1 LFS authentication compatibility — 2026-10-19

No package was added, downloaded or upgraded. The five workspace versions change
from 0.22.0 to 0.22.1; all external lockfile blocks remain identical. The installed
Git LFS runtime and launcher hashes/signatures were checked again and match the
0.22.0 audit below (`work/lfs-auth-helper-audit.json`). This is an unchanged helper
inventory check, not a claim of absence of all vulnerabilities.

Authentication behavior was traced in the installed version's official
[endpoint finder](https://github.com/git-lfs/git-lfs/blob/v3.7.1/lfsapi/endpoint_finder.go),
[endpoint conversion](https://github.com/git-lfs/git-lfs/blob/v3.7.1/lfshttp/endpoint.go),
[access modes](https://github.com/git-lfs/git-lfs/blob/v3.7.1/creds/access.go) and
[authentication contract](https://github.com/git-lfs/git-lfs/blob/v3.7.1/docs/api/authentication.md).
Git LFS writes `lfs.<endpoint>.access` when it learns an authentication mode; this
is separate from credentials. Rocket accepts only documented mode values at
credential-free HTTPS endpoints and only permits changes to the reviewed
endpoint's record during verification. Git/SSH continue to own authentication.
The isolated fixture pins GitHub's published Ed25519 host fingerprint from its
[official documentation](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints).

## 0.22.0 Git LFS and debug profiles — 2026-10-19

No package was added, downloaded or upgraded. The backend still uses only Rust's
standard library. The existing Git for Windows helper was inspected in place:

- Path: `C:\Program Files\Git\mingw64\bin\git-lfs.exe`.
- Reported version: `git-lfs/3.7.1 (GitHub; windows amd64; go1.25.1; git b84b3384)`.
- SHA-256: `690F49B6AAA4D9DD707E08419D18F11AC2CB18051AA6779A6BADFF7D7C4EE95A`.
- Windows Authenticode status: **Valid**, signer **GitHub, Inc.**
- The selected Git directory also provides its signed launcher at `C:\Program Files\Git\cmd\git-lfs.exe` (46,920 bytes). Its SHA-256 is `7B7971DD13F0C3A284E538601F2F9770B3A87DFACCB5FB52D68141C67ED22364`, Authenticode **Valid**, signer **Johannes Schindelin**. Rocket resolves that adjacent launcher first; it reports the same Git LFS 3.7.1 runtime.

The official [Git LFS advisory](https://github.com/git-lfs/git-lfs/security/advisories/GHSA-6pvw-g552-53c5)
identifies the link-handling fix in 3.7.1. Rocket enforces that minimum and adds
its own path pins, no-replacement publication and content verification. This review
does not establish absence of all vulnerabilities or authenticate arbitrary future
helper replacements. Existing credentials are left to the installed Git/LFS stack;
this feature's acceptance run uses local repositories and no real credentials.
The project's [published advisory list](https://github.com/git-lfs/git-lfs/security/advisories)
was also checked on the review date; its newest listed advisory was that link-handling issue.
All 271 external Cargo.lock package blocks remain identical to the 0.21.0 checkpoint.

Contracts were checked against official [fetch](https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-fetch.adoc),
[checkout](https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-checkout.adoc),
[pointer decoding](https://github.com/git-lfs/git-lfs/blob/v3.7.1/lfs/pointer.go),
[endpoint alias resolution](https://github.com/git-lfs/git-lfs/blob/v3.7.1/lfsapi/endpoint_finder.go) and
[Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html) documentation/source.
Rocket uses an independent safe publication path for a reviewed single-file checkout.

## 0.19.0 external tools — 2026-10-18

This feature uses Rust's standard library and Rocket's existing process-tree,
file-containment, diagnostics and conflict-save primitives. No package was added,
upgraded or downloaded. Only the five workspace package versions changed in
Cargo.lock. External applications are chosen explicitly by the user; Rocket does
not install, download or discover a program from repository configuration.
The acceptance helper is compiled from the checked-in Rust example source and
runs only against temporary repositories. Third-party tool compatibility must be
verified for each chosen tool's own wait/new-instance arguments.

## 0.18.0 word highlights and image comparison — 2026-10-18

The UI directly pins `unicode-segmentation = 1.13.3` and `image = 0.25.10`
(default features disabled; selects PNG, JPEG and BMP). Both were already in the
lockfile/cache. The JPEG feature now connects image to **zune-jpeg 0.5.15** and
**zune-core 0.5.3**, which were also already locked transitively. No external
package was added or upgraded, and no package archive, runtime or font was
downloaded. All 271 external versions/checksums match the checkpoint. Cargo
feature unification retains other existing framework codec dependencies; Rocket's
viewer explicitly permits only PNG/JPEG/BMP decoding. The Git backend still has
no third-party dependencies.

The [audit record](work/diff-review-dependency-audit.json) and
[reproduction script](work/audit-diff-deps.cjs) cover image, png 0.18.1,
unicode-segmentation and both zune packages. Checks include current crates.io
publisher/owner and non-yanked status, cached archive SHA-256 against registry
checksums, and every packaged source file against its installed cache contents.
The two zune packages contain no build script or procedural macro. Zune's
optimized codec contains upstream unsafe SIMD code; Rocket's UI continues to
forbid unsafe Rust in its own source. This is a bounded dependency review, not
a guarantee against unknown supply-chain attacks or codec vulnerabilities.

The complete [RustSec advisory tree](https://github.com/RustSec/advisory-db/tree/999ec89d5d3f10f20be14750ec8c2d8b44e0ac9b/crates)
at the recorded revision has two image advisories:
[RUSTSEC-2019-0014](https://rustsec.org/advisories/RUSTSEC-2019-0014.html)
(fixed in 0.21.3) and
[RUSTSEC-2020-0073](https://rustsec.org/advisories/RUSTSEC-2020-0073.html)
(fixed in 0.23.12). Neither applies to 0.25.10; no listed advisory in that snapshot
matches the other reviewed package names. This is not a new full-workspace audit.

Image input is untrusted. The integration uses the crate's documented
[decoder limits](https://docs.rs/image/0.25.10/image/struct.Limits.html), plus
16 MiB encoded-file, 4096-side, 8,388,608-pixel and graphics-device limits.
Decoding is serialized off the UI thread; cancellation and stale-result checks
surround codec work. No external image renderer, Git filter, textconv, checkout
or automatic object download is invoked. See `UI.md` for exact memory/cancellation
limits and `VALIDATION.md` for malformed-input and native UI evidence.

## 0.17.0 patch tools — 2026-10-18

No external package, runtime or font was added, upgraded or downloaded. The Git
backend remains free of third-party dependencies. The patch planner uses the
installed Git helper in a disposable repository, Rust standard-library parsing
and filesystem operations, and the existing guarded Windows file-publication
and recovery implementation. Template hooks and sandbox attribute conversions
are disabled. Source paths and revisions are data, never shell command fragments.
The implementation follows the official [git apply](https://git-scm.com/docs/git-apply)
and [git diff](https://git-scm.com/docs/git-diff) contracts, with a deliberately
bounded text-patch format. There is no application of untrusted patch bytes in
the real worktree and no use of `--unsafe-paths`, `--reject` or `--3way`.

## 0.16.0 whole-file recovery and diff tools — 2026-10-18

No external package, font or runtime was added, upgraded or downloaded. Recovery
uses the Rust standard library, installed Git helper and existing Windows APIs;
syntax colors use a bounded local lexer, and search uses the existing background
task and diagnostics infrastructure. The Git backend remains dependency-free.
Cargo.lock comparison against the pre-change checkpoint is recorded in
`VALIDATION.md`; workspace package versions are the only intended lockfile change.

The implementation was checked against Microsoft's
[ReplaceFileW documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)
and Git's [attributes](https://git-scm.com/docs/gitattributes) and
[cat-file](https://git-scm.com/docs/git-cat-file) documentation. Replacement uses
flags zero, retains the actual displaced file and verifies the result. Failures
retain recovery artifacts and do not attempt an automatic rollback. Custom
filters and nontrivial encoding conversions are refused rather than executed.
Only ordinary LF/CRLF rules are implemented; see `UI.md` for the supported scope.

## 0.8.0 terminal resize and stash inspection — 2026-10-16

Uses existing Rust and Git APIs. Cargo dependencies and the audited Microsoft
ConPTY runtime are unchanged. No package installation or download is needed.

## 0.7.1 terminal PSReadLine fix — 2026-10-16

No packages downloaded, added or upgraded. The terminal still uses the reviewed
Microsoft ConPTY runtime and the installed Windows PowerShell/PSReadLine modules.
Only the child environment's `PSModulePath` is removed; Windows PowerShell builds
its own matching paths. The environment is copied as UTF-16, preserving unrelated
variables, drive entries and execution-policy settings without changing the parent.
This follows Microsoft's documented intermediate-launcher behavior in
[about_PSModulePath](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_psmodulepath?view=powershell-7.6).
No policy bypass or replacement module is installed. Native tests exercise an
incompatible inherited module path, real editing/history/completion and an owned
history file; see `VALIDATION.md`.

## 0.7.0 file history and blame — 2026-10-16

Uses the installed Git helper and existing Rust standard-library/egui components.
No crates, native runtimes or fonts added or upgraded. The Git backend remains
dependency-free. Only workspace package versions advance; validation compares
all external Cargo.lock package blocks with the pre-change checkpoint.
The acceptance-only Git relay under `work/file-history-preview` is compiled from
local standard-library Rust source and is never shipped or installed on PATH.

## 0.6.0 reviewed commit operations — 2026-10-16

No dependency was added, downloaded or upgraded. Amend, cherry-pick, revert and
their recovery use the existing Git process runner, Rust standard library and
egui widgets. The reviewed Microsoft ConPTY runtime is unchanged. External
lockfile packages were compared with the 0.5.3 source checkpoint: all **271
external package blocks are unchanged**. Only the five workspace package versions
advance to 0.6.0. See `VALIDATION.md` for the evidence.

## 0.5.3 compact Changes layout — 2026-10-16

No dependency was added, downloaded or upgraded. This change uses the existing
egui scroll areas, text editor and native wheel input. The reviewed Microsoft
ConPTY runtime and all 271 external Cargo.lock package blocks remain unchanged
from the 0.5.2 checkpoint. Only workspace versions advance to 0.5.3.

## 0.5.2 terminal display columns — 2026-10-16

The terminal now uses **exactly pinned `unicode-width` 0.2.2**, with default
features disabled, and **`unicode-segmentation` 1.13.3**, already in the lockfile.
The former is the only new external Cargo package: all previous **270 external
package blocks are unchanged**, for **271** total. The segmentation pin does not
upgrade any package. `rocket-mcp` also references the local `rocket-terminal` crate.

The [unicode-width release](https://crates.io/crates/unicode-width/0.2.2) was
published on 2025-10-06 and was not yanked at review time. Crates.io metadata named
Manishearth as publisher and the existing unicode-rs maintainer set as owners.
The downloaded archive and Cargo cache both matched its registry SHA-256:

```
b4ac048d71ede7ee76d585517add45da530660ef4390e49b098733c6e897f254
```

The reviewed manifest has no build script or procedural macro. With these
features there are no additional runtime dependencies; the library is `no_std`
Unicode width logic/tables under MIT OR Apache-2.0, with MSRV 1.66. Source entry
points and width rules were inspected; no filesystem, network, process execution
or unsafe runtime code was found. Both source files matched the exact upstream
[unicode-rs commit](https://github.com/unicode-rs/unicode-width/tree/9d98411769fe13c7c18cab0b3fbbab29ba8350ea)
named by the package. These Unicode 17 tables cover width changes for variation
selectors, emoji modifiers, flags and joined sequences. See the
[width contract](https://docs.rs/unicode-width/0.2.2/unicode_width/).

The existing segmentation archive matched its lockfile checksum:
`c6f5d3c3b1bf09027a88a6bc961fc00497d651009560b5463668dc81b0fa87a8`.
Its reviewed manifest/source is `no_std`, has no build script or runtime
dependencies, and declares MSRV 1.85. The full, nontruncated official
[RustSec advisory tree](https://github.com/RustSec/advisory-db/tree/edb58f5974b83eb8302332821cb6d389777aa351)
contained no advisory directory for either package at the reviewed revision.
This is a dated advisory/source/provenance check, not a guarantee against future
advisories or undiscovered malicious code.

With explicit user approval, Windows builds also bundle the native
[Microsoft.Windows.Console.ConPTY 1.25.260930003 package](https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY/1.25.260930003).
The legacy Windows 10 host miscounted combining/emoji cursor positions even after
Rocket's cell model was fixed. The newer runtime supplies grapheme-width mode
without changing Windows or requiring .NET. The official stable NuGet archive
was fetched directly, redirects disabled, and hashed before loading. Only named
binary entries were extracted; no NuGet/MSBuild scripts ran. All six x64/x86/arm64
DLL/EXE payloads had **Valid Microsoft Corporation Authenticode signatures**.
Exact package and per-file hashes, sizes, version, license and distribution
instructions are in [the runtime provenance record](runtime/conpty/1.25.260930003/README.md)
and its manifest. x64 was run; x86/arm64 signatures were verified without executing
those architectures.

The first-party Cargo build script only copies the reviewed architecture-specific
runtime and license beside app/test/example executables. The owned dynamic loader
uses an absolute executable-relative path, restricts dependent DLL search to the
package directory and System32, and retains the module until the console closes.
It has no repository/PATH/system-runtime fallback. Audited signatures/hashes are
packaging evidence; startup trusts the app directory and does not repeat those
checks. Moving or replacing the application's executable or runtime remains
outside that trust boundary. Missing runtime files fail visibly.

The exact package archive, header, Rust source comparison and RustSec result are
retained under `work/terminal-dependency-review`. No fonts were downloaded; the
terminal prioritizes the already-installed Segoe UI Emoji face for wide clusters.

## 0.5.1 Unicode display — 2026-10-16

No external crates or fonts were downloaded, added or upgraded. The existing
egui/epaint 0.36.2 font loader reads a fixed list of installed Windows font files;
TTC collections use their first face. Font data is shared across window
reopenings. Glyph coverage is checked against real font character maps and native
captures; missing optional fonts fall back to the remaining installed/bundled set.

## 0.5.0 repository creation and identity — 2026-10-16

No external dependency was added or upgraded. Creation uses the installed Git
helper and an explicitly empty template, SHA-1 object format, files ref format
and initial branch; see [git init](https://git-scm.com/docs/git-init).
Identity updates use [git config](https://git-scm.com/docs/git-config) with an
explicit temporary file and includes disabled. Both keys are prepared under the
repository's config.lock before one atomic replacement. Global/include/worktree
files are read for effective values but never written by this editor.
Windows publication uses MoveFileExW without replacement for new metadata and
with replacement only for the locked repository config. Directory handles hold
existing creation-path components against rename during publication. These are
local filesystem guarantees, not protection from a hostile process with the same
user permissions. Existing helper/toolchain provenance is unchanged.

## 0.4.0 local HTTP service — 2026-10-16

No external package is added for the local HTTP service or its settings tab.
`rocket-mcp-control` is a local crate using Rust's standard library and Windows
CNG. Token generation uses a 32-byte buffer with `BCryptGenRandom` and the system
preferred RNG, following [Microsoft's API contract](https://learn.microsoft.com/en-us/windows/win32/api/bcrypt/nf-bcrypt-bcryptgenrandom).
The small FFI call documents its buffer and handle lifetime. Profile ownership
uses Windows file sharing restrictions; token/config writes use temporary files
and replacement. Default profiles inherit the current user's directory permissions.

The HTTP adapter uses existing `serde_json` and `std::net`, with bounded headers,
bodies, connections and deadlines. It implements the JSON response form of
[MCP Streamable HTTP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports),
with loopback binding, exact Host/Origin checks and a required bearer token.
No SSE, HTTP chunked requests or transport session IDs are implemented. These
limits are intentional for Rocket's local development control service. This
project-owned parser and lifecycle code need review and regression tests just
like the rest of Rocket; avoiding a new dependency is not a security guarantee.
Accepted sockets are explicitly returned to blocking mode on Windows before
worker I/O deadlines are applied, because [Winsock accept inherits the listener's properties](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-accept).
All 270 external Cargo.lock package blocks remain unchanged from the 0.3.0 checkpoint.

## 0.2.0 additions — 2026-10-16

No external package was downloaded, installed or added for repository refresh,
change review, versioning or the terminal. `rocket-terminal` is a local workspace
crate using Rust's standard library and documented Windows APIs. The external
dependency versions and checksums in Cargo.lock are unchanged from the checkpoint
before this work. This is dependency-change evidence, not a new assurance that all
previous dependencies are free of vulnerabilities.

The terminal uses Windows ConPTY and a kill-on-close job with suspended process
creation, plus separate pipe-reader/writer/supervisor threads. API lifetime details
were checked against Microsoft's [session guide](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session)
and [shutdown contract](https://learn.microsoft.com/en-us/windows/console/closepseudoconsole).
Explicit null standard handles prevent the PowerShell child from using MCP's stdio;
see the [Microsoft Terminal discussion](https://github.com/microsoft/terminal/discussions/15814).
The system PowerShell executable is resolved from GetSystemDirectoryW, without a
PATH search. The default no-profile session does not change any PowerShell or SSH
settings. This module is project code and needs ongoing review like the Git runner.

## Current inventory

`rocket-git` has **zero third-party Cargo dependencies**, including dev/build
dependencies and transitive dependencies. There is no `build.rs`, procedural
macro, vendored dependency, Git-sourced package, or registry-sourced package in
the backend crate. The workspace also includes `rocket-ui` and the development-only
`rocket-mcp` server; their separate dependency reviews appear below.

The implementation uses Rust's standard library, the existing Git executable,
and small, documented Windows `kernel32` interfaces for process ownership,
non-replacing clone publication, and console cancellation.
No Fork source, binary, or native library is included or loaded.
No dependency downloads, package installation, or Python execution are needed.

The existing Rust toolchain and Git installation are external prerequisites;
this review does not establish the provenance of their earlier installation.
There are no third-party crate versions in the backend slice to check against advisories.

## Native UI review — 2026-10-15

`rocket-ui` pins **eframe = 0.36.2**, with default features disabled and only
`default_fonts` and `glow` enabled. It renders a native Windows window with
egui/OpenGL. The standard library does not provide desktop widgets or a renderer;
writing this from Win32 calls would add a large custom rendering/input layer.
This framework also supports the virtual commit table, custom graph, text editing,
menus, DPI scaling, and on-demand repainting without a browser runtime.

- Upstream: [emilk/egui release 0.36.2](https://github.com/emilk/egui/releases/tag/0.36.2),
  release commit `49682f8`, published 2026-09-08. Registry metadata identifies
  publisher `rerunio`; current owners are `emilk` and `rerunio`. Historical ownership
  has not been independently reconstructed. Its declared minimum Rust is 1.95;
  the backend retains its separate 1.89 minimum.
- All **114 registry packages in the Windows metadata graph** were already in
  the local cache. Offline metadata resolution and all compilation used that cache;
  no crate, library, installer, or external binary was downloaded for this change.
  The lockfile additionally records target-specific packages for other platforms;
  those platforms are not approved or tested by this Windows review.
- SHA-256 hashes of all 114 cached archives matched the pinned lock checksums.
  Regular files in the extracted sources matched the archives. The eframe archive
  checksum also matched a fresh crates.io API response:
  `55ad41fac6e3149abb4866e93df37dbd33baa95db525c3368c944584e95a5dc2`.
- Advisory check: local RustSec snapshot
  `7eebec69c352c7191b1f13eb95dd510eeca5d1de`, plus a current GitHub API comparison
  to RustSec `main`. The only changed advisory for the selected Windows package
  names was a zlib-rs source-link correction, with no affected-version change.
  Fifteen historical advisory records matched package names; installed versions
  were in patched ranges, except the withdrawn xml-rs maintenance notice.
  [Upstream's security page](https://github.com/emilk/egui/security) had no published
  advisories. No applicable advisory was identified by these checks; this is not
  proof of absence of malicious code, undisclosed vulnerabilities, or compromise.
- Static review covered build-script entry points, process/filesystem/network
  patterns, proc-macro entry points, selected features, and native boundaries.
  Build scripts probe the Rust compiler, generate bindings/tables in `OUT_DIR`,
  and select platform link settings. Proc macros generate Rust code; the
  document-features macro reads the invoking package manifest. Windows import
  libraries, OpenGL driver loading, fonts, clipboard support, unsafe rendering and
  font/image dependencies remain trust boundaries. This was a focused source review,
  not an exhaustive audit of every dependency's unsafe code or its maintainers.
- Browser-link launching, persistence, wgpu, WebAssembly, and optional native
  AccessKit integration are not enabled. AccessKit's Windows bridge was not cached;
  screen-reader support is therefore not claimed for this first build. Keyboard
  and pointer support remain available. No arbitrary repository image decoding,
  remote avatar download, telemetry, or automatic update client was added.
- `rocket-ui` forbids unsafe code. Its only direct external dependency is eframe;
  `rocket-git` remains the sole Git operation implementation. Installed Git and its
  configured hooks, filters, signing and authentication helpers remain external
  execution boundaries, as before.

Evidence: `work/ui-dependency-review/{Cargo.lock,metadata.json,cache-review.json,
eframe-registry.json,rustsec-update.json,review-cache.cjs}`. The script is authored
locally and checks cached archives as data. Keep the lockfile pinned; review feature,
version, target, or ownership changes before widening this set. The approval rule
below still applies to any future **new package download**.

The native HTTPS/SSH UI acceptance runner adds no packages and leaves Cargo.lock
unchanged. Its probe uses the already reviewed eframe dependency and a std-only
BMP writer shared with the existing native probe. PNG conversion uses installed
Windows System.Drawing. Transport setup reuses installed Git, Git Credential
Manager and Windows OpenSSH. No helper or executable is downloaded. The isolated
SSH host-key pin was rechecked against [GitHub's published fingerprints](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints)
on 2026-10-15; strict verification remains enabled. This checks the configured
transport behavior, not the supply-chain provenance of those preinstalled tools.

## Persistent MCP test server review — 2026-10-15

`rocket-mcp` pins `serde_json = 1.0.151` and `png = 0.18.1`, alongside the already
reviewed `eframe = 0.36.2` and the local UI/backend crates. PNG encoding reuses the
existing GUI dependency graph. No MCP SDK, HTTP stack, service installer or new
native interface was added. MCP framing, dispatch and queues use Rust's standard
library; project-owned server code forbids unsafe code.

Four cached packages are newly active in the Windows build: `serde_json 1.0.151`,
`itoa 1.0.18`, `zmij 1.0.23` and `memchr 2.8.3`. The first three add lockfile entries;
memchr was already locked for another target and was separately reviewed when the
Windows metadata comparison identified its activation. All compilation used the
existing cache with `--offline --locked`; no package download or installation occurred.

- Fresh official crates.io metadata reports these exact versions as not yanked.
  All four archive SHA-256 checksums match that metadata and the lockfile. All 179
  extracted regular files match their cached archives (89, 14, 17 and 59).
- Registry metadata identifies dtolnay as publisher of serde_json/itoa/zmij and
  BurntSushi as publisher of memchr. Current owners were inspected, but historical
  ownership and prior installation provenance were not independently established.
- Static review covered package/build-script entry points and process, filesystem,
  network and unsafe-code patterns. serde_json's build script selects arithmetic
  configuration from the target architecture; zmij's probes the configured rustc
  version. itoa and memchr have no build script. Numeric conversion, byte search,
  SIMD and pointer code remain dependency trust boundaries; this was not an
  exhaustive audit of every unsafe operation.
- The local RustSec snapshot `7eebec69c352c7191b1f13eb95dd510eeca5d1de` was compared
  with fresh upstream `edb58f5974b83eb8302332821cb6d389777aa351`. None of the 192
  changed files concerned these four packages; no advisory folders for them were
  present in the local snapshot. No applicable advisory was found by this check.
  This does not establish absence of undisclosed compromise or vulnerabilities.

Evidence is in `work/mcp-dependency-review/`: official registry responses,
`cache-integrity.json`, `metadata.json`, `rustsec.json` and the locally authored
`check-cache.cjs`. The stdio acceptance client and lifecycle runner use Node's
built-in modules only. They execute authored fixture scripts and installed tools;
they do not download or execute external repository code.

## Clone dependency and native-interface review

The clone slice adds **zero crates, build scripts, downloads, or package installs**.
It reuses the standard-library runner, `WriteTask`, progress parser, and diagnostics.
Two native imports were added and reviewed against Microsoft's documented ABI:
[`MoveFileExW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
uses live NUL-terminated UTF-16 buffers and flags zero to disallow replacement and
cross-volume copying; no replacing `std::fs::rename` fallback is used.
[`SetConsoleCtrlHandler`](https://learn.microsoft.com/en-us/windows/console/setconsolectrlhandler)
registers a static callback with the system ABI; it only stores an atomic flag for
Ctrl+C/Break, and the CLI polls that flag to cancel its owned task. The handler is
removed after report delivery. This code does not install a system-wide handler.
Other platforms refuse clone publication before creating a workspace.

Clone uses the installed Git's documented
[`clone` options](https://git-scm.com/docs/git-clone), including `--no-local` for
independent object transfer, `--progress`, an empty template, and no recursive
submodules. Invocation-only hook overrides and a relative local worktree setting
prevent the tested inherited template/hook/worktree redirections. Configured
filters, Git transports, authentication helpers, and SSH agents remain external
execution boundaries. Their binaries are not audited by the Cargo inventory.
All new automatic acceptance tests use fresh local fixtures and file-only transport;
the smudge helpers are small scripts generated by our tests, not downloaded code.

Clone intersects the inherited `GIT_ALLOW_PROTOCOL` list with `file` for local
sources or `https:ssh` for URL sources, in the Git child only. This
[Git-documented allowlist](https://git-scm.com/docs/git#Documentation/git.txt-GITALLOWPROTOCOL)
also restricts URL rewrites and per-protocol disk configuration; a blanket
`protocol.allow=never` by itself does not override a configured per-protocol allow.

## Clone interruption and mockup tooling

The clone interruption acceptance example adds no dependency, native import,
build script, or package install. Its loopback relay uses `std::net` and bounded
threads/sockets; it forwards opaque TLS/SSH traffic only to `github.com:443` or
`github.com:22`. HTTP CONNECT rejects other destinations. SSH `HostKeyAlias`
preserves the published GitHub host pin when the child connects to loopback.
The installed Git/OpenSSH clients still perform authentication and encryption.
The public `ocornut/imgui` repository is downloaded only as test data and is
never built or executed. The existing private-clone script keeps normal behavior
and adds an explicit `-Interruptions` mode restricted to that public repository.

The UI mockup is a separate in-conversation preview with embedded sample data;
it adds no Rust UI framework or production dependency. Visual acceptance uses
the Codex-bundled Node/Playwright and installed Edge in an isolated headless
context, with network requests blocked. No browser package was installed and no
existing browser profile or GitHub settings were controlled. These installed
tools' provenance is not established by the zero-Cargo-dependency inventory.

## Review procedure for future dependencies

Before adding or executing an external crate:

1. Explain the need and check whether the standard library or existing code suffices.
2. Identify the exact package, version, upstream repository, release/tag, maintainers,
   and publication history. Investigate unexpected ownership or release changes.
3. Check current RustSec advisories and upstream security notices for that exact
   version and its complete normal, build, and development dependency tree.
4. Review build scripts, procedural macros, native libraries, unsafe interfaces,
   network activity, and bundled binaries before executing them.
5. Inspect the actual published package, not just the repository's current branch;
   compare its provenance and checksum, pin the reviewed resolution in Cargo.lock,
   and keep the dependency tree reviewable.
6. Present the concrete download/execution plan for approval before fetching new
   packages. A later version or ownership change requires another review.

An advisory search without findings is not proof that a package is safe.
The backend's zero-dependency choice avoids adding that external package risk
to `rocket-git`. The desktop framework is reviewed separately above.

The staging/unstaging/commit slice adds no external packages or native imports.
Its queue and background tasks use the standard library and the existing process
runner. Git hooks, clean filters, and signing programs remain configured external
executables, not Cargo dependencies; this inventory does not audit those programs.

The local-branch slice likewise adds no dependencies, build scripts, downloads, or
native imports. It reuses the existing Git runner and write queue. Git validates
branch names and performs branch/ref/worktree operations. Configured checkout
hooks and filters are external programs, subject to the same boundary above.

The stash/merge slice also uses only the standard library and existing Git runner.
No crate, build script, download, or native import was added. Git's configured merge
drivers, hooks, signing programs, and filters remain external execution boundaries;
reviewing Cargo dependencies does not audit those repository-configured programs.

Remotes/fetch adds no crates, build scripts, downloads, or native imports. Progress
uses a bounded standard-library channel and the existing pipe readers. The local
tests use the installed Git for Windows and its shell to execute fixture hooks.
On real remotes, Git can run configured transports, credential helpers, and hooks;
the zero-crate inventory does not establish the safety of those external programs.

Push likewise adds zero dependencies, downloads, native imports, or build scripts.
It reuses the standard-library runner, write gate, task lifecycle, and diagnostics.
Its Rust integration tests and `smoke-push-backend.ps1` use empty child-only Git settings
and file-only transports against newly created local repositories. Normal live
push can execute Git-configured transports, authentication/signing programs, and
hooks; these external tools are not audited by the Cargo inventory.

The separate opt-in `smoke-github-push.ps1` adds no package or download. It uses
Windows PowerShell 5.1 process/file APIs and the installed Git, Git Credential
Manager, and Windows OpenSSH tools. Source review covers child-only environment
isolation, fixture-local configuration, a get-only GCM wrapper, public-key-only
agent selection, pinned GitHub host trust, explicit single-branch pushes, and
expected-commit-lease teardown. Its local self-test verifies stale-lease rejection
before live HTTPS/SSH testing. This is validation of the runner and these installed
tools' behavior, not a supply-chain audit of their binaries. Cargo manifests and
lockfile are unchanged; `cargo tree --workspace --offline --locked` still lists
one workspace package with no dependency edges.

Fast-forward-only pull also adds no crate, build script, native import, package
installation, or download. It reuses the existing Git runner, fetch parser and
progress channel, merge arguments, write gate, and diagnostics. Pull integration
tests and `smoke-pull-backend.ps1` use child/process-only empty configuration and
file-only transports with fresh local repositories. Configured Git hooks,
filters, and authentication helpers remain external execution boundaries.
One parser-failure test compiles a generated standard-library Rust wrapper using
the installed `rustc`; it forwards to the existing Git executable and corrupts
only successful fetch stdout inside the disposable fixture. It is not a new
dependency or build script and does not download or execute external source.

The separate `smoke-github-pull.ps1` acceptance runner also adds no packages or
native imports. It follows the reviewed push runner's child-process isolation,
get-only GCM wrapper, public-key-only agent selection, pinned host trust, and
bounded process lifetime. It downloads only its generated branch's synthetic
commits, uses exact expected-commit leases for fixture writes/cleanup, and checks
the same branch workflow locally before live HTTPS/SSH acceptance. It does not
build or execute repository source from GitHub. This tests installed transport
tools' behavior; it is not a provenance audit of those binaries.

The diagnostic logging slice also adds **zero dependencies**, downloads, build
scripts, or project-owned native imports. It uses standard-library JSON formatting
of fixed labels/numeric metadata, file I/O, synchronization, clocks, and OS file
locking. The minimum Rust version is now **1.89** because that release stabilized
[`File::try_lock`](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock).
The installed Rust 1.99 toolchain supports it; the minimum toolchain itself has not
been installed or tested. Cargo.lock and the package dependency list are unchanged.

## Reproduce the inventory without network access

Rocket 0.21.0 submodule add/remove/sync adds no third-party crates, packages,
native libraries, build scripts or downloads. The backend uses the existing Git
runner, Rust standard-library file handling and reviewed Windows publication
primitive. Compared with the 0.20.0 checkpoint, all 271 external lockfile package
blocks are identical; only the five workspace versions change. This is an unchanged
dependency inventory check, not a fresh ecosystem-wide advisory audit.

```powershell
cargo tree --workspace --offline --locked
cargo metadata --format-version 1 --offline --locked
```

The workspace contains `rocket-git`, `rocket-ui`, `rocket-mcp`, `rocket-terminal`
and `rocket-mcp-control`, with the pinned
dependencies described above. `cargo tree -p rocket-git --offline --locked`
must still show only the backend package with no dependency edges. Keep builds
and tests `--offline --locked` against the reviewed cache and lockfile.

## Repository browser dependency review — 2026-10-18

Rocket 0.15.0 adds no third-party crates, packages, native libraries or downloads.
The existing Rust standard-library Git runner and already reviewed egui stack
provide the backend, background cancellation, native controls and localhost MCP.
`Cargo.lock` changes only the five workspace package versions from 0.14.0 to
0.15.0. Folder/blob reads use the installed Git helper. Relevant official command
contracts: [ls-tree](https://git-scm.com/docs/git-ls-tree) and
[cat-file](https://git-scm.com/docs/git-cat-file).


## 0.63.0 tool profiles and syntax — 2026-10-23

No dependency, grammar or external executable was downloaded or added. Named
profiles reuse the checked external-tool backend. Language-aware lexical rules
are maintained in Rocket; preparation has byte/span limits and runs with the
existing patch workers. The lockfile changes only Rocket workspace versions.

## Offline advisory scanner — 2026-10-25

Rocket's optional dependency-warning feature uses Google's OSV-Scanner **2.7.0**
Windows x64 portable binary. No Rust package, PATH entry, service or administrator
installation is added. OSV is optional and stays out of Rocket's release bundles.
Open **Edit > Preferences > Security > Scanner & data** for first-run setup
(or **Set up scanner** on the Overview):

1. Use **Get OSV-Scanner** to open Google's official release page, then obtain the
   Windows x64 executable. Rocket does not download or install that executable.
2. Use **Choose scanner** to select the file wherever you keep it. Rocket verifies
   the pinned checksum without executing it, then saves the absolute path in this
   profile's `dependency-security/scanner-selection`. A rejected or cancelled
   selection keeps the previous choice. Move/delete the file and you must choose it again.
3. Open a repository and choose **Download advisory data**. Later refreshes use
   **Update advisory data**. With background warnings enabled, scanning starts
   automatically after a successful update; **Scan dependencies** is also available.

The choice is revalidated on restart and before every helper launch. An invalid
saved choice is reported instead of silently selecting another executable.
With no saved choice, the existing automatic location remains supported:
`runtime/osv-scanner/2.7.0/osv-scanner.exe` beside Rocket (development builds also
search up to four containing directories). Each user's advisory cache is separate.

Official source: <https://github.com/google/osv-scanner/releases/tag/v2.7.0>.
The 58,999,296-byte Windows binary matches the publisher's SHA-256:
`9964ed64ec9a387391eb4cc006ad5c531154821aef29b793e02c448de4686438`.
Authenticode reports `NotSigned`. Defender was unavailable; the user reported a
clean Malwarebytes scan and explicitly approved running this unsigned,
checksum-verified binary. Evidence: `work/dependency-security-20261025/`.
Rocket checks the pinned checksum on every helper launch and retains a handle
that denies replacement until it exits. A different binary/version is refused.

Advisory data is downloaded only through **Download/Update advisory data**, using a
synthetic package name and the required public ecosystem names. The observed
2026-10-25 npm archive is about 219 MB; the crates.io archive is about 3.7 MB.
Archive sizes change over time. Data is kept under the application's state
folder in `dependency-security/data/osv-scalibr/<ecosystem>/all.zip`, with a
512 MiB bound per archive. The helper uses OSV's HTTPS data source; Rocket's
archive hashes detect local changes, not a separate publisher signature.
Scanning uses these local files and does not query an advisory API.

## Approved Git runtime security update (2026-10-29)

Git for Windows 2.56.0.windows.4 x64 is the exact approved system update for the
Brotli decoder issue, CVE-2026-111497. Official installer SHA-256:
`4961a4a8cc778d8b6e00a5819e5fe4d078b754555aff7aedc2d77a5d74c2de80`.
Publisher signature (Johannes Schindelin) and trusted timestamp were verified.
Defender was disabled and could not scan; its settings were not changed. The user
manually scanned the exact installer with Malwarebytes and reported no threats.
Installation and post-update version evidence are recorded in
`tools/maintenance/hosting-security-fixes-20261029`. Cargo dependencies are unchanged.
