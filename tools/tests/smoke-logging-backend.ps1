$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-logging-' + [guid]::NewGuid().ToString('N'))
$repository = Join-Path $scratch 'ROCKET_PRIVATE_repo'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'empty-hooks'
[IO.Directory]::CreateDirectory($repository) | Out-Null
[IO.Directory]::CreateDirectory($hooks) | Out-Null
$emptyConfig = Join-Path $scratch 'empty.config'
[IO.File]::WriteAllText($emptyConfig, '')
$transcript = Join-Path $scratch 'cli-output.log'

function Invoke-Git {
    param([string[]]$GitArguments)
    $output = @(& $gitPath -C $repository @GitArguments)
    if ($LASTEXITCODE -ne 0) { throw "Fixture Git failed: $GitArguments" }
    return $output
}
function Invoke-Inspect {
    param([string[]]$InspectArguments, [int]$ExpectedExit = 0, [string]$Repo = $repository)
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $inspector $Repo @InspectArguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    [IO.File]::AppendAllText($transcript, ("COMMAND: " + ($InspectArguments -join ' ') + "`n" + ($output -join "`n") + "`n"))
    if ($actual -ne $ExpectedExit) { throw "inspect exited $actual, expected $ExpectedExit. See $transcript" }
    $kind = if ($ExpectedExit -eq 0) { 'succeeded' } else { 'expected rejection' }
    Write-Host "PASS $kind : $($InspectArguments -join ' ') (exit $actual)"
    return $output
}

$names = @('ROCKET_LOG', 'ROCKET_LOG_DIR', 'GIT_CONFIG_GLOBAL', 'GIT_CONFIG_NOSYSTEM',
    'GIT_DIR', 'GIT_WORK_TREE', 'GIT_COMMON_DIR', 'GIT_INDEX_FILE', 'GIT_OBJECT_DIRECTORY',
    'GIT_ALTERNATE_OBJECT_DIRECTORIES', 'GIT_SHALLOW_FILE', 'GIT_NAMESPACE', 'GIT_CONFIG',
    'GIT_CONFIG_PARAMETERS', 'GIT_CONFIG_COUNT')
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $null, 'Process') }
    $env:ROCKET_LOG_DIR = $logs
    $env:GIT_CONFIG_GLOBAL = $emptyConfig
    $env:GIT_CONFIG_NOSYSTEM = '1'
    Invoke-Git @('init', '--initial-branch=main') | Out-Null
    Invoke-Git @('config', 'user.name', 'Rocket Log Smoke') | Out-Null
    Invoke-Git @('config', 'user.email', 'rocket@example.invalid') | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) | Out-Null
    Invoke-Git @('config', 'commit.gpgsign', 'false') | Out-Null
    Invoke-Git @('config', 'core.autocrlf', 'false') | Out-Null
    $file = Join-Path $repository 'ROCKET_PRIVATE_file'
    [IO.File]::WriteAllText($file, "ROCKET_PRIVATE_content`n")
    $logPath = Join-Path $logs 'rocket.jsonl'
    $reportedPath = & $inspector --log-path
    if ($LASTEXITCODE -ne 0 -or $reportedPath -cne $logPath) { throw 'Wrong configured log path' }
    Invoke-Inspect @('stage-all') | Out-Null
    Invoke-Inspect @('commit', '-m', 'ROCKET_PRIVATE_message') | Out-Null
    Invoke-Inspect @('status') | Out-Null
    Invoke-Inspect @('operation') | Out-Null
    Invoke-Inspect @('stage') -ExpectedExit 1 | Out-Null
    Invoke-Inspect @('commit', '-m', 'ROCKET_PRIVATE_no_changes') -ExpectedExit 1 | Out-Null
    Invoke-Inspect @('ROCKET_PRIVATE_unknown_operation') -ExpectedExit 1 | Out-Null
    Invoke-Inspect @('status') -ExpectedExit 1 -Repo (Join-Path $scratch 'ROCKET_PRIVATE_missing') | Out-Null
    $text = [IO.File]::ReadAllText($logPath)
    if ($text.Contains('ROCKET_PRIVATE') -or $text.Contains('rocket@example.invalid')) { throw 'Private fixture data leaked into diagnostics' }
    $records = @(Get-Content -LiteralPath $logPath | ForEach-Object { $_ | ConvertFrom-Json -ErrorAction Stop })
    if (@($records | Where-Object { $_.schema -ne 1 }).Count -ne 0) { throw 'Unexpected log schema' }
    $ends = @($records | Where-Object { $_.event -eq 'request_end' })
    if ($ends.Count -ne 8) { throw "Expected eight request results; got $($ends.Count)" }
    foreach ($outcome in @('ok', 'git_error', 'invalid_input', 'io_error')) {
        if (@($ends | Where-Object { $_.outcome -eq $outcome }).Count -eq 0) { throw "Missing request outcome: $outcome" }
    }
    foreach ($start in @($records | Where-Object { $_.event -eq 'git_start' })) {
        $matched = @($records | Where-Object { $_.event -eq 'git_end' -and $_.session -eq $start.session -and $_.id -eq $start.id -and $_.request -eq $start.request })
        if ($matched.Count -ne 1) { throw 'Git start/end correlation failed' }
    }
    if (@($records | Where-Object { $_.event -eq 'refresh' -and $_.outcome -eq 'ok' }).Count -lt 3) { throw 'Missing independent refresh results' }
    Write-Host 'PASS valid JSON, request/command correlation, outcomes, and privacy checks'

    $env:ROCKET_LOG = 'off'
    Invoke-Inspect @('status') | Out-Null
    if ([IO.File]::ReadAllText($logPath) -cne $text) { throw 'Disabled logging still wrote data' }
    Write-Host 'PASS logging disabled without changing status behavior'

    Remove-Item Env:ROCKET_LOG
    $blocked = Join-Path $scratch 'not-a-directory'
    [IO.File]::WriteAllText($blocked, 'keep this file')
    $env:ROCKET_LOG_DIR = $blocked
    [IO.File]::WriteAllText($file, "second content`n")
    Invoke-Inspect @('stage-all') | Out-Null
    $output = Invoke-Inspect @('commit', '-m', 'Log unavailable but commit succeeds')
    if (($output -join "`n") -notmatch 'diagnostic logging unavailable') { throw 'Missing logging warning' }
    if ((Invoke-Git @('rev-list', '--count', 'HEAD')) -ne '2') { throw 'Commit did not persist exactly once' }
    if ([IO.File]::ReadAllText($blocked) -cne 'keep this file') { throw 'Logger overwrote unrelated file' }
    Write-Host 'PASS unavailable logs preserve successful Git writes and show a warning'
    $env:ROCKET_LOG_DIR = $logs
    & (Join-Path $PSScriptRoot '../diagnostics/show-logs.ps1') -Directory $logs -WarningsOnly -Tail 10
    Write-Output "Logging smoke checks passed. Scratch repository: $repository"
    Write-Output "Diagnostics: $logPath"
    Write-Output "CLI transcript (fixture data only): $transcript"
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
}
