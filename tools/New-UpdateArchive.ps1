# Build the data-only ZIP expected by Rocket's public GitHub updater. Never uploads.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Bundle,[Parameter(Mandatory)][ValidatePattern('^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$')][string]$Version)
$ErrorActionPreference='Stop'
$bundleRoot=(Resolve-Path -LiteralPath $Bundle).ProviderPath.TrimEnd('\','/')
& (Join-Path $PSScriptRoot 'Verify-Release.ps1') -Bundle $bundleRoot
$cli=& (Join-Path $bundleRoot 'rocket-cli.exe') --json --version
if($LASTEXITCODE -ne 0){throw 'Could not read the verified local bundle version.'}
$cliVersion=$cli|ConvertFrom-Json
if(-not $cliVersion.ok -or $cliVersion.version -cne $Version -or $cliVersion.data.version -cne $Version){throw 'Archive version differs from the packaged application.'}
$archivePath=Join-Path ([IO.Path]::GetDirectoryName($bundleRoot)) "Rocket-$Version-windows-x64.zip"
if (Test-Path -LiteralPath $archivePath) {throw 'This update archive already exists; never overwrite a published version.'}
if (Test-Path -LiteralPath (Join-Path $bundleRoot 'rocket-update.json')) {throw 'Expected a freshly packaged bundle, not an extracted update.'}
$originalSums=(Get-FileHash -LiteralPath (Join-Path $bundleRoot 'SHA256SUMS.txt') -Algorithm SHA256).Hash
$entries=@(Get-ChildItem -LiteralPath $bundleRoot -Recurse -Force)
$files=@($entries | Where-Object { -not $_.PSIsContainer -and $_.FullName -ne (Join-Path $bundleRoot 'SHA256SUMS.txt') } | Sort-Object FullName)
$records=@(foreach($file in $files){
    [ordered]@{path=$file.FullName.Substring($bundleRoot.Length+1).Replace('\','/');sha256=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant();size=$file.Length}
})
if ($entries.Count+1 -gt 20000 -or @($files|Where-Object Length -gt 268435456).Count){throw 'Release exceeds the updater size limits.'}
$metadata=[ordered]@{schema=1;version=$Version;platform='windows-x64';entrypoint='rocket.exe';files=$records}|ConvertTo-Json -Depth 5 -Compress
$bytes=[Text.UTF8Encoding]::new($false).GetBytes($metadata)
if($bytes.Length -gt 4194304){throw 'Update metadata is too large.'}
$hasher=[Security.Cryptography.SHA256]::Create()
try{$metadataHash=([BitConverter]::ToString($hasher.ComputeHash($bytes))).Replace('-','').ToLowerInvariant()}finally{$hasher.Dispose()}
# The ZIP has a complete ordinary release manifest, including its update metadata.
# Metadata describes only payload files, avoiding any self-referential hash cycle.
$sumsBytes=[Text.UTF8Encoding]::new($false).GetBytes((@($records|ForEach-Object {"$($_.sha256)  $($_.path)"}) + "$metadataHash  rocket-update.json") -join "`n")
if(($files|Measure-Object -Property Length -Sum).Sum+$bytes.Length+$sumsBytes.Length -gt 1073741824 -or $sumsBytes.Length -gt 4194304){throw 'Complete update exceeds the unpacked size or metadata limit.'}
$partial=$archivePath+'.'+[Guid]::NewGuid().ToString('N')+'.partial'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$stream=[IO.File]::Open($partial,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
try {
    $zip=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Create,$true)
    try {
        foreach($record in $records){[IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,(Join-Path $bundleRoot $record.path),$record.path,[IO.Compression.CompressionLevel]::Optimal)|Out-Null}
        $entry=$zip.CreateEntry('rocket-update.json');$writer=$entry.Open()
        try{$writer.Write($bytes,0,$bytes.Length)}finally{$writer.Dispose()}
        $entry=$zip.CreateEntry('SHA256SUMS.txt');$writer=$entry.Open()
        try{$writer.Write($sumsBytes,0,$sumsBytes.Length)}finally{$writer.Dispose()}
    }finally{$zip.Dispose()}
}finally{$stream.Dispose()}
if((Get-Item -LiteralPath $partial).Length -gt 536870912){throw 'Compressed update exceeds 512 MiB; candidate retained for inspection.'}
foreach($record in $records){if((Get-FileHash -LiteralPath (Join-Path $bundleRoot $record.path) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $record.sha256){throw 'Bundle changed while archiving; candidate retained.'}}
if((Get-FileHash -LiteralPath (Join-Path $bundleRoot 'SHA256SUMS.txt') -Algorithm SHA256).Hash -ne $originalSums -or @(Get-ChildItem -LiteralPath $bundleRoot -Recurse -Force -File).Count -ne ($files.Count+1)){throw 'Bundle inventory changed while archiving; candidate retained.'}
[IO.File]::Move($partial,$archivePath)
[pscustomobject]@{archive=$archivePath;sha256=(Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant();size=(Get-Item -LiteralPath $archivePath).Length;version=$Version}
