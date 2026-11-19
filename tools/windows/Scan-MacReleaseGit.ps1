# The Mac job may build only the exact source bytes scanned on this disposable runner.
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows') {
    throw 'A disposable Windows Actions runner is required.'
}
$phase = 'locating Microsoft Defender'
trap {
    Write-Host "::error::Git source verification failed while $phase."
    exit 1
}
$status = Get-MpComputerStatus
if (-not $status.AMServiceEnabled -or -not $status.AntivirusEnabled -or -not $status.AntivirusSignatureVersion) {
    throw 'Microsoft Defender Antivirus must be active with security intelligence installed.'
}
$scanner = Join-Path $env:ProgramFiles 'Windows Defender/MpCmdRun.exe'
$signature = Get-AuthenticodeSignature -LiteralPath $scanner
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation(,|$)') {
    throw 'Microsoft Defender executable signature verification failed.'
}
$phase = 'verifying the official source archive'
$name = 'git-2.56.1.tar.gz'
$pin = (Get-Content -LiteralPath (Join-Path $PSScriptRoot '../macos/release-git.sha256') -Raw).Trim()
if ($pin -cnotmatch '^([0-9a-f]{64})  git-2\.56\.1\.tar\.gz$') {throw 'Invalid Git source digest pin.'}
$expected = $Matches[1]
$archive = Join-Path $env:RUNNER_TEMP $name
if (Test-Path -LiteralPath $archive) {throw 'Expected a fresh scan path.'}
Invoke-WebRequest -Uri "https://www.kernel.org/pub/software/scm/git/$name" -OutFile $archive -MaximumRedirection 0 -TimeoutSec 120
function Assert-PinnedArchive {
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expected) {
        throw 'Git source does not match its published SHA-256.'
    }
}
Assert-PinnedArchive
$phase = 'scanning the pinned archive with Microsoft Defender'
& $scanner -Scan -ScanType 3 -File $archive -DisableRemediation
if ($LASTEXITCODE -ne 0) {throw 'The required Defender scan failed or detected a threat.'}
Assert-PinnedArchive
"sha256=$expected" >> $env:GITHUB_OUTPUT
Write-Host 'Microsoft Defender passed the pinned Git 2.56.1 source archive.'
