# App-local Microsoft Visual C++ runtime

The Windows x64 portable package includes the unmodified Microsoft
`vcruntime140.dll` **14.51.36247.0**, 178,616 bytes, beside Rocket's executables.
Five Rocket executables import this DLL. Its own imports are Windows system
libraries, including the Universal CRT provided by Windows 10/11. ConPTY does
not add a dependency on another Visual C++ runtime DLL.

The Lua worker is different: its sandbox stages only the EXE into a private
directory. `Build-Local.ps1` links that worker's CRT statically in a separate Cargo
target build, then restores the caller's compiler flags before building the hosts.
`Package-Release.ps1` checks the worker's actual PE imports with the existing MSVC
toolchain and refuses a dynamic CRT dependency. Its embedded hash and sandbox
restrictions are unchanged. Static runtime servicing requires rebuilding the worker.

Source: the existing licensed Visual Studio installation, relative path
`VC/Redist/MSVC/14.51.36231/x64/Microsoft.VC145.CRT/vcruntime140.dll`.
No DLL was downloaded from a third-party site and no system runtime is installed
or replaced. Copyright Microsoft Corporation; this component is distributed
under the applicable Microsoft terms, not Rocket's MIT/Apache licenses.

SHA-256:

    d1f4225df2cd877dbf130d5668a021dce3f94118455ff5ec952061c30afc9ce7

The installed file has a valid Microsoft Windows Software Compatibility
Publisher Authenticode signature and Microsoft timestamp, verified 2026-11-03.
`tools/Copy-VcRuntime.ps1` and `tools/Verify-Release.ps1` require this exact
reviewed hash, Microsoft publisher and timestamp. Neither accepts an arbitrary
DLL just because a package manifest was regenerated. Hashes and signatures
establish identity and integrity, not the absence of undisclosed vulnerabilities.

Microsoft documents [app-local deployment](https://learn.microsoft.com/en-us/cpp/windows/deployment-in-visual-cpp?view=msvc-170)
and permits redistribution of the applicable unmodified VC redist files under
the [Visual Studio redistribution terms](https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution).
App-local libraries must be maintained with Rocket updates; they are not serviced
automatically by updating a system-wide VC runtime.

To package, use the existing Visual Studio install or supply
`Package-Release.ps1 -VcRuntimeDirectory <reviewed CRT directory>`.
If an installed version differs, stop and review it before changing the pins.
The binary is copied at packaging time and is not committed into this directory.
