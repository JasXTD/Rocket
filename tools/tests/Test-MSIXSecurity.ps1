# Isolated package and certificate tests. Only an in-memory store is used.
using namespace System.Security.Cryptography
using namespace System.Security.Cryptography.X509Certificates
using namespace System.IO.Compression
$ErrorActionPreference='Stop'
$toolsRoot=Split-Path $PSScriptRoot
. (Join-Path $PSScriptRoot 'MSIX-Compatibility.ps1')
. (Join-Path $toolsRoot 'windows/Package-Audit.ps1')

function Must-Fail([scriptblock]$Run,[string]$Pattern) {
    try {& $Run} catch {if($_.Exception.Message -notmatch $Pattern){throw};return}
    throw "Expected rejection matching $Pattern."
}
function Hash([byte[]]$Bytes){[Convert]::ToHexString([SHA256]::HashData($Bytes)).ToLowerInvariant()}
function Test-Zip($Entries,[scriptblock]$Check) {
    $buffer=[IO.MemoryStream]::new()
    try {
        $zip=[ZipArchive]::new($buffer,'Create',$true)
        try {
            foreach($entry in $Entries){
                $part=$zip.CreateEntry($entry.name)
                if($entry.attributes){$part.ExternalAttributes=$entry.attributes}
                $stream=$part.Open()
                try {$stream.Write([Text.Encoding]::UTF8.GetBytes($entry.text))} finally {$stream.Dispose()}
            }
        } finally {$zip.Dispose()}
        $buffer.Position=0
        $zip=[ZipArchive]::new($buffer,'Read',$true)
        try {& $Check $zip} finally {$zip.Dispose()}
    } finally {$buffer.Dispose()}
}

$testDir=Join-Path $toolsRoot ('../work/msix-security-check-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testDir)
$staged=Join-Path $testDir 'stage/tools'
[void][IO.Directory]::CreateDirectory($staged)
$runtime=Join-Path $toolsRoot 'Git-Runtime.ps1'
Copy-Item -LiteralPath $runtime -Destination $testDir
$cached=(Get-FileHash -LiteralPath $runtime).Hash
$bundleRoot=Split-Path $staged
$runtimeChecker=Join-Path $staged 'Git-Runtime.ps1'
$checked=@{};$hashes=@{};$checked[$runtimeChecker]=$true;$hashes[$runtimeChecker]=$cached
[IO.File]::WriteAllText($runtimeChecker,"throw 'Staged helper was invoked'")
$source=[IO.File]::ReadAllText((Join-Path $toolsRoot 'Verify-Release.ps1'))
$start=$source.IndexOf('    $runtimeChecker =')
$end=$source.IndexOf('    $inventory = Get-RocketGitInventory',$start)
if($start -lt 0 -or $end -le $start){throw 'Missing verifier helper checks.'}
$loader=Join-Path $testDir 'verify-helper.ps1'
[IO.File]::WriteAllText($loader,$source.Substring($start,$end-$start)+'(Get-RocketGitSpec).Version')
if((& $loader) -cne '2.56.1.windows.1'){throw 'Reviewed helper was not used.'}
$hashes[$runtimeChecker]='0'*64
Must-Fail {& $loader} 'Unreviewed bundled Git verification helper'
$hashes[$runtimeChecker]=$cached
[IO.File]::AppendAllText((Join-Path $testDir 'Git-Runtime.ps1'),"# modified")
Must-Fail {& $loader} 'Unreviewed bundled Git verification helper'
'PASS release verification uses its own pinned helper after staging changes.'

$report=Join-Path $testDir 'audit.json'
$summary=& node (Join-Path $toolsRoot 'windows/audit-payload.cjs') $staged $report
if($LASTEXITCODE -ne 0){throw 'Fixture privacy audit failed.'}
$audited=Read-PackageAudit $report $summary
if($audited.inventory.Count -ne 1){throw 'Missing audited file.'}
[IO.File]::AppendAllText($report,' ')
Must-Fail {Read-PackageAudit $report $summary} 'audit report changed'

$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $toolsRoot 'Package-MSIX.ps1'),[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Invalid packager script.'}
foreach($name in @('Copy-PayloadEntry','Assert-PackedInventory')){
    $fn=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true)
    . ([scriptblock]::Create($fn.Extent.Text))
}
$entries=@(@{name='AppxBlockMap.xml';text=''},@{name='[Content_Types].xml';text=''})
$row=@{path='app/docs/a+b';size=4;sha256=Hash ([Text.Encoding]::UTF8.GetBytes('good'))}
$item=@{name='app/docs/a%2Bb';text='good'}
$verify={param($zip) Assert-PackedInventory $zip @($row)}
Test-Zip ($entries+@($item)) $verify
Must-Fail {Test-Zip ($entries+@(@{name=$item.name;text='edit'})) $verify} 'Packaged bytes differ'
Must-Fail {Test-Zip $entries $verify} 'missing'
Must-Fail {Test-Zip ($entries+@($item,$item)) $verify} 'Unexpected MSIX entry'
Must-Fail {Test-Zip ($entries+@($item,@{name='app/extra';text=''})) $verify} 'Unexpected MSIX entry'
Must-Fail {Test-Zip ($entries+@(@{name=$item.name;text='good';attributes=1024})) $verify} 'linked or special'
'PASS sealed audit receipt and candidate inventory reject replaced, missing, extra and duplicate data.'

