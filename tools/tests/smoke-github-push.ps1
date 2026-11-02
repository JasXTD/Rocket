param(
    [ValidateSet('Https', 'Ssh')][string]$Transport = 'Https',
    [string]$Repository = 'JasXTD/rocket-auth-test',
    [string]$PublicKeyPath = 'C:\Users\Tommy\.ssh\id_ed25519_github.pub',
    [switch]$SelfTest,
    [switch]$ProtectionTest
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
$scratch = Join-Path (Join-Path $workspace 'work') "smoke-github-push-$mode-$runId"
$client = Join-Path $scratch 'ROCKET_PRIVATE_client'
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
$branch = "rocket-push-test/$mode-$runId"
$reference = "refs/heads/$branch"
$remote = if ($SelfTest) { Join-Path $scratch 'upstream.git' } elseif ($Transport -eq 'Https') { "https://github.com/$Repository.git" } else { "git@github.com:$Repository.git" }
foreach ($path in @($client, $hooks, $emptyHome)) { [IO.Directory]::CreateDirectory($path) | Out-Null }
[IO.File]::WriteAllText($emptyConfig, '')
[IO.File]::WriteAllText($denyAskpass, "@exit /b 1`r`n", [Text.Encoding]::ASCII)
$utf8 = [Text.UTF8Encoding]::new($false)
$phase = 'setup'
$failureHint = 'Check the fixture tools and configuration.'
$attempted = $false
$cleanupAllowed = $false
$testsPassed = $false
$cleanupVerified = $false
$protectionPending = $false
$ruleRemovalPrompted = $false
$initialTransport = $Transport
$serverHook = Join-Path $hooks 'pre-receive'
$ownedCommits = New-Object 'System.Collections.Generic.List[string]'
$runs = New-Object 'System.Collections.Generic.List[object]'
$steps = New-Object 'System.Collections.Generic.List[object]'
$evidence = [ordered]@{
    passed = $false; mode = $mode; remote = $remote; branch = $reference
    protection_test = [bool]$ProtectionTest
    started_utc = [DateTime]::UtcNow.ToString('o'); cleanup = 'not-needed'
    inspector_sha256 = (Get-FileHash -LiteralPath $inspector -Algorithm SHA256).Hash
}
# GitHub's published Ed25519 key, rechecked on 2026-10-14. Pin rotations must be
# reviewed against this source; never use unverified keyscan or disable trust.
$hostKeySource = 'https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints'
$hostKeyEntry = 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl'
$hostFingerprint = 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU'
$sshDir = Join-Path $env:WINDIR 'System32\OpenSSH'
$sshPath = Join-Path $sshDir 'ssh.exe'
$keygenPath = Join-Path $sshDir 'ssh-keygen.exe'
$sshAddPath = Join-Path $sshDir 'ssh-add.exe'

function Stop-Check([string]$Message) { $script:failureHint = $Message; throw 'Push smoke check failed' }
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
            Invoke-Git @('config', 'credential.helper', $helper) | Out-Null
        } else { Invoke-Git @('config', 'credential.helper', '') | Out-Null }
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
    Invoke-Git @('config', 'core.sshCommand', ((Quote-ShellArgument $sshPath.Replace('\', '/')) + ' -F ' + (Quote-ShellArgument $sshConfig.Replace('\', '/')))) | Out-Null
}

function Select-Transport([string]$Selected) {
    if ($SelfTest) { return }
    if ($Selected -notin @('Https', 'Ssh')) { Stop-Check 'Unsupported test transport.' }
    $script:Transport = $Selected
    $script:remote = if ($Selected -eq 'Https') { "https://github.com/$Repository.git" } else { "git@github.com:$Repository.git" }
    Invoke-Git @('remote', 'set-url', 'origin', $remote) | Out-Null
    Set-Authentication $true
}

function Wait-ForProtection([bool]$Install) {
    if ($SelfTest) {
        if (-not $Install -and (Test-Path -LiteralPath $serverHook)) { Remove-Item -LiteralPath $serverHook }
        $script:protectionPending = $Install
        return
    }
    $state = if ($Install) { 'waiting-for-protection' } else { 'waiting-for-rule-removal' }
    $script:phase = $state
    # Persist a checkpoint before waiting so a closed terminal does not hide the
    # branch identity. Input only confirms a user's UI action; it changes no rule.
    $evidence.phase = $state
    $evidence.owned_commits = $ownedCommits.ToArray()
    $evidence.push_attempts = $runs.ToArray()
    $evidence.harness_steps = $steps.ToArray()
    [IO.File]::WriteAllText($resultPath, ($evidence | ConvertTo-Json -Depth 8), $utf8)
    Write-Host "MANUAL STEP: $state"
    Write-Host "Repository: $Repository"
    Write-Host "Exact branch name: $branch"
    Write-Host "Checkpoint: $resultPath"
    if ($Install) {
        $script:protectionPending = $true
        Write-Host 'Add a classic protection rule for that exact branch (no wildcard).'
        Write-Host 'Enable Require a pull request before merging and Do not allow bypassing the above settings.'
        Write-Host 'Save the rule only if GitHub says it will be enforced. Leave existing rules unchanged.'
        $answer = Read-Host 'After saving, type PROTECTED; otherwise type STOP'
        if ($answer -cne 'PROTECTED') { Stop-Check 'Live protection was not confirmed. Remove any test rule before cleanup.' }
        $evidence.protection_setup = 'user-confirmed-classic-pull-request-and-enforce-admins'
    } else {
        $script:ruleRemovalPrompted = $true
        Write-Host 'Remove only the temporary protection rule matching the exact branch above.'
        Write-Host 'Keep all pre-existing rules. Typing REMOVED does not delete a GitHub rule.'
        $answer = Read-Host 'After removing that rule (or verifying it was never created), type REMOVED; type KEEP to preserve the branch'
        if ($answer -cne 'REMOVED') { Stop-Check 'Test rule removal was not confirmed; the remote branch was preserved.' }
        $script:protectionPending = $false
        $evidence.protection_removal = 'user-confirmed'
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
function Get-LocalState {
    [ordered]@{
        head = Invoke-Git @('rev-parse', 'HEAD')
        refs = Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/heads/', 'refs/tags/')
        index = (Get-FileHash -LiteralPath (Join-Path $client '.git\index') -Algorithm SHA256).Hash
        files = @(Get-ChildItem -LiteralPath $client -File | Sort-Object Name | ForEach-Object { [ordered]@{ name = $_.Name; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash } })
    } | ConvertTo-Json -Depth 4 -Compress
}
function Set-DirtyState {
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_file'), "staged fixture data`n", $utf8)
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_file'), "unstaged fixture data`n", $utf8)
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_untracked'), "untracked fixture data`n", $utf8)
    $script:beforeLocal = Get-LocalState
}
function Invoke-Push([string]$Source, [string]$ExpectedKind, [string]$FailurePattern = '', [switch]$Upstream) {
    $script:attempted = $true
    $arguments = @($client, 'push', 'origin', $Source, $branch)
    if ($Upstream) { $arguments += '--set-upstream' }
    $output = Invoke-Isolated $inspector $arguments 'rocket-push'
    $text = $output.stdout + $output.stderr
    if ($text -match 'gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|(?im)Authorization:\s*(Basic|Bearer)\s+\S+|-----BEGIN [^-]*PRIVATE KEY-----') {
        Stop-Check 'Captured output matched a secret format; it was not saved. Inspect credential handling before sharing evidence.'
    }
    [IO.File]::AppendAllText($transcript, "PHASE: $phase`n$text`n", $utf8)
    $failure = if ($text -match '\[([a-z_]+)\]') { $Matches[1] } else { $null }
    $outcome = if ($text -match 'Remote outcome: ([a-z_]+)') { $Matches[1] } else { $null }
    $runs.Add([ordered]@{ phase=$phase; exit=$output.exit; elapsed_ms=$output.ms; failure_code=$failure; remote_outcome=$outcome })
    if (-not $text.Contains('Branches after push:') -or -not $text.Contains('Status after push:') -or $text -match 'refresh failed:|parsing failed:|verification failed:|Git emitted error diagnostics despite remote success') {
        Stop-Check 'Push parsing or local reconciliation failed; inspect the separate remote outcome.'
    }
    if ($FailurePattern) {
        if ($output.exit -ne 1 -or $failure -cnotmatch "\A($FailurePattern)\z") { Stop-Check "Expected $FailurePattern; got exit $($output.exit) and code $failure. No automatic retry." }
        $wanted = if ($failure -in @('push_non_fast_forward', 'push_remote_rejected')) { 'rejected' } else { 'unknown' }
        if ($outcome -cne $wanted) { Stop-Check 'Incorrect remote outcome for the rejected/interrupted request.' }
        if ($failure -eq 'push_remote_rejected' -and (-not $text.Contains('Check branch protection, repository policy, and server hooks.') -or $text.Contains('ROCKET_PRIVATE_server'))) {
            Stop-Check 'The policy rejection omitted actionable guidance or exposed raw server output.'
        }
    } else {
        if ($output.exit -ne 0 -or $outcome -cne 'confirmed_success' -or -not $text.Contains("kind: $ExpectedKind,")) {
            Stop-Check "Push did not confirm $ExpectedKind (exit $($output.exit), code $failure). Check access/rules and remote state before retrying."
        }
        if ($Upstream -and -not $text.Contains('Upstream: configured and verified.')) { Stop-Check 'Requested upstream setup was not verified.' }
    }
    if ((Get-LocalState) -cne $script:beforeLocal) { Stop-Check 'Push changed local HEAD, branches/tags, index, or working-file bytes.' }
    Write-Host "PASS $mode $phase (exit $($output.exit), remote $outcome)"
}

function Invoke-LeasedDelete([string]$ExpectedTip) {
    if (-not $cleanupAllowed -or $reference -cne "refs/heads/rocket-push-test/$mode-$runId" -or -not $ownedCommits.Contains($ExpectedTip)) {
        Stop-Check 'Cleanup refused: the branch or expected commit is not owned by this run.'
    }
    # Native Git teardown only. Rocket's public push API deliberately has no
    # deletion/force mode. The exact lease rejects changes after our observation.
    Invoke-Isolated $gitPath @('-C', $client, '-c', 'remote.origin.mirror=false',
        '-c', 'push.followTags=false', '-c', 'push.autoSetupRemote=false',
        '-c', 'push.negotiate=false', '-c', 'push.pushOption=',
        'push', '--porcelain', '--no-follow-tags', '--no-prune', '--no-recurse-submodules',
        "--force-with-lease=${reference}:$ExpectedTip", '--', 'origin', ":$reference") 'git-cleanup'
}

function Assert-Diagnostics {
    $records = @()
    $diagnostics = ''
    foreach ($file in @(Get-ChildItem -LiteralPath $logs -Filter '*.jsonl' -File)) {
        $diagnostics += [IO.File]::ReadAllText($file.FullName)
        foreach ($line in [IO.File]::ReadAllLines($file.FullName)) { $records += ($line | ConvertFrom-Json -ErrorAction Stop) }
    }
    if ($records.Count -eq 0) { Stop-Check 'No diagnostic records were produced.' }
    if ($diagnostics -match 'ROCKET_PRIVATE_|rocket-push-test|refs/heads/|refs/remotes/|github\.com|gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|(?im)Authorization:\s*(Basic|Bearer)\s+\S+|-----BEGIN [^-]*PRIVATE KEY-----') {
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
    $sessions = @($records | Where-Object { $_.event -ceq 'request_start' -and $_.operation -ceq 'push' })
    if ($sessions.Count -ne $runs.Count) { Stop-Check 'Diagnostic request count does not match push attempts.' }
    for ($i = 0; $i -lt $sessions.Count; $i++) {
        $sessionRecords = @($records | Where-Object { $_.session -ceq $sessions[$i].session })
        if (@($sessionRecords | Where-Object { $_.event -ceq 'git_start' -and $_.operation -ceq 'push' }).Count -ne 1) { Stop-Check 'Expected exactly one Git push per request.' }
        foreach ($operation in @('branches', 'status')) {
            if (@($sessionRecords | Where-Object { $_.event -ceq 'refresh' -and $_.operation -ceq $operation -and $_.outcome -ceq 'ok' }).Count -ne 1) { Stop-Check 'Missing successful reconciliation diagnostic.' }
        }
        $outcomes = @($sessionRecords | Where-Object { $_.event -ceq 'push_outcome' })
        if ($outcomes.Count -ne 1 -or $outcomes[0].remote_outcome -cne $runs[$i].remote_outcome) { Stop-Check 'CLI and diagnostic remote outcomes differ.' }
        # Git normally emits error diagnostics for deliberate rejection cases.
        # An error alongside confirmed success needs separate investigation.
        if ($outcomes[0].remote_outcome -ceq 'confirmed_success' -and $outcomes[0].error_diagnostics) { Stop-Check 'A successful remote write also reported an unexpected Git error.' }
        if ($runs[$i].failure_code -and @($sessionRecords | Where-Object { $_.failure_code -ceq $runs[$i].failure_code }).Count -eq 0) { Stop-Check 'Missing classified failure diagnostic.' }
    }
    $evidence.diagnostic_records = $records.Count
    $evidence.diagnostic_sessions = $sessions.Count
    $evidence.diagnostic_dropped = 0
    $evidence.diagnostic_privacy_scan_passed = $true
    Write-Host 'PASS paired spans, one push per request, typed errors, remote outcomes, local refreshes and diagnostic privacy checks'
}

try {
    Invoke-Git @('init', '--initial-branch=main') | Out-Null
    foreach ($setting in @(
        @('user.name', 'Rocket Push Smoke'), @('user.email', 'rocket@example.invalid'),
        @('core.hooksPath', $hooks), @('core.autocrlf', 'false'), @('core.fsmonitor', 'false'),
        @('commit.gpgsign', 'false'), @('push.gpgsign', 'false'),
        @('credential.interactive', 'false'), @('credential.helper', ''),
        @('http.sslBackend', 'schannel'), @('http.sslVerify', 'true'),
        @('http.followRedirects', 'false'), @('http.extraHeader', '')
    )) { Invoke-Git @('config', $setting[0], $setting[1]) | Out-Null }
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_file'), "Rocket synthetic push fixture $runId`n", $utf8)
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    Invoke-Git @('commit', '-m', 'Rocket synthetic transport test base') | Out-Null
    $base = Invoke-Git @('rev-parse', 'HEAD')
    $ownedCommits.Add($base)
    Invoke-Git @('branch', 'before-advance') | Out-Null
    Invoke-Git @('remote', 'add', 'origin', $remote) | Out-Null

    if ($SelfTest) {
        Invoke-Git @('init', '--bare', '--initial-branch=main', $remote) | Out-Null
        Invoke-Git @('config', 'core.hooksPath', $hooks) -At $remote | Out-Null
        Invoke-Git @('push', '--', 'origin', 'refs/heads/main:refs/heads/main') | Out-Null
    } elseif ($Transport -eq 'Ssh' -or $ProtectionTest) {
        $phase = 'ssh-agent-preflight'
        Initialize-Ssh
    }
    Set-Authentication $true
    if (-not $SelfTest -and $Transport -eq 'Ssh') {
        $pin = Invoke-Isolated $keygenPath @('-lf', $knownHosts) 'host-key-fingerprint'
        if ($pin.exit -ne 0 -or -not $pin.stdout.Contains($hostFingerprint)) { Stop-Check 'Fixture host key does not match the reviewed GitHub fingerprint.' }
        $evidence.host_key_source = $hostKeySource
    }

    $phase = 'remote-preflight'
    $failureHint = 'Read-only remote access failed. Check the existing GCM login or Windows SSH agent and test repository access locally.'
    $script:beforeRemote = Get-RemoteState
    if ($beforeRemote.refs.Count -eq 0 -or -not $beforeRemote.head) { Stop-Check 'Use an initialized test repository with an existing default branch; this runner does not create default branches.' }
    if (Get-TestTip $beforeRemote) { Stop-Check 'The generated branch already exists; nothing will be overwritten or deleted.' }
    $cleanupAllowed = $true
    $evidence.before_remote = $beforeRemote
    Set-DirtyState

    if (-not $SelfTest -and -not $ProtectionTest) {
        if ($Transport -eq 'Https') {
            $phase = 'credentials-disabled'
            Set-Authentication $false
            Invoke-Push 'main' '' 'credentials_unavailable|authentication_rejected'
        } else {
            $phase = 'untrusted-host'
            Set-Authentication $true $false
            Invoke-Push 'main' '' 'ssh_host_verification_failed'
            $phase = 'identity-disabled'
            Set-Authentication $false
            Invoke-Push 'main' '' 'ssh_authentication_rejected'
        }
        Set-Authentication $true
        Assert-Remote (Get-RemoteState) ''
    }

    $phase = 'new-branch'
    Invoke-Push 'main' 'New' -Upstream
    Assert-Remote (Get-RemoteState) $base
    if ((Invoke-Git @('config', 'branch.main.remote')) -cne 'origin' -or (Invoke-Git @('config', 'branch.main.merge')) -cne $reference) { Stop-Check 'Upstream configuration selected a different destination.' }

    $phase = 'fixture-advance'
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_file'), "Rocket synthetic advance $runId`n", $utf8)
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    Invoke-Git @('commit', '-m', 'Rocket synthetic transport test advance') | Out-Null
    $advanced = Invoke-Git @('rev-parse', 'HEAD')
    $ownedCommits.Add($advanced)
    Set-DirtyState

    if ($ProtectionTest) {
        if (-not $SelfTest) {
            $phase = 'second-transport-preflight'
            foreach ($selected in @('Https', 'Ssh')) {
                Select-Transport $selected
                Assert-Remote (Get-RemoteState) $base
            }
            $pin = Invoke-Isolated $keygenPath @('-lf', $knownHosts) 'host-key-fingerprint'
            if ($pin.exit -ne 0 -or -not $pin.stdout.Contains($hostFingerprint)) { Stop-Check 'Fixture host key does not match the reviewed GitHub fingerprint.' }
            $evidence.host_key_source = $hostKeySource
            Select-Transport $initialTransport
        }
        Wait-ForProtection $true
        $cases = if ($SelfTest) { @('classic-fixture', 'ruleset-fixture') } else { @('Https', 'Ssh') }
        foreach ($case in $cases) {
            $phase = 'protected-push-' + $case.ToLowerInvariant()
            if ($SelfTest) {
                $message = if ($case -eq 'classic-fixture') { 'GH006: Protected branch update failed. Changes must be made through a pull request.' } else { 'GH013: Repository rule violations found. Changes must be made through a pull request.' }
                [IO.File]::WriteAllText($serverHook, "#!/bin/sh`necho 'error: $message ROCKET_PRIVATE_server' >&2`nexit 1`n", $utf8)
            } else { Select-Transport $case }
            Assert-Remote (Get-RemoteState) $base
            Invoke-Push 'main' '' 'push_remote_rejected'
            Assert-Remote (Get-RemoteState) $base
        }
        $evidence.protection_rejections_verified = $true
        Wait-ForProtection $false
        Select-Transport $initialTransport
        # The same commit must become pushable after the user's temporary rule
        # is removed. No commits are rebuilt or amended between these attempts.
    }
    $phase = 'fast-forward'
    Invoke-Push 'main' 'FastForward'
    Assert-Remote (Get-RemoteState) $advanced
    $phase = 'up-to-date'
    Invoke-Push 'main' 'UpToDate'
    Assert-Remote (Get-RemoteState) $advanced
    $phase = 'non-fast-forward'
    Invoke-Push 'before-advance' '' 'push_non_fast_forward'
    Assert-Remote (Get-RemoteState) $advanced

    if ($SelfTest) {
        $phase = 'cleanup-stale-lease-self-test'
        # Remote is at advanced; deliberately using the previous owned commit as
        # the expected value must fail without removing or changing the branch.
        $stale = Invoke-LeasedDelete $base
        if ($stale.exit -eq 0) { Stop-Check 'Deletion unexpectedly accepted a stale expected commit.' }
        Assert-Remote (Get-RemoteState) $advanced
        $evidence.cleanup_stale_lease_rejected = $true
        Write-Host 'PASS cleanup rejects a stale lease and preserves the remote branch'
    }
    $testsPassed = $true
} catch {
    $evidence.failure_phase = $phase
    $evidence.failure_hint = $failureHint
    $evidence.failure_type = $_.Exception.GetType().Name
    $evidence.failure_line = $_.InvocationInfo.ScriptLineNumber
} finally {
    if ($cleanupAllowed -and $attempted) {
        $phase = 'cleanup'
        $evidence.cleanup = 'needs-verification'
        try {
            if ($protectionPending) {
                if ($ruleRemovalPrompted) { Stop-Check 'Test rule removal was not confirmed; the remote branch was preserved.' }
                Wait-ForProtection $false
            }
            $phase = 'cleanup'
            if ($ProtectionTest) { Select-Transport $initialTransport } else { Set-Authentication $true }
            $observed = Get-RemoteState
            $tip = Get-TestTip $observed
            if ($tip) {
                if (-not $ownedCommits.Contains($tip)) { Stop-Check 'Cleanup refused: the branch changed to a commit outside this run. It was preserved for inspection.' }
                $deletion = Invoke-LeasedDelete $tip
                $evidence.cleanup_command_exit = $deletion.exit
                # Even an interrupted command may have deleted the branch. Read
                # the actual state; never retry a write solely from an exit code.
            }
            $after = Get-RemoteState
            $evidence.after_remote = $after
            Assert-Remote $after ''
            $evidence.cleanup = 'confirmed-absent'
            $cleanupVerified = $true
            Write-Host 'PASS temporary branch removed; original remote refs and default HEAD unchanged'
        } catch {
            $evidence.cleanup_hint = $failureHint
            $evidence.cleanup_failure_line = $_.InvocationInfo.ScriptLineNumber
        }
    } elseif (-not $attempted) {
        $cleanupVerified = $true
    }
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
    $evidence.phase = if ($evidence.passed) { 'complete' } else { 'stopped' }
    $evidence.finished_utc = [DateTime]::UtcNow.ToString('o')
    $evidence.owned_commits = $ownedCommits.ToArray()
    $evidence.push_attempts = $runs.ToArray()
    $evidence.harness_steps = $steps.ToArray()
    [IO.File]::WriteAllText($resultPath, ($evidence | ConvertTo-Json -Depth 8), $utf8)
}
if (-not $evidence.passed) {
    $hint = @($evidence.failure_hint, $evidence.cleanup_hint, $evidence.diagnostics_hint) | Where-Object { $_ }
    throw "Push smoke check stopped: $($hint -join ' ') Results: $resultPath"
}
Write-Host "GitHub push smoke checks passed ($mode). Results: $resultPath"
Write-Host "Diagnostics: $logs"
Write-Host "CLI transcript (contains private repository metadata; raw Git output omitted): $transcript"
