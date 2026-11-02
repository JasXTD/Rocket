$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$smokePath = Join-Path $workspacePath ('work\smoke-' + [guid]::NewGuid().ToString('N'))
[System.IO.Directory]::CreateDirectory($smokePath) | Out-Null

function Invoke-FixtureGit {
    param([string[]]$GitArguments)
    & $gitPath -C $smokePath @GitArguments
    if ($LASTEXITCODE -ne 0) { throw "Fixture Git failed: $GitArguments" }
}

Invoke-FixtureGit -GitArguments @('init', '--initial-branch=main')
Invoke-FixtureGit -GitArguments @('config', 'user.name', 'Rocket Smoke Test')
Invoke-FixtureGit -GitArguments @('config', 'user.email', 'rocket@example.invalid')
Invoke-FixtureGit -GitArguments @('config', 'core.autocrlf', 'false')
Invoke-FixtureGit -GitArguments @('config', 'commit.gpgsign', 'false')
$noHooksPath = Join-Path $smokePath '.no-hooks'
[System.IO.Directory]::CreateDirectory($noHooksPath) | Out-Null
Invoke-FixtureGit -GitArguments @('config', 'core.hooksPath', $noHooksPath)
[System.IO.File]::WriteAllText((Join-Path $smokePath 'hello.rs'), 'fn main() { println!("first"); }' + [Environment]::NewLine)
Invoke-FixtureGit -GitArguments @('add', '--', 'hello.rs')
Invoke-FixtureGit -GitArguments @('commit', '--no-verify', '-m', 'First local fixture commit')
[System.IO.File]::WriteAllText((Join-Path $smokePath 'hello.rs'), 'fn main() { println!("second"); }' + [Environment]::NewLine)
Invoke-FixtureGit -GitArguments @('add', '--', 'hello.rs')
Invoke-FixtureGit -GitArguments @('commit', '--no-verify', '-m', 'Second local fixture commit')
[System.IO.File]::WriteAllText((Join-Path $smokePath 'hello.rs'), 'fn main() { println!("staged"); }' + [Environment]::NewLine)
Invoke-FixtureGit -GitArguments @('add', '--', 'hello.rs')
[System.IO.File]::WriteAllText((Join-Path $smokePath 'hello.rs'), 'fn main() { println!("working"); }' + [Environment]::NewLine)
$smokePath
foreach ($operation in @('info', 'status', 'branches', 'history', 'diff', 'staged')) {
    & (Join-Path $workspacePath 'target\debug\examples\inspect.exe') $smokePath $operation
    if ($LASTEXITCODE -ne 0) { throw "Rocket inspection failed: $operation" }
}
