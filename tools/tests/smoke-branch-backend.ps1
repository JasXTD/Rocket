$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspectorPath = Join-Path $workspacePath 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspectorPath -PathType Leaf)) {
    throw 'Build the example first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$smokePath = Join-Path (Join-Path $workspacePath 'work') ('smoke-branches-' + [guid]::NewGuid().ToString('N'))
[System.IO.Directory]::CreateDirectory($smokePath) | Out-Null

function Invoke-FixtureGit {
    param([string[]]$GitArguments)
    $output = @(& $gitPath -C $smokePath @GitArguments)
    if ($LASTEXITCODE -ne 0) { throw "Fixture Git failed: $GitArguments" }
    return $output
}

function Invoke-Inspect {
    param([string[]]$InspectArguments, [int]$ExpectedExitCode = 0)
    # PowerShell 5.1 wraps redirected native stderr in ErrorRecord objects.
    # Capture those messages for review, then judge the native exit code explicitly.
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $inspectorPath $smokePath @InspectArguments 2>&1)
        $actualExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousAction
    }
    [System.IO.File]::AppendAllText($logPath, (("COMMAND: " + ($InspectArguments -join ' ')) + "`n" + ($output -join "`n") + "`n"))
    if ($actualExitCode -ne $ExpectedExitCode) {
        throw "Rocket $InspectArguments exited $actualExitCode; expected $ExpectedExitCode. See $logPath"
    }
    $kind = if ($ExpectedExitCode -eq 0) { 'succeeded' } else { 'expected rejection' }
    Write-Output ("PASS {0}: {1} (exit {2})" -f $kind, ($InspectArguments -join ' '), $actualExitCode)
}

function Assert-Equal {
    param($Actual, $Expected, [string]$Label)
    if ($Actual -cne $Expected) { throw "$Label : expected '$Expected', got '$Actual'" }
}

Invoke-FixtureGit -GitArguments @('init', '--initial-branch=main') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'user.name', 'Rocket Branch Smoke') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'user.email', 'rocket@example.invalid') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'core.autocrlf', 'false') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'commit.gpgsign', 'false') | Out-Null
$noHooksPath = Join-Path $smokePath '.git\rocket-empty-hooks'
[System.IO.Directory]::CreateDirectory($noHooksPath) | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'core.hooksPath', $noHooksPath) | Out-Null
$logPath = Join-Path $smokePath '.git\rocket-branch-smoke.log'
$filePath = Join-Path $smokePath 'hello.rs'
[System.IO.File]::WriteAllText($filePath, "base`n")
Invoke-FixtureGit -GitArguments @('add', '--', 'hello.rs') | Out-Null
Invoke-FixtureGit -GitArguments @('commit', '-m', 'Local branch smoke base') | Out-Null
$baseOid = Invoke-FixtureGit -GitArguments @('rev-parse', 'HEAD')

Invoke-Inspect -InspectArguments @('branch-create', 'feature/demo')
Assert-Equal (Invoke-FixtureGit -GitArguments @('symbolic-ref', '--short', 'HEAD')) 'main' 'Creation leaves checkout'
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-parse', 'refs/heads/feature/demo')) $baseOid 'Creation point'
Invoke-Inspect -InspectArguments @('branch-create', 'feature/demo') -ExpectedExitCode 1
Invoke-Inspect -InspectArguments @('branch-switch', 'feature/demo')
Assert-Equal (Invoke-FixtureGit -GitArguments @('symbolic-ref', '--short', 'HEAD')) 'feature/demo' 'Branch switch'
[System.IO.File]::WriteAllText($filePath, "feature edit`n")
Invoke-Inspect -InspectArguments @('stage', 'hello.rs')
Invoke-Inspect -InspectArguments @('commit', '-m', 'Local feature commit')
$featureOid = Invoke-FixtureGit -GitArguments @('rev-parse', 'HEAD')
Invoke-Inspect -InspectArguments @('branch-rename', 'feature/demo', 'main') -ExpectedExitCode 1
Invoke-Inspect -InspectArguments @('branch-rename', 'feature/demo', 'feature/renamed')
Assert-Equal (Invoke-FixtureGit -GitArguments @('symbolic-ref', '--short', 'HEAD')) 'feature/renamed' 'Rename updates HEAD'
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-parse', 'HEAD')) $featureOid 'Rename keeps commit'

Invoke-Inspect -InspectArguments @('branch-create', 'from-base', $baseOid)
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-parse', 'refs/heads/from-base')) $baseOid 'Explicit creation point'
Invoke-Inspect -InspectArguments @('branch-switch', 'main')
Assert-Equal ([System.IO.File]::ReadAllText($filePath)) "base`n" 'Switch updates working file'
Invoke-Inspect -InspectArguments @('branch-delete', 'feature/renamed') -ExpectedExitCode 1
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-parse', 'refs/heads/feature/renamed')) $featureOid 'Unmerged branch survives'
Invoke-Inspect -InspectArguments @('branch-delete', 'main') -ExpectedExitCode 1
Invoke-Inspect -InspectArguments @('branch-delete', 'from-base')
$remaining = @(Invoke-FixtureGit -GitArguments @('for-each-ref', '--format=%(refname)', 'refs/heads/'))
if ($remaining -contains 'refs/heads/from-base') { throw 'Merged branch was not deleted' }

[System.IO.File]::WriteAllText($filePath, "local edit must survive`n")
Invoke-Inspect -InspectArguments @('branch-switch', 'feature/renamed') -ExpectedExitCode 1
Assert-Equal ([System.IO.File]::ReadAllText($filePath)) "local edit must survive`n" 'Rejected switch preserves edit'
Assert-Equal (Invoke-FixtureGit -GitArguments @('symbolic-ref', '--short', 'HEAD')) 'main' 'Rejected switch preserves HEAD'
# Restore just this scratch script's edit, leaving a clean repository for manual trials.
[System.IO.File]::WriteAllText($filePath, "base`n")
Invoke-Inspect -InspectArguments @('branches')
Invoke-Inspect -InspectArguments @('status')
Assert-Equal @(Invoke-FixtureGit -GitArguments @('status', '--porcelain=v1')).Count 0 'Clean scratch repository'
Write-Output "Branch smoke checks passed. Scratch repository: $smokePath"
Write-Output 'Ready for manual trials: main and feature/renamed are local branches.'
Write-Output "Captured CLI output: $logPath"
