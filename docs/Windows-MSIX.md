# Windows MSIX packaging

MSIX adds a Microsoft Store distribution option alongside the
[Inno Setup installer](Windows-installer.md) and portable ZIP. All three build
routes remain supported. This work is local compatibility preparation; it does
not publish a Microsoft Store listing or remove the 0.92.5 installer.

## What changes for an installed app

Windows installs the app, bundled Git and helpers in its protected package
directory. Rocket still runs as a normal, unelevated desktop application. It does
not add a service, change PATH, load SSH keys, enable MCP or sign into an account
during installation. The package is offline and includes the same dependencies
as the verified portable ZIP.

The desktop process model preserves package context for bundled Git and terminal
helpers. The verified Lua executable uses a native byte-relay outside that package
context before starting its capability-free LPAC child. The relay never parses or
executes Lua. Both processes remain in the parent's kill-on-close job. Lua inherits
only three pipes; the trusted relay also gets a wait-only stop event so it can reap
Lua and remove its native profile on cancellation. The Lua child retains its
one-process/128 MiB sandbox, with a 160 MiB combined outer job limit. No sandbox
checks are removed or relaxed.

Rocket detects Windows package identity through `GetCurrentPackageFullName`.
An MSIX process does not check GitHub for portable updates, download a ZIP or run
the directory-replacement updater. Help → Check for Updates explains the
Windows-managed installation and offers Microsoft Store's update page. A
sideloaded compatibility package is updated by installing a newer signed MSIX;
opening the Store does not turn it into a Store installation. Portable and
existing Inno installations retain their current GitHub update behavior.

The manifest disables AppData and registry write virtualization so Rocket and
its Git tools keep using ordinary user state, shared with an existing portable
installation. Settings, drafts, credentials and external repositories survive
uninstall. This needs the `unvirtualizedResources` restricted capability as well
as `runFullTrust`; Microsoft must approve these capabilities for the Store.
Installation does not import, move or delete existing user data. Avoid using
different Rocket versions against the same settings at the same time.

## Build an unsigned package

Use the already-installed Microsoft Windows SDK's signed `MakeAppx.exe`. The
packager does not download software, build Rocket, sign, install or publish.
Supply **Name and Publisher exactly as assigned by Partner Center** for a Store
submission; do not invent a production identity or use the test publisher there.
The defaults are the supplied Store identity `TwinCatz.GitRocket`, publisher
`CN=3C1B7D92-578D-491F-BAF8-AED91B184402`, publisher display name `TwinCatz`, and
product display name `GitRocket`. These public identity strings are not signing keys.

```powershell
.\tools\Package-MSIX.ps1 `
  -Archive '.\work\msix-portable\Rocket-0.92.6-windows-x64.zip' `
  -ArchiveSha256 '<reviewed ZIP SHA-256>' `
  -MakeAppx '<Windows SDK>\x64\makeappx.exe' `
  -CompareLocalSshKeys
```

Use a **new build including MSIX update detection**, not the previously published
0.92.5 ZIP. For a local candidate without replacing published files:

```powershell
.\tools\Package-Release.ps1 -Offline -OutputDirectory '.\work\msix-portable'
```

The packager derives the Store version from the reviewed portable ZIP. It adds
one to the Rocket major version, preserves minor and patch, and appends the
Store-reserved zero. Rocket 0.92.6 produces MSIX 1.92.6.0, which upgrades
1.92.5.0 under the same identity and publisher. An explicit `-PackageVersion`
must match. A disposable `-CompatibilityTest` identity can override it to
exercise Windows package upgrades with the same Rocket build.

The packager binds its input to the reviewed ZIP hash, rejects unsafe archive
paths/links, verifies the source-controlled release manifest, and audits files
for credentials and development data. Optional local SSH comparison only reads
keys; reports contain no key values. Nothing from an input ZIP is executed.
The packager checks the privacy audit against the reviewed ZIP and hashes the
generated manifest and logos as it creates them. It verifies the files inside the
finished MSIX against those hashes, then unpacks it with MakeAppx. The
build record and value-free audit remain in its unique `work/msix-build-*` folder.

## Signing and distribution

Microsoft signs accepted Store packages. A GitHub-downloadable MSIX needs an
appropriately trusted signing identity; an unsigned package is not a replacement
for the portable ZIP. Never ask ordinary users to disable signature checks or
trust a developer's test certificate. A test certificate does not establish
production publisher trust.

The local acceptance package uses a random `Rocket.CompatibilityTest.*` identity
and a short-lived, non-exportable disposable signing key. Only its public
certificate may be added temporarily to LocalMachine/TrustedPeople, with explicit
approval and administrator consent. Remove that exact trust entry and signing
key after testing. Trust requires a currently valid code-signing leaf with a lifetime
of at most three days. Cleanup identifies it by the approved thumbprint and subject,
including after expiry. The lifecycle test resolves Git and MCP from the matching
Windows package registration after installation. Do not use Trusted Root Certification Authorities, export a
PFX, enable Developer Mode or change execution policy for this test.

## Focused verification

- `node --test tools/tests/test-installer-payload.cjs`: package privacy and paths.
- `tools/tests/Test-MSIXSecurity.ps1` checks package integrity, deployment identity
  and certificate policy with disposable files and an in-memory trust store.
- `node --test tools/tests/test-msix-installed-tools.cjs` checks which registered
  installation supplies the test executables.
- `tools/tests/Test-InstallerInput.ps1 -ScriptPath tools/Package-MSIX.ps1`: metadata
  validation without executing input binaries.
- `cargo test --locked --offline -p rocket-git --lib windows_package::`:
  fail-closed package-identity classification.
- `cargo test --locked --offline -p rocket-ui --lib app_updates::`: existing
  portable update verification, cancellation and preservation checks.
- `tools/tests/test-msix-lifecycle.cjs`: two signed compatibility package build
  records and a disposable signing record; install, package activation through
  localhost MCP, bundled Git, sandbox worker, terminal, compact update UI,
  persisted draft/settings, rejected damaged upgrade, valid upgrade and uninstall.

The native runner uses disposable AppData, repositories and SSH keys. It hashes
existing Rocket state and SSH files before/after without disclosing their values.
It never imports a test key into ssh-agent or contacts a real Git hosting service.
Only the exact random test identity can be removed. A failure preserves evidence
and reports any registration or process that could not be cleaned up.

Store certification, Partner Center identity/capability approval, production
signing, and a fresh Windows 11 installation are separate acceptance steps.
Maintainers can find the capability explanations, reviewer notes and restricted-use
caveat for `unvirtualizedResources` in `docs/Microsoft-Store-submission.md` in
the source checkout.

Microsoft references: [package identity](https://learn.microsoft.com/en-us/windows/msix/detect-package-identity),
[Win32 process models](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes),
[virtualization and uninstall](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization),
[package signing](https://learn.microsoft.com/en-us/windows/msix/package/create-certificate-package-signing).
