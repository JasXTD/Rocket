param(
    [string]$RemoteUrl = 'https://github.com/JasXTD/rocket-auth-test.git',
    [switch]$SelfTest,
    [ValidateSet('Revoked', 'StillValid', 'NetworkFailure', 'Public')]
    [string]$SelfTestScenario = 'Revoked'
)

$ErrorActionPreference = 'Stop'
if ($RemoteUrl -cnotmatch '\Ahttps://github[.]com/[A-Za-z0-9-]+/[A-Za-z0-9_.-]+\z') {
    throw 'Use a plain https://github.com/OWNER/REPOSITORY URL without credentials or query parameters.'
}
if (-not $SelfTest -and $SelfTestScenario -ne 'Revoked') { throw 'Scenarios apply only to -SelfTest.' }
if (-not $RemoteUrl.EndsWith('.git')) { $RemoteUrl += '.git' }
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
$helper = Join-Path $PSScriptRoot 'revocation-credential-helper.ps1'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$powershellPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-revoked-https-' + [guid]::NewGuid().ToString('N'))
$repository = Join-Path $scratch 'ROCKET_PRIVATE_client'
$logs = Join-Path $scratch 'logs'
$isolatedHome = Join-Path $scratch 'empty-home'
$hooks = Join-Path $scratch 'empty-hooks'
$emptyConfig = Join-Path $scratch 'empty.config'
$denyAskpass = Join-Path $scratch 'deny-askpass.cmd'
$marker = Join-Path $scratch 'helper-events.log'
$resultPath = Join-Path $scratch 'result.json'
foreach ($directory in @($repository, $isolatedHome, $hooks)) { [IO.Directory]::CreateDirectory($directory) | Out-Null }
[IO.File]::WriteAllText($emptyConfig, '')
[IO.File]::WriteAllText($denyAskpass, "@exit /b 1`r`n", [Text.Encoding]::ASCII)
$phase = 'setup'
$failureHint = 'Check the fixture setup and the built inspector.'
$token = $null
$secureToken = $null
$server = $null
$serverStarted = $false
$runs = New-Object 'System.Collections.Generic.List[object]'
$evidence = [ordered]@{
    passed = $false; mode = $(if ($SelfTest) { 'loopback-fake-token' } else { 'github-disposable-token' })
    started_utc = [DateTime]::UtcNow.ToString('o'); remote = $RemoteUrl
    authentication = 'test helper only; child environment; no credential store'
    self_test_scenario = $(if ($SelfTest) { $SelfTestScenario } else { $null })
}

function Stop-Check([string]$Message) { $script:failureHint = $Message; throw 'Check failed' }

