[CmdletBinding()]
param([Parameter(Mandatory)][string]$OutputDirectory, [string]$RuntimeDirectory)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $root) { throw 'Use a new test output directory.' }
[IO.Directory]::CreateDirectory($root) | Out-Null
$toolsRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$copy = Join-Path $toolsRoot 'Copy-VcRuntime.ps1'
$verify = Join-Path $toolsRoot 'Verify-Release.ps1'
$utf8 = [Text.UTF8Encoding]::new($false)
$cases = @()
function Reject([scriptblock]$Action, [string]$Expected) {
    $failure = $null
    try { & $Action | Out-Null } catch { $failure = $_.Exception.Message }
    if (-not $failure -or $failure -notmatch $Expected) { throw "Expected rejection '$Expected'; got '$failure'." }
}
function Sums([string]$Folder) {
    $records = @(Get-ChildItem -LiteralPath $Folder -File | Where-Object Name -ne 'SHA256SUMS.txt' | ForEach-Object {
        (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + $_.Name
    })
    [IO.File]::WriteAllLines((Join-Path $Folder 'SHA256SUMS.txt'), [string[]]$records, $utf8)
}
try {
    $good = Join-Path $root 'good'
    [IO.Directory]::CreateDirectory($good) | Out-Null
    & $copy -Destination $good -RuntimeDirectory $RuntimeDirectory
    $dll = Join-Path $good 'vcruntime140.dll'
    $hash = (Get-FileHash -LiteralPath $dll -Algorithm SHA256).Hash
    if ($hash -ne 'D1F4225DF2CD877DBF130D5668A021DCE3F94118455FF5EC952061C30AFC9CE7') { throw 'Unexpected runtime copy.' }
    $cases += 'Reviewed Microsoft runtime copied and hash verified'
    Reject { & $copy -Destination $good -RuntimeDirectory $RuntimeDirectory } 'destination already exists'
    if ((Get-FileHash -LiteralPath $dll -Algorithm SHA256).Hash -ne $hash) { throw 'Existing runtime changed.' }
    $cases += 'Existing destination refused without replacing its contents'
    $bad = Join-Path $root 'unreviewed-source'
    $empty = Join-Path $root 'empty-destination'
    [IO.Directory]::CreateDirectory($bad) | Out-Null
    [IO.Directory]::CreateDirectory($empty) | Out-Null
    [IO.File]::WriteAllText((Join-Path $bad 'vcruntime140.dll'), 'synthetic wrong runtime', $utf8)
    Reject { & $copy -Destination $empty -RuntimeDirectory $bad } 'Unreviewed Visual C\+\+ runtime source'
    if (@(Get-ChildItem -LiteralPath $empty -Force).Count) { throw 'Rejected source left a file in the destination.' }
    $cases += 'Unreviewed source refused before any destination write'
    Sums $empty
    Reject { & $verify -Bundle $empty } 'App-local Visual C\+\+ runtime missing'
    $cases += 'Verifier rejects a missing runtime even with a consistent manifest'
    Sums $bad
    Reject { & $verify -Bundle $bad } 'Unreviewed app-local Visual C\+\+ runtime'
    $cases += 'Verifier rejects a replaced runtime with a regenerated manifest'
    $cases | ForEach-Object { Write-Output "PASS $_" }
    [ordered]@{passed=$true;cases=$cases} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'result.json') -Encoding utf8
} catch {
    [ordered]@{passed=$false;cases=$cases;error=$_.Exception.Message} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'result.json') -Encoding utf8
    throw
}
