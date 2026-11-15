# Windows installer

Rocket keeps the portable ZIP. The optional Inno Setup package wraps the same
verified files; it does not download Git or another runtime during installation.
Download Setup or the portable ZIP from [GitHub Releases](https://github.com/JasXTD/Rocket/releases/tag/v0.92.5).

## Installation and removal

The installer uses a per-user folder, normally
`%LOCALAPPDATA%\Programs\Rocket`, and needs no administrator access. It adds a
Start menu shortcut and offers an optional desktop shortcut. It does not modify
PATH, start a service, enable MCP or change Git credentials and SSH configuration.
Opening Rocket at the end is optional.

The folder layout separates the app from its uninstaller:

```text
Rocket/
  app/                         verified portable payload
  uninstall/                   uninstaller and installation identity
  .rocket-setup-previous-.../   previous app retained after a Setup upgrade
  .rocket-previous-update-.../  previous app retained by the in-app updater
```

Keep repositories and personal files outside `app`. Setup refuses a nonempty
folder it does not own. Upgrade and uninstall check every current app file
against the package manifest and refuse changed files, extra files and links.
They do not recursively delete the installation folder. Uninstall preserves
Rocket's saved settings, drafts, repositories, credentials, SSH keys and retained
recovery backups. These can therefore continue to use disk space after uninstall.

**Help → Check for Updates** continues to use the portable update ZIP. Only
`app` is swapped, so the shortcut and uninstaller stay in place. Windows' installed
apps entry shows the version of the most recent Setup install; Rocket's About
dialog shows the running version after an in-app update.

If a file is locked, close Rocket and its helpers before retrying. If unexpected
files are reported, preserve them and move personal data out of `app`. If removal
stops after withdrawing the app, inspect the retained `.rocket-uninstall-...`
folder and uninstall log before recovering; do not delete unknown contents.
Setup and uninstall write Inno logs. Logs can contain local installation paths;
review them before sharing.

Setup reports a failure exit code and a failed completion page if the staged
payload cannot be published. It does not offer to launch an incomplete install.
Staging folders are retained for inspection; choose a new empty folder if a
failed first install cannot be retried in place.

## Build locally

Use the reviewed **Inno Setup 6.7.3** compiler from
[the official release](https://github.com/jrsoftware/issrc/releases/tag/is-6_7_3).
This build reuses an existing verified installation; it does not add another tool.
The packaging script never installs tools or downloads files. It requires the
compiler's pinned SHA-256 and valid publisher signature, plus the reviewed SHA-256
of an existing Windows portable ZIP.

Run in PowerShell 7 from the project root, using actual paths and the checksum
from that ZIP's verification record:

```powershell
.\tools\Package-Installer.ps1 `
  -Archive 'tools\releases\Rocket-0.92.5-windows-x64.zip' `
  -ArchiveSha256 '<reviewed ZIP SHA-256>' `
  -Compiler '<verified Inno Setup 6.7.3 directory>\ISCC.exe' `
  -CompareLocalSshKeys
```

The script validates archive paths and size limits, uses Rocket's trusted release
verifier, validates version metadata as data, audits the exact payload and creates an explicit
Inno file list. It rechecks payload hashes after compilation. It refuses to
overwrite output. Results are a local `Rocket-<version>-Setup-x64.exe`, its
`.sha256` sidecar, and `work/installer-build-.../result.json` with the build inputs
and audit. The portable ZIP is unchanged. Rocket's installer remains unsigned
unless a separate code-signing step is added; the compiler's signature does not
sign the generated installer.

Packaging never runs an executable from the input ZIP. Matching archive and manifest
hashes establish consistency, not publisher identity; select a ZIP from a trusted
build and its independent verification record. Executable version smoke tests run
on that trusted build, separately from packaging arbitrary input.

The package audit rejects development folders, profiles, private keys and
recognized credential shapes. Optional local-key comparison is read-only and
keeps key bytes in memory. Reports contain paths, finding types and counts, not
credential values. The pinned Git TLS library contains public cryptographic
self-test vectors; only its exact reviewed file hash is recognized as such.
Other findings stop packaging. This supplements release verification and is not
a guarantee that every possible secret format can be detected.

## Focused acceptance

Run with the shared build/MCP slot reserved:

```powershell
node --test tools/tests/test-installer-payload.cjs
node tools/tests/test-app-update-installer.cjs
node tools/tests/test-inno-installer.cjs '<build result.json>' '<verified ISCC.exe>'
```

The native runner compiles separate test installers from the reviewed script,
using a random `Rocket.InstallerTest.*` AppId and dedicated temporary directories.
It covers fresh install, an earlier-version upgrade, normal wizard cancellation,
changed/extra files, linked folders, locked installation, installed localhost MCP
startup, in-app update/cancellation and uninstall after that update. It checks
preservation of dirty repository data, settings, synthetic SSH configuration and
recovery backups. It never installs under Rocket's normal AppId or uses a live
hosting account. Logs and results stay under `work/inno-...` for review.

The source and runner require successful native acceptance before publishing an
installer. A clean Windows 11 machine remains a separate compatibility check.

Local acceptance on 14 November 2026 passed all 13 native lifecycle cases using
the released 0.92.5 payload, plus six package-audit and eight updater regression
cases. Native tests used Windows 10 and an isolated test registration/profile.
No real hosting login was used and no installer was published.