function Quote-WindowsArgument([string]$Value) {
    # CommandLineToArgvW / CRT rules, including empty values and trailing slashes.
    '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}

function New-IsolatedStartInfo([string]$Executable, [string[]]$Arguments) {
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $Executable
    $start.Arguments = ($Arguments | ForEach-Object { Quote-WindowsArgument $_ }) -join ' '
    $start.WorkingDirectory = $scratch
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.RedirectStandardInput = $true
    foreach ($key in @($start.EnvironmentVariables.Keys)) {
        if ($key -match '^(GIT_|GCM_|ROCKET_|SSH_|CURL_|SSL_CERT_)' -or
            $key -match '^(HTTP_PROXY|HTTPS_PROXY|ALL_PROXY|NO_PROXY|NETRC)$') {
            $start.EnvironmentVariables.Remove($key)
        }
    }
    # Changes are confined to the subprocess. The launching shell is unchanged.
    foreach ($key in @('HOME', 'USERPROFILE', 'XDG_CONFIG_HOME', 'APPDATA', 'LOCALAPPDATA')) {
        $start.EnvironmentVariables[$key] = $isolatedHome
    }
    $values = @{
        GIT_CONFIG_GLOBAL = $emptyConfig; GIT_CONFIG_SYSTEM = $emptyConfig; GIT_CONFIG_NOSYSTEM = '1'
        GIT_TEMPLATE_DIR = $hooks; GIT_TERMINAL_PROMPT = '0'; GIT_ASKPASS = $denyAskpass
        GIT_ALLOW_PROTOCOL = $(if ($SelfTest) { 'http' } else { 'https' })
        GIT_OPTIONAL_LOCKS = '0'; GIT_NO_LAZY_FETCH = '1'; GIT_NO_REPLACE_OBJECTS = '1'
        GCM_INTERACTIVE = '0'; ROCKET_LOG_DIR = $logs; LC_ALL = 'C'
        ROCKET_REVOCATION_MARKER = $marker
    }
    if ($script:remoteUri) {
        $values.ROCKET_REVOCATION_PROTOCOL = $script:remoteUri.Scheme
        $values.ROCKET_REVOCATION_HOST = $script:remoteUri.Authority
        $values.ROCKET_REVOCATION_PATH = $script:remoteUri.AbsolutePath.TrimStart('/')
        $values.ROCKET_REVOCATION_USER = $(if ($SelfTest) { 'rocket-test' } else { 'x-access-token' })
    }
    foreach ($key in $values.Keys) { $start.EnvironmentVariables[$key] = $values[$key] }
    return $start
}

function Invoke-Isolated {
    param([string]$Executable, [string[]]$Arguments, [switch]$WithToken, [string]$InputData = '')
    $start = New-IsolatedStartInfo $Executable $Arguments
    if ($WithToken) { $start.EnvironmentVariables['ROCKET_REVOCATION_TOKEN'] = $token }
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $start
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $started = $false
    try {
        [void]$process.Start()
        $started = $true
        $start.EnvironmentVariables.Remove('ROCKET_REVOCATION_TOKEN')
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.StandardInput.Write($InputData)
        $process.StandardInput.Close()
        if (-not $process.WaitForExit(90000)) { Stop-Check 'Child exceeded the test time limit; no auth conclusion.' }
        return [pscustomobject]@{ exit = $process.ExitCode; stdout = $stdout.Result; stderr = $stderr.Result; ms = $watch.ElapsedMilliseconds }
    } finally {
        $start.EnvironmentVariables.Remove('ROCKET_REVOCATION_TOKEN')
        try {
            if ($started -and -not $process.HasExited) {
                try { $process.Kill() }
                catch [InvalidOperationException] { if (-not $process.HasExited) { throw } }
                if (-not $process.WaitForExit(5000)) { Stop-Check 'Child cleanup did not complete; no auth conclusion.' }
            }
        } finally { $process.Dispose() }
    }
}

function Invoke-Git([string[]]$GitArguments, [string]$At = $repository) {
    $result = Invoke-Isolated $gitPath (@('-C', $At, '-c', 'gc.auto=0', '-c', 'maintenance.auto=false') + $GitArguments)
    if ($result.exit -ne 0) { Stop-Check 'Fixture Git command failed. No raw Git output was saved.' }
    return $result.stdout.TrimEnd("`r", "`n")
}

function Assert-NoSecret([string]$Text) {
    if ($token) {
        $basic = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("$($script:credentialUser):$token"))
        if ($Text.Contains($token) -or $Text.Contains($basic)) {
            Stop-Check 'A secret appeared in captured data. Do not share results; revoke the disposable token.'
        }
    }
}

function Get-HelperCount {
    if (Test-Path -LiteralPath $marker) { return @(Get-Content -LiteralPath $marker | Where-Object { $_ -ceq 'get_token' }).Count }
    return 0
}

