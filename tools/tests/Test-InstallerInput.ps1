# Packaging must inspect input as data; it must never launch an input executable.
param([string]$ScriptPath = (Join-Path $PSScriptRoot '../Package-MSIX.ps1'))
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
$validate = [ScriptBlock]::Create($source.Substring($start, $end - $start) + '; [pscustomobject]@{App=$version; Package=$PackageVersion}')
$payload = Join-Path $root ('work/installer-input-test-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($payload)
[IO.File]::WriteAllText((Join-Path $payload 'rocket-cli.exe'), 'MZ untrusted fixture - never execute')
$metadataPath = Join-Path $payload 'rocket-update.json'
$cases = @(
    @{version='0.92.4'; platform='windows-x64'; entrypoint='rocket.exe'; accept=$true},
    @{version='0.92.4; echo injected'; platform='windows-x64'; entrypoint='rocket.exe'; accept=$false},
    @{version='0.92.4'; platform='linux-x64'; entrypoint='rocket.exe'; accept=$false},
    @{version='0.92.4'; platform='windows-x64'; entrypoint='../other.exe'; accept=$false}
)
if ((Split-Path -Leaf $ScriptPath) -eq 'Package-MSIX.ps1') {
    $cases += @(
        @{version='0.92.6'; requested=$null; expected='1.92.6.0'; accept=$true},
        @{version='0.92.6'; requested='1.92.6.0'; expected='1.92.6.0'; accept=$true},
        @{version='0.92.6'; requested='1.92.5.0'; accept=$false},
        @{version='0.92.5'; requested='1.92.6.0'; accept=$false},
        @{version='0.92.6'; requested='2.0.0.0'; testIdentity=$true; expected='2.0.0.0'; accept=$true},
        @{version='0.65536.0'; accept=$false},
        @{version='0.92.6'; requested='1.92.65536.0'; testIdentity=$true; accept=$false},
        @{version='1.0.0'; expected='2.0.0.0'; accept=$true}
    )
}
foreach ($case in $cases) {
    $PackageVersion = $case.requested
    $CompatibilityTest = $case.testIdentity
    @{schema=1;version=$case.version;platform=($case.platform ?? 'windows-x64');entrypoint=($case.entrypoint ?? 'rocket.exe')} |
        ConvertTo-Json | Set-Content -LiteralPath $metadataPath -Encoding utf8
    $accepted = $false
    try {
        $value = & $validate
        $accepted = $value.App -ceq $case.version
        if ($case.expected -and $value.Package -cne $case.expected) {
            throw "Expected package $($case.expected) for Rocket $($case.version), got '$($value.Package)'."
        }
    }
    catch { if ($case.accept) { throw } }
    if ($accepted -ne $case.accept) { throw 'Unexpected metadata acceptance result.' }
}
Write-Output "PASS: $($cases.Count) metadata cases; no payload execution. Fixture: $payload"

# Exercise the production decompression copy without invoking a compiler. Bad
# directory lengths must fail before excess bytes reach the destination stream.
$copy = $ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Copy-PayloadEntry'}, $true)
if(-not $copy){throw 'Bounded payload copy is missing.'}
. ([ScriptBlock]::Create($copy.Extent.Text))
foreach($case in @(
    @{actual=0;declared=0;accept=$true},
    @{actual=131073;declared=131073;accept=$true},
    @{actual=32;declared=31;accept=$false},
    @{actual=32;declared=33;accept=$false},
    @{actual=0;declared=268435457;accept=$false}
)){
    $inputStream=[IO.MemoryStream]::new([byte[]]::new($case.actual))
    $outputStream=[IO.MemoryStream]::new()
    try {
        $accepted=$false
        try { $null=Copy-PayloadEntry $inputStream $outputStream $case.declared; $accepted=$true }
        catch { if($case.accept){throw} }
        if($accepted -ne $case.accept -or $outputStream.Length -gt $case.declared){throw 'Archive length enforcement failed.'}
        if($accepted -and $outputStream.Length -ne $case.actual){throw 'Archive bytes were lost.'}
    } finally {$inputStream.Dispose();$outputStream.Dispose()}
}
Write-Output 'PASS: 5 bounded extraction cases.'
