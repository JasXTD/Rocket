$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$remoteUrl = 'https://github.com/ocornut/imgui.git'
$trackingRef = 'refs/remotes/origin/master'
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-imgui-https-' + [guid]::NewGuid().ToString('N'))
$repository = Join-Path $scratch 'ROCKET_PRIVATE_client'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'empty-hooks'
$transcript = Join-Path $scratch 'cli-output.log'
$evidencePath = Join-Path $scratch 'result.json'
$emptyConfig = Join-Path $scratch 'empty.config'
$denyAskpass = Join-Path $scratch 'deny-askpass.cmd'
[IO.Directory]::CreateDirectory($repository) | Out-Null
[IO.Directory]::CreateDirectory($hooks) | Out-Null
[IO.File]::WriteAllText($emptyConfig, '')
[IO.File]::WriteAllText($denyAskpass, "@exit /b 1`r`n", [Text.Encoding]::ASCII)
$runs = New-Object 'System.Collections.Generic.List[object]'
$evidence = [ordered]@{
    passed = $false
    started_utc = [DateTime]::UtcNow.ToString('o')
    remote = $remoteUrl
    branch = 'master'
    repository = $repository
    authentication = 'anonymous; credential helpers and askpass disabled'
    tls = 'schannel; certificate verification enabled; redirects disabled'
    checkout = $false
    runs = @()
}