function Invoke-Fetch([switch]$WithToken) {
    $before = Get-HelperCount
    $result = Invoke-Isolated $inspector @($repository, 'fetch', 'origin') -WithToken:$WithToken
    $text = $result.stdout + $result.stderr
    Assert-NoSecret $text
    $failureCode = $null
    if ($text -match '\[([a-z_]+)\]') { $failureCode = $Matches[1] }
    $requests = (Get-HelperCount) - $before
    $runs.Add([ordered]@{ phase = $phase; exit = $result.exit; elapsed_ms = $result.ms; failure_code = $failureCode; token_requests = $requests })
    if (-not $text.Contains('Branches after fetch:') -or -not $text.Contains('Status after fetch:') -or $text.Contains('refresh failed:')) {
        Stop-Check 'Rocket branch/status reconciliation did not succeed.'
    }
    if ($WithToken -and $requests -lt 1 -and $result.exit -eq 0) { Stop-Check 'Fetch succeeded without using the disposable token; this is not a valid authentication test.' }
    if ($result.exit -eq 0 -and (-not $text.Contains('Ref updates:') -or $text.Contains('parsing its ref updates failed:'))) {
        Stop-Check 'Fetch succeeded but structured ref parsing failed.'
    }
    return [pscustomobject]@{ exit = $result.exit; failure_code = $failureCode; token_requests = $requests }
}

