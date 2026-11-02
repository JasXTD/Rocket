$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspectorPath = Join-Path $workspacePath 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspectorPath -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$smokePath = Join-Path (Join-Path $workspacePath 'work') ('smoke-stash-merge-' + [guid]::NewGuid().ToString('N'))
[System.IO.Directory]::CreateDirectory($smokePath) | Out-Null

function Invoke-FixtureGit {
    param([string[]]$GitArguments)
    $output = @(& $gitPath -C $smokePath @GitArguments)
    if ($LASTEXITCODE -ne 0) { throw "Fixture Git failed: $GitArguments" }
    return $output
}

function Invoke-Inspect {
    param([string[]]$InspectArguments, [int]$ExpectedExitCode = 0)
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $inspectorPath $smokePath @InspectArguments 2>&1)
        $actualExitCode = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    [System.IO.File]::AppendAllText($logPath, ("COMMAND: " + ($InspectArguments -join ' ') + "`n" + ($output -join "`n") + "`n"))
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
Invoke-FixtureGit -GitArguments @('config', 'user.name', 'Rocket Stash Merge Smoke') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'user.email', 'rocket@example.invalid') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'core.autocrlf', 'false') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'commit.gpgsign', 'false') | Out-Null
$noHooksPath = Join-Path $smokePath '.git\rocket-empty-hooks'
[System.IO.Directory]::CreateDirectory($noHooksPath) | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'core.hooksPath', $noHooksPath) | Out-Null
$logPath = Join-Path $smokePath '.git\rocket-stash-merge-smoke.log'
$filePath = Join-Path $smokePath 'hello.rs'
$notesPath = Join-Path $smokePath 'notes.md'
[System.IO.File]::WriteAllText($filePath, "base`n")
Invoke-FixtureGit -GitArguments @('add', '--', 'hello.rs') | Out-Null
Invoke-FixtureGit -GitArguments @('commit', '-m', 'Local stash merge smoke base') | Out-Null
$baseOid = Invoke-FixtureGit -GitArguments @('rev-parse', 'HEAD')

[System.IO.File]::WriteAllText($filePath, "staged edit`n")
Invoke-Inspect -InspectArguments @('stage', 'hello.rs')
[System.IO.File]::WriteAllText($filePath, "later working edit`n")
[System.IO.File]::WriteAllText($notesPath, "saved untracked notes`n")
Invoke-Inspect -InspectArguments @('stash-push', '-u', '-m', 'Smoke saved work')
$stashOid = Invoke-FixtureGit -GitArguments @('rev-parse', 'refs/stash')
Assert-Equal ([System.IO.File]::ReadAllText($filePath)) "base`n" 'Saved working file cleaned'
Assert-Equal (Test-Path -LiteralPath $notesPath) $false 'Untracked file saved and cleaned'
Assert-Equal @(Invoke-FixtureGit -GitArguments @('status', '--porcelain=v1')).Count 0 'Clean after stash'
Invoke-Inspect -InspectArguments @('stashes')
Invoke-Inspect -InspectArguments @('stash-apply', $stashOid, '--index')
Assert-Equal (Invoke-FixtureGit -GitArguments @('show', ':hello.rs')) 'staged edit' 'Staging restored'
Assert-Equal ([System.IO.File]::ReadAllText($filePath)) "later working edit`n" 'Working edit restored'
Assert-Equal ([System.IO.File]::ReadAllText($notesPath)) "saved untracked notes`n" 'Untracked notes restored'
Invoke-Inspect -InspectArguments @('stash-drop', '0', $baseOid) -ExpectedExitCode 1
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-parse', 'refs/stash')) $stashOid 'Wrong selection preserves stash'
Invoke-Inspect -InspectArguments @('stage-all')
Invoke-Inspect -InspectArguments @('commit', '-m', 'Save restored smoke work')
Invoke-Inspect -InspectArguments @('stash-drop', '0', $stashOid)
Assert-Equal @(Invoke-FixtureGit -GitArguments @('stash', 'list')).Count 0 'Selected stash dropped'

Invoke-Inspect -InspectArguments @('branch-create', 'feature/demo')
Invoke-Inspect -InspectArguments @('branch-switch', 'feature/demo')
[System.IO.File]::WriteAllText($filePath, "feature version`n")
Invoke-Inspect -InspectArguments @('stage', 'hello.rs')
Invoke-Inspect -InspectArguments @('commit', '-m', 'Feature smoke change')
$featureOid = Invoke-FixtureGit -GitArguments @('rev-parse', 'HEAD')
Invoke-Inspect -InspectArguments @('branch-switch', 'main')
[System.IO.File]::WriteAllText($filePath, "main version`n")
Invoke-Inspect -InspectArguments @('merge', 'feature/demo') -ExpectedExitCode 1
Assert-Equal ([System.IO.File]::ReadAllText($filePath)) "main version`n" 'Dirty merge preserves edit'
Invoke-Inspect -InspectArguments @('stage', 'hello.rs')
Invoke-Inspect -InspectArguments @('commit', '-m', 'Main smoke change')
$mainOid = Invoke-FixtureGit -GitArguments @('rev-parse', 'HEAD')
Invoke-Inspect -InspectArguments @('merge', 'feature/demo', '--ff-only') -ExpectedExitCode 1
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-parse', 'HEAD')) $mainOid 'Divergent ff-only leaves HEAD'
Invoke-Inspect -InspectArguments @('merge', 'feature/demo') -ExpectedExitCode 1
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-parse', 'MERGE_HEAD')) $featureOid 'Conflict retains merge target'
if (@(Invoke-FixtureGit -GitArguments @('ls-files', '-u')).Count -eq 0) { throw 'Expected unresolved index entries' }
Invoke-Inspect -InspectArguments @('operation')
Invoke-Inspect -InspectArguments @('merge-continue', '-m', 'Not resolved yet') -ExpectedExitCode 1
Invoke-Inspect -InspectArguments @('merge-abort')
Assert-Equal ([System.IO.File]::ReadAllText($filePath)) "main version`n" 'Abort restores original contents'
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-parse', 'HEAD')) $mainOid 'Abort preserves pre-merge HEAD'
Invoke-Inspect -InspectArguments @('merge-abort') -ExpectedExitCode 1

Invoke-Inspect -InspectArguments @('merge', 'feature/demo') -ExpectedExitCode 1
[System.IO.File]::WriteAllText($filePath, "resolved main and feature`n")
Invoke-Inspect -InspectArguments @('stage', 'hello.rs')
Invoke-Inspect -InspectArguments @('merge-continue', '-m', 'Resolve local smoke merge')
Assert-Equal (Invoke-FixtureGit -GitArguments @('show', '-s', '--format=%P', 'HEAD')) "$mainOid $featureOid" 'Merge has both parents'
Assert-Equal (Test-Path -LiteralPath (Join-Path $smokePath '.git\MERGE_HEAD')) $false 'Merge finished'
Invoke-Inspect -InspectArguments @('operation')
Invoke-Inspect -InspectArguments @('status')
Assert-Equal @(Invoke-FixtureGit -GitArguments @('status', '--porcelain=v1')).Count 0 'Final repository clean'
Write-Output "Stash and merge smoke checks passed. Scratch repository: $smokePath"
Write-Output "Captured CLI output: $logPath"
