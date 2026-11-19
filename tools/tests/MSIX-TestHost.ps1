# Native test operations are restricted to disposable compatibility identities.
# Trust is deliberately separate: run Trust only after explicit approval/UAC.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('PrepareSigning','Sign','Trust','RemoveTrust','Install','ExpectRejection','Inspect','Activate','Uninstall','RemoveSigning')][string]$Action,
    [Parameter(Mandatory)][string]$Record,
    [string]$SigningRecord,
    [ValidatePattern('^[A-Fa-f0-9]{40}$')][string]$ExpectedThumbprint,
    [string]$Profile,
    [string]$Candidate
)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'MSIX-Compatibility.ps1')
$recordPath=(Resolve-Path -LiteralPath $Record).ProviderPath
$build=Get-Content -LiteralPath $recordPath -Raw | ConvertFrom-Json
if(-not $build.compatibilityTest -or $build.identity -cnotmatch '^Rocket\.CompatibilityTest\.[a-f0-9]{12}$'){throw 'Only a unique compatibility package is allowed.'}
$identity=[string]$build.identity
$subject='CN=Rocket Compatibility Test '+$identity.Split('.')[-1]
if($build.publisher -cne $subject){throw 'Unexpected compatibility publisher.'}
$job=Split-Path $recordPath
$signing=if($SigningRecord){Get-Content -LiteralPath $SigningRecord -Raw|ConvertFrom-Json}else{$null}
if($signing -and ($signing.identity -cne $identity -or $signing.thumbprint -notmatch '^[A-Fa-f0-9]{40}$')){throw 'Signing identity mismatch.'}
if($Action -in @('Trust','RemoveTrust') -and (-not $ExpectedThumbprint -or $ExpectedThumbprint -ine $signing.thumbprint)){throw 'Trust changes require the explicitly approved certificate thumbprint.'}
function Cert([string]$store){
    if(-not $signing){throw 'A signing record is required.'}
    try {$cert=Get-Item -LiteralPath ("Cert:\"+$store+'\'+$signing.thumbprint) -ErrorAction Stop}
    catch [Management.Automation.ItemNotFoundException] {return}
    if($cert){Assert-CompatibilityIdentity $cert $subject $signing.thumbprint}
    $cert
}
function AppxModule { if($PSVersionTable.PSEdition -eq 'Core'){Import-Module Appx -UseWindowsPowerShell -WarningAction SilentlyContinue}else{Import-Module Appx} }
function Package { @(Get-AppxPackage -Name $identity -ErrorAction Stop) | Where-Object {$_.Name -ceq $identity -and $_.Publisher -ceq $subject} }
switch($Action){
    'PrepareSigning' {
        if($SigningRecord){throw 'PrepareSigning never reuses a certificate.'}
        if(Test-Path -LiteralPath (Join-Path $job 'signing.json')){throw 'Signing record already exists.'}
        $cert=New-SelfSignedCertificate -Type Custom -Subject $subject -FriendlyName $identity -CertStoreLocation Cert:\CurrentUser\My -KeyExportPolicy NonExportable -KeyUsage DigitalSignature -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256 -NotAfter (Get-Date).AddDays(2) -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3','2.5.29.19={text}')
        $publicPath=Join-Path $job 'test-signing.cer'
        [void](Export-Certificate -Cert $cert -FilePath $publicPath -Type CERT)
        $value=[ordered]@{identity=$identity;thumbprint=$cert.Thumbprint;publicCertificate=$publicPath;publicSha256=(Get-FileHash -LiteralPath $publicPath).Hash;expires=$cert.NotAfter.ToUniversalTime().ToString('o')}
        $value|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $job 'signing.json') -Encoding utf8
        $value|ConvertTo-Json -Compress
    }
    'Sign' {
        $cert=Cert 'CurrentUser\My';if(-not $cert -or -not $cert.HasPrivateKey){throw 'Disposable signing key is missing.'}
        Assert-CompatibilityCertificate $cert $subject $signing.thumbprint
        $tool=Join-Path (Split-Path $build.compiler) 'signtool.exe'
        $signature=Get-AuthenticodeSignature -LiteralPath $tool
        if($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Microsoft Corporation(?:,|$)'){throw 'SignTool signature check failed.'}
        $signed=Join-Path $job 'compatibility-signed.msix'
        if(Test-Path -LiteralPath $signed){throw 'Signed test package already exists.'}
        $source=[IO.File]::OpenRead($build.package)
        try {
            if((Get-FileHash -InputStream $source -Algorithm SHA256).Hash -ine $build.sha256){throw 'Unsigned package changed.'}
            $source.Position=0
            $zip=[IO.Compression.ZipArchive]::new($source,'Read',$true)
            try {Assert-CompatibilityPackage $zip $identity $subject $build.packageVersion} finally {$zip.Dispose()}
            $source.Position=0
            $copy=[IO.File]::Open($signed,'CreateNew','Write','None')
            try {$source.CopyTo($copy)} finally {$copy.Dispose()}
        } finally {$source.Dispose()}
        & $tool sign /fd SHA256 /s My /sha1 $cert.Thumbprint $signed *> (Join-Path $job 'sign.log')
        if($LASTEXITCODE -ne 0){throw 'Test signing failed; inspect sign.log.'}
        $value=[ordered]@{path=$signed;sha256=(Get-FileHash -LiteralPath $signed).Hash;thumbprint=$cert.Thumbprint}
        $value|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $job 'signed.json') -Encoding utf8
        $value|ConvertTo-Json -Compress
    }
    'Trust' {
        if(-not $signing){throw 'Signing record required.'}
        $bytes=[IO.File]::ReadAllBytes($signing.publicCertificate)
        if([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)) -ine $signing.publicSha256){throw 'Public certificate changed.'}
        if([Security.Cryptography.X509Certificates.X509Certificate2]::GetCertContentType($bytes) -ne 'Cert'){throw 'Trust requires a public certificate file.'}
        $public=[Security.Cryptography.X509Certificates.X509Certificate2]::new($bytes)
        # Add the exact in-memory certificate that was checked. Do not reopen a
        # mutable path after verification, and never import a private key here.
        $store=[Security.Cryptography.X509Certificates.X509Store]::new('TrustedPeople','LocalMachine')
        try {$store.Open('ReadWrite');Add-CompatibilityTrust $store $public $subject $ExpectedThumbprint} finally {$store.Dispose();$public.Dispose()}
        Write-Output 'Temporary public certificate trusted.'
    }
    'RemoveTrust' {
        $cert=Cert 'LocalMachine\TrustedPeople'
        if($cert){Remove-Item -LiteralPath $cert.PSPath}
        if(Cert 'LocalMachine\TrustedPeople'){throw 'Temporary trust remains.'}
        Write-Output 'Temporary trust removed.'
    }
    'RemoveSigning' {
        $cert=Cert 'CurrentUser\My'
        if($cert){Remove-Item -LiteralPath $cert.PSPath -DeleteKey}
        if(Cert 'CurrentUser\My'){throw 'Temporary signing key remains.'}
        Write-Output 'Temporary signing certificate and private key removed.'
    }
    {$_ -in @('Install','ExpectRejection')} {
        if($Action -eq 'Install'){
            $signed=Get-Content -LiteralPath (Join-Path $job 'signed.json') -Raw|ConvertFrom-Json
            $file=Join-Path $job 'compatibility-signed.msix'
            if(-not $signing -or $signed.thumbprint -ine $signing.thumbprint){throw 'Unexpected test signer.'}
        } else {$file=(Resolve-Path -LiteralPath $Candidate).ProviderPath}
        $locked=[IO.File]::OpenRead($file)
        try {
            $zip=[IO.Compression.ZipArchive]::new($locked,'Read',$true)
            try {Assert-CompatibilityPackage $zip $identity $subject $build.packageVersion} finally {$zip.Dispose()}
            $signature=Get-AuthenticodeSignature -LiteralPath $file
            AppxModule
            if($Action -eq 'ExpectRejection'){
                if($signature.Status -eq 'Valid'){throw 'Negative test requires an invalid or unsigned package.'}
                try {Add-AppxPackage -Path $file -ErrorAction Stop} catch {
                    if($_.Exception.Message -notmatch '0x(?:800B0100|80096010|80096004)'){throw}
                    Write-Output 'Windows rejected the compatibility package.'
                    break
                }
                throw 'Windows accepted the negative test package.'
            }
            $locked.Position=0
            if((Get-FileHash -InputStream $locked -Algorithm SHA256).Hash -ine $signed.sha256 -or $signature.Status -ne 'Valid'){throw 'Test package hash or signature is invalid.'}
            Assert-CompatibilityCertificate $signature.SignerCertificate $subject $signing.thumbprint
            if(-not (Cert 'LocalMachine\TrustedPeople')){throw 'Expected disposable certificate is not explicitly trusted.'}
            Add-AppxPackage -Path $file -ErrorAction Stop
            Package | Select-Object Name,Publisher,Version,PackageFullName,PackageFamilyName,InstallLocation,IsDevelopmentMode | ConvertTo-Json -Compress
        } finally {$locked.Dispose()}
    }
    'Inspect' {
        AppxModule
        @(Package | Select-Object Name,Publisher,Version,PackageFullName,PackageFamilyName,InstallLocation,IsDevelopmentMode) | ConvertTo-Json -Compress
    }
    'Uninstall' {
        AppxModule
        foreach($package in @(Package)){Remove-AppxPackage -Package $package.PackageFullName -ErrorAction Stop}
        if(@(Package).Count){throw 'Test package remains installed.'}
        Write-Output 'Disposable package removed.'
    }
    'Activate' {
        if(-not $Profile -or -not [IO.Path]::IsPathFullyQualified($Profile) -or $Profile.Contains('"')){throw 'Absolute MCP profile required.'}
        AppxModule
        $packages=@(Package);if($packages.Count -ne 1){throw 'Expected one installed compatibility package.'}
        # Activation is essential: directly running an EXE from the staging folder
        # does not prove it works under Windows package identity.
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
[ComImport, Guid("2e941141-7f97-4756-ba1d-9decde894a3d"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IActivationManager {
  [PreserveSig] int ActivateApplication([MarshalAs(UnmanagedType.LPWStr)] string id, [MarshalAs(UnmanagedType.LPWStr)] string args, uint options, out uint process);
}
public static class RocketMsixActivation {
  public static uint Start(string id, string args) {
    object obj=Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("45ba127d-10a8-46ea-8ab7-56ea9078943c")));
    try { uint process; int result=((IActivationManager)obj).ActivateApplication(id,args,0,out process); Marshal.ThrowExceptionForHR(result); return process; }
    finally { Marshal.ReleaseComObject(obj); }
  }
}
'@
        [RocketMsixActivation]::Start($packages[0].PackageFamilyName+'!McpTest','--http-profile "'+$Profile+'"')
    }
}
