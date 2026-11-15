# Packaging must inspect input as data; it must never launch an input executable.
param([string]$ScriptPath = (Join-Path $PSScriptRoot '../Package-Installer.ps1'))
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$source = [IO.File]::ReadAllText((Resolve-Path -LiteralPath $ScriptPath).ProviderPath)
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseInput($source, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Packaging script has parse errors.' }
$calls = $ast.FindAll({param($node) $node -is [Management.Automation.Language.CommandAst]}, $true)
foreach ($call in $calls) {
    if ($call.InvocationOperator -in @('Ampersand','Dot') -and
        $call.CommandElements[0].Extent.Text -match '\$payload\b|rocket-cli\.exe') {
        throw 'Packaging must not invoke a payload-supplied command.'
    }
}
# Exercise the production metadata statements, including anything placed between
# metadata reading and the audit. The old version check tried to execute the fake
# CLI below and failed here. No verifier or compiler is replaced by a test double.
$start = $source.IndexOf('$metadata=')
$end = $source.IndexOf('$auditPath=', $start)
if ($start -lt 0 -or $end -le $start) { throw 'Metadata validation statements not found.' }
$validate = [ScriptBlock]::Create($source.Substring($start, $end - $start) + '; $version')
$payload = Join-Path $root ('work/installer-input-test-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($payload)
[IO.File]::WriteAllText((Join-Path $payload 'rocket-cli.exe'), 'MZ untrusted fixture - never execute')
$metadataPath = Join-Path $payload 'rocket-update.json'
$cases = 0
foreach ($case in @(
    @{version='0.92.4'; platform='windows-x64'; entrypoint='rocket.exe'; accept=$true},
    @{version='0.92.4; echo injected'; platform='windows-x64'; entrypoint='rocket.exe'; accept=$false},
    @{version='0.92.4'; platform='linux-x64'; entrypoint='rocket.exe'; accept=$false},
    @{version='0.92.4'; platform='windows-x64'; entrypoint='../other.exe'; accept=$false}
)) {
    @{schema=1;version=$case.version;platform=$case.platform;entrypoint=$case.entrypoint} |
        ConvertTo-Json | Set-Content -LiteralPath $metadataPath -Encoding utf8
    $accepted = $false
    try { $value = & $validate; $accepted = $value -ceq $case.version }
    catch { if ($case.accept) { throw } }
    if ($accepted -ne $case.accept) { throw 'Unexpected metadata acceptance result.' }
    $cases++
}
Write-Output "PASS: $cases metadata cases; no payload execution. Fixture: $payload"
