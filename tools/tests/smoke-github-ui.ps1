param(
    [ValidateSet('Https','Ssh')][string]$Transport = 'Https',
    [string]$Repository = 'JasXTD/rocket-auth-test',
    [string]$PublicKeyPath = 'C:\Users\Tommy\.ssh\id_ed25519_github.pub',
    [switch]$SelfTest
)
$ErrorActionPreference = 'Stop'
if (-not $SelfTest -and $Repository -cne 'JasXTD/rocket-auth-test') { throw 'This runner is scoped to JasXTD/rocket-auth-test.' }
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$probe = Join-Path $workspace 'target\release\examples\network_ui_probe.exe'
if (-not (Test-Path -LiteralPath $probe -PathType Leaf)) { throw 'Build the release network_ui_probe example offline and locked first.' }
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$runId = [guid]::NewGuid().ToString('N')
$mode = if ($SelfTest) { 'local' } else { $Transport.ToLowerInvariant() }
$scratch = Join-Path (Join-Path $workspace 'work') "smoke-github-ui-$mode-$runId"
$client = Join-Path $scratch 'ROCKET_PRIVATE_client'
$publisher = Join-Path $scratch 'ROCKET_PRIVATE_publisher'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'empty-hooks'
$emptyHome = Join-Path $scratch 'empty-home'
$emptyConfig = Join-Path $scratch 'empty.config'
$denyAskpass = Join-Path $scratch 'deny-askpass.cmd'
$resultPath = Join-Path $scratch 'result.json'
$sshConfig = Join-Path $scratch 'ssh.config'
$knownHosts = Join-Path $scratch 'known_hosts'
$publicIdentity = Join-Path $scratch 'ROCKET_PRIVATE_identity.pub'
$branch = "rocket-ui-test/$mode-$runId"
$reference = "refs/heads/$branch"
$tracking = "refs/remotes/origin/$branch"
$remote = if ($SelfTest) { Join-Path $scratch 'upstream.git' } elseif ($Transport -eq 'Https') { "https://github.com/$Repository.git" } else { "git@github.com:$Repository.git" }
$utf8 = [Text.UTF8Encoding]::new($false)
foreach ($path in @($client,$publisher,$hooks,$emptyHome)) { [IO.Directory]::CreateDirectory($path) | Out-Null }
[IO.File]::WriteAllText($emptyConfig,'',$utf8)
[IO.File]::WriteAllText($denyAskpass,"@exit /b 1`r`n",[Text.Encoding]::ASCII)
$phase = 'setup'
$failureHint = 'Check the fixture tools and configuration.'
$remoteWriteAttempted = $false
$cleanupAllowed = $false
$testsPassed = $false
$cleanupVerified = $false
$ownedCommits = New-Object 'System.Collections.Generic.List[string]'
$runs = New-Object 'System.Collections.Generic.List[object]'
$steps = New-Object 'System.Collections.Generic.List[object]'
$evidence = [ordered]@{ passed=$false; mode=$mode; branch=$reference; repository=$Repository; cleanup='not-needed'; started_utc=[DateTime]::UtcNow.ToString('o'); probe_sha256=(Get-FileHash -LiteralPath $probe).Hash }
# Rechecked against GitHub's official publication on 2026-10-15; no keyscan or trust bypass.
$hostKeySource = 'https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints'
$hostKeyEntry = 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl'
$hostFingerprint = 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU'
$sshDir = Join-Path $env:WINDIR 'System32\OpenSSH'
$sshPath = Join-Path $sshDir 'ssh.exe'
$keygenPath = Join-Path $sshDir 'ssh-keygen.exe'
$sshAddPath = Join-Path $sshDir 'ssh-add.exe'
function Stop-Check([string]$Message) { $script:failureHint=$Message; throw 'Native UI transport check failed' }

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



