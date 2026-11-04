# Copies the reviewed app-local runtime from an existing Visual Studio install.
# Never downloads, installs, searches PATH for a DLL, or replaces existing files.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Destination,
    [string]$RuntimeDirectory
)
$ErrorActionPreference = 'Stop'
$destinationRoot = (Resolve-Path -LiteralPath $Destination).ProviderPath
$destinationItem = Get-Item -LiteralPath $destinationRoot -Force
if (-not $destinationItem.PSIsContainer -or ($destinationItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Runtime destination must be a normal directory.' }
$target = Join-Path $destinationRoot 'vcruntime140.dll'
if (Test-Path -LiteralPath $target) { throw 'Runtime destination already exists; nothing was replaced.' }
if (-not $RuntimeDirectory) {
    $locator = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path -LiteralPath $locator -PathType Leaf)) { throw 'Provide -RuntimeDirectory from an existing licensed Visual Studio installation.' }
    $installation = @(& $locator -latest -products '*' -property installationPath)
    if ($LASTEXITCODE -ne 0 -or $installation.Count -ne 1) { throw 'Could not locate the existing Visual Studio installation.' }
    $RuntimeDirectory = Join-Path $installation[0] 'VC/Redist/MSVC/14.51.36231/x64/Microsoft.VC145.CRT'
}
$source = Join-Path (Resolve-Path -LiteralPath $RuntimeDirectory).ProviderPath 'vcruntime140.dll'
$sourceItem = Get-Item -LiteralPath $source -Force
if ($sourceItem.PSIsContainer -or ($sourceItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Runtime source must be a normal file.' }
$expected = 'd1f4225df2cd877dbf130d5668a021dce3f94118455ff5ec952061c30afc9ce7'
# Keep a read lease through validation/copy so the verified source cannot be
# replaced or written between its hash check and the final copy.
$inputStream = [IO.File]::Open($source, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
try {
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expected) { throw 'Unreviewed Visual C++ runtime source; no files copied.' }
    $signature = Get-AuthenticodeSignature -LiteralPath $source
    if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate -or $signature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Microsoft Corporation(?:,|$)' -or -not $signature.TimeStamperCertificate) { throw 'Visual C++ runtime signature/publisher verification failed.' }
    $outputStream = [IO.File]::Open($target, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $inputStream.CopyTo($outputStream) } finally { $outputStream.Dispose() }
    if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expected) { throw 'Copied runtime hash differs; candidate retained for inspection.' }
} finally { $inputStream.Dispose() }
Write-Output 'Copied verified Microsoft x64 vcruntime140.dll 14.51.36247.0 (178,616 bytes).'
