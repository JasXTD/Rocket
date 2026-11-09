# Build and verify a new local unsigned release; never replace an existing version.
[CmdletBinding()]
param([switch]$Offline, [switch]$WithoutBundledGit, [string]$VcRuntimeDirectory)
$ErrorActionPreference = 'Stop'
$packageRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = [IO.File]::ReadAllText((Join-Path $packageRoot 'Cargo.toml'))
$section = [regex]::Match($manifest, '(?ms)^\[workspace\.package\]\s*\r?\n(.*?)(?=^\[|\z)').Groups[1].Value
$version = [regex]::Match($section, '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$').Groups[1].Value
if (-not $version) { throw 'Expected an explicit three-part workspace release version.' }
$releaseRoot = Join-Path $packageRoot 'tools/releases'
$bundle = Join-Path $releaseRoot $version
if (Test-Path -LiteralPath $bundle) { throw "Release $version already exists; choose a new version instead of replacing it." }

& (Join-Path $PSScriptRoot 'Build-Local.ps1') -Offline:$Offline
$source = Join-Path $packageRoot 'target/release'
$binaries = @('rocket.exe','rocket-mcp.exe','rocket-cli.exe','rocket-rebase-editor.exe','rocket-terminal-host.exe','rocket-lua-host.exe','rocket-ssh.exe')
$documents = @('CHANGELOG.md','CLI.md','DEPENDENCIES.md','HOSTING.md','INTEGRATIONS.md','LICENSE-APACHE','LICENSE-MIT','README.md','SECURITY.md','THIRD-PARTY-NOTICES.txt','tools/Verify-Release.ps1',
    'docs/images/rocket-logo.png','crates/rocket-ui/src/guide.md','tools/README.md','runtime/conpty/1.25.260930003/README.md',
    'runtime/git/README.md','tools/Git-Runtime.ps1','runtime/vc/README.md')
# User-facing wiki pages ship with the app. Developer/test notes stay local,
# matching their source-publication exclusions in .gitignore.
$documents += @(Get-ChildItem -LiteralPath (Join-Path $packageRoot 'docs/wiki') -File -Filter '*.md' | ForEach-Object { 'docs/wiki/' + $_.Name })
if (Test-Path -LiteralPath (Join-Path $packageRoot "docs/releases/$version.md") -PathType Leaf) {
    $documents += "docs/releases/$version.md"
}
foreach ($name in $binaries) {
    if (-not (Test-Path -LiteralPath (Join-Path $source $name) -PathType Leaf)) { throw "Missing build output: $name" }
}
# The sandbox copies only the worker EXE into its private directory. Inspect the
# actual PE imports using the existing compiler toolchain, not merely build flags.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$dumpbin = @(& $vswhere -latest -products '*' -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe') | Select-Object -First 1
if ($LASTEXITCODE -ne 0 -or -not $dumpbin) { throw 'Existing MSVC dumpbin is required to verify the sandbox worker imports.' }
$workerImports = @(& $dumpbin /DEPENDENTS (Join-Path $source 'rocket-lua-host.exe'))
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect sandbox worker imports.' }
$dlls = @($workerImports | ForEach-Object { if ($_ -match '^\s+([A-Za-z0-9_.-]+\.dll)\s*$') { $Matches[1] } })
if ($dlls -notcontains 'KERNEL32.dll' -or @($dlls | Where-Object { $_ -match '^(?:vcruntime|msvcp|msvcr|ucrtbase|api-ms-win-crt-)' }).Count) { throw 'Sandbox worker must use a statically linked CRT; a system or app-local runtime dependency remains.' }
Write-Output ('Sandbox worker PE imports verified: ' + ($dlls -join ', '))
$cliOutput = & (Join-Path $source 'rocket-cli.exe') --json --version
if ($LASTEXITCODE -ne 0) { throw 'Built CLI version check failed.' }
$cliVersion = $cliOutput | ConvertFrom-Json
if (-not $cliVersion.ok -or $cliVersion.data.version -cne $version -or $cliVersion.version -cne $version) { throw 'Built CLI version does not match the workspace release version.' }

# Only a successfully verified candidate receives the versioned release path.
$stage = Join-Path $releaseRoot ('.' + $version + '-' + [Guid]::NewGuid().ToString('N') + '.partial')
[IO.Directory]::CreateDirectory($stage) | Out-Null
foreach ($name in $binaries) { Copy-Item -LiteralPath (Join-Path $source $name) -Destination (Join-Path $stage $name) }
foreach ($name in $documents) {
    $destination = Join-Path $stage $name
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination)) | Out-Null
    Copy-Item -LiteralPath (Join-Path $packageRoot $name) -Destination $destination
}
$runtime = 'conpty/1.25.260930003'
[IO.Directory]::CreateDirectory((Join-Path $stage $runtime)) | Out-Null
foreach ($name in @('conpty.dll','OpenConsole.exe','LICENSE')) {
    Copy-Item -LiteralPath (Join-Path $source "$runtime/$name") -Destination (Join-Path $stage "$runtime/$name")
}
& (Join-Path $PSScriptRoot 'Copy-VcRuntime.ps1') -Destination $stage -RuntimeDirectory $VcRuntimeDirectory
if (-not $WithoutBundledGit) {
    & (Join-Path $PSScriptRoot 'Assemble-GitRuntime.ps1') -Destination $stage
}
$hashes = foreach ($file in Get-ChildItem -LiteralPath $stage -Recurse -File | Sort-Object FullName) {
    $relative = $file.FullName.Substring($stage.Length + 1).Replace('\','/')
    (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + $relative
}
[IO.File]::WriteAllLines((Join-Path $stage 'SHA256SUMS.txt'), $hashes, [Text.UTF8Encoding]::new($false))
& (Join-Path $stage 'tools/Verify-Release.ps1') -Bundle $stage -RequireBundledGit:(-not $WithoutBundledGit)
foreach ($name in $binaries) {
    if ((Get-FileHash -LiteralPath (Join-Path $stage $name) -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath (Join-Path $source $name) -Algorithm SHA256).Hash) { throw "Build output changed while packaging: $name" }
}
if ([IO.File]::ReadAllText((Join-Path $packageRoot 'Cargo.toml')) -cne $manifest) { throw 'Workspace manifest changed during packaging.' }
$releasePrefix = [IO.Path]::GetFullPath($releaseRoot).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
foreach ($destination in @($stage, $bundle)) {
    if (-not [IO.Path]::GetFullPath($destination).StartsWith($releasePrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Release move would leave the intended directory.' }
}
if (Test-Path -LiteralPath $bundle) { throw 'Another process created this release version; candidate preserved.' }
[IO.Directory]::Move($stage, $bundle)
Write-Output "Verified unsigned release: $bundle"
& (Join-Path $PSScriptRoot 'New-UpdateArchive.ps1') -Bundle $bundle -Version $version
