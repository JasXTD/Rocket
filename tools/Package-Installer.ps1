# Wrap an already verified Windows update/portable ZIP. Never builds, uploads or replaces a release.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Archive,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ArchiveSha256,
    [Parameter(Mandatory)][string]$Compiler,
    [string]$OutputDirectory,
    [switch]$CompareLocalSshKeys,
    [ValidatePattern('^[a-z0-9]{8,32}$')][string]$TestIdentity
)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $OutputDirectory){$OutputDirectory=Join-Path $PSScriptRoot 'releases'}
$output=[IO.Path]::GetFullPath($OutputDirectory)
$archivePath=(Resolve-Path -LiteralPath $Archive).ProviderPath
$compilerPath=(Resolve-Path -LiteralPath $Compiler).ProviderPath
if((Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash -ine $ArchiveSha256){throw 'Portable ZIP differs from its reviewed SHA-256.'}
if((Get-Item -LiteralPath $archivePath).Length -gt 536870912){throw 'Portable ZIP exceeds the 512 MiB archive limit.'}
$compilerVersion='6.7.3'
# This installed compiler reports 0.0.0.0 in PE version resources. Pin its reviewed
# bytes and publisher instead of treating those resources as a version guarantee.
if((Get-FileHash -LiteralPath $compilerPath).Hash -ine '0a8757031b33777e4c9cbffee40f11a5062b36d25cbe144c1db73b6102b80ad7'){throw 'Use the reviewed Inno Setup 6.7.3 compiler; audit a different compiler before updating this pin.'}
$compilerSignature=Get-AuthenticodeSignature -LiteralPath $compilerPath
if($compilerSignature.Status -ne 'Valid' -or $compilerSignature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Pyrsys B\.V\.(?:,|$)'){throw 'Inno compiler signature/publisher check failed.'}
foreach($name in @('ISCmplr.dll','ISPP.dll','islzma.dll','islzma64.exe')){
    $signature=Get-AuthenticodeSignature -LiteralPath (Join-Path (Split-Path $compilerPath) $name)
    if($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Pyrsys B\.V\.(?:,|$)'){throw "Inno supporting component signature failed: $name"}
}
[void](Get-Command node -ErrorAction Stop)
$job=Join-Path $projectRoot ('work/installer-build-'+[Guid]::NewGuid().ToString('N'))
$payload=Join-Path $job 'payload'
[void][IO.Directory]::CreateDirectory($payload)
[void][IO.Directory]::CreateDirectory($output)
$zip=[IO.Compression.ZipFile]::OpenRead($archivePath)
try {
    $names=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    [long]$total=0
    if($zip.Entries.Count -gt 20000){throw 'Too many package entries.'}
    foreach($entry in $zip.Entries){
        $name=$entry.FullName
        if($name -match '[\\:\x00-\x1f<>"|?*]' -or $name.StartsWith('/') -or $name.EndsWith('/') -or
           @($name.Split('/')|Where-Object {$_ -in @('','.','..') -or $_.EndsWith('.') -or $_.EndsWith(' ')}).Count -or
           -not $names.Add($name)){throw 'Unsafe, directory or duplicate archive path.'}
        $kind=($entry.ExternalAttributes -shr 16) -band 0xF000
        if($kind -notin @(0,0x8000) -or $entry.Length -gt 268435456){throw 'Archive contains a link, special or oversized file.'}
        $total+=$entry.Length;if($total -gt 1073741824){throw 'Archive exceeds the unpacked size limit.'}
    }
    foreach($entry in $zip.Entries){
        $destination=[IO.Path]::GetFullPath((Join-Path $payload $entry.FullName))
        if(-not $destination.StartsWith($payload+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Archive path leaves staging.'}
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
        $entryStream=$entry.Open();$file=[IO.File]::Open($destination,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        try{$entryStream.CopyTo($file);if($file.Length -ne $entry.Length){throw 'Extracted file length differs from the ZIP directory.'}}
        finally{$file.Dispose();$entryStream.Dispose()}
    }
}finally{$zip.Dispose()}
# Use the trusted source verifier, never execute a verifier supplied by an archive.
& (Join-Path $PSScriptRoot 'Verify-Release.ps1') -Bundle $payload -RequireBundledGit
$metadata=Get-Content -LiteralPath (Join-Path $payload 'rocket-update.json') -Raw|ConvertFrom-Json
$version=[string]$metadata.version
if($metadata.schema -ne 1 -or $metadata.platform -cne 'windows-x64' -or $metadata.entrypoint -cne 'rocket.exe' -or $version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'){throw 'Unexpected update metadata.'}
$cli=& (Join-Path $payload 'rocket-cli.exe') --json --version
if($LASTEXITCODE -ne 0){throw 'Packaged CLI version check failed.'}
$cliVersion=$cli|ConvertFrom-Json
if(-not $cliVersion.ok -or $cliVersion.version -cne $version -or $cliVersion.data.version -cne $version){throw 'Package binary version differs from metadata.'}
$auditPath=Join-Path $job 'payload-audit.json'
$auditArgs=@((Join-Path $PSScriptRoot 'windows/audit-payload.cjs'),$payload,$auditPath)
if($CompareLocalSshKeys){$auditArgs+='--compare-local-ssh'}
& node @auditArgs
if($LASTEXITCODE -ne 0){throw 'Package privacy check refused the payload. See the value-free audit report.'}
$audit=Get-Content -LiteralPath $auditPath -Raw|ConvertFrom-Json
function Quote-Inno([string]$value){return '"'+$value.Replace('"','""').Replace('{','{{')+'"'}
$lines=foreach($entry in $audit.inventory){
    $native=$entry.path.Replace('/','\')
    $dir=[IO.Path]::GetDirectoryName($native)
    $dest='{code:GetStageDir}';if($dir){$dest+='\'+$dir.Replace('{','{{').Replace('"','""')}
    'Source: '+(Quote-Inno (Join-Path $payload $native))+'; DestDir: "'+$dest+'"; Flags: ignoreversion uninsneveruninstall'
}
$include=Join-Path $job 'payload.iss'
[IO.File]::WriteAllLines($include,$lines,[Text.UTF8Encoding]::new($true))
$manifestHash=(Get-FileHash -LiteralPath (Join-Path $payload 'SHA256SUMS.txt') -Algorithm SHA256).Hash.ToLowerInvariant()
$base="Rocket-$version-Setup-x64"
$definitions=@("/DRocketVersion=$version","/DPayloadInclude=$include","/DPayloadManifestHash=$manifestHash","/DOutputDirectory=$job")
if($TestIdentity){$base+="-test-$TestIdentity";$definitions+=@("/DProductId=Rocket.InstallerTest.$TestIdentity","/DProductName=Rocket Installer Test $TestIdentity")}
$definitions+="/DOutputBase=$base"
$destination=[IO.Path]::GetFullPath((Join-Path $output ($base+'.exe')))
if(-not $destination.StartsWith($output.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Installer output leaves the selected directory.'}
if((Test-Path -LiteralPath $destination) -or (Test-Path -LiteralPath ($destination+'.sha256'))){throw 'Installer output already exists; refuse to overwrite it.'}
$scriptPath=Join-Path $PSScriptRoot 'windows/rocket.iss'
$inputs=@($scriptPath,$include,(Join-Path $projectRoot 'crates/rocket-ui/src/rocket.ico'))
$inputHashes=@{};foreach($inputPath in $inputs){$inputHashes[$inputPath]=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash.ToLowerInvariant()}
& $compilerPath @definitions $scriptPath 2>&1 | Tee-Object -FilePath (Join-Path $job 'compile.log')
if($LASTEXITCODE -ne 0){throw 'Inno Setup compilation failed; candidate and log retained.'}
foreach($inputPath in $inputs){if((Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $inputHashes[$inputPath]){throw 'Installer source changed while compiling.'}}
foreach($entry in $audit.inventory){if((Get-FileHash -LiteralPath (Join-Path $payload $entry.path) -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry.sha256){throw 'Payload changed while compiling.'}}
if((Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash -ine $ArchiveSha256){throw 'Portable ZIP changed during packaging.'}
$candidate=Join-Path $job ($base+'.exe')
$setupHash=(Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::Move($candidate,$destination)
[IO.File]::WriteAllText($destination+'.sha256',"$setupHash  $base.exe`n",[Text.UTF8Encoding]::new($false))
$result=[ordered]@{schema=1;version=$version;installer=$destination;sha256=$setupHash;portableArchive=$archivePath;portableSha256=$ArchiveSha256.ToLowerInvariant();compilerVersion=$compilerVersion;compilerSha256=(Get-FileHash -LiteralPath $compilerPath -Algorithm SHA256).Hash.ToLowerInvariant();compilerPublisher=$compilerSignature.SignerCertificate.Subject;payload=$payload;payloadAudit=$auditPath;testIdentity=$TestIdentity;unsignedRocketInstaller=$true;published=$false}
$result.installerScriptSha256=$inputHashes[$scriptPath]
$result|ConvertTo-Json -Depth 4|Set-Content -LiteralPath (Join-Path $job 'result.json') -Encoding utf8
$result|ConvertTo-Json -Depth 4
