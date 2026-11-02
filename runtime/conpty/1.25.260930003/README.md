# Bundled Microsoft ConPTY runtime

Reviewed on 2026-10-16, with explicit user approval to ship the native runtime.
This directory contains unmodified binaries from Microsoft's stable
[Microsoft.Windows.Console.ConPTY 1.25.260930003 NuGet package](https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY/1.25.260930003),
published 2026-10-02. It is native code; Rocket does not acquire a .NET dependency.

The package was downloaded directly from the official NuGet flat-container URL
with redirects disabled. Its SHA-256 is:

```
02b07b349af66d801159bdf9e440d4a1ce78bb951f37fc8609731665afdae7ee
```

Only the named `conpty.dll` and `OpenConsole.exe` payloads were extracted for
`x64`, `x86` and `arm64`. No NuGet/MSBuild package scripts were executed. All six
files had **Valid** Authenticode signatures with **Microsoft Corporation** as the
signer and file version **1.25.2609.30003**. [manifest.json](manifest.json) records
their exact sizes and SHA-256 hashes. The x64 pair is 1,208,984 bytes.

[LICENSE](LICENSE) is Microsoft's MIT license from the corresponding
[Terminal v1.25.2733.0 release source](https://github.com/microsoft/terminal/tree/v1.25.2733.0).
The audit archive, extracted API header and metadata are retained under
`work/terminal-dependency-review` in the development workspace.

`crates/rocket-terminal/build.rs` copies only the selected architecture's two
files and license into the versioned `conpty/1.25.260930003` folder beside Cargo's
app, test and example executables. It performs no downloads or process execution.
Ship that folder alongside `rocket.exe` and `rocket-mcp.exe` (a shared folder is
sufficient when they are in the same directory).

The runtime loader resolves an absolute path relative to the executable and
restricts dependency loading to the package directory and System32. It retains
the module until the pseudoconsole closes. It does not search the repository,
PATH, VS Code installation or legacy system ConPTY as a fallback. Missing files
produce a startup error. Hashes/signatures were verified during this packaging
review, not on every startup; the installed app directory is trusted like the
Rocket executable itself. Signature and hash checks establish provenance, not
proof that native code has no vulnerabilities.

The runtime's grapheme-width creation flag aligns Windows console cursor reports
with Rocket's Unicode cell model. x64 was exercised on this Windows 10 machine;
x86 and arm64 payload provenance was checked, but those architectures were not run.
