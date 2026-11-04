param(
    [Parameter(Mandatory)][string]$Bundle,
    [ValidatePattern('^[0-9A-Fa-f]{40}$')][string]$PublisherThumbprint,
    [Alias('AllowUnsignedLocal')][switch]$AllowUnsigned,
    [switch]$RequireBundledGit
)
$ErrorActionPreference = 'Stop'
if ($AllowUnsigned -and $PublisherThumbprint) { throw 'Choose unsigned verification or publisher verification, not both.' }
# Unsigned releases are supported. Publisher verification is optional and explicit.
$bundleRoot = (Resolve-Path -LiteralPath $Bundle).ProviderPath
$rootItem = Get-Item -LiteralPath $bundleRoot -Force
if (-not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Bundle root must be a normal directory' }
$manifest = Join-Path $bundleRoot 'SHA256SUMS.txt'
$files = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$pending = [Collections.Generic.Stack[string]]::new()
$pending.Push($bundleRoot)
while ($pending.Count) {
    $directory = $pending.Pop()
    foreach ($entry in Get-ChildItem -LiteralPath $directory -Force) {
        if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Bundle contains a link or reparse point' }
        if ($entry.PSIsContainer) { $pending.Push($entry.FullName) }
        elseif (-not $files.Add($entry.FullName)) { throw 'Duplicate bundle path' }
    }
}
if (-not $files.Remove($manifest)) { throw 'Release hash manifest is missing' }
$checked = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$hashes = @{}
foreach ($line in [IO.File]::ReadAllLines($manifest)) {
    if ($line -notmatch '^([0-9a-fA-F]{64})  (.+)$') { throw 'Invalid release manifest record' }
    $expected = $Matches[1]
    $relative = $Matches[2]
    $parts = $relative -split '[/\\]'
    if ([IO.Path]::IsPathRooted($relative) -or $relative.Contains(':') -or $parts -contains '..' -or $parts -contains '.' -or $parts -contains '') { throw 'Manifest path must be a normal relative path' }
    $full = [IO.Path]::GetFullPath((Join-Path $bundleRoot $relative))
    if (-not $full.StartsWith($bundleRoot.TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Manifest path escapes bundle' }
    if (-not $checked.Add($full)) { throw 'Duplicate manifest path' }
    if (-not $files.Contains($full)) { throw "Manifest file is missing: $relative" }
    $actual = (Get-FileHash -LiteralPath $full -Algorithm SHA256).Hash
    if ($actual -ne $expected) { throw "Hash mismatch: $relative" }
    $hashes[$full] = $actual
}
if (-not $checked.SetEquals($files)) { throw 'Unlisted file in release bundle' }

# The x64 MSVC executables import this runtime even on a machine that has no
# developer tools. A regenerated manifest alone must not authorize another DLL.
$vcRuntime = Join-Path $bundleRoot 'vcruntime140.dll'
if (-not $checked.Contains($vcRuntime)) { throw 'App-local Visual C++ runtime missing from manifest: vcruntime140.dll' }
if ($hashes[$vcRuntime].ToLowerInvariant() -cne 'd1f4225df2cd877dbf130d5668a021dce3f94118455ff5ec952061c30afc9ce7') { throw 'Unreviewed app-local Visual C++ runtime.' }
$vcSignature = Get-AuthenticodeSignature -LiteralPath $vcRuntime
if ($vcSignature.Status -ne 'Valid' -or -not $vcSignature.SignerCertificate -or $vcSignature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Microsoft Corporation(?:,|$)' -or -not $vcSignature.TimeStamperCertificate) { throw 'Visual C++ runtime signature/publisher verification failed.' }

# Current bundles must include the worker. Historical bundles retain their own verifier.
$rocketBinaries = 'rocket.exe','rocket-mcp.exe','rocket-cli.exe','rocket-rebase-editor.exe','rocket-terminal-host.exe','rocket-lua-host.exe'
foreach ($name in $rocketBinaries) {
    $full = Join-Path $bundleRoot $name
    if (-not $checked.Contains($full)) { throw "Executable missing from manifest: $name" }
    $signature = Get-AuthenticodeSignature -LiteralPath $full
    if (-not $PublisherThumbprint -and $signature.Status -eq 'NotSigned') { continue }
    if ($signature.Status -ne 'Valid') { throw "Release blocked: $name has signature status $($signature.Status)" }
    if ($PublisherThumbprint -and $signature.SignerCertificate.Thumbprint -ne $PublisherThumbprint) { throw "Unexpected publisher: $name" }
    if (-not $signature.TimeStamperCertificate) { throw "Missing trusted timestamp: $name" }
}

# Catch incomplete or mixed builds even when their manifest was regenerated.
# This checks the compiled-in digest, not publisher identity or an untrusted host.
$workerHash = $hashes[(Join-Path $bundleRoot 'rocket-lua-host.exe')].ToLowerInvariant()
foreach ($name in @('rocket.exe','rocket-mcp.exe')) {
    $image = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes((Join-Path $bundleRoot $name)))
    if (-not $image.Contains($workerHash)) { throw "Mismatched embedded Lua worker hash: $name" }
}
$image = $null

# Reviewed Microsoft.Windows.Console.ConPTY 1.25.260930003 payloads. Pin each
# architecture as a pair as well as verifying its publisher and signature.
$runtimePairs = @(
    @('feeef341d891643c62d30b6b07800bc70f0bc148f44cb8c3bee84aa557ae805a','3d66b23d0a71bb8eed2b77edc8b9df9bf54ce6c8fb74c863a30e760997f80586'),
    @('d145f621bf64a9da3b7395587ddef9831ca404805d8987ec8c9903d0c4603d93','e3e475547d57da0689dc9fc5e0101d10390e2a4719a6eb5dcad569f6cb33c1a3'),
    @('151a02355227710c20c9f8f1eaf68ac4668aa09cdc921adc7ec661804d8eb1bf','4313825a3ba634d113cc6ed3bc8ce735e3c19d360cecd985794787479469f91f')
)
$runtimePaths = @('conpty/1.25.260930003/conpty.dll','conpty/1.25.260930003/OpenConsole.exe')
$runtimeHashes = foreach ($relative in $runtimePaths) {
    $full = [IO.Path]::GetFullPath((Join-Path $bundleRoot $relative))
    if (-not $checked.Contains($full)) { throw "Runtime missing from manifest: $relative" }
    $signature = Get-AuthenticodeSignature -LiteralPath $full
    if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate -or $signature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Microsoft Corporation(?:,|$)' -or -not $signature.TimeStamperCertificate) { throw "Runtime signature/publisher verification failed: $relative" }
    $hashes[$full]
}
if (-not ($runtimePairs | Where-Object { $_[0] -eq $runtimeHashes[0] -and $_[1] -eq $runtimeHashes[1] })) { throw 'Runtime payload is not a reviewed matching architecture pair' }
$knownBinaries = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($relative in ($rocketBinaries + $runtimePaths)) { [void]$knownBinaries.Add([IO.Path]::GetFullPath((Join-Path $bundleRoot $relative))) }
[void]$knownBinaries.Add($vcRuntime)
$gitManifest = Join-Path $bundleRoot 'git/manifest.tsv'
if ($checked.Contains($gitManifest)) {
    $runtimeChecker = Join-Path $bundleRoot 'tools/Git-Runtime.ps1'
    if (-not $checked.Contains($runtimeChecker)) { throw 'Bundled Git verifier is missing.' }
    # The ordinary release manifest is not an independent trust anchor. Do not
    # execute a replacement helper merely because that manifest was regenerated.
    if ($hashes[$runtimeChecker].ToLowerInvariant() -cne 'e30aebad51bb28bbbe15935ca91e87178aa0d68f2944c80a12a335b0a9ba3ac5') { throw 'Unreviewed bundled Git verification helper.' }
    . $runtimeChecker
    $inventory = Get-RocketGitInventory -Manifest $gitManifest
    $gitRoot = Join-Path $bundleRoot "git/$($inventory.Spec.Version)"
    [void](Test-RocketGitRuntime -RuntimeRoot $gitRoot -Inventory $inventory)
    foreach ($entry in $inventory.Entries.Values) {
        $full = [IO.Path]::GetFullPath((Join-Path $gitRoot $entry.Path))
        if (-not $checked.Contains($full) -or $hashes[$full] -ne $entry.Hash) { throw 'Bundled Git does not match the reviewed release inventory.' }
        if ([IO.Path]::GetExtension($full) -in @('.exe','.dll')) { [void]$knownBinaries.Add($full) }
    }
    foreach ($name in @('rocket.exe','rocket-mcp.exe','rocket-cli.exe')) {
        $image = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes((Join-Path $bundleRoot $name)))
        if (-not $image.Contains($inventory.Spec.ManifestSha256)) { throw "Mismatched compiled Git runtime inventory: $name" }
    }
    $image = $null
    $gitPrefix = [IO.Path]::GetFullPath((Join-Path $bundleRoot 'git')).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    $versionPrefix = [IO.Path]::GetFullPath($gitRoot).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    foreach ($file in $files) {
        if ($file.StartsWith($gitPrefix,[StringComparison]::OrdinalIgnoreCase) -and $file -ne $gitManifest -and -not $file.StartsWith($versionPrefix,[StringComparison]::OrdinalIgnoreCase)) { throw 'Unexpected bundled Git version or metadata file.' }
    }
    Write-Output "Pinned bundled Git $($inventory.Spec.Version) verified ($($inventory.Entries.Count) files)."
} elseif ($RequireBundledGit -or (Test-Path -LiteralPath (Join-Path $bundleRoot 'git'))) {
    throw 'The required bundled Git inventory is missing.'
}
foreach ($file in $files) {
    if ([IO.Path]::GetExtension($file) -in @('.exe','.dll') -and -not $knownBinaries.Contains($file)) { throw 'Unexpected executable or DLL in release manifest' }
}
if (-not $PublisherThumbprint) { 'Exact bundle contents, hashes and signed Microsoft runtime verified; Rocket binaries may be unsigned. This does not authenticate a Rocket publisher.' }
else { 'Exact bundle contents, hashes, runtime and timestamped publisher signatures verified.' }
