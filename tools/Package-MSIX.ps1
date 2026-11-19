# Wrap a reviewed portable ZIP in an unsigned MSIX. Never installs, signs or publishes.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Archive,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ArchiveSha256,
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9.-]{2,49}$')][string]$IdentityName = 'TwinCatz.GitRocket',
    [string]$Publisher = 'CN=3C1B7D92-578D-491F-BAF8-AED91B184402',
    [ValidateLength(1,256)][string]$DisplayName = 'GitRocket',
    [ValidateLength(1,256)][string]$PublisherDisplayName = 'TwinCatz',
    [ValidatePattern('^[1-9][0-9]*\.[0-9]+\.[0-9]+\.0$')][string]$PackageVersion,
    [Parameter(Mandatory)][string]$MakeAppx,
    [string]$OutputDirectory,
    [switch]$CompareLocalSshKeys,
    [switch]$CompatibilityTest
)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $OutputDirectory){$OutputDirectory=Join-Path $PSScriptRoot 'releases'}
$output=[IO.Path]::GetFullPath($OutputDirectory)
$archivePath=(Resolve-Path -LiteralPath $Archive).ProviderPath
$compilerPath=(Resolve-Path -LiteralPath $MakeAppx).ProviderPath
if($Publisher.Length -gt 1024 -or $Publisher -match '[\x00-\x1f]'){throw 'Invalid publisher identity.'}
[void][Security.Cryptography.X509Certificates.X500DistinguishedName]::new($Publisher)
if($CompatibilityTest -and $IdentityName -cnotmatch '^Rocket\.CompatibilityTest\.[a-f0-9]{12}$'){throw 'Compatibility tests require a unique Rocket.CompatibilityTest identity.'}
$signature=Get-AuthenticodeSignature -LiteralPath $compilerPath
if($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Microsoft Corporation(?:,|$)' -or [IO.Path]::GetFileName($compilerPath) -ine 'makeappx.exe'){throw 'Use a valid Microsoft-signed Windows SDK MakeAppx.exe.'}
$compilerHash=(Get-FileHash -LiteralPath $compilerPath).Hash
[void](Get-Command node -ErrorAction Stop)
function Copy-PayloadEntry([IO.Stream]$InputStream,[IO.Stream]$OutputStream,[long]$Length) {
    if($Length -lt 0 -or $Length -gt 268435456){throw 'Unsupported archive entry size.'}
    $buffer=[byte[]]::new(65536)
    [long]$remaining=$Length
    $digest=[Security.Cryptography.IncrementalHash]::CreateHash('SHA256')
    try {
        while(($count=$InputStream.Read($buffer,0,[int][Math]::Min($buffer.Length,$remaining+1))) -gt 0){
            if($count -gt $remaining){throw 'Archive entry exceeds its declared size.'}
            $OutputStream.Write($buffer,0,$count)
            $digest.AppendData($buffer,0,$count)
            $remaining-=$count
        }
        if($remaining -ne 0){throw 'Archive entry is shorter than its declared size.'}
        [Convert]::ToHexString($digest.GetHashAndReset()).ToLowerInvariant()
    } finally {$digest.Dispose()}
}
$job=Join-Path $projectRoot ('work/msix-build-'+[Guid]::NewGuid().ToString('N'))
$stage=Join-Path $job 'stage'
$payload=Join-Path $stage 'app'
[void][IO.Directory]::CreateDirectory($payload)
[void][IO.Directory]::CreateDirectory($output)
$archiveEntries=[Collections.Generic.Dictionary[string,string]]::new([StringComparer]::Ordinal)
$archiveStream=[IO.File]::OpenRead($archivePath)
try {
    if($archiveStream.Length -gt 536870912 -or (Get-FileHash -InputStream $archiveStream -Algorithm SHA256).Hash -ine $ArchiveSha256){throw 'Portable ZIP differs from its reviewed SHA-256 or exceeds the size limit.'}
    $archiveStream.Position=0
    $zip=[IO.Compression.ZipArchive]::new($archiveStream,'Read')
    $names=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    [long]$total=0
    if($zip.Entries.Count -gt 20000){throw 'Too many package entries.'}
    foreach($entry in $zip.Entries){
        $name=$entry.FullName
        if($name -match '[\\:\x00-\x1f<>"|?*]' -or $name.StartsWith('/') -or $name.EndsWith('/') -or
           @($name.Split('/')|Where-Object {$_ -in @('','.','..') -or $_.EndsWith('.') -or $_.EndsWith(' ')}).Count -or
           -not $names.Add($name)){throw 'Unsafe, directory or duplicate archive path.'}
        $kind=($entry.ExternalAttributes -shr 16) -band 0xF000
        if($kind -notin @(0,0x8000) -or ($entry.ExternalAttributes -band [IO.FileAttributes]::ReparsePoint) -or $entry.Length -gt 268435456){throw 'Archive contains a link, special or oversized file.'}
        $total+=$entry.Length;if($total -gt 1073741824){throw 'Archive exceeds the unpacked size limit.'}
    }
    foreach($entry in $zip.Entries){
        $destination=[IO.Path]::GetFullPath((Join-Path $payload $entry.FullName))
        if(-not $destination.StartsWith($payload+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Archive path leaves staging.'}
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
        $entryStream=$entry.Open();$file=[IO.File]::Open($destination,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        try{$archiveEntries.Add($entry.FullName,(Copy-PayloadEntry $entryStream $file $entry.Length))}
        finally{$file.Dispose();$entryStream.Dispose()}
    }
}finally{if($zip){$zip.Dispose()};$archiveStream.Dispose()}
# Use the trusted source verifier, never execute a verifier supplied by an archive.
& (Join-Path $PSScriptRoot 'Verify-Release.ps1') -Bundle $payload -RequireBundledGit
$metadata=[IO.File]::ReadAllBytes((Join-Path $payload 'rocket-update.json'))
$metadataHash=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($metadata))
$metadata=[Text.Encoding]::UTF8.GetString($metadata).TrimStart([char]0xfeff) | ConvertFrom-Json
$version=[string]$metadata.version
if($metadata.schema -ne 1 -or $metadata.platform -cne 'windows-x64' -or $metadata.entrypoint -cne 'rocket.exe' -or $version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'){throw 'Unexpected update metadata.'}
$appVersion = [version]$version
$storeVersion = '{0}.{1}.{2}.0' -f (1 + $appVersion.Major), $appVersion.Minor, $appVersion.Build
if (-not $PackageVersion) {
    $PackageVersion = $storeVersion
} elseif (-not $CompatibilityTest -and $PackageVersion -cne $storeVersion) {
    throw "Rocket $version requires MSIX version $storeVersion."
}
foreach ($part in $PackageVersion.Split('.')) {
    if ([int]$part -gt 65535) { throw 'MSIX version components must be at most 65535.' }
}
# Package metadata is data, not proof of publisher identity. Never run executables
# from an input ZIP here: an archive and its caller-provided digest may both be
# attacker controlled. Binary version smoke checks belong to the trusted build's
# acceptance tests, before selecting the ZIP for packaging.
$auditPath=Join-Path $job 'payload-audit.json'
if($metadataHash -ine $archiveEntries['rocket-update.json']){throw 'Update metadata differs from the reviewed ZIP.'}
$auditArgs=@((Join-Path $PSScriptRoot 'windows/audit-payload.cjs'),$payload,$auditPath)
if($CompareLocalSshKeys){$auditArgs+='--compare-local-ssh'}
$auditSummary=& node @auditArgs
if($LASTEXITCODE -ne 0){throw 'Package privacy check refused the payload. See the value-free audit report.'}
. (Join-Path $PSScriptRoot 'windows/Package-Audit.ps1')
$audit=Read-PackageAudit $auditPath $auditSummary
$inventory=[Collections.Generic.List[object]]::new()
if($audit.inventory.Count -ne $archiveEntries.Count){throw 'Audited payload differs from the reviewed ZIP.'}
foreach($entry in $audit.inventory){
    if($archiveEntries[$entry.path] -cne $entry.sha256){throw "Audited bytes differ from the reviewed ZIP: $($entry.path)"}
    $inventory.Add([pscustomobject]@{path='app/'+$entry.path;sha256=$entry.sha256;size=$entry.size})
}
function Write-GeneratedFile([string]$Name,[byte[]]$Bytes){
    $inventory.Add([pscustomobject]@{path=$Name;sha256=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant();size=$Bytes.Length})
    [IO.File]::WriteAllBytes((Join-Path $stage $Name),$Bytes)
}

# Reuse Rocket's source artwork; never load an image from the input archive.
Add-Type -AssemblyName System.Drawing
$assets=Join-Path $stage 'Assets'
[void][IO.Directory]::CreateDirectory($assets)
$logo=[Drawing.Image]::FromFile((Join-Path $projectRoot 'docs/images/rocket-logo.png'))
try {
    foreach($size in @(44,50,150)){
        $bitmap=[Drawing.Bitmap]::new($size,$size)
        $graphics=[Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.Clear([Drawing.Color]::Transparent)
            $graphics.InterpolationMode=[Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $edge=[int]($size*0.84);$inset=[int](($size-$edge)/2)
            $graphics.DrawImage($logo,$inset,$inset,$edge,$edge)
            $png=[IO.MemoryStream]::new()
            try {
                $bitmap.Save($png,[Drawing.Imaging.ImageFormat]::Png)
                Write-GeneratedFile "Assets/Logo$size.png" $png.ToArray()
            } finally {$png.Dispose()}
        } finally {$graphics.Dispose();$bitmap.Dispose()}
    }
} finally {$logo.Dispose()}
function Xml([string]$value){[Security.SecurityElement]::Escape($value)}
$display=if($CompatibilityTest){'Rocket Compatibility Test'}else{Xml $DisplayName}
$testApplication=''
if($CompatibilityTest){
    # Only the disposable acceptance package exposes this hidden activation target.
    # It runs the existing restricted MCP host, not a new or unrestricted server.
    $testApplication=@'
    <Application Id="McpTest" Executable="app\rocket-mcp.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="Rocket MCP compatibility test" Description="Isolated Rocket compatibility checks" BackgroundColor="transparent" Square150x150Logo="Assets\Logo150.png" Square44x44Logo="Assets\Logo44.png" AppListEntry="none" />
    </Application>
'@
}
# Desktop Bridge preserves the context needed by bundled Git/terminal helpers.
# The bound Lua helper uses its separate unpackaged bridge before creating LPAC.
$manifest=@"
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
 xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
 xmlns:desktop6="http://schemas.microsoft.com/appx/manifest/desktop/windows10/6"
 xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
 IgnorableNamespaces="uap desktop6 rescap">
  <Identity Name="$(Xml $IdentityName)" Publisher="$(Xml $Publisher)" Version="$(Xml $PackageVersion)" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>$display</DisplayName><PublisherDisplayName>$(Xml $PublisherDisplayName)</PublisherDisplayName><Logo>Assets\Logo50.png</Logo>
    <desktop6:FileSystemWriteVirtualization>disabled</desktop6:FileSystemWriteVirtualization>
    <desktop6:RegistryWriteVirtualization>disabled</desktop6:RegistryWriteVirtualization>
  </Properties>
  <Resources><Resource Language="en-US" /></Resources>
  <Dependencies><TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" MaxVersionTested="10.0.26100.0" /></Dependencies>
  <Applications>
    <Application Id="Rocket" Executable="app\rocket.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="$display" Description="Desktop Git client" BackgroundColor="transparent" Square150x150Logo="Assets\Logo150.png" Square44x44Logo="Assets\Logo44.png" />
    </Application>
$testApplication
  </Applications>
  <Capabilities><rescap:Capability Name="runFullTrust" /><rescap:Capability Name="unvirtualizedResources" /></Capabilities>
</Package>
"@
Write-GeneratedFile 'AppxManifest.xml' ([Text.Encoding]::UTF8.GetBytes($manifest))
$base="Rocket-$version-windows-x64"
if($CompatibilityTest){$base+='-compatibility-'+$IdentityName.Split('.')[-1]}
$base+='-'+$PackageVersion
$candidate=Join-Path $job ($base+'.msix')
$destination=Join-Path $output ($base+'.msix')
if(Test-Path -LiteralPath $destination){throw 'MSIX output already exists; refusing replacement.'}
& $compilerPath pack /d $stage /p $candidate /o *> (Join-Path $job 'makeappx-pack.log')
if($LASTEXITCODE -ne 0){throw "MakeAppx refused the package; inspect $job/makeappx-pack.log"}
# Verify what MakeAppx actually packed, not only the source tree. This also catches
# changed-and-restored input during packaging. No payload executable is launched.
function Assert-PackedInventory([IO.Compression.ZipArchive]$Package,$Inventory) {
    $expected=[Collections.Generic.Dictionary[string,object]]::new([StringComparer]::Ordinal)
    foreach($row in $Inventory){$expected.Add($row.path,$row)}
    $metadataNames=[Collections.Generic.HashSet[string]]::new([string[]]@('AppxBlockMap.xml','[Content_Types].xml'),[StringComparer]::Ordinal)
    foreach($entry in $Package.Entries){
        $kind=($entry.ExternalAttributes -shr 16) -band 0xF000
        if($kind -notin @(0,0x8000) -or ($entry.ExternalAttributes -band [IO.FileAttributes]::ReparsePoint)){throw 'Unexpected linked or special MSIX entry.'}
        if($metadataNames.Remove($entry.FullName)){continue}
        if($entry.FullName -match '%(?:2f|5c)'){throw 'Encoded path separator in package.'}
        $name=[Uri]::UnescapeDataString($entry.FullName)
        $row=$expected[$name]
        if(-not $row -or -not $expected.Remove($name)){throw "Unexpected MSIX entry: $name"}
        if($entry.Length -ne $row.size){throw "Packaged bytes differ: $name"}
        $stream=$entry.Open()
        try {$digest=Copy-PayloadEntry $stream ([IO.Stream]::Null) $row.size} finally {$stream.Dispose()}
        if($digest -ine $row.sha256){throw "Packaged bytes differ: $name"}
    }
    if($expected.Count -or $metadataNames.Count){throw 'Required MSIX entries are missing.'}
}
$verified=Join-Path $job 'verified'
$candidateStream=[IO.File]::OpenRead($candidate)
try {
    $package=[IO.Compression.ZipArchive]::new($candidateStream,'Read',$true)
    try {Assert-PackedInventory $package $inventory} finally {$package.Dispose()}
    & $compilerPath unpack /p $candidate /d $verified /o *> (Join-Path $job 'makeappx-unpack.log')
    if($LASTEXITCODE -ne 0){throw 'Could not verify the built MSIX.'}
    if((Get-FileHash -LiteralPath $compilerPath).Hash -cne $compilerHash){throw 'Compiler changed during packaging.'}
    $candidateStream.Position=0
    $hash=(Get-FileHash -InputStream $candidateStream -Algorithm SHA256).Hash.ToLowerInvariant()
    $candidateStream.Position=0
    $outputStream=[IO.File]::Open($destination,'CreateNew','Write','None')
    try {$candidateStream.CopyTo($outputStream)} catch {
        $outputStream.Dispose()
        [IO.File]::Delete($destination)
        throw
    } finally {$outputStream.Dispose()}
} finally {$candidateStream.Dispose()}
[IO.File]::WriteAllText($destination+'.sha256',"$hash  $([IO.Path]::GetFileName($destination))`n",[Text.UTF8Encoding]::new($false))
$result=[ordered]@{schema=1;version=$version;packageVersion=$PackageVersion;identity=$IdentityName;publisher=$Publisher;compatibilityTest=[bool]$CompatibilityTest;package=$destination;sha256=$hash;archive=$archivePath;archiveSha256=$ArchiveSha256;payload=$payload;stage=$stage;verified=$verified;inventory=$inventory;compiler=$compilerPath;compilerSha256=$compilerHash.ToLowerInvariant();signed=$false;audit=$auditPath}
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $job 'result.json') -Encoding utf8
Write-Output "Verified unsigned MSIX: $destination"
Write-Output "Build record: $job/result.json"
