param(
    [ValidateSet('Https', 'Ssh')][string]$Transport = 'Https',
    [string]$Repository = 'JasXTD/rocket-auth-test',
    [string]$PublicKeyPath = 'C:\Users\Tommy\.ssh\id_ed25519_github.pub',
    [switch]$SelfTest
)

$ErrorActionPreference = 'Stop'
if ($Repository -cnotmatch '\A[A-Za-z0-9-]+/[A-Za-z0-9_.-]+\z' -or $Repository.EndsWith('.git')) {
    throw 'Use OWNER/REPOSITORY without a URL or .git suffix.'
}
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) { throw 'Build the inspect example first, offline and locked.' }
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$runId = [guid]::NewGuid().ToString('N')
$mode = if ($SelfTest) { 'local' } else { $Transport.ToLowerInvariant() }
$scratch = Join-Path (Join-Path $workspace 'work') "smoke-github-pull-$mode-$runId"
$client = Join-Path $scratch 'ROCKET_PRIVATE_client'
$publisher = Join-Path $scratch 'ROCKET_PRIVATE_publisher'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'empty-hooks'
$emptyHome = Join-Path $scratch 'empty-home'
$emptyConfig = Join-Path $scratch 'empty.config'
$denyAskpass = Join-Path $scratch 'deny-askpass.cmd'
$transcript = Join-Path $scratch 'cli-output.log'
$resultPath = Join-Path $scratch 'result.json'
$sshConfig = Join-Path $scratch 'ssh.config'
$knownHosts = Join-Path $scratch 'known_hosts'
$publicIdentity = Join-Path $scratch 'ROCKET_PRIVATE_identity.pub'
$branch = "rocket-pull-test/$mode-$runId"
$reference = "refs/heads/$branch"
$tracking = "refs/remotes/origin/$branch"
$remote = if ($SelfTest) { Join-Path $scratch 'upstream.git' } elseif ($Transport -eq 'Https') { "https://github.com/$Repository.git" } else { "git@github.com:$Repository.git" }
$utf8 = [Text.UTF8Encoding]::new($false)
foreach ($path in @($client, $publisher, $hooks, $emptyHome)) { [IO.Directory]::CreateDirectory($path) | Out-Null }
[IO.File]::WriteAllText($emptyConfig, '', $utf8)
[IO.File]::WriteAllText($denyAskpass, "@exit /b 1`r`n", [Text.Encoding]::ASCII)
$phase = 'setup'
$failureHint = 'Check the fixture tools and configuration.'
$remoteWriteAttempted = $false
$cleanupAllowed = $false
$testsPassed = $false
$cleanupVerified = $false
$ownedCommits = New-Object 'System.Collections.Generic.List[string]'
$runs = New-Object 'System.Collections.Generic.List[object]'
$steps = New-Object 'System.Collections.Generic.List[object]'
$evidence = [ordered]@{
    passed=$false; mode=$mode; remote=$remote; branch=$reference
    started_utc=[DateTime]::UtcNow.ToString('o'); cleanup='not-needed'
    inspector_sha256=(Get-FileHash -LiteralPath $inspector -Algorithm SHA256).Hash
}
# Rechecked against GitHub's published Ed25519 key on 2026-10-15.
$hostKeySource = 'https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints'
$hostKeyEntry = 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl'
$hostFingerprint = 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU'
$sshDir = Join-Path $env:WINDIR 'System32\OpenSSH'
$sshPath = Join-Path $sshDir 'ssh.exe'
$keygenPath = Join-Path $sshDir 'ssh-keygen.exe'
$sshAddPath = Join-Path $sshDir 'ssh-add.exe'

function Stop-Check([string]$Message) { $script:failureHint = $Message; throw 'Pull smoke check failed' }

