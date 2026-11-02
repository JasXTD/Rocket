$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-pull-' + [guid]::NewGuid().ToString('N'))
$seed = Join-Path $scratch 'ROCKET_PRIVATE_publisher'
$bare = Join-Path $scratch 'ROCKET_PRIVATE_upstream.git'
$repository = Join-Path $scratch 'ROCKET_PRIVATE_client'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'empty-hooks'
$transcript = Join-Path $scratch 'cli-output.log'
$resultPath = Join-Path $scratch 'result.json'
$utf8 = New-Object Text.UTF8Encoding($false)
[IO.Directory]::CreateDirectory($seed) | Out-Null
[IO.Directory]::CreateDirectory($hooks) | Out-Null
$emptyConfig = Join-Path $scratch 'empty.config'
[IO.File]::WriteAllText($emptyConfig, '', $utf8)
$runs = New-Object 'System.Collections.Generic.List[object]'
$evidence = [ordered]@{ passed=$false; started_utc=[DateTime]::UtcNow.ToString('o'); repository=$repository; transport='local-file-only' }
$phase = 'setup'

function Invoke-Git([string[]]$Arguments, [string]$At = $repository) {
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $gitPath -C $At @Arguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    if ($actual -ne 0) { throw "Fixture Git failed ($actual): $Arguments`n$($output -join "`n")" }
    return ($output -join "`n")
}
function Invoke-Pull([string[]]$Arguments, [int]$ExpectedExit = 0, [string]$ExpectedText = '') {
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $inspector $repository @Arguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    $text = $output -join "`n"
    [IO.File]::AppendAllText($transcript, "PHASE: $phase`n$text`n", $utf8)
    $runs.Add([ordered]@{phase=$phase; exit=$actual; expected_exit=$ExpectedExit})
    if ($actual -ne $ExpectedExit -or ($ExpectedText -and -not $text.Contains($ExpectedText))) {
        throw "Unexpected pull result during $phase. Inspect $transcript"
    }
    Write-Host "PASS $phase (exit $actual)"
    return $text
}
function Local-State {
    return [ordered]@{
        head=(Invoke-Git @('rev-parse','HEAD'))
        index=(Get-FileHash -LiteralPath (Join-Path $repository '.git\index') -Algorithm SHA256).Hash
        file=[IO.File]::ReadAllText((Join-Path $repository 'file'))
    } | ConvertTo-Json -Compress
}
function Publish([string]$Contents) {
    [IO.File]::WriteAllText((Join-Path $seed 'file'), "$Contents`n", $utf8)
    Invoke-Git @('add','--all','--') -At $seed | Out-Null
    Invoke-Git @('commit','-m','Rocket local pull smoke fixture') -At $seed | Out-Null
    Invoke-Git @('push',$bare,'main') -At $seed | Out-Null
}

# Changes affect this runner process and its children only. Remove inherited
# Git repository/config/proxy overrides and restore the original environment.
$names = @('ROCKET_LOG','ROCKET_LOG_DIR','GIT_CONFIG_GLOBAL','GIT_CONFIG_SYSTEM','GIT_CONFIG_NOSYSTEM','GIT_ALLOW_PROTOCOL','GIT_TEMPLATE_DIR','GIT_TERMINAL_PROMPT')
$names += @(Get-ChildItem Env: | Where-Object { $_.Name -match '^(GIT_|SSH_)|_PROXY$' } | ForEach-Object { $_.Name })
$names = @($names | Select-Object -Unique)
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    foreach ($name in $names) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
    $env:ROCKET_LOG_DIR = $logs
    $env:GIT_CONFIG_GLOBAL = $emptyConfig
    $env:GIT_CONFIG_SYSTEM = $emptyConfig
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_ALLOW_PROTOCOL = 'file'
    $env:GIT_TEMPLATE_DIR = $hooks
    $env:GIT_TERMINAL_PROMPT = '0'
    Invoke-Git @('init','--initial-branch=main') -At $seed | Out-Null
    foreach ($setting in @(@('user.name','Rocket Pull Smoke'), @('user.email','rocket@example.invalid'), @('core.hooksPath',$hooks), @('core.autocrlf','false'), @('commit.gpgsign','false'))) {
        Invoke-Git @('config',$setting[0],$setting[1]) -At $seed | Out-Null
    }
    [IO.File]::WriteAllText((Join-Path $seed 'file'), "base`n", $utf8)
    Invoke-Git @('add','file') -At $seed | Out-Null
    Invoke-Git @('commit','-m','Rocket local pull base') -At $seed | Out-Null
    Invoke-Git @('clone','--bare',$seed,$bare) -At $scratch | Out-Null
    Invoke-Git @('clone',$bare,$repository) -At $scratch | Out-Null
    foreach ($setting in @(@('user.name','Rocket Pull Smoke'), @('user.email','rocket@example.invalid'), @('core.hooksPath',$hooks), @('core.autocrlf','false'), @('commit.gpgsign','false'))) {
        Invoke-Git @('config',$setting[0],$setting[1]) | Out-Null
    }
    Invoke-Git @('config','core.hooksPath',$hooks) -At $bare | Out-Null
    $phase = 'unsupported-option'
    Invoke-Pull @('pull','--rebase') 1 'usage: inspect' | Out-Null
    $phase = 'missing-upstream'
    Invoke-Git @('config','--unset','branch.main.remote') | Out-Null
    Invoke-Pull @('pull') 1 '[pull_missing_upstream]' | Out-Null
    Invoke-Git @('config','branch.main.remote','origin') | Out-Null
    if (Test-Path -LiteralPath (Join-Path $repository '.git\FETCH_HEAD')) { throw 'Preflight failure attempted a fetch.' }
    Publish 'advance'
    [IO.File]::WriteAllText((Join-Path $repository 'file'), "staged edit`n", $utf8)
    Invoke-Git @('add','file') | Out-Null
    [IO.File]::WriteAllText((Join-Path $repository 'file'), "unstaged edit`n", $utf8)
    $before = Local-State
    $phase = 'dirty-worktree'
    Invoke-Pull @('pull') 1 '[pull_dirty_worktree]' | Out-Null
    if ((Local-State) -cne $before) { throw 'Dirty pull changed local state.' }
    Invoke-Git @('reset','--hard','HEAD') | Out-Null # Owned synthetic fixture only.
    [IO.File]::WriteAllText((Join-Path $repository 'untracked'), "keep`n", $utf8)
    $phase = 'fast-forward'
    $text = Invoke-Pull @('pull') 0 'Pull branch update: FastForward'
    foreach ($label in @('Pull fetch: Git command succeeded.','Status after pull:','Branches after pull:','Operation state after pull:')) {
        if (-not $text.Contains($label)) { throw "Missing $label" }
    }
    if ((Invoke-Git @('rev-parse','HEAD')) -cne (Invoke-Git @('rev-parse','HEAD') -At $seed)) { throw 'Fast-forward tip mismatch.' }
    if ([IO.File]::ReadAllText((Join-Path $repository 'untracked')) -cne "keep`n") { throw 'Untracked bytes changed.' }
    $phase = 'up-to-date'
    Invoke-Pull @('pull') 0 'Pull branch update: UpToDate' | Out-Null
    Invoke-Git @('commit','--allow-empty','-m','Local only') | Out-Null
    $phase = 'local-ahead'
    Invoke-Pull @('pull') 0 'Pull branch update: UpToDate' | Out-Null
    Publish 'remote divergence'
    $before = Local-State
    $phase = 'diverged'
    $text = Invoke-Pull @('pull') 1 '[pull_diverged]'
    if (-not $text.Contains('Pull fetch: Git command succeeded.') -or (Local-State) -cne $before) { throw 'Divergence lost fetch outcome or changed local state.' }
    Invoke-Git @('reset','--hard','origin/main') | Out-Null # Owned fixture only.
    Publish 'lock test'
    $lock = Join-Path $repository '.git\index.lock'
    [IO.File]::WriteAllText($lock, "owned fixture lock`n", $utf8)
    $before = Local-State
    $phase = 'locked-index'
    Invoke-Pull @('pull') 1 '[pull_update_failed]' | Out-Null
    if ((Local-State) -cne $before -or [IO.File]::ReadAllText($lock) -cne "owned fixture lock`n") { throw 'Locked index or local state was changed.' }
    Remove-Item -LiteralPath $lock # Exact fixture-created file, after Git exited.
    Invoke-Git @('reset','--hard','origin/main') | Out-Null
    Publish 'authentication-shaped failure'
    $hook = Join-Path $hooks 'reference-transaction'
    [IO.File]::WriteAllText($hook, "#!/bin/sh`nif [ `"`$1`" = prepared ]; then`necho 'fatal: Authentication failed ROCKET_PRIVATE_server' >&2`nexit 1`nfi`n", $utf8)
    $before = Local-State
    $phase = 'fetch-authentication-diagnostic'
    $text = Invoke-Pull @('pull') 1 '[authentication_rejected]'
    if ($text.Contains('ROCKET_PRIVATE_server') -or -not $text.Contains('Pull branch update: not attempted.') -or (Local-State) -cne $before) { throw 'Failed fetch privacy, skipped update, or state preservation check failed.' }
    $phase = 'explicit-raw-error'
    Invoke-Pull @('pull','--raw-error') 1 'ROCKET_PRIVATE_server' | Out-Null
    Remove-Item -LiteralPath $hook

    $warningHook = Join-Path $hooks 'post-merge'
    [IO.File]::WriteAllText($warningHook, "#!/bin/sh`necho 'error: ROCKET_PRIVATE_post_merge' >&2`nexit 1`n", $utf8)
    $phase = 'successful-update-hook-warning'
    $text = Invoke-Pull @('pull') 0 'Git emitted error diagnostics despite a successful branch update.'
    if (-not $text.Contains('Pull branch update: FastForward') -or $text.Contains('ROCKET_PRIVATE_post_merge')) { throw 'Successful update warning was hidden or leaked raw hook output.' }
    if ((Invoke-Git @('rev-parse','HEAD')) -cne (Invoke-Git @('rev-parse','HEAD') -At $seed)) { throw 'Expected successful update despite hook warning.' }
    Remove-Item -LiteralPath $warningHook

    $phase = 'diagnostics'
    $records = @()
    $diagnostics = ''
    foreach ($file in @(Get-ChildItem -LiteralPath $logs -Filter '*.jsonl' -File)) {
        $diagnostics += [IO.File]::ReadAllText($file.FullName)
        foreach ($line in [IO.File]::ReadAllLines($file.FullName)) { $records += ($line | ConvertFrom-Json -ErrorAction Stop) }
    }
    if ($records.Count -eq 0 -or @($records | Where-Object { $_.schema -ne 1 -or $_.dropped_before -ne 0 }).Count) { throw 'Missing/invalid/dropped diagnostic events.' }
    foreach ($start in @($records | Where-Object { $_.event -in @('request_start','step_start','git_start') })) {
        $ends = @($records | Where-Object { $_.event -ceq $start.event.Replace('_start','_end') -and $_.session -ceq $start.session -and $_.id -eq $start.id })
        if ($ends.Count -ne 1) { throw 'Unpaired diagnostic span.' }
    }
    foreach ($code in @('pull_missing_upstream','pull_dirty_worktree','pull_diverged','pull_update_failed','authentication_rejected')) {
        if (-not $diagnostics.Contains(('"failure_code":"' + $code + '"'))) { throw "Missing logged failure $code" }
    }
    $outcomes = @($records | Where-Object { $_.event -ceq 'pull_outcome' })
    if ($outcomes.Count -ne 8) { throw "Expected 8 attempted pulls; saw $($outcomes.Count)." }
    foreach ($outcome in $outcomes) {
        $sessionRecords = @($records | Where-Object { $_.session -ceq $outcome.session })
        if (@($sessionRecords | Where-Object { $_.event -ceq 'git_start' -and $_.operation -ceq 'fetch' }).Count -ne 1) { throw 'Pull retried or omitted fetch.' }
        $mergeCount = @($sessionRecords | Where-Object { $_.event -ceq 'git_start' -and $_.operation -ceq 'merge' }).Count
        if ($mergeCount -gt 1 -or ($outcome.update_outcome -ceq 'skipped' -and $mergeCount -ne 0)) { throw 'Unexpected merge attempt.' }
        foreach ($operation in @('status','branches','operation')) {
            if (@($sessionRecords | Where-Object { $_.event -ceq 'refresh' -and $_.operation -ceq $operation -and $_.outcome -ceq 'ok' }).Count -ne 1) { throw "Missing $operation refresh." }
        }
    }
    if ($diagnostics -match 'ROCKET_PRIVATE_|refs/heads/|refs/remotes/|gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|Authorization:|PRIVATE KEY' -or $diagnostics.Contains($scratch)) { throw 'Diagnostic privacy scan failed.' }
    $evidence.diagnostic_records = $records.Count
    $evidence.diagnostic_dropped = 0
    $evidence.attempted_pulls = $outcomes.Count
    $evidence.passed = $true
    Write-Host 'PASS phase outcomes, paired spans, independent refreshes, no retries, zero drops, and diagnostic privacy'
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    $evidence.phase = if ($evidence.passed) { 'complete' } else { $phase }
    $evidence.finished_utc = [DateTime]::UtcNow.ToString('o')
    $evidence.cli_runs = $runs.ToArray()
    [IO.File]::WriteAllText($resultPath, ($evidence | ConvertTo-Json -Depth 6), $utf8)
}
Write-Host "Pull smoke checks passed. Results: $resultPath"
Write-Host "Scratch repository: $repository"
Write-Host "Diagnostics: $logs"
Write-Host "CLI transcript (synthetic fixture data only): $transcript"
