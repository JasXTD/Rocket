# App updates

[Wiki home](Home.md) · [Project overview](../../README.md)

Rocket checks public releases from
[JasXTD/Rocket](https://github.com/JasXTD/Rocket/releases) once when the desktop app
starts. The request runs in the background. There is no periodic check or silent
installation. Use **Help → Check for Updates** for another check.

The 0.92.1 packages are also available for manual download from GitHub Releases.
The Mac preview ZIP cannot be installed through the updater because Developer ID
signing and notarization are still pending.

When a version is available, the startup notice offers **View update** and
**Not now**. Dismissing it leaves **Update available** in the status bar. That
entry reopens the known result without another check. A downloaded update shows
**Update ready** there until you choose when to install it.
If a download is already verified, Help also returns to its ready screen instead
of discarding the prepared update.

## Install an update

1. Choose **View update** or the status-bar entry to review the installed and
   available versions. **Full release notes** shows the complete notes from
   GitHub; **View on GitHub** opens the release page. Help can run a fresh check.
2. Choose **Download update**. Rocket verifies the complete archive and every
   extracted file. Progress names the actual download or verification phase.
   **Cancel download** stops preparation without replacing the app.
3. When ready, choose **Install and restart**. Normal close protections still
   apply. Drafts are saved; terminal sessions close. Cancelling the close keeps
   the existing app and installation. Download again when you want to retry.
4. Rocket waits for its old process to exit, rechecks both directories, replaces
   the packaged installation and reopens the repositories.

**Cancel check**, **Cancel download**, Close and Escape cancel the active step.
**Stopping** means the request has been sent and the worker is releasing its
files. Errors offer **Error details** and **Try again**. At compact sizes, the
body scrolls while the close control and action footer remain accessible.

The old directory is retained beside the installation with a name beginning
`.rocket-previous-update-`. If replacement or launching fails, Rocket attempts
to restore and reopen that copy. It does not recursively delete either tree.
Do not remove the backup until you have checked the updated application.

Development output such as `target/release` is not an in-place installation.
It can check and download; use **Open download folder** to inspect the result.
For in-place updates, run Rocket from the versioned directory produced by
`tools/Package-Release.ps1`. Changed or extra files in that directory stop an
update rather than being overwritten. Keep projects and personal files elsewhere.

On Apple Silicon Macs, in-place installation requires a Developer ID signed,
notarized `Rocket.app` in a writable folder. The update must have the same publisher
and app identity. Development/ad-hoc builds cannot install public updates. The old
app is retained beside the new one; installation or launch failure triggers
rollback, while later application crashes are not automatically rolled back.

## If something goes wrong

| Message or situation | What to do |
| --- | --- |
| No public release available | The repository may still be private or have no published stable release. Check again after it is public and a release is published. |
| Request failed or rate-limited | Keep using the current version and try the Help action later. Rocket does not automatically retry. |
| Hash, archive or manifest verification failed | Do not install that download. The current installation remains untouched. |
| Installation contains changed or extra files | Preserve those files and use the verified bundle in a separate folder. |
| Another update is active or needs recovery | Inspect the update receipt before retrying. Do not delete a lock while its helper is running. |
| Replacement could not be restored | The receipt records the original, prepared and backup directories. Keep all three; close Rocket instances and inspect them before restoring the previous directory. |

Download jobs and `install.json`, `transaction.json` and `result.json` receipts
live under `updates` in Rocket's application state directory. Activity diagnostics
record request/step outcomes and timings, excluding release notes, URLs and
downloaded content. An abrupt power loss during the directory swap can require
manual recovery from the retained directories; this is not an unattended updater.

## Release requirements for maintainers

- Publish a stable `major.minor.patch` or `vmajor.minor.patch` tag, with notes.
- Attach the exact file `Rocket-<version>-windows-x64.zip` produced by
  `tools/New-UpdateArchive.ps1` or `tools/Package-Release.ps1`. Do not attach a
  single executable: the GUI, helpers and runtimes must match.
- For Linux x64, attach `Rocket-<version>-linux-x64.tar.gz` from
  `tools/package-release.sh`. It holds one folder, `Rocket-<version>-linux-x64`,
  that people unpack anywhere in their own folders and run `./rocket` from; the
  same file updates them. Rocket accepts only POSIX ustar entries that are plain
  files and folders inside that folder: no links, devices or `..` paths.
- Keep `tools/Verify-Release.ps1` tracked. It verifies release contents and the
  reviewed runtimes before packaging; it is not a generated test output.
- For Apple Silicon, attach `Rocket-<version>-macos-arm64.tar.gz` from
  `tools/macos/update-package.cjs` after Developer ID signing and notarization.
  The portable development ZIP is separate. See [Mac distribution requirements](../MacOS-desktop-integrations.md).
- GitHub's release asset must provide its SHA-256 digest. Rocket also checks the
  contained version, platform, file hashes, paths and size limits. ZIP size is
  limited to 512 MiB; extracted contents to 1 GiB and individual files to 256 MiB.
- Downloads use public HTTPS only. Rocket does not use Git Credential Manager,
  account tokens or SSH keys for application updates. Publishing a release is a
  separate explicit action; local packaging never pushes or uploads anything.
- Downloads require the installed Windows curl version 8.4.0 or newer so limits
  also apply when a response does not announce its size. On Linux they use the
  system's `/usr/bin/curl`, 8.4.0 or newer, and unpack with its `/usr/bin/gzip`;
  both must belong to root and be changeable by no one else. No replacement helper
  is installed automatically. This follows [curl's size-limit contract](https://curl.se/docs/manpage.html#--max-filesize).

These checks trust the release hosted by the configured GitHub repository. A
checksum is not a publisher signature; unsigned Rocket builds remain supported.
On macOS, the additional Developer ID/notarization gate is mandatory; unsigned
builds can check releases but cannot install them in place. The updater never
executes a script downloaded in the archive.
