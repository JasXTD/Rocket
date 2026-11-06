# Windows release testing

Use a normal, non-elevated account. Rocket deliberately refuses elevated
execution. Keep the accepted release immutable and run checks sequentially.
Neither runner downloads tools, contacts a Git hosting service, changes execution
policy, installs Windows features, or uses your normal Rocket/Git/SSH profile.

## Focused source gates

From the source checkout in PowerShell 7:

    .\tools\Test-ReleaseChecks.ps1 -Offline

This compiles only the backend library tests and runs five exact cases in separate,
sequential processes. It covers diagnostic contention and durable recovery, the
HTTP runtime gate and local transport, and Lua worker limits/cancellation.
The existing three-second diagnostic assertion and five-second Lua command
deadline remain unchanged. Failures stop the run; there is no automatic retry or
acceptance of empty/ignored filters. Each result retains its original output.
This is a focused release gate, not a claim that the entire workspace suite passed.

## Packaged application

Use the package runner with an existing reviewed bundle directory.
In PowerShell 7:

    $result = Join-Path ([IO.Path]::GetTempPath()) ('Rocket-package-' + [guid]::NewGuid().ToString('N'))
    .\tools\Test-WindowsPackage.ps1 -Bundle .\tools\releases\0.92.0 -OutputDirectory $result

The output directory must be new and outside any Git checkout. This matters:
Rocket correctly refuses to initialize a new repository inside another one.
Keep execution policy as configured; this task does not require changing it.
PowerShell 5.1 syntax compatibility alone is not a completed 5.1 runtime test.

The runner verifies and copies the complete package, then:

- Uses fresh child settings and an OS-only PATH without installed Git, Rust or Node.
- Creates a synthetic Unicode-path repository and commits through CLI JSON.
- Checks a structured invalid-repository error.
- Opens and normally closes the ordinary desktop executable.
- Confirms the desktop and MCP processes load the app-local Microsoft runtime.
- Executes Lua inside the real sandbox and checks direct unsandboxed startup is refused.
- Drives the packaged native UI through authenticated localhost MCP.
- Checks bundled Git selection, external edits, Unicode names, terminal execution
  and captures at 1280 x 800 and 780 x 620.
- Verifies local files/HEAD, diagnostic privacy and owned-process shutdown.
- Rechecks the package inventory after use.

The runner waits for actual startup completion and refreshes .NET's cached window
handle before closing the real Rocket window. Its temporary MCP token is removed
on exit. Failed runs preserve their original evidence and stop their owned processes.
The disposable copy is several hundred megabytes; remove only that recorded
temporary output directory once its reports and screenshots are no longer needed.

Read result.json, the verification logs and PNG captures. A completed process
or a successful tool click alone is not acceptance. The report's fresh_os_proven
field remains false: the runner cannot attest that its host is a newly installed OS.

## Clean Windows 11 acceptance

A fresh profile on a development computer does not prove clean-Windows support.
Record the Windows build, display scaling, graphics driver and VM/physical-machine
provenance separately. The current isolated-host run used Windows 10 build 19045;
the user will perform Windows 11 acceptance on a separate PC.

Transfer the complete `tools/releases/Rocket-0.89.6-windows-x64.zip`, extract it into a new writable
application folder, and run rocket.exe normally. Keep projects outside that
application folder. No installer, system Git, Rust, Node or additional .NET runtime
is needed by Rocket. PowerShell 7 is only the optional test runner's host.

Use 0.89.6 for the current handoff; it corrects acknowledgment strings with escaped
newlines falsely blocking secret scans in older history. The 0.89.5 ZIP predates
that correction. Releases from 0.89.3 include the runtime fix;
do not use the original 0.89.2 ZIP for clean-machine acceptance. PE import inspection
found that all six original executables depend on `vcruntime140.dll`; the old
package omitted it and the development machine supplied a system copy. The new
package includes the reviewed Microsoft runtime beside the five ordinary
executables; the isolated Lua worker links its runtime statically. Its full
inventory pins and verifies the DLL, and hosts bind the rebuilt worker's hash.
`tools/tests/test-vc-runtime.ps1` covers missing/replaced runtime rejection and
preservation of existing destination files. The additional local runtime is
maintained with Rocket's packages; see [provenance](../runtime/vc/README.md).

First check ordinary startup, bundled Git selection, a new local repository,
stage/commit, an external edit, terminal input/interruption, window resizing,
draft persistence and normal close/reopen. Use synthetic files and repository-local
identity; GitHub sign-in is unnecessary. Existing production repositories and
credentials are outside this acceptance pass.

Also try **File → Repository manager → Scan folder** (refined in 0.89.5) with a parent
containing temporary repositories. Check filtering, already-saved entries, saving
selected shortcuts, Stop and compact-window layout. Scanning does not open tabs
or change repository files; existing shortcut labels and groups are preserved.

The build is unsigned. Record any Windows launch warning rather than disabling
protection. No public release was uploaded by this task, so the updater may report
that no public release is available. Real GitHub update delivery and clean Windows
11 results remain separate from the successful local fixture checks.