function Invoke-Git {
    param([string[]]$GitArguments)
    $previousAction = $ErrorActionPreference
    try {
        # Windows PowerShell 5.1 wraps native stderr in ErrorRecord objects.
        $ErrorActionPreference = 'Continue'
        $output = @(& $gitPath -C $repository @GitArguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    if ($actual -ne 0) { throw "Fixture Git failed ($actual): $GitArguments`n$($output -join "`n")" }
    return $output
}

function Invoke-Inspect {
    param([string[]]$InspectArguments)
    $previousAction = $ErrorActionPreference
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $inspector $repository @InspectArguments 2>&1)
        $actual = $LASTEXITCODE
    } finally {
        $watch.Stop()
        $ErrorActionPreference = $previousAction
    }
    $runs.Add([ordered]@{ command = ($InspectArguments -join ' '); exit = $actual; elapsed_ms = $watch.ElapsedMilliseconds })
    [IO.File]::AppendAllText($transcript, ("COMMAND: " + ($InspectArguments -join ' ') + "`n" + ($output -join "`n") + "`n"))
    if ($actual -ne 0) { throw "inspect exited $actual. No automatic retry. See $transcript and $logs" }
    Write-Host "PASS $($InspectArguments -join ' ') ($($watch.ElapsedMilliseconds) ms)"
    return ($output -join "`n")
}

function Assert-LocalState {
    if ((Invoke-Git @('rev-parse', 'HEAD')) -cne $beforeHead) { throw 'Fetch changed local HEAD' }
    if ((Get-FileHash -LiteralPath $index -Algorithm SHA256).Hash -cne $beforeIndex) { throw 'Fetch changed the index bytes' }
    if ((Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/heads/', 'refs/tags/') | Out-String) -cne $beforeRefs) {
        throw 'Fetch changed local branches or tags'
    }
    if ((Get-FileHash -LiteralPath $tracked -Algorithm SHA256).Hash -cne $beforeTracked) { throw 'Fetch changed unstaged file bytes' }
    if ((Get-FileHash -LiteralPath $untracked -Algorithm SHA256).Hash -cne $beforeUntracked) { throw 'Fetch changed untracked file bytes' }
    if (@(Get-ChildItem -LiteralPath $repository -Force | Where-Object { $_.Name -ne '.git' }).Count -ne 2) {
        throw 'Unexpected worktree contents after fetch'
    }
}

function Assert-FetchReport {
    param([string]$Output)
    foreach ($required in @('Fetch: Git command succeeded.', 'Ref updates:', 'Branches after fetch:', 'Status after fetch:')) {
        if (-not $Output.Contains($required)) { throw "Missing successful fetch report component: $required" }
    }
    if ($Output.Contains('refresh failed:') -or $Output.Contains('parsing its ref updates failed:')) {
        throw 'Fetch succeeded but its parser or reconciliation failed'
    }
}

# This child script isolates Git configuration and authentication without reading
# credential stores or changing global config. Restore the caller environment if
# the script is invoked in-process. Keep the scratch repo and evidence on failure.
$names = @('ROCKET_LOG', 'ROCKET_LOG_DIR', 'GIT_CONFIG_GLOBAL', 'GIT_CONFIG_SYSTEM',
    'GIT_CONFIG_NOSYSTEM', 'GIT_TEMPLATE_DIR', 'GIT_TERMINAL_PROMPT', 'GIT_ASKPASS',
    'GIT_ALLOW_PROTOCOL', 'GIT_OPTIONAL_LOCKS', 'GIT_NO_LAZY_FETCH', 'GIT_NO_REPLACE_OBJECTS')
$names += @(Get-ChildItem Env: | Where-Object { $_.Name -like 'GIT_*' } | ForEach-Object { $_.Name })
$names = @($names | Select-Object -Unique)
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    foreach ($name in $names) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
    $env:ROCKET_LOG_DIR = $logs
    $env:GIT_CONFIG_GLOBAL = $emptyConfig
    $env:GIT_CONFIG_SYSTEM = $emptyConfig
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_TEMPLATE_DIR = $hooks
    $env:GIT_TERMINAL_PROMPT = '0'
    $env:GIT_ASKPASS = $denyAskpass
    $env:GIT_ALLOW_PROTOCOL = 'https'
    $env:GIT_OPTIONAL_LOCKS = '0'
    $env:GIT_NO_LAZY_FETCH = '1'
    $env:GIT_NO_REPLACE_OBJECTS = '1'
    $evidence['git_version'] = (Invoke-Git @('--version') | Out-String).Trim()
    $evidence['inspector_sha256'] = (Get-FileHash -LiteralPath $inspector -Algorithm SHA256).Hash
    Invoke-Git @('init', '--initial-branch=main') | Out-Null
    Invoke-Git @('config', 'user.name', 'Rocket HTTPS Smoke') | Out-Null
    Invoke-Git @('config', 'user.email', 'rocket@example.invalid') | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) | Out-Null
    Invoke-Git @('config', 'core.autocrlf', 'false') | Out-Null
    Invoke-Git @('config', 'commit.gpgsign', 'false') | Out-Null
    # Write empty helper/header values directly: PowerShell 5.1 drops empty
    # native arguments. Only this newly created repository config is modified.
    [IO.File]::AppendAllText((Join-Path $repository '.git\config'), @'

[credential]
    helper =
    interactive = false
[http]
    extraHeader =
    sslBackend = schannel
    sslVerify = true
    followRedirects = false
[gc]
    auto = 0
[maintenance]
    auto = false
'@)
    $tracked = Join-Path $repository 'ROCKET_PRIVATE_file'
    $untracked = Join-Path $repository 'ROCKET_PRIVATE_untracked'
    [IO.File]::WriteAllText($tracked, "base`n")
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    Invoke-Git @('commit', '-m', 'Local HTTPS fixture base') | Out-Null
    [IO.File]::WriteAllText($tracked, "staged locally`n")
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    [IO.File]::WriteAllText($tracked, "unstaged locally`n")
    [IO.File]::WriteAllText($untracked, "untracked locally`n")
    Invoke-Git @('remote', 'add', '-t', 'master', 'origin', $remoteUrl) | Out-Null
    $beforeHead = Invoke-Git @('rev-parse', 'HEAD')
    $index = Join-Path $repository '.git\index'
    $beforeIndex = (Get-FileHash -LiteralPath $index -Algorithm SHA256).Hash
    $beforeRefs = Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/heads/', 'refs/tags/') | Out-String
    $beforeTracked = (Get-FileHash -LiteralPath $tracked -Algorithm SHA256).Hash
    $beforeUntracked = (Get-FileHash -LiteralPath $untracked -Algorithm SHA256).Hash

    Write-Host "Testing anonymous HTTPS against $remoteUrl (master history; no source checkout)."
    Write-Host "Scratch repository: $repository"
    $listed = Invoke-Inspect @('remotes')
    if (-not $listed.Contains($remoteUrl)) { throw 'Remote listing does not match the selected HTTPS URL' }
    $first = Invoke-Inspect @('fetch', 'origin')
    Assert-FetchReport $first
    if ($first -notmatch 'kind: New,') { throw 'Initial fetch did not report a new remote-tracking branch' }
    Assert-LocalState
    $firstOid = (Invoke-Git @('rev-parse', '--verify', $trackingRef) | Out-String).Trim()
    if ((Invoke-Git @('cat-file', '-t', $firstOid)) -cne 'commit') { throw 'Fetched tip is not a commit object' }
    $fetchHead = [IO.File]::ReadAllText((Join-Path $repository '.git\FETCH_HEAD'))
    if (-not $fetchHead.StartsWith($firstOid + "`t")) { throw 'FETCH_HEAD differs from fetched tracking ref' }
    if (@(Invoke-Git @('for-each-ref', '--format=%(refname)', 'refs/remotes/')).Count -ne 1) { throw 'Unexpected remote-tracking refs' }
    $evidence['first_fetched_commit'] = $firstOid
    $evidence['commit_count'] = [long](Invoke-Git @('rev-list', '--count', $trackingRef) | Out-String).Trim()
    Write-Host "PASS fetched master tip $firstOid; local HEAD, index, branches, tags, and files preserved"

    $second = Invoke-Inspect @('fetch', 'origin')
    Assert-FetchReport $second
    Assert-LocalState
    $secondOid = (Invoke-Git @('rev-parse', '--verify', $trackingRef) | Out-String).Trim()
    if ($firstOid -ceq $secondOid) {
        if ($second -notmatch 'kind: UpToDate,') { throw 'Unchanged remote tip did not report UpToDate' }
        $evidence['repeat_result'] = 'UpToDate'
    } else {
        # A public repository can advance during this test; record that honestly.
        if ($second -notmatch 'kind: (FastForward|Forced),') { throw 'Changed remote tip lacks an update result' }
        $evidence['repeat_result'] = 'Remote changed between requests; up-to-date case not observed'
    }
    $evidence['second_fetched_commit'] = $secondOid
    Invoke-Inspect @('status') | Out-Null
    Assert-LocalState

    $logPath = Join-Path $logs 'rocket.jsonl'
    $logText = [IO.File]::ReadAllText($logPath)
    foreach ($private in @('ROCKET_PRIVATE', 'rocket@example.invalid', 'ocornut', 'github.com', $firstOid, $secondOid, $beforeHead)) {
        if ($logText.Contains($private)) { throw 'Fixture data or remote identity leaked into diagnostic records' }
    }
    $records = @(Get-Content -LiteralPath $logPath | ForEach-Object { $_ | ConvertFrom-Json -ErrorAction Stop })
    if (@($records | Where-Object { $_.event -eq 'write_result' -and $_.operation -eq 'fetch' -and $_.outcome -eq 'ok' }).Count -ne 2) { throw 'Expected two successful fetch results in diagnostics' }
    if (@($records | Where-Object { $_.event -eq 'parse' -and $_.operation -eq 'fetch' -and $_.outcome -eq 'ok' }).Count -ne 2) { throw 'Expected two successful fetch parses in diagnostics' }
    if (@($records | Where-Object { $_.event -eq 'refresh' -and $_.outcome -eq 'ok' }).Count -ne 4) { throw 'Expected two successful branch and status refreshes' }
    if (@($records | Where-Object { $_.dropped_before -ne 0 }).Count -ne 0) { throw 'Diagnostic events were dropped' }
    $evidence['fetch_git_events'] = @($records | Where-Object { $_.event -eq 'git_end' -and $_.operation -eq 'fetch' } | Select-Object elapsed_ms, outcome, exit_code, stdout_bytes, stderr_bytes)
    $evidence['progress_observed'] = ($first -match 'Fetch (Enumerating|Counting|Compressing|Receiving|Resolving): [0-9]')
    $evidence['local_state_preserved'] = $true
    $evidence['diagnostic_records'] = $records.Count
    $evidence['dropped_diagnostic_records'] = 0
    $evidence['passed'] = $true
    Write-Host 'PASS repeated fetch, result parsing, branch/status refreshes, and diagnostic privacy'
    Write-Output "HTTPS smoke checks passed. Repository: $repository"
    Write-Output "Diagnostics: $logPath"
    Write-Output "CLI transcript: $transcript"
    Write-Output "Machine-readable result: $evidencePath"
} catch {
    $evidence['failure'] = $_.Exception.Message
    throw
} finally {
    $evidence['finished_utc'] = [DateTime]::UtcNow.ToString('o')
    $evidence['runs'] = @($runs.ToArray())
    try { $evidence | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $evidencePath -Encoding UTF8 }
    finally {
        foreach ($name in $names) {
            if ($null -eq $saved[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
            else { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
        }
    }
}