function Get-LocalState {
    [ordered]@{
        head = Invoke-Git @('rev-parse', 'HEAD')
        refs = Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/heads/', 'refs/tags/')
        index = (Get-FileHash -LiteralPath (Join-Path $repository '.git\index') -Algorithm SHA256).Hash
        files = @(Get-ChildItem -LiteralPath $repository -Force | Where-Object { $_.Name -ne '.git' } | Sort-Object Name | ForEach-Object {
            if ($_.PSIsContainer) { Stop-Check 'Unexpected checkout directory.' }
            [ordered]@{ name = $_.Name; hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
        })
    } | ConvertTo-Json -Depth 4 -Compress
}

function Assert-LocalState {
    if ((Get-LocalState) -cne $script:beforeState) { Stop-Check 'Local HEAD, refs, index, or worktree changed.' }
}

try {
    Write-Host "Test folder: $scratch"
    Invoke-Git @('init', '--initial-branch=main') | Out-Null
    Invoke-Git @('config', 'user.name', 'Rocket Revocation Test') | Out-Null
    Invoke-Git @('config', 'user.email', 'rocket@example.invalid') | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) | Out-Null
    Invoke-Git @('config', 'core.autocrlf', 'false') | Out-Null
    Invoke-Git @('config', 'commit.gpgsign', 'false') | Out-Null
    [IO.File]::AppendAllText((Join-Path $repository '.git\config'), @'

[credential]
    helper =
    useHttpPath = true
    interactive = false
[http]
    extraHeader =
    proxy =
    sslBackend = schannel
    sslVerify = true
    followRedirects = false
[gc]
    auto = 0
[maintenance]
    auto = false
'@)
    # Git executes !helpers with sh; quote paths independently of Windows argv.
    $shPowerShell = "'" + $powershellPath.Replace('\', '/').Replace("'", "'\''") + "'"
    $shHelper = "'" + $helper.Replace('\', '/').Replace("'", "'\''") + "'"
    Invoke-Git @('config', '--add', 'credential.helper', "!$shPowerShell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $shHelper") | Out-Null
    $tracked = Join-Path $repository 'ROCKET_PRIVATE_file'
    $untracked = Join-Path $repository 'ROCKET_PRIVATE_untracked'
    [IO.File]::WriteAllText($tracked, "base`n")
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    Invoke-Git @('commit', '-m', 'Local revocation fixture') | Out-Null
    [IO.File]::WriteAllText($tracked, "staged`n")
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    [IO.File]::WriteAllText($tracked, "unstaged`n")
    [IO.File]::WriteAllText($untracked, "untracked`n")

    if ($SelfTest) {
        $seed = Join-Path $scratch 'http-seed'
        [IO.Directory]::CreateDirectory($seed) | Out-Null
        Invoke-Git @('init', '--initial-branch=main') -At $seed | Out-Null
        [IO.File]::WriteAllText((Join-Path $seed 'README'), "remote-only fixture content`n")
        Invoke-Git @('add', '--', 'README') -At $seed | Out-Null
        Invoke-Git @('-c', 'user.name=Rocket Offline', '-c', 'user.email=offline@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '-m', 'Remote fixture') -At $seed | Out-Null
        Invoke-Git @('update-server-info') -At $seed | Out-Null
        $serverScript = Join-Path $PSScriptRoot 'revocation-http-fixture.ps1'
        $serverArgs = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $serverScript, '-Repository', (Join-Path $seed '.git'), '-ControlDirectory', $scratch)
        if ($SelfTestScenario -eq 'Public') { $serverArgs += '-Public' }
        $server = New-Object Diagnostics.Process
        $server.StartInfo = New-IsolatedStartInfo $powershellPath $serverArgs
        [void]$server.Start()
        $serverStarted = $true
        $server.StandardInput.Close()
        $serverOut = $server.StandardOutput.ReadToEndAsync()
        $serverErr = $server.StandardError.ReadToEndAsync()
        $ready = Join-Path $scratch 'ready.port'
        $readyDeadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not (Test-Path -LiteralPath $ready)) {
            if ($server.HasExited -or [DateTime]::UtcNow -gt $readyDeadline) { Stop-Check 'Offline server did not start.' }
            Start-Sleep -Milliseconds 30
        }
        $port = [int][IO.File]::ReadAllText($ready)
        $RemoteUrl = "http://127.0.0.1:$port/rocket-auth-test.git"
        $evidence.remote = $RemoteUrl
        $token = [string]::Concat('ROCKET_FAKE_TOKEN_ONLY_', '0123456789')
    }
    $script:remoteUri = [uri]$RemoteUrl
    $script:credentialUser = $(if ($SelfTest) { 'rocket-test' } else { 'x-access-token' })
    if ($SelfTest) {
        # Exercise the real helper protocol with invented input only, never print
        # its credential response. The live path never runs credential fill/get.
        $phase = 'helper-protocol'
        $context = "protocol=http`nhost=$($script:remoteUri.Authority)`npath=rocket-auth-test.git`n"
        $helperArgs = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $helper)
        $get = Invoke-Isolated $powershellPath ($helperArgs + 'get') -WithToken -InputData ($context + "`n")
        if ($get.exit -ne 0 -or $get.stdout -cne "username=rocket-test`r`npassword=$token`r`n`r`n") { Stop-Check 'The credential helper did not return the expected fake credential.' }
        $noToken = Invoke-Isolated $powershellPath ($helperArgs + 'get') -InputData ($context + "`n")
        if ($noToken.exit -ne 0 -or $noToken.stdout) { Stop-Check 'The credential helper supplied a credential without a token.' }
        foreach ($wrong in @($context.Replace('protocol=http', 'protocol=https'), $context.Replace($script:remoteUri.Authority, 'other.invalid'), $context.Replace('rocket-auth-test.git', 'another.git'), ($context + "username=another-user`n"))) {
            $rejected = Invoke-Isolated $powershellPath ($helperArgs + 'get') -WithToken -InputData ($wrong + "`n")
            if ($rejected.exit -ne 0 -or $rejected.stdout) { Stop-Check 'The credential helper accepted the wrong credential context.' }
        }
        foreach ($operation in @('store', 'erase')) {
            $ignored = Invoke-Isolated $powershellPath ($helperArgs + $operation) -WithToken -InputData ($context + "username=rocket-test`npassword=$token`n`n")
            if ($ignored.exit -ne 0 -or $ignored.stdout) { Stop-Check 'Credential store/erase was not a no-op.' }
        }
        Write-Host 'PASS helper context restrictions, missing-token rejection, and store/erase no-ops'
    }
    Invoke-Git @('remote', 'add', 'origin', $RemoteUrl) | Out-Null
    $script:beforeState = Get-LocalState
    $evidence.inspector_sha256 = (Get-FileHash -LiteralPath $inspector -Algorithm SHA256).Hash

    $phase = 'credentials-disabled'
    $anonymous = Invoke-Fetch
    Assert-LocalState
    if ($anonymous.exit -eq 0) { Stop-Check 'The repository is accessible without the test token. Use a private test repository.' }
    if ($anonymous.failure_code -notin @('credentials_unavailable', 'authentication_rejected', 'access_denied', 'repository_unavailable') -or $anonymous.token_requests -ne 0) {
        Stop-Check 'The anonymous check did not reach an expected authentication/access failure. Network/TLS failures are inconclusive.'
    }
    if (Invoke-Git @('for-each-ref', '--format=%(refname)', 'refs/remotes/')) { Stop-Check 'Rejected anonymous fetch created remote refs.' }
    Write-Host 'PASS anonymous fetch rejected; saved login bypassed'

    if (-not $SelfTest) {
        $phase = 'token-entry'
        Write-Host 'Paste only the NEW disposable token at the hidden prompt below. It is not saved to a file or command history.'
        $secureToken = Read-Host 'Disposable test token (hidden)' -AsSecureString
        $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureToken)
        try { $token = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer) }
        finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer) }
        if ($token -cnotmatch '\A(?:github_pat_|ghp_)[A-Za-z0-9_]{20,250}\z') { Stop-Check 'The input does not look like a GitHub PAT. Run again with the disposable token.' }
    }

    $phase = 'valid-token'
    $valid = Invoke-Fetch -WithToken
    Assert-LocalState
    if ($valid.exit -ne 0 -or $valid.token_requests -lt 1) {
        Stop-Check "Baseline did not authenticate ($($valid.failure_code)). Keep the test token valid; check its repository permission/approval and connectivity."
    }
    $remoteRefs = Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/remotes/origin/')
    if (-not $remoteRefs) { Stop-Check 'The private repository needs at least one commit on a branch. Keep the token valid and rerun after adding a README.' }
    foreach ($ref in ($remoteRefs -split "`n")) {
        $oid = ($ref -split ' ', 2)[1].Trim()
        if ((Invoke-Git @('cat-file', '-t', $oid)) -cne 'commit') { Stop-Check 'Fetched branch tip is not a commit.' }
    }
    Write-Host 'PASS valid disposable token fetched private branches; local edits preserved'

    $phase = 'awaiting-revocation'
    if ($SelfTest) {
        if ($SelfTestScenario -eq 'Revoked') { [IO.File]::WriteAllText((Join-Path $scratch 'revoked'), 'revoked') }
        elseif ($SelfTestScenario -eq 'NetworkFailure') { $server.Kill(); $server.WaitForExit() }
    } else {
        Write-Host ''
        Write-Host 'NOW delete ONLY the new disposable test token on GitHub:'
        Write-Host 'https://github.com/settings/personal-access-tokens'
        Write-Host 'Leave your regular tokens, OAuth applications, and SSH keys alone. Keep this terminal open.'
        $answer = Read-Host 'After deleting that test token, type REVOKED (anything else stops)'
        if ($answer -cne 'REVOKED') { Stop-Check 'Stopped before the revoked-token attempt. If still valid, delete the disposable token when finished.' }
    }
    $phase = 'revoked-token'
    $revokedResult = Invoke-Fetch -WithToken
    Assert-LocalState
    if ((Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/remotes/origin/')) -cne $remoteRefs) {
        Stop-Check 'Remote tracking refs changed during the failed attempt.'
    }
    if ($revokedResult.exit -eq 0) { Stop-Check 'The same token still authenticated. Verify you deleted the disposable token; this is not a revocation pass.' }
    if ($revokedResult.token_requests -lt 1 -or $revokedResult.failure_code -notin @('authentication_rejected', 'access_denied', 'repository_unavailable')) {
        Stop-Check "Failure was $($revokedResult.failure_code), without a confirmed token rejection. Network/TLS/SSO/helper failures are inconclusive."
    }
    $phase = 'diagnostics'
    $logPath = Join-Path $logs 'rocket.jsonl'
    $logText = [IO.File]::ReadAllText($logPath)
    Assert-NoSecret $logText
    foreach ($private in @('ROCKET_PRIVATE', 'rocket@example.invalid', $script:remoteUri.Host, $script:remoteUri.AbsolutePath)) {
        if ($logText.Contains($private)) { Stop-Check 'Private fixture or remote metadata appeared in diagnostics.' }
    }
    $records = @(Get-Content -LiteralPath $logPath | ForEach-Object { $_ | ConvertFrom-Json })
    if (@($records | Where-Object { $_.dropped_before -ne 0 }).Count -ne 0) { Stop-Check 'Diagnostic records were dropped.' }
    foreach ($expected in @(@('write_result', 'ok', 1), @('write_result', 'git_error', 2), @('refresh', 'ok', 6))) {
        if (@($records | Where-Object { $_.event -eq $expected[0] -and $_.outcome -eq $expected[1] }).Count -ne $expected[2]) { Stop-Check 'Unexpected diagnostic outcome/refresh counts.' }
    }
    foreach ($start in @($records | Where-Object { $_.event -in @('step_start', 'git_start', 'request_start') })) {
        $endEvent = $start.event.Replace('_start', '_end')
        if (@($records | Where-Object { $_.session -eq $start.session -and $_.id -eq $start.id -and $_.event -eq $endEvent }).Count -ne 1) { Stop-Check 'Missing or duplicate diagnostic span completion.' }
    }
    if (@($records | Where-Object { $_.event -eq 'write_result' -and $_.failure_code -eq $revokedResult.failure_code }).Count -lt 1) { Stop-Check 'The rejection category was not logged.' }
    foreach ($file in @((Join-Path $repository '.git\config'), $marker, $emptyConfig, $denyAskpass)) {
        Assert-NoSecret ([IO.File]::ReadAllText($file))
    }
    $evidence.revoked_failure_code = $revokedResult.failure_code
    $evidence.diagnostic_records = $records.Count
    $evidence.local_state_preserved = $true
    $evidence.remote_refs_preserved_on_rejection = $true
    $evidence.passed = $true
    $phase = 'complete'
    Write-Host "PASS same token rejected [$($revokedResult.failure_code)]; state, refreshes, and log privacy verified"
    if ($SelfTest) { Write-Host 'Offline simulation passed. Real GitHub revocation is still a separate manual test.' }
    else { Write-Host 'GitHub disposable-token revocation checks passed.' }
} catch {
    # Do not serialize arbitrary exceptions or raw child output from a secret test.
    $evidence.stopped_at = $phase
    $evidence.failure = $failureHint
    throw "Revocation check stopped during $phase. $failureHint Results: $resultPath"
} finally {
    $token = $null
    if ($null -ne $secureToken) { $secureToken.Dispose() }
    if ($null -ne $server) {
        try {
            [IO.File]::WriteAllText((Join-Path $scratch 'stop'), 'stop')
            if ($serverStarted -and -not $server.WaitForExit(2000)) {
                $server.Kill()
                if (-not $server.WaitForExit(5000)) { throw 'Offline server cleanup did not complete.' }
            }
        } finally { $server.Dispose() }
    }
    $evidence.finished_utc = [DateTime]::UtcNow.ToString('o')
    $evidence.runs = @($runs.ToArray())
    $evidence | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $resultPath -Encoding UTF8
    Write-Host "Results: $resultPath"
    Write-Host "Diagnostics: $logs"
}
