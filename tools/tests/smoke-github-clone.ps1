param(
    [ValidateSet('Https', 'Ssh')][string]$Transport = 'Https',
    [string]$Repository = 'JasXTD/rocket-auth-test',
    [string]$PublicKeyPath = 'C:\Users\Tommy\.ssh\id_ed25519_github.pub',
    [switch]$SelfTest,
    [switch]$Interruptions
)

$ErrorActionPreference = 'Stop'
if ($Interruptions -and ($SelfTest -or $Repository -cne 'ocornut/imgui')) { throw 'Interruption mode uses only -Repository ocornut/imgui, without -SelfTest. Its relay unit tests run offline through cargo test.' }
if ($Repository -cnotmatch '\A[A-Za-z0-9-]+/[A-Za-z0-9_.-]+\z' -or $Repository.EndsWith('.git')) { throw 'Use OWNER/REPOSITORY without a URL or .git suffix.' }
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) { throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect' }
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$mode = if ($SelfTest) { 'local' } else { $Transport.ToLowerInvariant() }
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-github-clone-' + $(if ($Interruptions) { 'interruption-' }) + $mode + '-' + [guid]::NewGuid().ToString('N'))
$target = Join-Path $scratch 'ROCKET_PRIVATE_clone with spaces'
$repeatTarget = Join-Path $scratch 'ROCKET_PRIVATE_repeat'
$logs = Join-Path $scratch 'logs'
$emptyHome = Join-Path $scratch 'empty-home'
$config = Join-Path $scratch 'isolated.config'
$hooks = Join-Path $scratch 'empty-hooks'
$denyAskpass = Join-Path $scratch 'deny-askpass.cmd'
$transcript = Join-Path $scratch 'cli-output.log'
$resultPath = Join-Path $scratch 'result.json'
$sshConfig = Join-Path $scratch 'ssh.config'
$knownHosts = Join-Path $scratch 'known_hosts'
$publicIdentity = Join-Path $scratch 'selected-identity.pub'
$utf8 = [Text.UTF8Encoding]::new($false)
foreach ($directory in @($scratch, $emptyHome, $hooks)) { [IO.Directory]::CreateDirectory($directory) | Out-Null }
[IO.File]::WriteAllText($denyAskpass, "@exit /b 1`r`n", $utf8)
[IO.File]::WriteAllText($config, @'
[credential]
    helper =
    interactive = false
[http]
    sslVerify = true
    followRedirects = false
[core]
    autocrlf = false
[gc]
    auto = 0
[maintenance]
    auto = false
[protocol]
    allow = never
[protocol "https"]
    allow = always
[protocol "ssh"]
    allow = always
[protocol "file"]
    allow = always
'@, $utf8)
$source = if ($SelfTest) { Join-Path $scratch 'source' } elseif ($Transport -eq 'Https') { "https://github.com/$Repository.git" } else { "ssh://git@github.com/$Repository.git" }
$sourceKind = if ($SelfTest) { '--local' } else { '--url' }
$phase = 'setup'
$steps = [Collections.Generic.List[object]]::new()
$requests = [Collections.Generic.List[object]]::new()
$privacyValues = [Collections.Generic.List[string]]::new()
foreach ($value in @('ROCKET_PRIVATE', $scratch, $Repository, $source, 'Authorization:', 'ghp_', 'github_pat_', 'PRIVATE KEY')) { $privacyValues.Add($value) }
$evidence = [ordered]@{
    passed = $false; started_utc = [DateTime]::UtcNow.ToString('o'); transport = $mode
    source = $source; scratch = $scratch; destination = $target; remote_writes = $false
    inspector_sha256 = (Get-FileHash -LiteralPath $inspector -Algorithm SHA256).Hash
    runner_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
    interruptions = [bool]$Interruptions
}
# GitHub's published Ed25519 host key was rechecked on 2026-10-15:
# https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints
$hostKeyEntry = 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl'
$hostFingerprint = 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU'
$sshDir = Join-Path $env:WINDIR 'System32\OpenSSH'
$sshPath = Join-Path $sshDir 'ssh.exe'
$keygenPath = Join-Path $sshDir 'ssh-keygen.exe'
$sshAddPath = Join-Path $sshDir 'ssh-add.exe'

function Quote-WindowsArgument([string]$Value) {
    '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}
function Quote-ShellArgument([string]$Value) {
    $quote = [string][char]39
    $escaped = $quote + [char]34 + $quote + [char]34 + $quote
    $quote + $Value.Replace($quote, $escaped) + $quote
}
function Quote-SshPath([string]$Value) {
    if ($Value -match '[\r\n"]' -or $Value.Contains('${')) { throw 'Unsupported SSH configuration path.' }
    [string][char]34 + $Value.Replace('\', '/').Replace('%', '%%') + [char]34
}
function Invoke-Isolated([string]$Executable, [string[]]$Arguments) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.Arguments = ($Arguments | ForEach-Object { Quote-WindowsArgument $_ }) -join ' '
    $start.WorkingDirectory = $scratch
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardOutputEncoding = $utf8
    $start.StandardErrorEncoding = $utf8
    foreach ($key in @($start.EnvironmentVariables.Keys)) {
        if ($key -match '^(GIT_|GCM_|ROCKET_|SSH_|CURL_|SSL_CERT_)|_PROXY$|^NETRC$') { $start.EnvironmentVariables.Remove($key) }
    }
    # Keep Windows profile/store access for GCM. Only child Git/curl homes and
    # config are isolated. Never read helper output or private-key contents.
    $values = @{
        HOME = $emptyHome; XDG_CONFIG_HOME = $emptyHome; CURL_HOME = $emptyHome
        GIT_CONFIG_GLOBAL = $config; GIT_CONFIG_SYSTEM = $config; GIT_CONFIG_NOSYSTEM = '1'
        GIT_TEMPLATE_DIR = $hooks; GIT_TERMINAL_PROMPT = '0'; GIT_ASKPASS = $denyAskpass
        SSH_ASKPASS = $denyAskpass; SSH_ASKPASS_REQUIRE = 'never'; GIT_SSH_VARIANT = 'ssh'
        GCM_INTERACTIVE = '0'; GCM_TRACE = '0'; GCM_TRACE_SECRETS = '0'; GCM_DEBUG = '0'
        GIT_ALLOW_PROTOCOL = $(if ($SelfTest) { 'file' } else { $Transport.ToLowerInvariant() })
        GIT_NO_LAZY_FETCH = '1'; GIT_NO_REPLACE_OBJECTS = '1'; GIT_OPTIONAL_LOCKS = '0'
        ROCKET_LOG_DIR = $logs; LC_ALL = 'C'
    }
    foreach ($key in $values.Keys) { $start.EnvironmentVariables[$key] = $values[$key] }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $started = $false
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        [void]$process.Start(); $started = $true
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.StandardInput.Close()
        while (-not $process.WaitForExit(500)) {
            if ($watch.Elapsed.TotalSeconds -gt $(if ($Interruptions) { 150 } else { 90 })) { throw 'Acceptance child exceeded its time limit.' }
        }
        if (-not $stdout.Wait(5000) -or -not $stderr.Wait(5000)) { throw 'Acceptance child pipes did not close.' }
        $steps.Add([ordered]@{phase = $phase; exit = $process.ExitCode; elapsed_ms = $watch.ElapsedMilliseconds})
        return [pscustomobject]@{exit = $process.ExitCode; stdout = $stdout.Result; stderr = $stderr.Result; ms = $watch.ElapsedMilliseconds}
    } finally {
        try {
            if ($started -and -not $process.HasExited) {
                & (Join-Path $env:WINDIR 'System32\taskkill.exe') /PID $process.Id /T /F 2>&1 | Out-Null
                if (-not $process.WaitForExit(5000)) { throw 'Owned acceptance child did not stop.' }
            }
        } finally { $process.Dispose() }
    }
}
function Invoke-Git([string[]]$Arguments, [string]$At = $scratch) {
    $output = Invoke-Isolated $gitPath (@('-C', $At) + $Arguments)
    if ($output.exit -ne 0) { throw "Git verification failed during $phase (exit $($output.exit)); raw authentication output was not saved." }
    return $output.stdout.TrimEnd("`r", "`n")
}
function Set-Authentication([bool]$Enabled, [bool]$TrustHost = $true) {
    if ($SelfTest) { return }
    if ($Transport -eq 'Https') {
        # Only Git invokes get. Ignore store/erase so rejection cannot remove the
        # user's saved login. Neither the script nor evidence sees the credential.
        $helper = if ($Enabled) { '!f() { if test "$1" = get; then ' + (Quote-ShellArgument $gitPath.Replace('\', '/')) + ' credential-manager get; else cat >/dev/null; fi; }; f' } else { '' }
        Invoke-Git @('config', '--file', $config, '--replace-all', 'credential.helper', $helper) | Out-Null
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
    if (-not $PublicKeyPath.EndsWith('.pub', [StringComparison]::OrdinalIgnoreCase)) { throw 'Select the .pub file, never the private key.' }
    $script:resolvedPublicKey = (Resolve-Path -LiteralPath $PublicKeyPath -ErrorAction Stop).Path
    $publicText = [IO.File]::ReadAllText($resolvedPublicKey).Trim()
    if ($publicText -cnotmatch '\Assh-ed25519 [A-Za-z0-9+/]+={0,3}(?: [^\r\n]*)?\z') { throw 'The selected public file must contain one Ed25519 key.' }
    $privacyValues.Add(($publicText -split ' ')[1])
    [IO.File]::WriteAllText($publicIdentity, "$publicText`n", $utf8)
    $script:publicHashBefore = (Get-FileHash -LiteralPath $resolvedPublicKey -Algorithm SHA256).Hash
    $fingerprintResult = Invoke-Isolated $keygenPath @('-lf', $publicIdentity)
    if ($fingerprintResult.exit -ne 0 -or $fingerprintResult.stdout -notmatch '\bSHA256:[A-Za-z0-9+/]+') { throw 'Cannot validate the public-key fingerprint.' }
    $fingerprint = $Matches[0]
    $agent = Invoke-Isolated $sshAddPath @('-l')
    if ($agent.exit -ne 0 -or -not $agent.stdout.Contains($fingerprint)) { throw 'Load id_ed25519_github into the Windows SSH agent locally, then rerun with a fresh fixture.' }
    $evidence.selected_agent_key_available = $true
    Invoke-Git @('config', '--file', $config, 'core.sshCommand', ((Quote-ShellArgument $sshPath.Replace('\', '/')) + ' -F ' + (Quote-ShellArgument $sshConfig.Replace('\', '/')))) | Out-Null
    Set-Authentication $true
    $hostCheck = Invoke-Isolated $keygenPath @('-lf', $knownHosts)
    if ($hostCheck.exit -ne 0 -or -not $hostCheck.stdout.Contains($hostFingerprint)) { throw 'Pinned host key does not match the published fingerprint.' }
}
function Read-RemoteState {
    $head = Invoke-Git @('ls-remote', '--symref', '--', $source, 'HEAD')
    $oid = [regex]::Match($head, '(?m)^([a-f0-9]{40}|[a-f0-9]{64})\tHEAD$')
    $branch = [regex]::Match($head, '(?m)^ref: (refs/heads/[^\r\n]+)\tHEAD$')
    if (-not $oid.Success -or -not $branch.Success) { throw 'Remote did not advertise a usable default HEAD.' }
    $refs = Invoke-Git @('ls-remote', '--refs', '--', $source)
    [string[]]$sorted = @($refs -split "`n" | Where-Object { $_ -ne '' })
    foreach ($line in $sorted) { if ($line -cnotmatch '\A(?:[a-f0-9]{40}|[a-f0-9]{64})\trefs/\S+\z') { throw 'Unexpected remote reference output.' } }
    [Array]::Sort($sorted, [StringComparer]::Ordinal)
    $hash = [Security.Cryptography.SHA256]::Create()
    try { $digest = [BitConverter]::ToString($hash.ComputeHash($utf8.GetBytes(($sorted -join "`n")))).Replace('-', '') } finally { $hash.Dispose() }
    [pscustomobject]@{oid = $oid.Groups[1].Value; branch = $branch.Groups[1].Value; refs_sha256 = $digest; ref_count = $sorted.Count}
}
function Invoke-Clone([string]$Destination, [string]$ExpectedFailure = '', [string]$SelectedSource = $source) {
    $output = Invoke-Isolated $inspector @('clone', $sourceKind, $SelectedSource, $Destination)
    $text = $output.stdout + $output.stderr
    if ($text -match 'gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|(?im)Authorization:\s*(Basic|Bearer)\s+\S+|-----BEGIN [^-]*PRIVATE KEY-----') { throw 'Captured CLI output matched a secret format and was not saved.' }
    if (-not $SelfTest -and ($text.Contains($Repository) -or $text.Contains($source))) { throw 'Default CLI output exposed private source metadata and was not saved.' }
    [IO.File]::AppendAllText($transcript, "PHASE: $phase`n$text`n", $utf8)
    $failure = if ($text -match '\[([a-z_]+)\]') { $Matches[1] } else { $null }
    $requests.Add([ordered]@{phase = $phase; exit = $output.exit; elapsed_ms = $output.ms; failure_code = $failure; expected_failure = $ExpectedFailure})
    if ($ExpectedFailure) {
        if ($output.exit -ne 1 -or $failure -cnotmatch "\A($ExpectedFailure)\z" -or -not $text.Contains('Next: ')) { throw "Expected $ExpectedFailure and recovery guidance; got exit $($output.exit), code $failure." }
        if ($ExpectedFailure -ne 'clone_destination_exists') {
            foreach ($label in @('Clone command: failed','Clone verification: skipped','Clone publication: skipped','Workspace retained for inspection:')) {
                if (-not $text.Contains($label)) { throw "Missing failure-phase guidance: $label" }
            }
            if (Test-Path -LiteralPath $Destination) { throw 'Rejected clone published its destination.' }
        }
    } else {
        if ($output.exit -ne 0) { throw "Authenticated clone failed (exit $($output.exit), code $failure)." }
        foreach ($label in @('Clone phase: Cloning','Clone phase: Verifying','Clone phase: Publishing','Clone phase: Opening','Clone command: succeeded','Clone verification: succeeded','Clone publication: succeeded','Clone published at')) {
            if (-not $text.Contains($label)) { throw "Missing successful phase/progress: $label" }
        }
    }
    Write-Host "PASS $phase (exit $($output.exit)$(if($failure){", $failure"}))"
    return $text
}
function Check-Clone([string]$At, $Remote) {
    if ((Invoke-Git @('rev-parse', 'HEAD') -At $At) -cne $Remote.oid -or (Invoke-Git @('rev-parse', '@{u}') -At $At) -cne $Remote.oid -or (Invoke-Git @('symbolic-ref', 'HEAD') -At $At) -cne $Remote.branch) { throw 'Clone HEAD/upstream/default branch mismatch.' }
    if ((Invoke-Git @('status', '--porcelain=v1', '--untracked-files=all') -At $At) -ne '') { throw 'Published checkout is not clean.' }
    if ((Invoke-Git @('config', '--get', 'remote.origin.url') -At $At) -cne $source) { throw 'Origin URL mismatch.' }
    Invoke-Git @('fsck', '--connectivity-only', '--no-dangling') -At $At | Out-Null
}

try {
    if ($SelfTest) {
        [IO.Directory]::CreateDirectory($source) | Out-Null
        Invoke-Git @('init', '--initial-branch=main') -At $source | Out-Null
        Invoke-Git @('config', 'user.name', 'Rocket Clone Acceptance') -At $source | Out-Null
        Invoke-Git @('config', 'user.email', 'rocket@example.invalid') -At $source | Out-Null
        [IO.File]::WriteAllText((Join-Path $source 'README.md'), "ROCKET_PRIVATE synthetic clone fixture.`n", $utf8)
        Invoke-Git @('add', '--all', '--') -At $source | Out-Null
        Invoke-Git @('commit', '-m', 'Synthetic clone acceptance') -At $source | Out-Null
    } elseif ($Transport -eq 'Ssh') { Initialize-Ssh }
    Set-Authentication (-not $Interruptions -or $Transport -eq 'Ssh')
    $phase = 'remote-before'
    $before = Read-RemoteState
    $evidence.remote_before = $before
    $privacyValues.Add($before.oid)
    if ($Interruptions) {
        $probe = Join-Path $workspace 'target\debug\examples\clone_transport_probe.exe'
        if (-not (Test-Path -LiteralPath $probe -PathType Leaf)) { throw 'Build the clone_transport_probe example first.' }
        $evidence.probe_sha256 = (Get-FileHash -LiteralPath $probe -Algorithm SHA256).Hash
        $evidence.probes = @()
        $actions = if ($Transport -eq 'Ssh') { @('cancel', 'disconnect', 'complete') } else { @('disconnect', 'complete') }
        foreach ($action in $actions) {
            $phase = "transport-$action"
            $run = Join-Path $scratch $action
            $arguments = @($mode, $action, $run, $config)
            if ($Transport -eq 'Ssh') { $arguments += @($sshPath, $sshConfig) }
            $output = Invoke-Isolated $probe $arguments
            $text = $output.stdout + $output.stderr
            if ($text -match 'gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|(?im)Authorization:\s*(Basic|Bearer)\s+\S+|-----BEGIN [^-]*PRIVATE KEY-----|(?m)^stderr: |(?m)^stdout: ') { throw 'Probe output was not safe to save.' }
            [IO.File]::AppendAllText($transcript, "PHASE: $phase`n$text`n", $utf8)
            $probeResult = Get-Content -LiteralPath (Join-Path $run 'probe-result.json') -Raw | ConvertFrom-Json
            $evidence.probes += $probeResult
            if ($output.exit -ne 0 -or -not $probeResult.passed) { throw "Transport $action did not satisfy structural acceptance checks." }
            $expected = if ($action -eq 'disconnect') { 'connection_interrupted' } elseif ($action -eq 'cancel') { 'cancelled' } else { '' }
            $requests.Add([ordered]@{phase = $phase; exit = $probeResult.exit; elapsed_ms = $output.ms; failure_code = $probeResult.failure_code; expected_failure = $expected})
            if ($action -eq 'disconnect' -and $probeResult.failure_code -cne $expected) { throw "Interrupted transport produced $($probeResult.failure_code), expected $expected. Structural evidence is retained." }
            if ($action -eq 'complete') { Check-Clone (Join-Path $run 'clone') $before }
            Write-Host "PASS $phase; encrypted relay connections drained; destination/phase results verified"
        }
    } else {
    $phase = 'complete-clone'
    $complete = Invoke-Clone $target
    Check-Clone $target $before
    $receiving = [regex]::Matches($complete, '(?m)^Clone Receiving: ([0-9]+) / Some\(([0-9]+)\) objects')
    if ($receiving.Count -eq 0) { throw 'Clone emitted no Receiving object progress.' }
    $evidence.progress = [ordered]@{receiving_events = $receiving.Count; checkout_events = [regex]::Matches($complete, '(?m)^Clone checkout:').Count; phases_present = $true}
    if (@(Get-ChildItem -LiteralPath $scratch -Directory -Force -Filter '.rocket-clone-*').Count -ne 0) { throw 'Successful clone left a staging workspace.' }
    $phase = 'existing-destination'
    $indexBefore = (Get-FileHash -LiteralPath (Join-Path $target '.git\index')).Hash
    Invoke-Clone $target 'clone_destination_exists' | Out-Null
    if ((Get-FileHash -LiteralPath (Join-Path $target '.git\index')).Hash -cne $indexBefore) { throw 'Existing destination index changed.' }
    Check-Clone $target $before

    if ($SelfTest) {
        $phase = 'local-command-rejected'
        $notGit = Join-Path $scratch 'not-a-repository'
        [IO.Directory]::CreateDirectory($notGit) | Out-Null
        Invoke-Clone (Join-Path $scratch 'local-rejected') 'repository_unavailable' $notGit | Out-Null
    } elseif ($Transport -eq 'Https') {
        $phase = 'credentials-disabled'
        Set-Authentication $false
        Invoke-Clone (Join-Path $scratch 'credentials-rejected') 'credentials_unavailable|authentication_rejected' | Out-Null
    } else {
        $phase = 'host-untrusted'
        Set-Authentication $true $false
        Invoke-Clone (Join-Path $scratch 'host-rejected') 'ssh_host_verification_failed' | Out-Null
        $phase = 'identity-disabled'
        Set-Authentication $false
        Invoke-Clone (Join-Path $scratch 'identity-rejected') 'ssh_authentication_rejected' | Out-Null
    }
    $phase = 'authenticated-after-rejections'
    Set-Authentication $true
    Invoke-Clone $repeatTarget | Out-Null
    Check-Clone $repeatTarget $before
    Check-Clone $target $before
    }
    $phase = 'remote-after'
    $after = Read-RemoteState
    $evidence.remote_after = $after
    if (($before | ConvertTo-Json -Compress) -cne ($after | ConvertTo-Json -Compress)) { throw 'Remote refs/default HEAD changed during testing; exact comparison is inconclusive.' }
    if (-not $SelfTest -and $Transport -eq 'Ssh' -and (Get-FileHash -LiteralPath $resolvedPublicKey -Algorithm SHA256).Hash -cne $publicHashBefore) { throw 'Selected public-key file changed during the run.' }

    $phase = 'diagnostics'
    $records = @(); $logText = ''
    foreach ($file in @(Get-ChildItem -LiteralPath $logs -File -Filter '*.jsonl')) {
        $logText += [IO.File]::ReadAllText($file.FullName)
        foreach ($line in [IO.File]::ReadAllLines($file.FullName)) { $records += ($line | ConvertFrom-Json -ErrorAction Stop) }
    }
    if ($records.Count -eq 0 -or @($records | Where-Object { $_.schema -ne 1 -or $_.dropped_before -ne 0 }).Count) { throw 'Missing, invalid, or dropped diagnostic records.' }
    foreach ($start in @($records | Where-Object { $_.event -in @('request_start','step_start','git_start') })) {
        if (@($records | Where-Object { $_.event -ceq $start.event.Replace('_start','_end') -and $_.session -ceq $start.session -and $_.id -eq $start.id }).Count -ne 1) { throw 'Unpaired diagnostic span.' }
    }
    $requestEnds = @($records | Where-Object event -eq 'request_end')
    $successCount = if ($Interruptions) { 1 } else { 2 }
    if ($requestEnds.Count -ne $requests.Count -or @($requestEnds | Where-Object outcome -eq 'ok').Count -ne $successCount) { throw 'Request outcomes do not match acceptance results.' }
    if ($Interruptions -and @($requestEnds | Where-Object outcome -eq 'cancelled').Count -ne $(if ($Transport -eq 'Ssh') { 1 } else { 0 })) { throw 'Cancellation was not distinguished from connection failure.' }
    foreach ($request in $requests) {
        if ($request.failure_code -and @($requestEnds | Where-Object failure_code -eq $request.failure_code).Count -ne 1) { throw 'Expected failure code missing or duplicated in request logs.' }
    }
    $outcomes = @($records | Where-Object event -eq 'clone_outcome')
    $commandFailures = @($requests | Where-Object { $_.expected_failure -and $_.expected_failure -ne 'clone_destination_exists' }).Count
    if ($outcomes.Count -ne ($successCount + $commandFailures) -or @($records | Where-Object { $_.event -eq 'git_start' -and $_.operation -eq 'clone' }).Count -ne $outcomes.Count) { throw 'Clone attempts are missing or unexpectedly retried.' }
    if (@($outcomes | Where-Object { $_.clone_command -eq 'succeeded' -and $_.verification -eq 'succeeded' -and $_.publication -eq 'succeeded' -and $_.reopen -eq 'succeeded' -and $_.cleanup -eq 'succeeded' -and -not $_.error_diagnostics }).Count -ne $successCount) { throw 'Successful clone phase outcomes are incomplete.' }
    if (@($outcomes | Where-Object { $_.clone_command -eq 'failed' -and $_.verification -eq 'skipped' -and $_.publication -eq 'skipped' -and $_.reopen -eq 'skipped' -and $_.cleanup -eq 'skipped' }).Count -ne $commandFailures) { throw 'Rejected clone phase outcomes are incomplete.' }
    $parents = if ($Interruptions) { @($actions | ForEach-Object { Join-Path $scratch $_ }) } else { @($scratch) }
    $retained = @($parents | ForEach-Object { Get-ChildItem -LiteralPath $_ -Directory -Force -Filter '.rocket-clone-*' } | ForEach-Object FullName)
    if ($retained.Count -ne $commandFailures) { throw 'Failed workspace retention mismatch.' }
    foreach ($value in $privacyValues) { if ($logText.Contains($value)) { throw 'Private repository metadata or credential-shaped content entered diagnostics.' } }
    $evidence.diagnostic_records = $records.Count
    $evidence.dropped_events = 0
    $evidence.retained_workspaces = $retained
    $evidence.passed = $true
    Write-Host 'PASS clean clones, unchanged remote refs, explicit failure phases/guidance, recovery, and private diagnostics'
} catch {
    $evidence.failure = $_.Exception.Message
    throw
} finally {
    $evidence.finished_utc = [DateTime]::UtcNow.ToString('o')
    $evidence.final_phase = $phase
    $evidence.requests = @($requests.ToArray())
    $evidence.steps = @($steps.ToArray())
    $evidence | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $resultPath -Encoding UTF8
    Write-Host "Results: $resultPath"
}
Write-Host "GitHub clone acceptance passed ($mode). Diagnostics: $logs"
Write-Host "CLI transcript (local recovery paths; no raw Git output): $transcript"
