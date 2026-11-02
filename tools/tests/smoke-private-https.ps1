param([string]$RemoteUrl = 'https://github.com/JasXTD/rocket-auth-test.git')

$ErrorActionPreference = 'Stop'
if ($RemoteUrl -notmatch '\Ahttps://github[.]com/[A-Za-z0-9-]+/[A-Za-z0-9_.-]+\z') {
    throw 'Use a plain https://github.com/OWNER/REPOSITORY URL without credentials, query parameters, or a fragment.'
}
if (-not $RemoteUrl.EndsWith('.git')) { $RemoteUrl += '.git' }
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-private-https-' + [guid]::NewGuid().ToString('N'))
$repository = Join-Path $scratch 'ROCKET_PRIVATE_client'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'empty-hooks'
$transcript = Join-Path $scratch 'cli-output.log'
$resultPath = Join-Path $scratch 'result.json'
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
    remote = $RemoteUrl
    repository = $repository
    authentication = 'existing Git Credential Manager credentials; no interactive login'
    tls = 'schannel; certificate verification enabled; redirects disabled'
    checkout = $false
}

function Invoke-Git {
    param([string[]]$GitArguments)
    $previousAction = $ErrorActionPreference
    try {
        # PowerShell 5.1 represents native stderr as ErrorRecord objects.
        $ErrorActionPreference = 'Continue'
        $output = @(& $gitPath -C $repository @GitArguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    if ($actual -ne 0) { throw "Fixture Git failed ($actual): $GitArguments`n$($output -join "`n")" }
    return $output
}

function Invoke-Inspect {
    param([string[]]$InspectArguments, [string]$Phase, [int]$ExpectedExit = 0)
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
    $runs.Add([ordered]@{ phase = $Phase; command = ($InspectArguments -join ' '); exit = $actual; elapsed_ms = $watch.ElapsedMilliseconds })
    [IO.File]::AppendAllText($transcript, ("PHASE: $Phase`nCOMMAND: " + ($InspectArguments -join ' ') + "`n" + ($output -join "`n") + "`n"))
    if ($actual -ne $ExpectedExit) {
        throw "Unexpected inspect exit $actual during $Phase (expected $ExpectedExit). No automatic retry. See $transcript. If sign-in is needed, run git credential-manager github login --browser in your terminal first."
    }
    return ($output -join "`n")
}

function Assert-LocalState {
    if ((Invoke-Git @('rev-parse', 'HEAD')) -cne $beforeHead) { throw 'Fetch changed local HEAD' }
    if ((Get-FileHash -LiteralPath $index -Algorithm SHA256).Hash -cne $beforeIndex) { throw 'Fetch changed index bytes' }
    if ((Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/heads/', 'refs/tags/') | Out-String) -cne $beforeRefs) {
        throw 'Fetch changed local branches or tags'
    }
    foreach ($file in $beforeFiles) {
        if ((Get-FileHash -LiteralPath $file.path -Algorithm SHA256).Hash -cne $file.sha256) { throw 'Fetch changed worktree bytes' }
    }
    if (@(Get-ChildItem -LiteralPath $repository -Force | Where-Object { $_.Name -ne '.git' }).Count -ne 2) {
        throw 'Unexpected worktree contents after fetch'
    }
}

function Assert-Refresh {
    param([string]$Output)
    if (-not $Output.Contains('Branches after fetch:') -or -not $Output.Contains('Status after fetch:') -or $Output.Contains('refresh failed:')) {
        throw 'Branch or status reconciliation did not succeed'
    }
}

function Assert-Fetch {
    param([string]$Output)
    Assert-Refresh $Output
    if (-not $Output.Contains('Fetch: Git command succeeded.') -or -not $Output.Contains('Ref updates:') -or $Output.Contains('parsing its ref updates failed:')) {
        throw 'Fetch command or structured result parser did not succeed'
    }
}

# Only the owned fixture config changes. For the negative case, disable the
# helper rather than deleting or revoking the user's existing credentials.
# GCM handles credential contents internally; never invoke credential fill/get
# directly or enable Git/cURL/GCM traces, which may disclose sensitive headers.
$names = @('ROCKET_LOG', 'ROCKET_LOG_DIR', 'GIT_CONFIG_GLOBAL', 'GIT_CONFIG_SYSTEM',
    'GIT_CONFIG_NOSYSTEM', 'GIT_TEMPLATE_DIR', 'GIT_TERMINAL_PROMPT', 'GIT_ASKPASS',
    'GIT_ALLOW_PROTOCOL', 'GIT_OPTIONAL_LOCKS', 'GIT_NO_LAZY_FETCH', 'GIT_NO_REPLACE_OBJECTS',
    'GCM_INTERACTIVE', 'GCM_TRACE', 'GCM_TRACE_SECRETS', 'GCM_TRACE_MSAUTH', 'GCM_DEBUG')
$names += @(Get-ChildItem Env: | Where-Object { $_.Name -like 'GIT_*' -or $_.Name -like 'GCM_TRACE*' } | ForEach-Object { $_.Name })
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
    $env:GCM_INTERACTIVE = '0'
    $env:GCM_TRACE = '0'
    $env:GCM_TRACE_SECRETS = '0'
    $env:GCM_TRACE_MSAUTH = '0'
    $env:GCM_DEBUG = '0'
    $evidence['git_version'] = (Invoke-Git @('--version') | Out-String).Trim()
    $evidence['gcm_version'] = (Invoke-Git @('credential-manager', '--version') | Out-String).Trim()
    $evidence['inspector_sha256'] = (Get-FileHash -LiteralPath $inspector -Algorithm SHA256).Hash
    Invoke-Git @('init', '--initial-branch=main') | Out-Null
    Invoke-Git @('config', 'user.name', 'Rocket Private HTTPS Smoke') | Out-Null
    Invoke-Git @('config', 'user.email', 'rocket@example.invalid') | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) | Out-Null
    Invoke-Git @('config', 'core.autocrlf', 'false') | Out-Null
    Invoke-Git @('config', 'commit.gpgsign', 'false') | Out-Null
    # PowerShell 5.1 drops empty native arguments, so write the empty values
    # directly to the newly created local config.
    [IO.File]::AppendAllText((Join-Path $repository '.git\config'), @'

[credential]
    helper =
    interactive = false
    trace = false
    traceSecrets = false
    debug = false
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
    Invoke-Git @('commit', '-m', 'Local private HTTPS fixture base') | Out-Null
    [IO.File]::WriteAllText($tracked, "staged locally`n")
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    [IO.File]::WriteAllText($tracked, "unstaged locally`n")
    [IO.File]::WriteAllText($untracked, "untracked locally`n")
    Invoke-Git @('remote', 'add', 'origin', $RemoteUrl) | Out-Null
    $beforeHead = Invoke-Git @('rev-parse', 'HEAD')
    $index = Join-Path $repository '.git\index'
    $beforeIndex = (Get-FileHash -LiteralPath $index -Algorithm SHA256).Hash
    $beforeRefs = Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/heads/', 'refs/tags/') | Out-String
    $beforeFiles = @($tracked, $untracked) | ForEach-Object { [pscustomobject]@{ path = $_; sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash } }

    Write-Host "Scratch repository: $repository"
    $listed = Invoke-Inspect @('remotes') -Phase 'list'
    if (-not $listed.Contains($RemoteUrl)) { throw 'Configured remote differs from the selected HTTPS URL' }
    $denied = Invoke-Inspect @('fetch', 'origin') -Phase 'credentials-disabled' -ExpectedExit 1
    Assert-Refresh $denied
    if ($denied -notmatch '\[(credentials_unavailable|authentication_rejected)\]') {
        throw 'The negative request failed without a recognized authentication error; do not count network/configuration failures as an auth test pass'
    }
    Assert-LocalState
    if (@(Invoke-Git @('for-each-ref', '--format=%(refname)', 'refs/remotes/')).Count -ne 0) { throw 'Rejected fetch unexpectedly created tracking refs' }
    $evidence['credentials_disabled_rejected'] = $true
    Write-Host 'PASS credentials-disabled fetch rejected; branches/status refreshed; local state preserved'

    # Re-enable only the installed GCM helper for this fixture. Cached credentials
    # may be used/refreshed by GCM, but no browser or account-selection UI is allowed.
    Invoke-Git @('config', 'credential.helper', 'manager') | Out-Null
    $first = Invoke-Inspect @('fetch', 'origin') -Phase 'authenticated-initial'
    Assert-Fetch $first
    Assert-LocalState
    $firstRefs = @(Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/remotes/origin/'))
    if ($firstRefs.Count -eq 0) { throw 'Authenticated fetch succeeded but no branches exist. Initialize the private test repository with a README commit.' }
    if ($first -notmatch 'kind: New,') { throw 'First authenticated fetch did not report new tracking refs' }
    $firstOids = @($firstRefs | ForEach-Object { ($_ -split ' ', 2)[1] })
    $fetchHead = [IO.File]::ReadAllText((Join-Path $repository '.git\FETCH_HEAD'))
    foreach ($oid in $firstOids) {
        if ((Invoke-Git @('cat-file', '-t', $oid)) -cne 'commit') { throw 'Fetched branch tip is not a commit object' }
        if ($fetchHead -notmatch ('(?m)^' + [regex]::Escape($oid) + '\t')) { throw 'Fetched tracking tip is absent from FETCH_HEAD' }
    }
    $evidence['first_remote_refs'] = $firstRefs
    $evidence['fetched_commit_count'] = [long](Invoke-Git @('rev-list', '--count', '--remotes=origin') | Out-String).Trim()
    Write-Host 'PASS authenticated Rocket fetch; new tracking refs and commit objects verified'

    $second = Invoke-Inspect @('fetch', 'origin') -Phase 'authenticated-repeat'
    Assert-Fetch $second
    Assert-LocalState
    $secondRefs = @(Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/remotes/origin/'))
    if (($firstRefs -join "`n") -ceq ($secondRefs -join "`n")) {
        if (([regex]::Matches($second, 'kind: UpToDate,')).Count -ne $secondRefs.Count -or $second -match 'kind: (New|FastForward|Forced),') {
            throw 'Unchanged remote refs did not all report UpToDate'
        }
        $evidence['repeat_result'] = 'UpToDate'
    } else {
        $evidence['repeat_result'] = 'Remote changed between requests; up-to-date case not fully observed'
    }
    $evidence['second_remote_refs'] = $secondRefs
    Invoke-Inspect @('status') -Phase 'final-status' | Out-Null
    Assert-LocalState

    $logPath = Join-Path $logs 'rocket.jsonl'
    $logText = [IO.File]::ReadAllText($logPath)
    $remoteParts = ([uri]$RemoteUrl).AbsolutePath.Trim('/').Split('/')
    $secondOids = @($secondRefs | ForEach-Object { ($_ -split ' ', 2)[1] })
    foreach ($private in (@('ROCKET_PRIVATE', 'rocket@example.invalid', 'github.com', $beforeHead) + $remoteParts + $firstOids + $secondOids)) {
        if ($logText.IndexOf($private, [StringComparison]::OrdinalIgnoreCase) -ge 0) { throw 'Fixture identity or remote metadata leaked into diagnostic records' }
    }
    $records = @(Get-Content -LiteralPath $logPath | ForEach-Object { $_ | ConvertFrom-Json -ErrorAction Stop })
    foreach ($expected in @(@('write_result', 'ok', 2), @('write_result', 'git_error', 1), @('parse', 'ok', 2))) {
        if (@($records | Where-Object { $_.event -eq $expected[0] -and $_.operation -eq 'fetch' -and $_.outcome -eq $expected[1] }).Count -ne $expected[2]) {
            throw 'Unexpected diagnostic fetch outcome/parse counts'
        }
    }
    if (@($records | Where-Object { $_.event -eq 'refresh' -and $_.outcome -eq 'ok' }).Count -ne 6) { throw 'Expected six successful reconciliation records' }
    if (@($records | Where-Object { $_.dropped_before -ne 0 }).Count -ne 0) { throw 'Diagnostic events were dropped' }
    $evidence['fetch_git_events'] = @($records | Where-Object { $_.event -eq 'git_end' -and $_.operation -eq 'fetch' } | Select-Object elapsed_ms, outcome, exit_code, stdout_bytes, stderr_bytes)
    $evidence['diagnostic_records'] = $records.Count
    $evidence['dropped_diagnostic_records'] = 0
    $evidence['local_state_preserved'] = $true
    $evidence['passed'] = $true
    Write-Host 'PASS repeated authenticated fetch, diagnostic privacy, and unchanged local HEAD/index/files'
    Write-Output "Private HTTPS smoke checks passed. Results: $resultPath"
    Write-Output "Diagnostics: $logPath"
    Write-Output "CLI transcript (contains private repository metadata): $transcript"
} catch {
    $evidence['failure'] = $_.Exception.Message
    throw
} finally {
    $evidence['finished_utc'] = [DateTime]::UtcNow.ToString('o')
    $evidence['runs'] = @($runs.ToArray())
    try { $evidence | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $resultPath -Encoding UTF8 }
    finally {
        foreach ($name in $names) {
            if ($null -eq $saved[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
            else { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
        }
    }
}