function Save-Checkpoint {
    $evidence.phase=$phase
    $evidence.owned_commits=$ownedCommits.ToArray()
    $evidence.ui_runs=$runs.ToArray()
    $evidence.harness_steps=$steps.ToArray()
    [IO.File]::WriteAllText($resultPath,($evidence | ConvertTo-Json -Depth 8),$utf8)
}
function Get-RemoteState {
    $text=Invoke-Git @('ls-remote','--refs','--','origin')
    [string[]]$refs=@($text -split "`n" | Where-Object { $_ -ne '' })
    foreach ($line in $refs) { if ($line -cnotmatch '\A(?:[a-f0-9]{40}|[a-f0-9]{64})\trefs/\S+\z') { Stop-Check 'Unexpected remote ref output.' } }
    [Array]::Sort($refs,[StringComparer]::Ordinal)
    [pscustomobject]@{ refs=$refs; head=(Invoke-Git @('ls-remote','--symref','--','origin','HEAD')) }
}
function Get-TestTip($State) {
    $found=@($State.refs | Where-Object { $_.EndsWith("`t$reference",[StringComparison]::Ordinal) })
    if ($found.Count -gt 1) { Stop-Check 'Duplicate remote test branch.' }
    if ($found.Count -eq 1) { return ($found[0] -split "`t",2)[0] }
    return ''
}
function Assert-Remote($State,[string]$Expected) {
    if ((Get-TestTip $State) -cne $Expected) { Stop-Check 'The remote test branch does not have the expected commit.' }
    $others=@($State.refs | Where-Object { -not $_.EndsWith("`t$reference",[StringComparison]::Ordinal) }) -join "`n"
    if ($others -cne ($script:beforeRemote.refs -join "`n") -or $State.head -cne $script:beforeRemote.head) { Stop-Check 'Other remote refs or default HEAD changed; they will not be modified by this test.' }
}
function Get-LocalState {
    [ordered]@{
        head=Invoke-Git @('rev-parse','HEAD')
        branches=Invoke-Git @('for-each-ref','--format=%(refname) %(objectname)','refs/heads/','refs/tags/')
        index=(Get-FileHash -LiteralPath (Join-Path $client '.git\index')).Hash
        files=@(Get-ChildItem -LiteralPath $client -File | Sort-Object Name | ForEach-Object { [ordered]@{name=$_.Name;sha256=(Get-FileHash -LiteralPath $_.FullName).Hash} })
    } | ConvertTo-Json -Depth 4 -Compress
}
function Assert-Local([string]$Expected) { if ((Get-LocalState) -cne $Expected) { Stop-Check 'Local HEAD, index, branch refs, or file bytes changed unexpectedly.' } }
function Assert-Tip([string]$Head,[string]$RemoteTip,[string]$Counts) {
    if ((Invoke-Git @('rev-parse','HEAD')) -cne $Head -or (Invoke-Git @('rev-parse',$tracking)) -cne $RemoteTip) { Stop-Check 'Local HEAD or tracking ref differs from the expected commit.' }
    if (((Invoke-Git @('rev-list','--left-right','--count',"HEAD...$tracking")) -replace '\s+',' ') -cne $Counts) { Stop-Check 'Unexpected ahead/behind counts.' }
}
function Invoke-Ui([string]$Operation,[string]$FailurePattern='', [switch]$Upstream, [string]$PullKind='') {
    Save-Checkpoint
    $directory=Join-Path $scratch ('ui-{0:00}-{1}' -f ($runs.Count+1),$phase)
    $arguments=@($client,$directory,$Operation)
    if ($Upstream) { $arguments += 'set-upstream' }
    $processResult=Invoke-Isolated $probe $arguments 'rocket-native-ui'
    $resultFile=Join-Path $directory 'result.json'
    if ($processResult.exit -ne 0 -or -not (Test-Path -LiteralPath $resultFile)) {
        if ($SelfTest -and $processResult.stderr -notmatch 'gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|-----BEGIN [^-]*PRIVATE KEY-----') {
            # File-only self-test contains generated data, no credentials or private remote.
            [IO.File]::WriteAllText((Join-Path $directory 'probe.stderr.log'),$processResult.stderr,$utf8)
        }
        # Live stderr is not persisted: it can contain private repository or helper text.
        if ($processResult.stderr -match 'network_ui_probe\.rs:\d+:\d+') { $evidence.probe_failure_location=$Matches[0] }
        Stop-Check "The native UI probe did not finish (exit $($processResult.exit)); review $directory."
    }
    $result=Get-Content -LiteralPath $resultFile -Raw | ConvertFrom-Json
    $detailText=$result.details -join "`n"
    $failure=if ($detailText -match '\[([a-z_]+)\]') { $Matches[1] } else { '' }
    $runs.Add([ordered]@{phase=$phase;operation=$Operation;directory=$directory;failure_code=$failure;expected_failure=$FailurePattern;pull_kind=$PullKind;command_succeeded=$result.command_succeeded;elapsed_ms=$result.elapsed_ms})
    if (-not $result.ui_completed -or $result.operation -cne $Operation -or $result.activity_count -ne 1 -or -not $result.console_visible -or -not $result.refresh_ok) { Stop-Check 'The UI omitted its result, refreshed data, or visible console, or ran more than one operation.' }
    if ($FailurePattern) {
        if ($result.command_succeeded -or $failure -cnotmatch "\A($FailurePattern)\z" -or -not $detailText.Contains('Next: ')) { Stop-Check "The UI did not show expected failure $FailurePattern with recovery guidance (observed $failure)." }
    } elseif (-not $result.command_succeeded -or $result.details.Count -ne 0) { Stop-Check 'The UI did not confirm a clean successful operation.' }
    foreach ($name in @('dialog.bmp','result.bmp')) { if (-not (Test-Path -LiteralPath (Join-Path $directory $name))) { Stop-Check 'A native rendered capture is missing.' } }
    if (($result.summary + $detailText) -match 'gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|(?im)Authorization:\s*(Basic|Bearer)\s+\S+|-----BEGIN [^-]*PRIVATE KEY-----') { Stop-Check 'A secret-shaped value reached the visible UI result.' }
    Write-Host "PASS $mode UI $phase"
}
function Invoke-LeasedWrite([string]$NewTip,[string]$ExpectedTip) {
    if (-not $cleanupAllowed -or $reference -cne "refs/heads/rocket-ui-test/$mode-$runId" -or ($NewTip -and -not $ownedCommits.Contains($NewTip)) -or ($ExpectedTip -and -not $ownedCommits.Contains($ExpectedTip))) { Stop-Check 'Fixture write refused: ref or expected commit is outside this run.' }
    $script:remoteWriteAttempted=$true
    Save-Checkpoint
    Invoke-Isolated $gitPath @('-C',$publisher,'-c','remote.origin.mirror=false','-c','push.followTags=false','-c','push.autoSetupRemote=false','-c','push.negotiate=false',
        'push','--porcelain','--no-follow-tags','--no-prune','--no-recurse-submodules',"--force-with-lease=${reference}:$ExpectedTip",'--','origin',"${NewTip}:$reference") 'git-fixture-write'
}
function Add-Commit([string]$Directory,[string]$Name,[string]$Contents) {
    [IO.File]::WriteAllText((Join-Path $Directory $Name),"$Contents`n",$utf8)
    Invoke-Git @('add','--',$Name) -At $Directory | Out-Null
    Invoke-Git @('commit','-m','Rocket synthetic UI transport fixture') -At $Directory | Out-Null
    $tip=Invoke-Git @('rev-parse','HEAD') -At $Directory
    $ownedCommits.Add($tip)
    $tip
}
function Sync-Publisher {
    Invoke-Git @('fetch','--no-tags','--no-prune','--no-recurse-submodules','--','origin',"${reference}:$tracking") -At $publisher | Out-Null
    Invoke-Git @('merge','--ff-only','--no-edit',$tracking) -At $publisher | Out-Null
}
function Assert-Diagnostics {
    $total=0
    foreach ($run in $runs) {
        $records=@();$text=''
        foreach ($file in Get-ChildItem -LiteralPath (Join-Path $run.directory 'logs') -Filter '*.jsonl' -File) {
            $text += [IO.File]::ReadAllText($file.FullName)
            $records += @([IO.File]::ReadAllLines($file.FullName) | ForEach-Object { $_ | ConvertFrom-Json })
        }
        if (-not $records.Count -or @($records | Where-Object { $_.schema -ne 1 -or $_.dropped_before -ne 0 }).Count) { Stop-Check 'Missing, invalid, or dropped diagnostic events.' }
        if ($text -match 'ROCKET_PRIVATE_|rocket-ui-test|refs/heads/|refs/remotes/|github\.com|gh[pousr]_[A-Za-z0-9]{16,}|github_pat_|(?im)Authorization:\s*(Basic|Bearer)\s+\S+|-----BEGIN [^-]*PRIVATE KEY-----') { Stop-Check 'Private fixture metadata or a secret format reached diagnostics.' }
        foreach ($value in @($Repository,$scratch,$hostFingerprint)+$ownedCommits.ToArray()) { if ($text.Contains($value)) { Stop-Check 'A private fixture value reached diagnostics.' } }
        foreach ($start in @($records | Where-Object { $_.event -in @('request_start','step_start','git_start') })) {
            $ends=@($records | Where-Object { $_.event -ceq $start.event.Replace('_start','_end') -and $_.session -ceq $start.session -and $_.id -eq $start.id })
            if ($ends.Count -ne 1) { Stop-Check 'An operation span is incomplete or duplicated.' }
        }
        if (@($records | Where-Object { $_.event -ceq 'request_start' -and $_.operation -ceq $run.operation }).Count -ne 1) { Stop-Check 'Expected exactly one UI operation request in the log.' }
        if ($run.failure_code -and @($records | Where-Object { $_.failure_code -ceq $run.failure_code }).Count -eq 0) { Stop-Check 'The displayed failure has no matching diagnostic failure code.' }
        $operationCommands=@($records | Where-Object { $_.event -ceq 'git_start' -and $_.operation -ceq $(if ($run.operation -eq 'pull') {'fetch'} else {$run.operation}) })
        $expectedCommands=if ($run.failure_code -eq 'pull_dirty_worktree') {0} else {1}
        if ($operationCommands.Count -ne $expectedCommands) { Stop-Check 'Missing operation command or unexpected automatic retry.' }
        if ($run.pull_kind) {
            $outcome=@($records | Where-Object { $_.event -ceq 'pull_outcome' })
            if ($outcome.Count -ne 1 -or $outcome[0].update_kind -cne $run.pull_kind -or $outcome[0].verification -cne 'ok' -or $outcome[0].fetch_outcome -cne 'succeeded') { Stop-Check 'Pull phase outcomes do not confirm the expected update.' }
        }
        $total += $records.Count
    }
    $evidence.diagnostic_records=$total
    $evidence.diagnostic_privacy_scan_passed=$true
    $evidence.diagnostic_dropped=0
    Write-Host 'PASS visible guidance, paired diagnostic spans, failure codes, no retries, and log privacy'
}
try {
    foreach ($directory in @($client,$publisher)) {
        Invoke-Git @('init',"--initial-branch=$branch",'--object-format=sha1') -At $directory | Out-Null
        foreach ($setting in @(
            @('user.name','Rocket UI Test'),@('user.email','rocket@example.invalid'),@('core.hooksPath',$hooks),@('core.autocrlf','false'),@('core.fsmonitor','false'),
            @('commit.gpgsign','false'),@('push.gpgsign','false'),@('credential.interactive','false'),@('credential.helper',''),
            @('http.sslBackend','schannel'),@('http.sslVerify','true'),@('http.followRedirects','false'),@('http.extraHeader','')
        )) { Invoke-Git @('config',$setting[0],$setting[1]) -At $directory | Out-Null }
        Invoke-Git @('remote','add','origin',$remote) -At $directory | Out-Null
        Invoke-Git @('config','remote.origin.fetch',"+${reference}:$tracking") -At $directory | Out-Null
    }
    [IO.File]::WriteAllText((Join-Path $client '.git\rocket-ui-fixture'),$branch,$utf8)
    $base=Add-Commit $client 'ROCKET_PRIVATE_file' "Synthetic UI base $runId"
    if ($SelfTest) {
        Invoke-Git @('init','--bare','--initial-branch=main',$remote) | Out-Null
        Invoke-Git @('config','core.hooksPath',$hooks) -At $remote | Out-Null
        Invoke-Git @('push','--','origin','HEAD:refs/heads/main') | Out-Null
    } elseif ($Transport -eq 'Ssh') { $phase='ssh-agent-preflight'; Initialize-Ssh }
    Set-Authentication $true
    if (-not $SelfTest -and $Transport -eq 'Ssh') {
        $pin=Invoke-Isolated $keygenPath @('-lf',$knownHosts) 'host-key-fingerprint'
        if ($pin.exit -ne 0 -or -not $pin.stdout.Contains($hostFingerprint)) { Stop-Check 'The pinned host key differs from the reviewed GitHub fingerprint.' }
        $evidence.host_key_source=$hostKeySource
    }
    $phase='remote-preflight'
    $failureHint='Read-only access failed. Check your existing Git Credential Manager login or load id_ed25519_github in the Windows SSH agent locally.'
    $script:beforeRemote=Get-RemoteState
    if (-not $beforeRemote.refs.Count -or -not $beforeRemote.head) { Stop-Check 'Use an initialized test repository.' }
    if (Get-TestTip $beforeRemote) { Stop-Check 'Generated test branch already exists; it will not be reused.' }
    $cleanupAllowed=$true
    $phase='push-new-branch'
    $remoteWriteAttempted=$true
    Invoke-Ui 'push' -Upstream
    Assert-Remote (Get-RemoteState) $base
    if ((Invoke-Git @('rev-parse','--symbolic-full-name','@{upstream}')) -cne $tracking) { Stop-Check 'The UI upstream option was not applied.' }
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_untracked'),"Preserve this untracked fixture $runId`n",$utf8)
    $untrackedHash=(Get-FileHash -LiteralPath (Join-Path $client 'ROCKET_PRIVATE_untracked')).Hash
    $beforeLocal=Get-LocalState
    if (-not $SelfTest) {
        Set-Authentication $false
        $authPattern=if ($Transport -eq 'Https') {'credentials_unavailable|authentication_rejected'} else {'ssh_authentication_rejected'}
        foreach ($operation in @('fetch','pull','push')) {
            $phase="$operation-authentication-unavailable"
            Invoke-Ui $operation $authPattern
            Assert-Local $beforeLocal
        }
        Set-Authentication $true
        Assert-Remote (Get-RemoteState) $base
    }
    $phase='fetch-up-to-date'; Invoke-Ui 'fetch'; Assert-Local $beforeLocal
    Invoke-Git @('fetch','--no-tags','--no-prune','--no-recurse-submodules','--','origin',"${reference}:$tracking") -At $publisher | Out-Null
    Invoke-Git @('switch','--create',$branch,'--no-track',$tracking) -At $publisher | Out-Null
    $phase='publish-first-advance'
    $advance=Add-Commit $publisher 'ROCKET_PRIVATE_file' "Synthetic remote advance $runId"
    $published=Invoke-LeasedWrite $advance $base
    if ($published.exit -ne 0) { Stop-Check 'Fixture publisher did not confirm its advance.' }
    Assert-Remote (Get-RemoteState) $advance
    $phase='fetch-new-commit'; Invoke-Ui 'fetch'; Assert-Local $beforeLocal; Assert-Tip $base $advance '0 1'
    $phase='pull-fast-forward'; Invoke-Ui 'pull' -PullKind 'fast_forward'; Assert-Tip $advance $advance '0 0'
    if ((Get-FileHash -LiteralPath (Join-Path $client 'ROCKET_PRIVATE_untracked')).Hash -cne $untrackedHash) { Stop-Check 'Pull changed the untracked sentinel.' }
    if ([IO.File]::ReadAllText((Join-Path $client 'ROCKET_PRIVATE_file')) -cne "Synthetic remote advance $runId`n") { Stop-Check 'Pull did not update working file contents.' }
    $phase='pull-up-to-date'; $beforeLocal=Get-LocalState; Invoke-Ui 'pull' -PullKind 'up_to_date'; Assert-Local $beforeLocal
    $clientAdvance=Add-Commit $client 'ROCKET_PRIVATE_client_only' "Synthetic client advance $runId"
    $phase='push-fast-forward'; $beforeLocal=Get-LocalState; Invoke-Ui 'push'; Assert-Local $beforeLocal; Assert-Remote (Get-RemoteState) $clientAdvance
    $phase='push-up-to-date'; Invoke-Ui 'push'; Assert-Local $beforeLocal; Assert-Remote (Get-RemoteState) $clientAdvance
    $phase='publish-divergence'; Sync-Publisher
    $diverged=Add-Commit $publisher 'ROCKET_PRIVATE_remote_only' "Synthetic divergence $runId"
    $published=Invoke-LeasedWrite $diverged $clientAdvance
    if ($published.exit -ne 0) { Stop-Check 'Fixture publisher did not confirm the divergence setup.' }
    Assert-Remote (Get-RemoteState) $diverged
    $localAhead=Add-Commit $client 'ROCKET_PRIVATE_client_only' "Synthetic competing commit $runId"
    $beforeLocal=Get-LocalState
    $phase='push-fetch-first'; Invoke-Ui 'push' 'push_fetch_first'; Assert-Local $beforeLocal; Assert-Remote (Get-RemoteState) $diverged
    $phase='fetch-diverged'; Invoke-Ui 'fetch'; Assert-Local $beforeLocal; Assert-Tip $localAhead $diverged '1 1'
    $phase='push-non-fast-forward'; Invoke-Ui 'push' 'push_non_fast_forward'; Assert-Local $beforeLocal; Assert-Remote (Get-RemoteState) $diverged
    $phase='pull-diverged'; Invoke-Ui 'pull' 'pull_diverged'; Assert-Local $beforeLocal
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_file'),"staged fixture bytes`n",$utf8)
    Invoke-Git @('add','--','ROCKET_PRIVATE_file') | Out-Null
    [IO.File]::WriteAllText((Join-Path $client 'ROCKET_PRIVATE_file'),"unstaged fixture bytes`n",$utf8)
    $phase='pull-dirty-worktree'; $beforeLocal=Get-LocalState; Invoke-Ui 'pull' 'pull_dirty_worktree'; Assert-Local $beforeLocal
    if ($SelfTest) {
        $phase='stale-cleanup-lease'
        $stale=Invoke-LeasedWrite '' $base
        if ($stale.exit -eq 0) { Stop-Check 'The stale cleanup lease unexpectedly succeeded.' }
        Assert-Remote (Get-RemoteState) $diverged
        $evidence.stale_cleanup_lease_rejected=$true
    }
    $testsPassed=$true
} catch {
    $evidence.failure_phase=$phase; $evidence.failure_hint=$failureHint; $evidence.failure_line=$_.InvocationInfo.ScriptLineNumber; $evidence.failure_type=$_.Exception.GetType().Name
} finally {
    if ($cleanupAllowed -and $remoteWriteAttempted) {
        $phase='cleanup'; $evidence.cleanup='needs-verification'
        try {
            Set-Authentication $true
            $tip=Get-TestTip (Get-RemoteState)
            if ($tip) {
                if (-not $ownedCommits.Contains($tip)) { Stop-Check 'Cleanup refused: an unowned commit is now on the test branch; branch preserved.' }
                $deleted=Invoke-LeasedWrite '' $tip
                $evidence.cleanup_command_exit=$deleted.exit
            }
            Assert-Remote (Get-RemoteState) ''
            $cleanupVerified=$true; $evidence.cleanup='confirmed-absent'; $evidence.original_refs_and_default_head_unchanged=$true
            Write-Host 'PASS temporary branch removed; original refs and default HEAD unchanged'
        } catch { $evidence.cleanup_hint=$failureHint; $evidence.cleanup_failure_line=$_.InvocationInfo.ScriptLineNumber }
    } elseif (-not $remoteWriteAttempted) { $cleanupVerified=$true }
    $logsPassed=$false
    if ($testsPassed) {
        try {
            $phase='diagnostics'; Assert-Diagnostics
            if ($resolvedPublicKey -and (Get-FileHash -LiteralPath $resolvedPublicKey).Hash -cne $publicHashBefore) { Stop-Check 'The selected public key changed.' }
            $logsPassed=$true
        } catch { $evidence.diagnostics_hint=$failureHint; $evidence.diagnostics_failure_line=$_.InvocationInfo.ScriptLineNumber }
    }
    $evidence.passed=$testsPassed -and $cleanupVerified -and $logsPassed
    $phase=if ($evidence.passed) {'complete'} else {'stopped'}
    $evidence.finished_utc=[DateTime]::UtcNow.ToString('o')
    Save-Checkpoint
    # Fixture-only native captures. These can show the synthetic branch and local paths.
    Add-Type -AssemblyName System.Drawing
    foreach ($file in Get-ChildItem -LiteralPath $scratch -Filter '*.bmp' -Recurse -File) {
        $bitmap=[Drawing.Image]::FromFile($file.FullName)
        try { $bitmap.Save([IO.Path]::ChangeExtension($file.FullName,'.png'),[Drawing.Imaging.ImageFormat]::Png) } finally { $bitmap.Dispose() }
    }
}
if (-not $evidence.passed) { throw "Native UI transport checks stopped. Inspect $resultPath" }
Write-Host "Native UI transport checks passed. Results: $resultPath"
