$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) { throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect' }
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-auth-failures-' + [guid]::NewGuid().ToString('N'))
$repository = Join-Path $scratch 'ROCKET_PRIVATE_client'
$logs = Join-Path $scratch 'logs'
$emptyConfig = Join-Path $scratch 'empty.config'
$hooks = Join-Path $scratch 'empty-hooks'
$helper = Join-Path $scratch 'ssh-fixture'
$diagnostic = Join-Path $scratch 'diagnostic'
$calls = Join-Path $scratch 'calls'
$transcript = Join-Path $scratch 'cli-output.log'
$resultPath = Join-Path $scratch 'result.json'
$null = [IO.Directory]::CreateDirectory($repository)
$null = [IO.Directory]::CreateDirectory($hooks)
[IO.File]::WriteAllText($emptyConfig, '')

function Quote-Shell([string]$Value) {
    $q = [string][char]39
    return $q + $Value.Replace('\', '/').Replace($q, ($q + [char]34 + $q + [char]34 + $q)) + $q
}
# This controlled transport only emits fixture diagnostics. It cannot connect to
# a server, load a key, or invoke a credential helper. .invalid is never resolved.
[IO.File]::WriteAllText($helper, "#!/bin/sh`nprintf 'call\n' >> $(Quote-Shell $calls)`ncat $(Quote-Shell $diagnostic) >&2`nexit 1`n", [Text.UTF8Encoding]::new($false))

function Invoke-Git([string[]]$GitArguments) {
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $gitPath -C $repository @GitArguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    if ($actual -ne 0) { throw "Fixture Git exited $actual" }
    return $output
}
function Invoke-Inspect([string[]]$InspectArguments, [int]$ExpectedExit) {
    $previousAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $inspector $repository @InspectArguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
    [IO.File]::AppendAllText($transcript, "COMMAND: $($InspectArguments -join ' ')`n$text`n")
    if ($actual -ne $ExpectedExit) { throw "Unexpected CLI exit $actual; expected $ExpectedExit. See $transcript" }
    return $text
}
$cases = @(
    @{ code = 'ssh_host_key_changed'; diagnostic = 'WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!' },
    @{ code = 'ssh_host_verification_failed'; diagnostic = 'Host key verification failed.' },
    @{ code = 'ssh_authentication_rejected'; diagnostic = 'git@auth.invalid: Permission denied (publickey).' },
    @{ code = 'ssh_key_unavailable'; diagnostic = 'Warning: Identity file ROCKET_PRIVATE not accessible: No such file or directory.' },
    @{ code = 'ssh_key_unlock_failed'; diagnostic = 'Load key "ROCKET_PRIVATE": incorrect passphrase supplied to decrypt private key' },
    @{ code = 'ssh_agent_failure'; diagnostic = 'sign_and_send_pubkey: signing failed: agent refused operation' },
    @{ code = 'credentials_unavailable'; diagnostic = "fatal: could not read Username for 'https://ROCKET_PRIVATE': terminal prompts disabled" },
    @{ code = 'authentication_rejected'; diagnostic = "fatal: Authentication failed for 'https://ROCKET_PRIVATE'" },
    @{ code = 'sso_required'; diagnostic = 'remote: The organization has enabled or enforced SAML SSO.' },
    @{ code = 'access_denied'; diagnostic = 'fatal: The requested URL returned error: 403' },
    @{ code = 'repository_unavailable'; diagnostic = 'remote: Repository not found.' },
    @{ code = 'proxy_authentication_required'; diagnostic = 'fatal: CONNECT tunnel failed, response 407' },
    @{ code = 'tls_verification_failed'; diagnostic = 'fatal: SSL certificate problem: self-signed certificate' },
    @{ code = 'name_resolution_failed'; diagnostic = 'ssh: Could not resolve hostname ROCKET_PRIVATE: No such host' },
    @{ code = 'connection_failed'; diagnostic = 'ssh: connect to host ROCKET_PRIVATE port 22: Connection refused' },
    @{ code = 'connection_timed_out'; diagnostic = 'ssh: connect to host ROCKET_PRIVATE port 22: Connection timed out' },
    @{ code = 'unclassified_fetch_failure'; diagnostic = 'unrecognized ROCKET_PRIVATE diagnostic' }
)
$evidence = [ordered]@{ passed = $false; network = $false; cases = @(); started_utc = [DateTime]::UtcNow.ToString('o') }
$names = @('ROCKET_LOG', 'ROCKET_LOG_DIR', 'GIT_CONFIG_GLOBAL', 'GIT_CONFIG_SYSTEM', 'GIT_CONFIG_NOSYSTEM', 'GIT_TEMPLATE_DIR', 'GIT_TERMINAL_PROMPT', 'GIT_ALLOW_PROTOCOL')
$names += @(Get-ChildItem Env: | Where-Object { $_.Name -like 'GIT_*' -or $_.Name -like 'SSH_*' } | ForEach-Object { $_.Name })
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
    $env:GIT_ALLOW_PROTOCOL = 'ssh'
    Invoke-Git @('init', '--initial-branch=main') | Out-Null
    Invoke-Git @('config', 'user.name', 'Rocket Auth Fixture') | Out-Null
    Invoke-Git @('config', 'user.email', 'rocket@example.invalid') | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) | Out-Null
    Invoke-Git @('config', 'core.autocrlf', 'false') | Out-Null
    Invoke-Git @('config', 'commit.gpgsign', 'false') | Out-Null
    Invoke-Git @('config', 'core.sshCommand', (Quote-Shell $helper)) | Out-Null
    Invoke-Git @('config', 'ssh.variant', 'ssh') | Out-Null
    $tracked = Join-Path $repository 'file'
    $untracked = Join-Path $repository 'untracked'
    [IO.File]::WriteAllText($tracked, "base`n")
    Invoke-Git @('add', 'file') | Out-Null
    Invoke-Git @('commit', '-m', 'Local authentication fixture') | Out-Null
    [IO.File]::WriteAllText($tracked, "staged`n")
    Invoke-Git @('add', 'file') | Out-Null
    [IO.File]::WriteAllText($tracked, "unstaged`n")
    [IO.File]::WriteAllText($untracked, "ROCKET_PRIVATE contents`n")
    Invoke-Git @('remote', 'add', 'origin', 'git@auth.invalid:ROCKET_PRIVATE/repo.git') | Out-Null
    $beforeHead = Invoke-Git @('rev-parse', 'HEAD')
    $beforeRefs = Invoke-Git @('show-ref') | Out-String
    $index = Join-Path $repository '.git\index'
    $beforeFiles = @($index, $tracked, $untracked) | ForEach-Object { @{ path = $_; hash = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash } }
    foreach ($case in $cases) {
        [IO.File]::WriteAllText($diagnostic, "$($case.diagnostic)`nAuthorization: Bearer ROCKET_SECRET_CANARY`n", [Text.UTF8Encoding]::new($false))
        $output = Invoke-Inspect @('fetch', 'origin') 1
        if (-not $output.Contains("[$($case.code)]") -or -not $output.Contains('Next: ')) { throw "Missing failure code/guidance: $($case.code)" }
        if ($output.Contains('ROCKET_SECRET_CANARY') -or $output.Contains('Authorization:')) { throw 'Raw credential-like output escaped into the CLI' }
        if (-not $output.Contains('Branches after fetch:') -or -not $output.Contains('Status after fetch:') -or $output.Contains('refresh failed:')) { throw 'Reconciliation failed' }
        if ((Invoke-Git @('rev-parse', 'HEAD')) -cne $beforeHead -or (Invoke-Git @('show-ref') | Out-String) -cne $beforeRefs) { throw 'Local refs changed' }
        foreach ($file in $beforeFiles) { if ((Get-FileHash -LiteralPath $file.path -Algorithm SHA256).Hash -cne $file.hash) { throw 'Index or file bytes changed' } }
        $evidence.cases += $case.code
        Write-Host "PASS $($case.code): guidance, refresh, local state, and CLI privacy"
    }
    $null = Invoke-Inspect @('fetch', 'missing') 1
    $null = Invoke-Inspect @('status') 0
    [IO.File]::WriteAllText($diagnostic, "ROCKET_SECRET_CANARY$([char]27)[31m`n", [Text.UTF8Encoding]::new($false))
    $rawOutput = Invoke-Inspect @('fetch', 'origin', '--raw-error') 1
    if (-not $rawOutput.Contains('ROCKET_SECRET_CANARY') -or -not $rawOutput.Contains('Raw Git output') -or $rawOutput.Contains([string][char]27)) { throw 'Explicit raw error inspection or terminal escaping failed' }
    if (@([IO.File]::ReadAllLines($calls)).Count -ne ($cases.Count + 1)) { throw 'Unexpected transport invocation count or retry' }
    $logText = [IO.File]::ReadAllText((Join-Path $logs 'rocket.jsonl'))
    foreach ($private in @('ROCKET_PRIVATE', 'ROCKET_SECRET_CANARY', 'auth.invalid', 'Authorization:', 'rocket@example.invalid', $beforeHead)) {
        if ($logText.Contains($private)) { throw 'Private fixture value in diagnostics' }
    }
    $records = @(Get-Content -LiteralPath (Join-Path $logs 'rocket.jsonl') | ForEach-Object { $_ | ConvertFrom-Json -ErrorAction Stop })
    foreach ($start in @($records | Where-Object { $_.event -in @('request_start', 'step_start', 'git_start') })) {
        $endEvent = $start.event.Replace('_start', '_end')
        $ends = @($records | Where-Object { $_.session -eq $start.session -and $_.id -eq $start.id -and $_.event -eq $endEvent })
        if ($ends.Count -ne 1 -or $ends[0].request -ne $start.request -or $ends[0].seq -le $start.seq) { throw 'Broken request/step/command correlation' }
    }
    foreach ($case in $cases) {
        foreach ($event in @('write_result', 'request_end')) {
            $expected = if ($case.code -eq 'unclassified_fetch_failure') { 2 } else { 1 }
            if (@($records | Where-Object { $_.event -eq $event -and $_.failure_code -eq $case.code -and $_.outcome -eq 'git_error' }).Count -ne $expected) { throw "Missing categorized $event for $($case.code)" }
        }
    }
    if (@($records | Where-Object { $_.dropped_before -ne 0 }).Count -ne 0) { throw 'Dropped diagnostic events' }
    $evidence['diagnostic_records'] = $records.Count
    $evidence['dropped_diagnostic_records'] = 0
    $evidence['passed'] = $true
    Write-Output "Authentication failure smoke checks passed. Results: $resultPath"
    Write-Output "Diagnostics: $logs"
} catch {
    $evidence['failure'] = $_.Exception.Message
    throw
} finally {
    $evidence['finished_utc'] = [DateTime]::UtcNow.ToString('o')
    try { $evidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $resultPath -Encoding UTF8 }
    finally {
        foreach ($name in $names) {
            if ($null -eq $saved[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
            else { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
        }
    }
}
