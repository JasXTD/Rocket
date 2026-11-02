$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-remotes-' + [guid]::NewGuid().ToString('N'))
$seed = Join-Path $scratch 'ROCKET_PRIVATE_publisher'
$bare = Join-Path $scratch 'ROCKET_PRIVATE_upstream.git'
$repository = Join-Path $scratch 'ROCKET_PRIVATE_client'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'empty-hooks'
[IO.Directory]::CreateDirectory($seed) | Out-Null
[IO.Directory]::CreateDirectory($hooks) | Out-Null
$emptyConfig = Join-Path $scratch 'empty.config'
[IO.File]::WriteAllText($emptyConfig, '')
$transcript = Join-Path $scratch 'cli-output.log'

function Invoke-Git {
    param([string[]]$GitArguments, [string]$At = $repository)
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $gitPath -C $At @GitArguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    if ($actual -ne 0) { throw "Fixture Git failed ($actual): $GitArguments`n$($output -join "`n")" }
    return $output
}
function Invoke-Inspect {
    param([string[]]$InspectArguments, [int]$ExpectedExit = 0)
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $inspector $repository @InspectArguments 2>&1)
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
    'GIT_CONFIG_PARAMETERS', 'GIT_CONFIG_COUNT', 'GIT_TEMPLATE_DIR')
$names += @(Get-ChildItem Env: | Where-Object { $_.Name -match '^GIT_CONFIG_(KEY|VALUE)_' } | ForEach-Object { $_.Name })
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    foreach ($name in $names) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
    $env:ROCKET_LOG_DIR = $logs
    $env:GIT_CONFIG_GLOBAL = $emptyConfig
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_TEMPLATE_DIR = $hooks
    Invoke-Git @('init', '--initial-branch=main') -At $seed | Out-Null
    Invoke-Git @('config', 'user.name', 'Rocket Remote Smoke') -At $seed | Out-Null
    Invoke-Git @('config', 'user.email', 'rocket@example.invalid') -At $seed | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) -At $seed | Out-Null
    Invoke-Git @('config', 'commit.gpgsign', 'false') -At $seed | Out-Null
    Invoke-Git @('config', 'core.autocrlf', 'false') -At $seed | Out-Null
    [IO.File]::WriteAllText((Join-Path $seed 'file'), "base`n")
    Invoke-Git @('add', 'file') -At $seed | Out-Null
    Invoke-Git @('commit', '-m', 'Local fixture base') -At $seed | Out-Null
    Invoke-Git @('clone', '--bare', $seed, $bare) -At $scratch | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) -At $bare | Out-Null
    Invoke-Git @('clone', $bare, $repository) -At $scratch | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) | Out-Null
    Invoke-Git @('config', 'core.autocrlf', 'false') | Out-Null
    $listed = Invoke-Inspect @('remotes')
    if (($listed -join "`n") -notmatch 'Remote "origin"') { throw 'Remote listing missing origin' }
    Invoke-Inspect @('fetch', 'origin') | Out-Null
    Invoke-Inspect @('fetch') -ExpectedExit 1 | Out-Null
    Invoke-Inspect @('fetch', 'missing') -ExpectedExit 1 | Out-Null

    [IO.File]::WriteAllText((Join-Path $repository 'file'), "staged locally`n")
    Invoke-Git @('add', 'file') | Out-Null
    [IO.File]::WriteAllText((Join-Path $repository 'file'), "unstaged locally`n")
    $beforeHead = Invoke-Git @('rev-parse', 'HEAD')
    $index = Join-Path $repository '.git\index'
    $beforeIndex = (Get-FileHash -LiteralPath $index -Algorithm SHA256).Hash
    [IO.File]::WriteAllText((Join-Path $seed 'file'), "remote advance`n")
    Invoke-Git @('add', 'file') -At $seed | Out-Null
    Invoke-Git @('commit', '-m', 'Local fixture advance') -At $seed | Out-Null
    # This setup command copies fixture commits to the newly created local bare repo.
    Invoke-Git @('push', $bare, 'main', 'main:refs/heads/ROCKET_PRIVATE_feature') -At $seed | Out-Null
    Invoke-Inspect @('fetch', 'origin') | Out-Null
    if ((Invoke-Git @('rev-parse', 'HEAD')) -cne $beforeHead) { throw 'Fetch changed local HEAD' }
    if ((Get-FileHash -LiteralPath $index -Algorithm SHA256).Hash -cne $beforeIndex) { throw 'Fetch changed the index' }
    if ([IO.File]::ReadAllText((Join-Path $repository 'file')) -cne "unstaged locally`n") { throw 'Fetch changed worktree bytes' }
    if ((Invoke-Git @('rev-list', '--count', 'HEAD..origin/main')) -ne '1') { throw 'Expected one commit behind' }
    Invoke-Git @('show-ref', '--verify', 'refs/remotes/origin/ROCKET_PRIVATE_feature') | Out-Null
    Write-Host 'PASS remote refs and behind count refreshed; local HEAD, index, and edits preserved'

    Invoke-Git @('config', 'remote.origin.fetch', '+refs/heads/*:refs/heads/*') | Out-Null
    Invoke-Inspect @('fetch', 'origin') -ExpectedExit 1 | Out-Null
    Invoke-Git @('config', 'remote.origin.fetch', '+refs/heads/*:refs/remotes/origin/*') | Out-Null
    Invoke-Git @('remote', 'set-url', 'origin', (Join-Path $scratch 'missing.git')) | Out-Null
    $failed = Invoke-Inspect @('fetch', 'origin') -ExpectedExit 1
    if (($failed -join "`n") -notmatch 'Status after fetch:') { throw 'Missing reconciliation after transport failure' }
    Invoke-Git @('remote', 'set-url', 'origin', $bare) | Out-Null
    Invoke-Inspect @('status') | Out-Null

    $logPath = Join-Path $logs 'rocket.jsonl'
    $text = [IO.File]::ReadAllText($logPath)
    if ($text.Contains('ROCKET_PRIVATE') -or $text.Contains('rocket@example.invalid')) { throw 'Private fixture data leaked into diagnostics' }
    $records = @(Get-Content -LiteralPath $logPath | ForEach-Object { $_ | ConvertFrom-Json -ErrorAction Stop })
    foreach ($outcome in @('ok', 'git_error')) {
        if (@($records | Where-Object { $_.event -eq 'write_result' -and $_.operation -eq 'fetch' -and $_.outcome -eq $outcome }).Count -eq 0) { throw "Missing fetch outcome $outcome" }
    }
    if (@($records | Where-Object { $_.event -eq 'refresh' -and $_.outcome -eq 'ok' }).Count -ne 6) { throw 'Expected two refreshes for each of three attempted fetches' }
    if (@($records | Where-Object { $_.dropped_before -ne 0 }).Count -ne 0) { throw 'Diagnostic events were dropped' }
    Write-Host 'PASS structured fetch outcomes, failure reconciliation, and log privacy'
    Write-Output "Remote smoke checks passed. Scratch repository: $repository"
    Write-Output "Local bare remote: $bare"
    Write-Output "Diagnostics: $logPath"
    Write-Output "CLI transcript (fixture data only): $transcript"
} finally {
    foreach ($name in $names) {
        if ($null -eq $saved[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    }
}
