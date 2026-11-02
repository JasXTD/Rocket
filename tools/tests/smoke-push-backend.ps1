$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-push-' + [guid]::NewGuid().ToString('N'))
$repository = Join-Path $scratch 'ROCKET_PRIVATE_client'
$bare = Join-Path $scratch 'ROCKET_PRIVATE_upstream.git'
$peer = Join-Path $scratch 'ROCKET_PRIVATE_peer'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'client-hooks'
$serverHooks = Join-Path $scratch 'server-hooks'
foreach ($directory in @($repository, $hooks, $serverHooks)) { [IO.Directory]::CreateDirectory($directory) | Out-Null }
$emptyConfig = Join-Path $scratch 'empty.config'
[IO.File]::WriteAllText($emptyConfig, '')
$transcript = Join-Path $scratch 'cli-output.log'
$resultPath = Join-Path $scratch 'result.json'
$checks = New-Object 'System.Collections.Generic.List[string]'
$result = [ordered]@{ passed = $false; mode = 'local-file-only'; phase = 'setup'; checks = @(); diagnostic_records = 0 }

function Invoke-Git {
    param([string[]]$GitArguments, [string]$At = $repository)
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $gitPath -C $At @GitArguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    if ($actual -ne 0) { throw "Fixture Git failed ($actual): $GitArguments`n$($output -join "`n")" }
    return ($output -join "`n")
}
function Invoke-Inspect {
    param([string[]]$InspectArguments, [int]$ExpectedExit = 0, [string]$ExpectedText)
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $inspector $repository @InspectArguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    $text = $output -join "`n"
    [IO.File]::AppendAllText($transcript, ("COMMAND: " + ($InspectArguments -join ' ') + "`n$text`n"))
    if ($actual -ne $ExpectedExit) { throw "inspect exited $actual, expected $ExpectedExit. See $transcript" }
    if ($ExpectedText -and $text -notmatch [regex]::Escape($ExpectedText)) { throw "Missing expected CLI diagnostic: $ExpectedText" }
    if ($InspectArguments -notcontains '--raw-error' -and $text -match 'ROCKET_PRIVATE_(server|client)_canary') {
        throw 'Default CLI output exposed raw hook diagnostics'
    }
    $kind = if ($ExpectedExit -eq 0) { 'succeeded' } else { 'expected rejection' }
    Write-Host "PASS $kind : $($InspectArguments -join ' ') (exit $actual)"
    return $text
}
function Record-Check([string]$Name) {
    $checks.Add($Name)
    Write-Host "PASS $Name"
}
function Assert-Preserved {
    if ((Invoke-Git @('rev-parse', 'HEAD')) -cne $script:beforeHead) { throw 'Push changed local HEAD' }
    if ((Get-FileHash -LiteralPath $script:index -Algorithm SHA256).Hash -cne $script:beforeIndex) { throw 'Push changed the index' }
    if ([IO.File]::ReadAllText((Join-Path $repository 'file')) -cne "unstaged locally`n") { throw 'Push changed working-file bytes' }
    if ([IO.File]::ReadAllText((Join-Path $repository 'untracked')) -cne "preserve me`n") { throw 'Push changed untracked bytes' }
}

# Empty global/system configuration and file-only transports apply to this process
# and its children. Saved login, SSH agents, keys, and real repositories are unused.
$names = @('ROCKET_LOG', 'ROCKET_LOG_DIR', 'GIT_CONFIG_GLOBAL', 'GIT_CONFIG_SYSTEM',
    'GIT_CONFIG_NOSYSTEM', 'GIT_ALLOW_PROTOCOL', 'GIT_TEMPLATE_DIR', 'GIT_TERMINAL_PROMPT')
$names += @(Get-ChildItem Env: | Where-Object { $_.Name -match '^(GIT_|SSH_)|_PROXY$' } | ForEach-Object { $_.Name })
$names = @($names | Select-Object -Unique)
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $null, 'Process') }
    $env:ROCKET_LOG_DIR = $logs
    $env:GIT_CONFIG_GLOBAL = $emptyConfig
    $env:GIT_CONFIG_SYSTEM = $emptyConfig
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_ALLOW_PROTOCOL = 'file'
    $env:GIT_TEMPLATE_DIR = $hooks
    $env:GIT_TERMINAL_PROMPT = '0'
    Invoke-Git @('init', '--initial-branch=main') | Out-Null
    foreach ($setting in @(
        @('user.name', 'Rocket Push Smoke'), @('user.email', 'rocket@example.invalid'),
        @('core.hooksPath', $hooks), @('core.autocrlf', 'false'), @('commit.gpgsign', 'false')
    )) { Invoke-Git @('config', $setting[0], $setting[1]) | Out-Null }
    [IO.File]::WriteAllText((Join-Path $repository 'file'), "base`n")
    Invoke-Git @('add', 'file') | Out-Null
    Invoke-Git @('commit', '-m', 'Local push fixture base') | Out-Null
    Invoke-Git @('init', '--bare', '--initial-branch=main', $bare) | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $serverHooks) -At $bare | Out-Null
    Invoke-Git @('remote', 'add', 'origin', $bare) | Out-Null
    Invoke-Git @('branch', 'extra') | Out-Null
    Invoke-Git @('tag', '-a', 'extra-tag', '-m', 'tag') | Out-Null
    foreach ($setting in @(
        @('remote.origin.mirror', 'true'), @('remote.origin.push', '+refs/heads/*:refs/heads/*'),
        @('push.followTags', 'true'), @('push.autoSetupRemote', 'true'), @('push.pushOption', 'fixture-option')
    )) { Invoke-Git @('config', $setting[0], $setting[1]) | Out-Null }

    $result.phase = 'selection-and-first-push'
    Invoke-Inspect @('push') -ExpectedExit 1 | Out-Null
    Invoke-Inspect @('push', 'missing', 'main', 'main') -ExpectedExit 1 -ExpectedText 'remote is not configured' | Out-Null
    Invoke-Inspect @('push', 'origin', 'main', 'main', '--force') -ExpectedExit 1 | Out-Null
    $configBefore = (Get-FileHash -LiteralPath (Join-Path $repository '.git\config') -Algorithm SHA256).Hash
    Invoke-Inspect @('push', 'origin', 'main', 'main') -ExpectedText 'kind: New' | Out-Null
    if ((Invoke-Git @('for-each-ref', '--format=%(refname)') -At $bare) -cne 'refs/heads/main') { throw 'Push sent an unselected ref or tag' }
    if ((Get-FileHash -LiteralPath (Join-Path $repository '.git\config') -Algorithm SHA256).Hash -cne $configBefore) { throw 'Push changed config without explicit upstream setup' }
    Invoke-Inspect @('push', 'origin', 'main', 'main', '--set-upstream') -ExpectedText 'Upstream: configured and verified.' | Out-Null
    if ((Invoke-Git @('config', 'branch.main.remote')) -cne 'origin') { throw 'Wrong upstream remote' }
    if ((Invoke-Git @('config', 'branch.main.merge')) -cne 'refs/heads/main') { throw 'Wrong upstream branch' }
    Record-Check 'explicit branch only; mirror/tags/defaults contained; optional upstream verified'

    $result.phase = 'fast-forward-and-local-state'
    [IO.File]::WriteAllText((Join-Path $repository 'file'), "advance`n")
    Invoke-Git @('add', 'file') | Out-Null
    Invoke-Git @('commit', '-m', 'Local push fixture advance') | Out-Null
    [IO.File]::WriteAllText((Join-Path $repository 'file'), "staged locally`n")
    Invoke-Git @('add', 'file') | Out-Null
    [IO.File]::WriteAllText((Join-Path $repository 'file'), "unstaged locally`n")
    [IO.File]::WriteAllText((Join-Path $repository 'untracked'), "preserve me`n")
    $beforeHead = Invoke-Git @('rev-parse', 'HEAD')
    $index = Join-Path $repository '.git\index'
    $beforeIndex = (Get-FileHash -LiteralPath $index -Algorithm SHA256).Hash
    Invoke-Inspect @('push', 'origin', 'main', 'main') -ExpectedText 'kind: FastForward' | Out-Null
    Assert-Preserved
    if ((Invoke-Git @('rev-parse', 'refs/heads/main') -At $bare) -cne $beforeHead) { throw 'Remote did not receive selected commit' }
    Invoke-Inspect @('push', 'origin', 'main', 'main') -ExpectedText 'kind: UpToDate' | Out-Null
    Assert-Preserved
    Record-Check 'new/fast-forward/up-to-date outcomes; HEAD, index, staged/unstaged/untracked data preserved'

    $result.phase = 'history-rejections'
    Invoke-Git @('clone', $bare, $peer) -At $scratch | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) -At $peer | Out-Null
    [IO.File]::WriteAllText((Join-Path $peer 'peer'), "independent remote commit`n")
    Invoke-Git @('add', 'peer') -At $peer | Out-Null
    Invoke-Git @('-c', 'user.name=Rocket Peer', '-c', 'user.email=rocket@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '-m', 'Independent fixture commit') -At $peer | Out-Null
    Invoke-Git @('push', 'origin', 'refs/heads/main:refs/heads/main') -At $peer | Out-Null
    $remoteBefore = Invoke-Git @('rev-parse', 'refs/heads/main') -At $bare
    Invoke-Inspect @('push', 'origin', 'main', 'main') -ExpectedExit 1 -ExpectedText 'push_fetch_first' | Out-Null
    Invoke-Inspect @('fetch', 'origin') | Out-Null
    Invoke-Inspect @('push', 'origin', 'main', 'main') -ExpectedExit 1 -ExpectedText 'push_non_fast_forward' | Out-Null
    Assert-Preserved
    if ((Invoke-Git @('rev-parse', 'refs/heads/main') -At $bare) -cne $remoteBefore) { throw 'Rejected push replaced remote history' }
    Record-Check 'fetch-first and non-fast-forward have distinct errors and preserve remote history'

    $result.phase = 'hooks-and-privacy'
    $serverHook = Join-Path $serverHooks 'pre-receive'
    [IO.File]::WriteAllText($serverHook, "#!/bin/sh`necho 'ROCKET_PRIVATE_server_canary' >&2`nexit 1`n", [Text.UTF8Encoding]::new($false))
    Invoke-Inspect @('push', 'origin', 'main', 'blocked') -ExpectedExit 1 -ExpectedText 'push_remote_rejected' | Out-Null
    # Explicit raw output is exercised only with this invented fixture marker.
    Invoke-Inspect @('push', 'origin', 'main', 'blocked', '--raw-error') -ExpectedExit 1 -ExpectedText 'ROCKET_PRIVATE_server_canary' | Out-Null
    Remove-Item -LiteralPath $serverHook
    $clientHook = Join-Path $hooks 'pre-push'
    [IO.File]::WriteAllText($clientHook, "#!/bin/sh`necho 'ROCKET_PRIVATE_client_canary' >&2`nexit 1`n", [Text.UTF8Encoding]::new($false))
    Invoke-Inspect @('push', 'origin', 'main', 'blocked') -ExpectedExit 1 -ExpectedText 'Remote outcome: unknown' | Out-Null
    Remove-Item -LiteralPath $clientHook
    Invoke-Git @('config', '--add', 'remote.origin.pushurl', $bare) | Out-Null
    Invoke-Git @('config', '--add', 'remote.origin.pushurl', $bare) | Out-Null
    Invoke-Inspect @('push', 'origin', 'main', 'blocked') -ExpectedExit 1 -ExpectedText 'exactly one nonempty push URL' | Out-Null
    Invoke-Git @('config', '--unset-all', 'remote.origin.pushurl') | Out-Null
    if ((Invoke-Git @('for-each-ref', '--format=%(refname)') -At $bare) -cne 'refs/heads/main') { throw 'Rejected selection created an unexpected remote branch' }
    Assert-Preserved
    Record-Check 'server/local hook failures, unknown outcome guidance, raw-output opt-in, multiple URL rejection'

    $result.phase = 'tracking-metadata-warning'
    $trackingLock = Join-Path $repository '.git\refs\remotes\origin\metadata-check.lock'
    [IO.File]::WriteAllText($trackingLock, 'owned fixture lock')
    Invoke-Inspect @('push', 'origin', 'main', 'metadata-check') -ExpectedText 'Git emitted error diagnostics despite remote success' | Out-Null
    if ((Invoke-Git @('rev-parse', 'refs/heads/metadata-check') -At $bare) -cne $beforeHead) { throw 'Expected remote success despite local tracking failure' }
    if ([IO.File]::ReadAllText($trackingLock) -cne 'owned fixture lock') { throw 'Push removed or replaced a pre-existing tracking lock' }
    Remove-Item -LiteralPath $trackingLock
    Assert-Preserved
    Record-Check 'remote success with local tracking error is visible and existing lock is preserved'

    $result.phase = 'diagnostics'
    $records = @()
    $diagnostics = ''
    foreach ($path in @(Get-ChildItem -LiteralPath $logs -Filter '*.jsonl' -File)) {
        $text = [IO.File]::ReadAllText($path.FullName)
        $diagnostics += $text
        foreach ($line in [IO.File]::ReadAllLines($path.FullName)) { $records += ($line | ConvertFrom-Json -ErrorAction Stop) }
    }
    if ($diagnostics -match 'ROCKET_PRIVATE_|fixture-option|refs/heads/|refs/remotes/' -or $diagnostics.Contains($beforeHead)) { throw 'Diagnostics exposed repository/hook data' }
    if (@($records | Where-Object { $_.dropped_before -ne 0 }).Count -ne 0) { throw 'Diagnostics dropped events' }
    foreach ($event in @('request_start', 'request_end', 'step_start', 'step_end', 'git_start', 'git_end', 'write_result', 'parse', 'refresh', 'push_outcome')) {
        if (@($records | Where-Object { $_.event -ceq $event }).Count -eq 0) { throw "Missing event: $event" }
    }
    foreach ($code in @('push_fetch_first', 'push_non_fast_forward', 'push_remote_rejected', 'unclassified_push_failure')) {
        if (@($records | Where-Object { $_.failure_code -ceq $code }).Count -eq 0) { throw "Missing failure code: $code" }
    }
    foreach ($outcome in @('confirmed_success', 'rejected', 'unknown')) {
        if (@($records | Where-Object { $_.remote_outcome -ceq $outcome }).Count -eq 0) { throw "Missing remote outcome: $outcome" }
    }
    if (@($records | Where-Object { $_.event -ceq 'push_outcome' -and $_.remote_outcome -ceq 'confirmed_success' -and $_.error_diagnostics -eq $true -and $_.level -ceq 'warn' }).Count -eq 0) {
        throw 'Missing warning for successful remote write with local metadata diagnostics'
    }
    foreach ($start in @($records | Where-Object { $_.event -in @('request_start', 'step_start', 'git_start') })) {
        $endEvent = $start.event.Replace('_start', '_end')
        $ends = @($records | Where-Object { $_.event -ceq $endEvent -and $_.session -ceq $start.session -and $_.id -eq $start.id })
        if ($ends.Count -ne 1) { throw "Unpaired diagnostic span: $($start.event)" }
    }
    $result.diagnostic_records = $records.Count
    Record-Check 'paired diagnostic spans, typed failures, remote outcomes, zero dropped events and no private payloads'
    $result.passed = $true
    $result.phase = 'complete'
} finally {
    try {
        $result.checks = @($checks.ToArray())
        [IO.File]::WriteAllText($resultPath, ($result | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
    } finally {
        foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    }
}
Write-Host "Push smoke checks passed. Local fixture: $scratch"
Write-Host "Results: $resultPath"
Write-Host "Diagnostics: $logs"
Write-Host "CLI transcript (invented fixture data only): $transcript"
