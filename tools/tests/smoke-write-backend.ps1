$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspectorPath = Join-Path $workspacePath 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspectorPath -PathType Leaf)) {
    throw 'Build the example first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$smokePath = Join-Path (Join-Path $workspacePath 'work') ('smoke-write-' + [guid]::NewGuid().ToString('N'))
[System.IO.Directory]::CreateDirectory($smokePath) | Out-Null

function Invoke-FixtureGit {
    param([string[]]$GitArguments)
    $output = @(& $gitPath -C $smokePath @GitArguments)
    if ($LASTEXITCODE -ne 0) { throw "Fixture Git failed: $GitArguments" }
    return $output
}

function Invoke-Inspect {
    param([string[]]$InspectArguments, [int]$ExpectedExitCode = 0)
    $output = @(& $inspectorPath $smokePath @InspectArguments)
    $actualExitCode = $LASTEXITCODE
    [System.IO.File]::AppendAllText($logPath, (($output -join "`n") + "`n"))
    if ($actualExitCode -ne $ExpectedExitCode) {
        throw "Rocket $InspectArguments exited $actualExitCode; expected $ExpectedExitCode. See $logPath"
    }
    Write-Output ("PASS {0} (exit {1})" -f ($InspectArguments -join ' '), $actualExitCode)
}

function Assert-Equal {
    param($Actual, $Expected, [string]$Label)
    if ($Actual -cne $Expected) { throw "$Label : expected '$Expected', got '$Actual'" }
}

Invoke-FixtureGit -GitArguments @('init', '--initial-branch=main') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'user.name', 'Rocket Smoke Test') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'user.email', 'rocket@example.invalid') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'core.autocrlf', 'false') | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'commit.gpgsign', 'false') | Out-Null
$noHooksPath = Join-Path $smokePath '.git\rocket-empty-hooks'
[System.IO.Directory]::CreateDirectory($noHooksPath) | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'core.hooksPath', $noHooksPath) | Out-Null
$logPath = Join-Path $smokePath '.git\rocket-smoke.stdout.log'
$helloPath = Join-Path $smokePath 'hello.rs'
[System.IO.File]::WriteAllText($helloPath, "staged version`n")
[System.IO.Directory]::CreateDirectory((Join-Path $smokePath 'notes')) | Out-Null
[System.IO.File]::WriteAllText((Join-Path $smokePath 'notes\space [file].txt'), "literal file`n")

# An omitted selection must fail, not silently stage everything.
Invoke-Inspect -InspectArguments @('stage') -ExpectedExitCode 1
Assert-Equal @(Invoke-FixtureGit -GitArguments @('ls-files')).Count 0 'Empty selection index'
Invoke-Inspect -InspectArguments @('stage', 'hello.rs')
Assert-Equal (Invoke-FixtureGit -GitArguments @('show', ':hello.rs')) 'staged version' 'Selected index blob'
[System.IO.File]::WriteAllText($helloPath, "working version`n")
Invoke-Inspect -InspectArguments @('unstage', 'hello.rs')
Assert-Equal @(Invoke-FixtureGit -GitArguments @('ls-files')).Count 0 'Unborn unstage index'
Assert-Equal ([System.IO.File]::ReadAllText($helloPath)) "working version`n" 'Unborn working file'

Invoke-Inspect -InspectArguments @('stage-all')
Assert-Equal @(Invoke-FixtureGit -GitArguments @('ls-files')).Count 2 'Stage all index'
Invoke-Inspect -InspectArguments @('unstage-all')
Assert-Equal @(Invoke-FixtureGit -GitArguments @('ls-files')).Count 0 'Unstage all index'
Invoke-Inspect -InspectArguments @('stage', 'hello.rs')
[System.IO.File]::WriteAllText($helloPath, "edit after staging`n")
Invoke-Inspect -InspectArguments @('commit', '-m', 'First Rocket write smoke commit')
Assert-Equal (Invoke-FixtureGit -GitArguments @('show', 'HEAD:hello.rs')) 'working version' 'Commit uses index'
Assert-Equal (Invoke-FixtureGit -GitArguments @('status', '--porcelain=v1', '--', 'hello.rs')) ' M hello.rs' 'Unstaged edit survives'
Invoke-Inspect -InspectArguments @('commit', '-m', 'No staged changes') -ExpectedExitCode 1
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-list', '--count', 'HEAD')) '1' 'Failed commit leaves history'

Invoke-Inspect -InspectArguments @('stage', 'hello.rs')
Invoke-Inspect -InspectArguments @('unstage', 'hello.rs')
Assert-Equal (Invoke-FixtureGit -GitArguments @('show', ':hello.rs')) 'working version' 'Existing HEAD unstage'
Assert-Equal ([System.IO.File]::ReadAllText($helloPath)) "edit after staging`n" 'Existing HEAD working file'
Invoke-Inspect -InspectArguments @('stage-all')
Invoke-Inspect -InspectArguments @('commit', '-m', 'Second Rocket write smoke commit')
Assert-Equal (Invoke-FixtureGit -GitArguments @('rev-list', '--count', 'HEAD')) '2' 'Second commit history'
Assert-Equal @(Invoke-FixtureGit -GitArguments @('status', '--porcelain=v1')).Count 0 'Clean repository'

# Leave one unstaged edit so the printed path is immediately useful for manual trials.
[System.IO.File]::WriteAllText($helloPath, "ready for your staging test`n")
Invoke-Inspect -InspectArguments @('status')
Assert-Equal (Invoke-FixtureGit -GitArguments @('status', '--porcelain=v1')) ' M hello.rs' 'Ready for manual trial'
Write-Output "Smoke checks passed. Scratch repository: $smokePath"
Write-Output "Captured CLI stdout: $logPath"