$name='Rocket.CompatibilityTest.012345abcdef'
$subject='CN=Rocket Compatibility Test 012345abcdef'
$version='1.2.3.0'
$xml="<Package xmlns='http://schemas.microsoft.com/appx/manifest/foundation/windows10'><Identity Name='$name' Publisher='$subject' Version='$version'/></Package>"
$manifest=@{name='AppxManifest.xml';text=$xml}
$verify={param($zip) Assert-CompatibilityPackage $zip $name $subject $version}
Test-Zip @($manifest) $verify
foreach($wrong in @($name,$subject,$version)){
    Must-Fail {Test-Zip @(@{name=$manifest.name;text=$xml.Replace($wrong,'different')}) $verify} 'does not match'
}
Must-Fail {Test-Zip @($manifest,$manifest) $verify} 'single compatibility manifest'
'PASS deployment requires one matching package manifest.'

class MemoryTrustStore {
    [X509Certificate2Collection]$Certificates=[X509Certificate2Collection]::new()
    [bool]$FailAfterAdd
    [int]$Adds
    [void] Add([X509Certificate2]$cert) {
        $this.Adds++
        [void]$this.Certificates.Add($cert)
        if($this.FailAfterAdd){throw 'Simulated store failure'}
    }
    [void] Remove([X509Certificate2]$cert){$this.Certificates.Remove($cert)}
}
$rsa=[RSA]::Create(2048)
function New-Leaf([int]$Start,[int]$End,[bool]$CA=$false,[string]$EKU='1.3.6.1.5.5.7.3.3',[string]$Usage='DigitalSignature') {
    $req=[CertificateRequest]::new($subject,$rsa,[HashAlgorithmName]::SHA256,[RSASignaturePadding]::Pkcs1)
    $req.CertificateExtensions.Add([X509BasicConstraintsExtension]::new($CA,$false,0,$true))
    $oids=[OidCollection]::new();[void]$oids.Add([Oid]::new($EKU))
    $req.CertificateExtensions.Add([X509EnhancedKeyUsageExtension]::new($oids,$true))
    $req.CertificateExtensions.Add([X509KeyUsageExtension]::new($Usage,$true))
    $now=[DateTimeOffset]::UtcNow
    $leaf=$req.CreateSelfSigned($now.AddHours($Start),$now.AddHours($End))
    try {return [X509Certificate2]::new($leaf.RawData)} finally {$leaf.Dispose()}
}
try {
    $store=[MemoryTrustStore]::new()
    foreach($case in @(@(-1,96),@(-120,24),@(-48,-1),@(1,48),@(-1,48,$true),@(-1,48,$false,'1.3.6.1.5.5.7.3.1'),@(-1,48,$false,'1.3.6.1.5.5.7.3.3','KeyEncipherment'))){
        $cert=New-Leaf @case
        try {
            Must-Fail {Add-CompatibilityTrust $store $cert $subject $cert.Thumbprint} 'validity period|code-signing leaf'
            Assert-CompatibilityIdentity $cert $subject $cert.Thumbprint
        } finally {$cert.Dispose()}
    }
    $cert=New-Leaf -1 48
    try {
        Must-Fail {Add-CompatibilityTrust $store $cert $subject ('0'*40)} 'certificate identity'
        if($store.Adds -or $store.Certificates.Count){throw 'Invalid certificate reached trust insertion.'}
        $store.FailAfterAdd=$true
        Must-Fail {Add-CompatibilityTrust $store $cert $subject $cert.Thumbprint} 'Simulated store failure'
        if($store.Certificates.Count){throw 'Temporary trust survived failed insertion.'}
        $store.FailAfterAdd=$false
        Add-CompatibilityTrust $store $cert $subject $cert.Thumbprint
        Must-Fail {Add-CompatibilityTrust $store $cert $subject $cert.Thumbprint} 'already trusted'
        if($store.Certificates.Count -ne 1 -or $store.Adds -ne 2){throw 'Existing trust was changed.'}
    } finally {$cert.Dispose()}
    $overlong=New-Leaf -1 100
    try {
        $buildRecord=Join-Path $testDir 'build.json'
        $signingRecord=Join-Path $testDir 'signing.json'
        @{compatibilityTest=$true;identity=$name;publisher=$subject} | ConvertTo-Json | Set-Content -LiteralPath $buildRecord
        @{identity=$name;thumbprint=$overlong.Thumbprint} | ConvertTo-Json | Set-Content -LiteralPath $signingRecord
        $storePath='Cert:\LocalMachine\TrustedPeople\'+$overlong.Thumbprint
        $overlong | Add-Member NoteProperty PSPath $storePath
        $fixture=@{certificate=$overlong}
        # Exercise the host's actual cleanup action against only this fake provider entry.
        & {
            function Get-Item([string]$LiteralPath) {
                if($LiteralPath -cne $storePath){throw 'Unexpected certificate lookup.'}
                if($fixture.certificate){return $fixture.certificate}
                throw [Management.Automation.ItemNotFoundException]::new()
            }
            function Remove-Item([string]$LiteralPath) {
                if($LiteralPath -cne $storePath){throw 'Unexpected certificate removal.'}
                $fixture.certificate=$null
            }
            $hostScript=Join-Path $PSScriptRoot 'MSIX-TestHost.ps1'
            Must-Fail {& $hostScript -Action RemoveTrust -Record $buildRecord -SigningRecord $signingRecord -ExpectedThumbprint ('0'*40)} 'explicitly approved'
            if(-not $fixture.certificate){throw 'Cleanup accepted a different thumbprint.'}
            $null=& $hostScript -Action RemoveTrust -Record $buildRecord -SigningRecord $signingRecord -ExpectedThumbprint $overlong.Thumbprint
            if($fixture.certificate){throw 'Approved overlong certificate was left behind.'}
        }
    } finally {$overlong.Dispose()}
} finally {$rsa.Dispose()}
'PASS invalid certificates are rejected before insertion, partial insertion rolls back, and cleanup can identify expired or overlong certificates.'
