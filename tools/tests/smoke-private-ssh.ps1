param(
    [string]$RemoteUrl = 'git@github.com:JasXTD/rocket-auth-test.git',
    [string]$PublicKeyPath = 'C:\Users\Tommy\.ssh\id_ed25519_github.pub'
)

$ErrorActionPreference = 'Stop'
if ($RemoteUrl -notmatch '\Agit@github[.]com:[A-Za-z0-9-]+/[A-Za-z0-9_.-]+\z') {
    throw 'Use a plain git@github.com:OWNER/REPOSITORY.git URL.'
}
if (-not $RemoteUrl.EndsWith('.git')) { $RemoteUrl += '.git' }
if (-not $PublicKeyPath.EndsWith('.pub', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Select the public .pub file. This test does not load a private-key file.'
}
$PublicKeyPath = (Resolve-Path -LiteralPath $PublicKeyPath -ErrorAction Stop).ProviderPath
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$sshDirectory = Join-Path $env:WINDIR 'System32\OpenSSH'
$sshPath = Join-Path $sshDirectory 'ssh.exe'
$keygenPath = Join-Path $sshDirectory 'ssh-keygen.exe'
$sshAddPath = Join-Path $sshDirectory 'ssh-add.exe'
foreach ($program in @($sshPath, $keygenPath, $sshAddPath)) {
    if (-not (Test-Path -LiteralPath $program -PathType Leaf)) { throw 'Windows OpenSSH tools are required' }
}
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-private-ssh-' + [guid]::NewGuid().ToString('N'))
$repository = Join-Path $scratch 'ROCKET_PRIVATE_client'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'empty-hooks'
$transcript = Join-Path $scratch 'cli-output.log'
$resultPath = Join-Path $scratch 'result.json'
$emptyConfig = Join-Path $scratch 'empty.config'
$sshConfig = Join-Path $scratch 'ssh.config'
$knownHosts = Join-Path $scratch 'known_hosts'
$publicIdentity = Join-Path $scratch 'ROCKET_PRIVATE_identity.pub'
$denyAskpass = Join-Path $scratch 'deny-askpass.cmd'
[IO.Directory]::CreateDirectory($repository) | Out-Null
[IO.Directory]::CreateDirectory($hooks) | Out-Null
[IO.File]::WriteAllText($emptyConfig, '')
[IO.File]::WriteAllText($knownHosts, '')
[IO.File]::WriteAllText($denyAskpass, "@exit /b 1`r`n", [Text.Encoding]::ASCII)

# Reviewed against GitHub's official fingerprint page on 2026-10-14. A future
# key rotation requires reviewing that source and updating this pin explicitly.
# Never replace this with an unverified ssh-keyscan or disable host checking.
$hostKeySource = 'https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints'
$hostKeyEntry = 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl'
$hostFingerprint = 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU'
$runs = New-Object 'System.Collections.Generic.List[object]'
$evidence = [ordered]@{
    passed = $false
    started_utc = [DateTime]::UtcNow.ToString('o')
    remote = $RemoteUrl
    repository = $repository
    authentication = 'Windows SSH agent; selected public key only; no interactive prompts'
    host_key_source = $hostKeySource
    pinned_host_fingerprint = $hostFingerprint
    checkout = $false
}

function Invoke-Program {
    param([string]$Program, [string[]]$Arguments)
    $previousAction = $ErrorActionPreference
    try {
        # Windows PowerShell 5.1 wraps native stderr in ErrorRecord objects.
        $ErrorActionPreference = 'Continue'
        $output = @(& $Program @Arguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousAction }
    if ($actual -ne 0) { throw "Fixture tool failed ($actual): $Program`n$($output -join "`n")" }
    return $output
}

function Invoke-Git {
    param([string[]]$GitArguments)
    Invoke-Program $gitPath (@('-C', $repository) + $GitArguments)
}

function Quote-ShellArgument {
    param([string]$Value)
    # core.sshCommand is interpreted by Git's shell. Quote paths as shell
    # literals; do not interpolate raw filesystem paths into command text.
    $singleQuote = [string][char]39
    $escapedQuote = $singleQuote + [char]34 + $singleQuote + [char]34 + $singleQuote
    return $singleQuote + $Value.Replace($singleQuote, $escapedQuote) + $singleQuote
}

function Quote-SshPath {
    param([string]$Value)
    if ($Value -match '[\r\n"]' -or $Value.Contains('${')) { throw 'Unsupported characters in an SSH configuration path' }
    return [string][char]34 + $Value.Replace('\', '/').Replace('%', '%%') + [char]34
}

function Set-FixtureSsh {
    param([bool]$UseAgent)
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
    if ($UseAgent) {
        # No corresponding private key exists in the fixture. SSH selects the
        # matching key from the Windows agent using this public-key-only copy.
        $configuration += "`n    IdentityFile $(Quote-SshPath $publicIdentity)`n"
    } else {
        $configuration += "`n    IdentityAgent none`n    IdentityFile none`n"
    }
    [IO.File]::WriteAllText($sshConfig, $configuration)
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
    if ($actual -ne $ExpectedExit) { throw "Unexpected inspect exit $actual during $Phase (expected $ExpectedExit). No automatic retry. See $transcript" }
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
    if (@(Get-ChildItem -LiteralPath $repository -Force | Where-Object { $_.Name -ne '.git' }).Count -ne 2) { throw 'Unexpected worktree contents' }
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

$names = @('ROCKET_LOG', 'ROCKET_LOG_DIR', 'GIT_CONFIG_GLOBAL', 'GIT_CONFIG_SYSTEM',
    'GIT_CONFIG_NOSYSTEM', 'GIT_TEMPLATE_DIR', 'GIT_TERMINAL_PROMPT', 'GIT_ASKPASS',
    'GIT_ALLOW_PROTOCOL', 'GIT_OPTIONAL_LOCKS', 'GIT_NO_LAZY_FETCH', 'GIT_NO_REPLACE_OBJECTS',
    'GIT_SSH_VARIANT', 'SSH_AUTH_SOCK', 'SSH_ASKPASS', 'SSH_ASKPASS_REQUIRE')
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
    $env:GIT_ALLOW_PROTOCOL = 'ssh'
    $env:GIT_OPTIONAL_LOCKS = '0'
    $env:GIT_NO_LAZY_FETCH = '1'
    $env:GIT_NO_REPLACE_OBJECTS = '1'
    $env:GIT_SSH_VARIANT = 'ssh'
    $env:SSH_ASKPASS = $denyAskpass
    $env:SSH_ASKPASS_REQUIRE = 'never'
    # With SSH_AUTH_SOCK unset, Windows OpenSSH uses its Windows agent service.
    # No private-key file or passphrase is read or copied by this script.
    $publicText = [IO.File]::ReadAllText($PublicKeyPath).Trim()
    if ($publicText -notmatch '\Assh-ed25519 [A-Za-z0-9+/]+={0,2}(?: [^\r\n]*)?\z') { throw 'Expected one Ed25519 public-key line in the .pub file' }
    [IO.File]::WriteAllText($publicIdentity, $publicText + "`n")
    $identityDescription = (Invoke-Program $keygenPath @('-lf', $publicIdentity) | Out-String).Trim()
    $identityFingerprint = ($identityDescription -split '\s+')[1]
    $agentList = Invoke-Program $sshAddPath @('-l') | Out-String
    if (-not $agentList.Contains($identityFingerprint)) {
        throw 'The selected public key is not loaded in the Windows SSH agent. Load the corresponding private key with Windows ssh-add in your terminal, then rerun.'
    }
    $evidence['client_key_fingerprint'] = $identityFingerprint
    $evidence['git_version'] = (Invoke-Git @('--version') | Out-String).Trim()
    # ssh -V writes to stderr even on success. Convert PowerShell 5.1's
    # ErrorRecord wrapper to its message before formatting this metadata field.
    $evidence['ssh_version'] = ((Invoke-Program $sshPath @('-V') | ForEach-Object { $_.ToString() }) -join "`n").Trim()
    $evidence['inspector_sha256'] = (Get-FileHash -LiteralPath $inspector -Algorithm SHA256).Hash
    Invoke-Git @('init', '--initial-branch=main') | Out-Null
    Invoke-Git @('config', 'user.name', 'Rocket Private SSH Smoke') | Out-Null
    Invoke-Git @('config', 'user.email', 'rocket@example.invalid') | Out-Null
    Invoke-Git @('config', 'core.hooksPath', $hooks) | Out-Null
    Invoke-Git @('config', 'core.autocrlf', 'false') | Out-Null
    Invoke-Git @('config', 'commit.gpgsign', 'false') | Out-Null
    Invoke-Git @('config', 'gc.auto', '0') | Out-Null
    $sshCommand = (Quote-ShellArgument $sshPath.Replace('\', '/')) + ' -F ' + (Quote-ShellArgument $sshConfig.Replace('\', '/'))
    Invoke-Git @('config', 'core.sshCommand', $sshCommand) | Out-Null
    Set-FixtureSsh $false
    $tracked = Join-Path $repository 'ROCKET_PRIVATE_file'
    $untracked = Join-Path $repository 'ROCKET_PRIVATE_untracked'
    [IO.File]::WriteAllText($tracked, "base`n")
    Invoke-Git @('add', '--', 'ROCKET_PRIVATE_file') | Out-Null
    Invoke-Git @('commit', '-m', 'Local private SSH fixture base') | Out-Null
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
    if (-not $listed.Contains($RemoteUrl)) { throw 'Configured remote differs from the selected SSH URL' }
    $untrusted = Invoke-Inspect @('fetch', 'origin') -Phase 'untrusted-host' -ExpectedExit 1
    Assert-Refresh $untrusted
    Assert-LocalState
    if ($untrusted -notmatch '\[ssh_host_verification_failed\]') { throw 'Expected a host-verification rejection, not a network/configuration failure' }
    if (@(Invoke-Git @('for-each-ref', '--format=%(refname)', 'refs/remotes/')).Count -ne 0) { throw 'Rejected fetch created tracking refs' }
    $evidence['untrusted_host_rejected'] = $true
    Write-Host 'PASS untrusted host rejected with strict host-key checking; local state preserved'

    [IO.File]::WriteAllText($knownHosts, $hostKeyEntry + "`n", [Text.Encoding]::ASCII)
    $hostDescription = Invoke-Program $keygenPath @('-lf', $knownHosts) | Out-String
    if (-not $hostDescription.Contains($hostFingerprint)) { throw 'Pinned GitHub host-key fingerprint does not match the reviewed value' }
    $denied = Invoke-Inspect @('fetch', 'origin') -Phase 'identity-disabled' -ExpectedExit 1
    Assert-Refresh $denied
    Assert-LocalState
    if ($denied -notmatch '\[ssh_authentication_rejected\]') { throw 'Expected a public-key authentication rejection, not a network/configuration failure' }
    if (@(Invoke-Git @('for-each-ref', '--format=%(refname)', 'refs/remotes/')).Count -ne 0) { throw 'Rejected fetch created tracking refs' }
    $evidence['identity_disabled_rejected'] = $true
    Write-Host 'PASS missing client identity rejected; branches/status refreshed; local state preserved'

    Set-FixtureSsh $true
    $first = Invoke-Inspect @('fetch', 'origin') -Phase 'authenticated-initial'
    Assert-Fetch $first
    Assert-LocalState
    $firstRefs = @(Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/remotes/origin/'))
    if ($firstRefs.Count -eq 0 -or $first -notmatch 'kind: New,') { throw 'Expected new tracking refs; the private test repository needs at least one branch/commit' }
    $firstOids = @($firstRefs | ForEach-Object { ($_ -split ' ', 2)[1] })
    $fetchHead = [IO.File]::ReadAllText((Join-Path $repository '.git\FETCH_HEAD'))
    foreach ($oid in $firstOids) {
        if ((Invoke-Git @('cat-file', '-t', $oid)) -cne 'commit') { throw 'Fetched branch tip is not a commit object' }
        if ($fetchHead -notmatch ('(?m)^' + [regex]::Escape($oid) + '\t')) { throw 'Fetched tracking tip is absent from FETCH_HEAD' }
    }
    $evidence['first_remote_refs'] = $firstRefs
    $evidence['fetched_commit_count'] = [long](Invoke-Git @('rev-list', '--count', '--remotes=origin') | Out-String).Trim()
    Write-Host 'PASS authenticated Rocket SSH fetch using the selected Windows agent key'

    $second = Invoke-Inspect @('fetch', 'origin') -Phase 'authenticated-repeat'
    Assert-Fetch $second
    Assert-LocalState
    $secondRefs = @(Invoke-Git @('for-each-ref', '--format=%(refname) %(objectname)', 'refs/remotes/origin/'))
    if (($firstRefs -join "`n") -ceq ($secondRefs -join "`n")) {
        if (([regex]::Matches($second, 'kind: UpToDate,')).Count -ne $secondRefs.Count -or $second -match 'kind: (New|FastForward|Forced),') { throw 'Unchanged remote refs did not all report UpToDate' }
        $evidence['repeat_result'] = 'UpToDate'
    } else { $evidence['repeat_result'] = 'Remote changed between requests; up-to-date case not fully observed' }
    $evidence['second_remote_refs'] = $secondRefs
    Invoke-Inspect @('status') -Phase 'final-status' | Out-Null
    Assert-LocalState

    $logPath = Join-Path $logs 'rocket.jsonl'
    $logText = [IO.File]::ReadAllText($logPath)
    $remoteParts = $RemoteUrl.Substring('git@github.com:'.Length).Split('/')
    $secondOids = @($secondRefs | ForEach-Object { ($_ -split ' ', 2)[1] })
    foreach ($private in (@('ROCKET_PRIVATE', 'rocket@example.invalid', 'github.com', $beforeHead, $identityFingerprint, $hostFingerprint) + $remoteParts + $firstOids + $secondOids)) {
        if ($logText.IndexOf($private, [StringComparison]::OrdinalIgnoreCase) -ge 0) { throw 'Fixture identity or remote/key metadata leaked into diagnostic records' }
    }
    $records = @(Get-Content -LiteralPath $logPath | ForEach-Object { $_ | ConvertFrom-Json -ErrorAction Stop })
    foreach ($expected in @(@('write_result', 'ok', 2), @('write_result', 'git_error', 2), @('parse', 'ok', 2))) {
        if (@($records | Where-Object { $_.event -eq $expected[0] -and $_.operation -eq 'fetch' -and $_.outcome -eq $expected[1] }).Count -ne $expected[2]) { throw 'Unexpected diagnostic fetch outcome/parse counts' }
    }
    if (@($records | Where-Object { $_.event -eq 'refresh' -and $_.outcome -eq 'ok' }).Count -ne 8) { throw 'Expected eight successful reconciliation records' }
    if (@($records | Where-Object { $_.dropped_before -ne 0 }).Count -ne 0) { throw 'Diagnostic events were dropped' }
    if (([IO.File]::ReadAllText($knownHosts)) -cne ($hostKeyEntry + "`n")) { throw 'Fixture host trust changed during the test' }
    $evidence['fetch_git_events'] = @($records | Where-Object { $_.event -eq 'git_end' -and $_.operation -eq 'fetch' } | Select-Object elapsed_ms, outcome, exit_code, stdout_bytes, stderr_bytes)
    $evidence['diagnostic_records'] = $records.Count
    $evidence['dropped_diagnostic_records'] = 0
    $evidence['local_state_preserved'] = $true
    $evidence['passed'] = $true
    Write-Host 'PASS repeated authenticated SSH fetch, diagnostic privacy, and unchanged local HEAD/index/files'
    Write-Output "Private SSH smoke checks passed. Results: $resultPath"
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