# These process/authentication helpers follow the existing GitHub push runner.
# Only Git receives helper credentials; neither this script nor its evidence
# reads credential-helper output or private-key contents.
function Quote-WindowsArgument([string]$Value) {
    '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}

function Quote-ShellArgument([string]$Value) {
    $quote = [string][char]39
    $escaped = $quote + [char]34 + $quote + [char]34 + $quote
    $quote + $Value.Replace($quote, $escaped) + $quote
}

function Quote-SshPath([string]$Value) {
    if ($Value -match '[\r\n"]' -or $Value.Contains('${')) { Stop-Check 'Unsupported SSH configuration path.' }
    [string][char]34 + $Value.Replace('\', '/').Replace('%', '%%') + [char]34
}

function Invoke-Isolated([string]$Executable, [string[]]$Arguments, [string]$Label) {
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $Executable
    $start.Arguments = ($Arguments | ForEach-Object { Quote-WindowsArgument $_ }) -join ' '
    $start.WorkingDirectory = $scratch
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.RedirectStandardInput = $true
    $start.StandardOutputEncoding = $utf8
    $start.StandardErrorEncoding = $utf8
    foreach ($key in @($start.EnvironmentVariables.Keys)) {
        if ($key -match '^(GIT_|GCM_|ROCKET_|SSH_|CURL_|SSL_CERT_)|_PROXY$|^NETRC$') { $start.EnvironmentVariables.Remove($key) }
    }
    # Keep Windows profile/store access for GCM; isolate Git config, curl home,
    # traces, prompts, and transport overrides. The parent environment is untouched.
    $values = @{
        HOME = $emptyHome; GIT_CONFIG_GLOBAL = $emptyConfig; GIT_CONFIG_SYSTEM = $emptyConfig
        GIT_CONFIG_NOSYSTEM = '1'; GIT_TEMPLATE_DIR = $hooks; GIT_TERMINAL_PROMPT = '0'
        GIT_ASKPASS = $denyAskpass; SSH_ASKPASS = $denyAskpass; SSH_ASKPASS_REQUIRE = 'never'
        GIT_ALLOW_PROTOCOL = $(if ($SelfTest) { 'file' } else { $Transport.ToLowerInvariant() })
        GIT_NO_LAZY_FETCH = '1'; GIT_NO_REPLACE_OBJECTS = '1'; GIT_OPTIONAL_LOCKS = '0'
        GCM_INTERACTIVE = '0'; GCM_TRACE = '0'; GCM_TRACE_SECRETS = '0'; GCM_DEBUG = '0'
        ROCKET_LOG_DIR = $logs; LC_ALL = 'C'; GIT_SSH_VARIANT = 'ssh'
    }
    foreach ($key in $values.Keys) { $start.EnvironmentVariables[$key] = $values[$key] }
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $start
    $started = $false
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        [void]$process.Start(); $started = $true
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.StandardInput.Close()
        if (-not $process.WaitForExit(90000)) { Stop-Check 'Child exceeded the harness time limit; remote state must be verified before another write.' }
        $steps.Add([ordered]@{ phase = $phase; tool = $Label; exit = $process.ExitCode; elapsed_ms = $watch.ElapsedMilliseconds })
        return [pscustomobject]@{ exit = $process.ExitCode; stdout = $stdout.Result; stderr = $stderr.Result; ms = $watch.ElapsedMilliseconds }
    } finally {
        try {
            if ($started -and -not $process.HasExited) {
                # This PID belongs to the live child whose handle we still own.
                $kill = Join-Path $env:WINDIR 'System32\taskkill.exe'
                & $kill /PID $process.Id /T /F 2>&1 | Out-Null
                if (-not $process.WaitForExit(5000)) { Stop-Check 'Owned child did not terminate; inspect the fixture before retrying.' }
            }
        } finally { $process.Dispose() }
    }
}

function Invoke-Git([string[]]$GitArguments, [string]$At = $client) {
    $output = Invoke-Isolated $gitPath (@('-C', $At, '-c', 'gc.auto=0', '-c', 'maintenance.auto=false') + $GitArguments) 'git'
    if ($output.exit -ne 0) { Stop-Check "Fixture Git failed during $phase (exit $($output.exit)); raw transport output was not saved." }
    $output.stdout.TrimEnd("`r", "`n")
}

function Set-Authentication([bool]$Enabled, [bool]$TrustHost = $true) {
    if ($SelfTest) { return }
    if ($Transport -eq 'Https') {
        if ($Enabled) {
            # Only Git invokes this helper. Credentials pass internally from GCM
            # to Git; do not run credential get/fill directly or capture its output.
            # Ignore store/erase so a rejected test cannot tell GCM to delete login.
            $helper = '!f() { if test "$1" = get; then ' + (Quote-ShellArgument $gitPath.Replace('\', '/')) + ' credential-manager get; else cat >/dev/null; fi; }; f'
        } else { $helper = '' }
        foreach ($directory in @($client, $publisher)) {
            Invoke-Git @('config', 'credential.helper', $helper) -At $directory | Out-Null
        }
        return
    }
    $configuration = @"
Host github.com
    HostName github.com
    User git
    Port 22
    BatchMode yes
    ConnectTimeout 10
    ConnectionAttempts 1
    StrictHostKeyChecking yes
    HostKeyAlgorithms ssh-ed25519
    UserKnownHostsFile $(Quote-SshPath $knownHosts)
    GlobalKnownHostsFile none
    UpdateHostKeys no
    VerifyHostKeyDNS no
    PreferredAuthentications publickey
    PasswordAuthentication no
    KbdInteractiveAuthentication no
    IdentitiesOnly yes
    AddKeysToAgent no
    ForwardAgent no
    ControlMaster no
    ControlPath none
"@
    if ($Enabled) { $configuration += "`n    IdentityFile $(Quote-SshPath $publicIdentity)`n" }
    else { $configuration += "`n    IdentityAgent none`n    IdentityFile none`n" }
    [IO.File]::WriteAllText($sshConfig, $configuration, $utf8)
    [IO.File]::WriteAllText($knownHosts, $(if ($TrustHost) { "$hostKeyEntry`n" } else { '' }), $utf8)
}

function Initialize-Ssh {
    $script:failureHint = 'Load id_ed25519_github into the Windows SSH agent locally, then retry with a fresh fixture.'
    if (-not $PublicKeyPath.EndsWith('.pub', [StringComparison]::OrdinalIgnoreCase)) { Stop-Check 'Select the .pub file; never pass the private key.' }
    $script:resolvedPublicKey = (Resolve-Path -LiteralPath $PublicKeyPath -ErrorAction Stop).Path
    $publicText = [IO.File]::ReadAllText($resolvedPublicKey).Trim()
    if ($publicText -cnotmatch '\Assh-ed25519 [A-Za-z0-9+/]+={0,3}(?: [^\r\n]*)?\z') { Stop-Check 'The selected file must contain one Ed25519 public key.' }
    [IO.File]::WriteAllText($publicIdentity, "$publicText`n", $utf8)
    $script:publicHashBefore = (Get-FileHash -LiteralPath $resolvedPublicKey -Algorithm SHA256).Hash
    $fingerprintResult = Invoke-Isolated $keygenPath @('-lf', $publicIdentity) 'public-key-fingerprint'
    if ($fingerprintResult.exit -ne 0 -or $fingerprintResult.stdout -notmatch '\bSHA256:[A-Za-z0-9+/]+') { Stop-Check 'Cannot validate the public-key fingerprint.' }
    $fingerprint = $Matches[0]
    $agent = Invoke-Isolated $sshAddPath @('-l') 'agent-list'
    if ($agent.exit -ne 0 -or -not $agent.stdout.Contains($fingerprint)) { Stop-Check $failureHint }
    $evidence.selected_agent_key_available = $true
    foreach ($directory in @($client, $publisher)) {
        Invoke-Git @('config', 'core.sshCommand', ((Quote-ShellArgument $sshPath.Replace('\', '/')) + ' -F ' + (Quote-ShellArgument $sshConfig.Replace('\', '/')))) -At $directory | Out-Null
    }
}

function Get-RemoteState {
    $text = Invoke-Git @('ls-remote', '--refs', '--', 'origin')
    [string[]]$refs = @($text -split "`n" | Where-Object { $_ -ne '' })
    foreach ($line in $refs) {
        if ($line -cnotmatch '\A(?:[a-f0-9]{40}|[a-f0-9]{64})\trefs/\S+\z') { Stop-Check 'Unexpected remote ref output.' }
    }
    [Array]::Sort($refs, [StringComparer]::Ordinal)
    [pscustomobject]@{ refs = $refs; head = (Invoke-Git @('ls-remote', '--symref', '--', 'origin', 'HEAD')) }
}

function Get-TestTip($State) {
    $match = @($State.refs | Where-Object { $_.EndsWith("`t$reference", [StringComparison]::Ordinal) })
    if ($match.Count -gt 1) { Stop-Check 'Duplicate test ref returned by remote.' }
    if ($match.Count -eq 1) { return ($match[0] -split "`t", 2)[0] }
    return ''
}

function Assert-Remote($State, [string]$ExpectedTip) {
    if ((Get-TestTip $State) -cne $ExpectedTip) { Stop-Check 'Remote test branch does not match the expected commit.' }
    $other = @($State.refs | Where-Object { -not $_.EndsWith("`t$reference", [StringComparison]::Ordinal) }) -join "`n"
    if ($other -cne ($script:beforeRemote.refs -join "`n") -or $State.head -cne $script:beforeRemote.head) {
        Stop-Check 'Other remote refs or default HEAD changed; this test will not modify them.'
    }
}

function Save-Checkpoint {
    $evidence.phase = $phase
    $evidence.owned_commits = $ownedCommits.ToArray()
    $evidence.pull_attempts = $runs.ToArray()
    $evidence.harness_steps = $steps.ToArray()
    [IO.File]::WriteAllText($resultPath, ($evidence | ConvertTo-Json -Depth 8), $utf8)
}
function Get-LocalState {
    [ordered]@{
        head = Invoke-Git @('rev-parse', 'HEAD')
        refs = Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/heads/', 'refs/tags/')
        index = (Get-FileHash -LiteralPath (Join-Path $client '.git\index') -Algorithm SHA256).Hash
        files = @(Get-ChildItem -LiteralPath $client -File | Sort-Object Name | ForEach-Object { [ordered]@{ name=$_.Name; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash } })
    } | ConvertTo-Json -Depth 4 -Compress
}
function Assert-Local([string]$Expected) {
    if ((Get-LocalState) -cne $Expected) { Stop-Check 'Local HEAD, branches/tags, index, or file bytes changed unexpectedly.' }
}
function Assert-Tip([string]$ExpectedHead, [string]$ExpectedTracking, [string]$ExpectedCounts) {
    if ((Invoke-Git @('rev-parse', 'HEAD')) -cne $ExpectedHead -or (Invoke-Git @('rev-parse', $tracking)) -cne $ExpectedTracking) {
        Stop-Check 'The local HEAD or selected tracking branch has an unexpected commit.'
    }
    $counts = (Invoke-Git @('rev-list', '--left-right', '--count', "HEAD...$tracking")) -replace '\s+', ' '
    if ($counts -cne $ExpectedCounts) { Stop-Check 'The independent ahead/behind count is incorrect.' }
}
function Invoke-Pull([string]$ExpectedKind, [string]$FailurePattern = '', [switch]$FetchFailure, [switch]$Preflight) {
    Save-Checkpoint
    $output = Invoke-Isolated $inspector @($client, 'pull') 'rocket-pull'
    $text = $output.stdout + $output.stderr
    if ($text -match 'gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|(?im)Authorization:\s*(Basic|Bearer)\s+\S+|-----BEGIN [^-]*PRIVATE KEY-----') {
        Stop-Check 'Captured output matched a secret format and was not saved.'
    }
    [IO.File]::AppendAllText($transcript, "PHASE: $phase`n$text`n", $utf8)
    $failure = if ($text -match '\[([a-z_]+)\]') { $Matches[1] } else { $null }
    $runs.Add([ordered]@{
        phase=$phase; exit=$output.exit; elapsed_ms=$output.ms; failure_code=$failure
        expected_kind=$ExpectedKind; preflight=[bool]$Preflight; fetch_failure=[bool]$FetchFailure
    })
    if ($FailurePattern) {
        if ($output.exit -ne 1 -or $failure -cnotmatch "\A($FailurePattern)\z") {
            Stop-Check "Expected $FailurePattern; got exit $($output.exit) and code $failure. No automatic retry."
        }
        if (-not $text.Contains('Next: ')) { Stop-Check 'The rejected operation omitted recovery guidance.' }
    } elseif ($output.exit -ne 0 -or -not $text.Contains("Pull branch update: $ExpectedKind (Git command succeeded).")) {
        Stop-Check "Pull did not confirm $ExpectedKind (exit $($output.exit), code $failure)."
    }
    if ($Preflight) {
        if ($text.Contains('Pull fetch:')) { Stop-Check 'A preflight rejection unexpectedly reached fetch.' }
    } else {
        foreach ($label in @('Status after pull:', 'Branches after pull:', 'Operation state after pull:')) {
            if (-not $text.Contains($label)) { Stop-Check 'Pull did not show every independent refresh result.' }
        }
        if ($text -match 'refresh failed:|parsing its updates failed:|verification failed:|Git emitted error diagnostics despite') {
            Stop-Check 'Pull parsing, verification, refresh, or helper diagnostics need investigation.'
        }
        if ($FetchFailure) {
            if (-not $text.Contains('Pull fetch: failed or interrupted; branch update was not attempted.') -or -not $text.Contains('Pull branch update: not attempted.')) {
                Stop-Check 'An authentication failure was not shown as a failed fetch with skipped update.'
            }
        } elseif (-not $text.Contains('Pull fetch: Git command succeeded.')) {
            Stop-Check 'The successful fetch was hidden by the later branch-update result.'
        }
    }
    Write-Host "PASS $mode $phase (exit $($output.exit))"
}
function Invoke-RemoteWrite([string]$NewTip, [string]$ExpectedTip) {
    if (-not $cleanupAllowed -or $reference -cne "refs/heads/rocket-pull-test/$mode-$runId" -or ($NewTip -and -not $ownedCommits.Contains($NewTip)) -or ($ExpectedTip -and -not $ownedCommits.Contains($ExpectedTip))) {
        Stop-Check 'Fixture write refused: the branch or expected commit is not owned by this run.'
    }
    # Setup/teardown use native Git, with an exact expected-commit lease even for
    # first creation (empty expected tip). Rocket's pull command is the subject.
    $script:remoteWriteAttempted = $true
    Save-Checkpoint
    Invoke-Isolated $gitPath @('-C', $publisher, '-c', 'remote.origin.mirror=false',
        '-c', 'push.followTags=false', '-c', 'push.autoSetupRemote=false',
        '-c', 'push.negotiate=false', '-c', 'push.pushOption=',
        'push', '--porcelain', '--no-follow-tags', '--no-prune', '--no-recurse-submodules',
        "--force-with-lease=${reference}:$ExpectedTip", '--', 'origin', "${NewTip}:$reference") 'git-fixture-write'
}
function Publish([string]$Contents, [string]$ExpectedTip) {
    [IO.File]::WriteAllText((Join-Path $publisher 'ROCKET_PRIVATE_file'), "$Contents`n", $utf8)
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') -At $publisher | Out-Null
    Invoke-Git @('commit', '-m', 'Rocket synthetic pull transport advance') -At $publisher | Out-Null
    $tip = Invoke-Git @('rev-parse', 'HEAD') -At $publisher
    $ownedCommits.Add($tip)
    Invoke-Git @('merge-base', '--is-ancestor', $ExpectedTip, $tip) -At $publisher | Out-Null
    $output = Invoke-RemoteWrite $tip $ExpectedTip
    if ($output.exit -ne 0) { Stop-Check 'Fixture publish did not confirm success; inspect remote state before another write.' }
    Assert-Remote (Get-RemoteState) $tip
    return $tip
}
function Assert-Diagnostics {
    $records = @()
    $diagnostics = ''
    foreach ($file in @(Get-ChildItem -LiteralPath $logs -Filter '*.jsonl' -File | Sort-Object LastWriteTimeUtc, Name)) {
        $diagnostics += [IO.File]::ReadAllText($file.FullName)
        foreach ($line in [IO.File]::ReadAllLines($file.FullName)) { $records += ($line | ConvertFrom-Json -ErrorAction Stop) }
    }
    if ($records.Count -eq 0) { Stop-Check 'No diagnostic records were produced.' }
    if ($diagnostics -match 'ROCKET_PRIVATE_|rocket-pull-test|refs/heads/|refs/remotes/|github\.com|gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|(?im)Authorization:\s*(Basic|Bearer)\s+\S+|-----BEGIN [^-]*PRIVATE KEY-----') {
        Stop-Check 'Diagnostic privacy scan found repository or credential-shaped data.'
    }
    foreach ($value in @($Repository, $scratch, $hostFingerprint) + $ownedCommits.ToArray()) {
        if ($diagnostics.Contains($value)) { Stop-Check 'Diagnostic logs contain private fixture metadata.' }
    }
    if (@($records | Where-Object { $_.schema -ne 1 -or $_.dropped_before -ne 0 }).Count) { Stop-Check 'Invalid schema or dropped diagnostic events.' }
    foreach ($start in @($records | Where-Object { $_.event -in @('request_start', 'step_start', 'git_start') })) {
        $ends = @($records | Where-Object { $_.event -ceq $start.event.Replace('_start', '_end') -and $_.session -ceq $start.session -and $_.id -eq $start.id })
        if ($ends.Count -ne 1) { Stop-Check 'An incomplete or duplicated diagnostic span was found.' }
    }
    $sessions = @($records | Where-Object { $_.event -ceq 'request_start' -and $_.operation -ceq 'pull' })
    if ($sessions.Count -ne $runs.Count) { Stop-Check 'Diagnostic request count does not match pull attempts.' }
    for ($i = 0; $i -lt $sessions.Count; $i++) {
        $recordsForRun = @($records | Where-Object { $_.session -ceq $sessions[$i].session })
        $run = $runs[$i]
        $fetchCount = if ($run.preflight) { 0 } else { 1 }
        $mergeCount = if ($run.expected_kind) { 1 } else { 0 }
        if (@($recordsForRun | Where-Object { $_.event -ceq 'git_start' -and $_.operation -ceq 'fetch' }).Count -ne $fetchCount -or @($recordsForRun | Where-Object { $_.event -ceq 'git_start' -and $_.operation -ceq 'merge' }).Count -ne $mergeCount) {
            Stop-Check 'Fetch/update command counts indicate an omitted phase or unintended retry.'
        }
        $outcomes = @($recordsForRun | Where-Object { $_.event -ceq 'pull_outcome' })
        if ($run.preflight) {
            if ($outcomes.Count -ne 0 -or @($recordsForRun | Where-Object { $_.event -ceq 'refresh' }).Count -ne 0) { Stop-Check 'A preflight rejection unexpectedly entered mutation reconciliation.' }
        } else {
            $fetchOutcome = if ($run.fetch_failure) { 'failed' } else { 'succeeded' }
            $updateOutcome = if ($run.fetch_failure) { 'skipped' } elseif ($run.expected_kind) { 'succeeded' } else { 'failed' }
            $kind = if ($run.expected_kind -ceq 'FastForward') { 'fast_forward' } elseif ($run.expected_kind -ceq 'UpToDate') { 'up_to_date' } else { 'none' }
            $verification = if ($run.expected_kind) { 'ok' } else { 'not_available' }
            if ($outcomes.Count -ne 1 -or $outcomes[0].fetch_outcome -cne $fetchOutcome -or $outcomes[0].update_outcome -cne $updateOutcome -or $outcomes[0].update_kind -cne $kind -or $outcomes[0].verification -cne $verification) {
                Stop-Check 'CLI and diagnostic pull phase outcomes differ.'
            }
            foreach ($operation in @('status', 'branches', 'operation')) {
                if (@($recordsForRun | Where-Object { $_.event -ceq 'refresh' -and $_.operation -ceq $operation -and $_.outcome -ceq 'ok' }).Count -ne 1) { Stop-Check 'Missing successful reconciliation diagnostic.' }
            }
            if ($run.expected_kind -and $outcomes[0].error_diagnostics) { Stop-Check 'A successful update also reported unexpected Git error diagnostics.' }
        }
        if ($run.failure_code -and @($recordsForRun | Where-Object { $_.failure_code -ceq $run.failure_code }).Count -eq 0) { Stop-Check 'Missing classified failure diagnostic.' }
    }
    $evidence.diagnostic_records = $records.Count
    $evidence.diagnostic_sessions = $sessions.Count
    $evidence.diagnostic_dropped = 0
    $evidence.diagnostic_privacy_scan_passed = $true
    Write-Host 'PASS phase outcomes, paired spans, command counts, refreshes, failure codes, and diagnostic privacy'
}

try {
    foreach ($directory in @($client, $publisher)) {
        Invoke-Git @('init', '--initial-branch=main', '--object-format=sha1') -At $directory | Out-Null
        foreach ($setting in @(
            @('user.name', 'Rocket Pull Smoke'), @('user.email', 'rocket@example.invalid'),
            @('core.hooksPath', $hooks), @('core.autocrlf', 'false'), @('core.fsmonitor', 'false'),
            @('commit.gpgsign', 'false'), @('push.gpgsign', 'false'),
            @('credential.interactive', 'false'), @('credential.helper', ''),
            @('http.sslBackend', 'schannel'), @('http.sslVerify', 'true'),
            @('http.followRedirects', 'false'), @('http.extraHeader', '')
        )) { Invoke-Git @('config', $setting[0], $setting[1]) -At $directory | Out-Null }
        Invoke-Git @('remote', 'add', 'origin', $remote) -At $directory | Out-Null
        # Only the synthetic branch is downloaded. Existing private source is
        # never checked out, built, or executed by this acceptance test.
        Invoke-Git @('config', 'remote.origin.fetch', "+${reference}:$tracking") -At $directory | Out-Null
    }
    [IO.File]::WriteAllText((Join-Path $publisher 'ROCKET_PRIVATE_file'), "Rocket synthetic pull base $runId`n", $utf8)
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') -At $publisher | Out-Null
    Invoke-Git @('commit', '-m', 'Rocket synthetic pull transport base') -At $publisher | Out-Null
    $base = Invoke-Git @('rev-parse', 'HEAD') -At $publisher
    $ownedCommits.Add($base)
    if ($SelfTest) {
        Invoke-Git @('init', '--bare', '--initial-branch=main', '--object-format=sha1', $remote) | Out-Null
        Invoke-Git @('config', 'core.hooksPath', $hooks) -At $remote | Out-Null
        Invoke-Git @('push', '--', 'origin', 'refs/heads/main:refs/heads/main') -At $publisher | Out-Null
    } elseif ($Transport -eq 'Ssh') {
        $phase = 'ssh-agent-preflight'
        Initialize-Ssh
    }
    Set-Authentication $true
    if (-not $SelfTest -and $Transport -eq 'Ssh') {
        $pin = Invoke-Isolated $keygenPath @('-lf', $knownHosts) 'host-key-fingerprint'
        if ($pin.exit -ne 0 -or -not $pin.stdout.Contains($hostFingerprint)) { Stop-Check 'The fixture host key does not match the reviewed GitHub fingerprint.' }
        $evidence.host_key_source = $hostKeySource
    }
    $phase = 'remote-preflight'
    $failureHint = 'Read-only access failed. Check the existing GCM login or Windows SSH agent and test repository access locally.'
    $script:beforeRemote = Get-RemoteState
    if ($beforeRemote.refs.Count -eq 0 -or -not $beforeRemote.head) { Stop-Check 'Use an initialized test repository with an existing default branch.' }
    if (Get-TestTip $beforeRemote) { Stop-Check 'The generated test branch already exists; it will not be overwritten or deleted.' }
    $cleanupAllowed = $true
    $evidence.before_remote = $beforeRemote
    $phase = 'create-synthetic-branch'
    $created = Invoke-RemoteWrite $base ''
    if ($created.exit -ne 0) { Stop-Check 'Synthetic branch creation did not confirm success; cleanup will inspect its actual state.' }
    Assert-Remote (Get-RemoteState) $base
    $phase = 'bootstrap-client'
    Invoke-Git @('fetch', '--no-tags', '--no-prune', '--no-recurse-submodules', '--', 'origin', "${reference}:$tracking") | Out-Null
    Invoke-Git @('switch', '--create', 'main', '--no-track', $tracking) | Out-Null
    Invoke-Git @('config', 'branch.main.remote', 'origin') | Out-Null
    Invoke-Git @('config', 'branch.main.merge', $reference) | Out-Null
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_untracked'), "keep untracked $runId`n", $utf8)
    Assert-Tip $base $base '0 0'
    $phase = 'publish-first-advance'
    $advanced = Publish "Rocket synthetic first advance $runId" $base
    $beforeLocal = Get-LocalState
    if (-not $SelfTest) {
        if ($Transport -eq 'Https') {
            $phase = 'credentials-disabled'
            Set-Authentication $false
            Invoke-Pull '' 'credentials_unavailable|authentication_rejected' -FetchFailure
            Assert-Local $beforeLocal
            Assert-Tip $base $base '0 0'
        } else {
            $phase = 'untrusted-host'
            Set-Authentication $true $false
            Invoke-Pull '' 'ssh_host_verification_failed' -FetchFailure
            Assert-Local $beforeLocal
            Assert-Tip $base $base '0 0'
            $phase = 'identity-disabled'
            Set-Authentication $false
            Invoke-Pull '' 'ssh_authentication_rejected' -FetchFailure
            Assert-Local $beforeLocal
            Assert-Tip $base $base '0 0'
        }
        Set-Authentication $true
        Assert-Remote (Get-RemoteState) $advanced
    }
    $phase = 'dirty-preflight'
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_file'), "staged fixture edit`n", $utf8)
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_file'), "unstaged fixture edit`n", $utf8)
    $dirty = Get-LocalState
    Invoke-Pull '' 'pull_dirty_worktree' -Preflight
    Assert-Local $dirty
    Assert-Tip $base $base '0 0'
    # Restore only this generated fixture file after proving the refusal kept it.
    Invoke-Git @('restore', '--source=HEAD', '--staged', '--worktree', '--', 'ROCKET_PRIVATE_file') | Out-Null
    $phase = 'fast-forward'
    Invoke-Pull 'FastForward'
    Assert-Tip $advanced $advanced '0 0'
    if ([IO.File]::ReadAllText((Join-Path $client 'ROCKET_PRIVATE_file')) -cne "Rocket synthetic first advance $runId`n" -or [IO.File]::ReadAllText((Join-Path $client 'ROCKET_PRIVATE_untracked')) -cne "keep untracked $runId`n") {
        Stop-Check 'Fast-forward file content or untracked preservation was incorrect.'
    }
    Assert-Remote (Get-RemoteState) $advanced
    $beforeLocal = Get-LocalState
    $phase = 'up-to-date'
    Invoke-Pull 'UpToDate'
    Assert-Local $beforeLocal
    Assert-Tip $advanced $advanced '0 0'
    $phase = 'local-ahead'
    Invoke-Git @('commit', '--allow-empty', '-m', 'Rocket synthetic local-only commit') | Out-Null
    $ahead = Invoke-Git @('rev-parse', 'HEAD')
    $ownedCommits.Add($ahead)
    $beforeLocal = Get-LocalState
    Invoke-Pull 'UpToDate'
    Assert-Local $beforeLocal
    Assert-Tip $ahead $advanced '1 0'
    Assert-Remote (Get-RemoteState) $advanced
    $phase = 'publish-divergent-advance'
    $diverged = Publish "Rocket synthetic second advance $runId" $advanced
    $phase = 'diverged'
    Invoke-Pull '' 'pull_diverged'
    Assert-Local $beforeLocal
    Assert-Tip $ahead $diverged '1 1'
    Assert-Remote (Get-RemoteState) $diverged
    if ($SelfTest) {
        $phase = 'cleanup-stale-lease-self-test'
        $stale = Invoke-RemoteWrite '' $base
        if ($stale.exit -eq 0) { Stop-Check 'Cleanup unexpectedly accepted a stale expected commit.' }
        Assert-Remote (Get-RemoteState) $diverged
        $evidence.cleanup_stale_lease_rejected = $true
        Write-Host 'PASS cleanup refuses a stale expected commit and preserves the branch'
    }
    $testsPassed = $true
} catch {
    $evidence.failure_phase = $phase
    $evidence.failure_hint = $failureHint
    $evidence.failure_type = $_.Exception.GetType().Name
    $evidence.failure_line = $_.InvocationInfo.ScriptLineNumber
} finally {
    if ($cleanupAllowed -and $remoteWriteAttempted) {
        $phase = 'cleanup'
        $evidence.cleanup = 'needs-verification'
        try {
            Set-Authentication $true
            $observed = Get-RemoteState
            $tip = Get-TestTip $observed
            if ($tip) {
                if (-not $ownedCommits.Contains($tip)) { Stop-Check 'Cleanup refused: the branch has a commit outside this run and was preserved.' }
                $deleted = Invoke-RemoteWrite '' $tip
                $evidence.cleanup_command_exit = $deleted.exit
            }
            # Read after even an unsuccessful/ambiguous delete; never retry it.
            $after = Get-RemoteState
            $evidence.after_remote = $after
            Assert-Remote $after ''
            $evidence.cleanup = 'confirmed-absent'
            $cleanupVerified = $true
            Write-Host 'PASS temporary branch removed; original refs and default HEAD unchanged'
        } catch {
            $evidence.cleanup_hint = $failureHint
            $evidence.cleanup_failure_line = $_.InvocationInfo.ScriptLineNumber
        }
    } elseif (-not $remoteWriteAttempted) { $cleanupVerified = $true }
    $phase = 'diagnostics'
    $logsPassed = $false
    if ($testsPassed) {
        try {
            Assert-Diagnostics
            if ($resolvedPublicKey -and (Get-FileHash -LiteralPath $resolvedPublicKey -Algorithm SHA256).Hash -cne $publicHashBefore) { Stop-Check 'The selected public-key file changed during the run.' }
            $logsPassed = $true
        } catch {
            $evidence.diagnostics_hint = $failureHint
            $evidence.diagnostics_failure_line = $_.InvocationInfo.ScriptLineNumber
        }
    }
    $evidence.passed = $testsPassed -and $cleanupVerified -and $logsPassed
    $phase = if ($evidence.passed) { 'complete' } else { 'stopped' }
    $evidence.finished_utc = [DateTime]::UtcNow.ToString('o')
    Save-Checkpoint
}
if (-not $evidence.passed) {
    $hint = @($evidence.failure_hint, $evidence.cleanup_hint, $evidence.diagnostics_hint) | Where-Object { $_ }
    throw "Pull smoke check stopped: $($hint -join ' ') Results: $resultPath"
}
Write-Host "GitHub pull smoke checks passed ($mode). Results: $resultPath"
Write-Host "Diagnostics: $logs"
Write-Host "CLI transcript (private fixture metadata, raw Git output omitted): $transcript"
