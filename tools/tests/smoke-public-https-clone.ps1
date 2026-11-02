param([switch]$SelfTest)

$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
$cancelProbe = Join-Path $workspace 'target\debug\examples\clone_cancel_probe.exe'
foreach ($binary in @($inspector, $cancelProbe)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect --example clone_cancel_probe'
    }
}
$gitPath = (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$mode = if ($SelfTest) { 'local' } else { 'https' }
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-public-clone-' + $mode + '-' + [guid]::NewGuid().ToString('N'))
$target = Join-Path $scratch 'ROCKET_PRIVATE_imgui with spaces'
$cancelled = Join-Path $scratch 'ROCKET_PRIVATE_cancelled'
$logs = Join-Path $scratch 'logs'
$emptyHome = Join-Path $scratch 'empty-home'
$config = Join-Path $scratch 'isolated.config'
$hooks = Join-Path $scratch 'empty-hooks'
$denyAskpass = Join-Path $scratch 'deny-askpass.cmd'
$eventsPath = Join-Path $scratch 'cli-events.jsonl'
$resultPath = Join-Path $scratch 'result.json'
$utf8 = [Text.UTF8Encoding]::new($false)
foreach ($directory in @($scratch, $emptyHome, $hooks)) { [IO.Directory]::CreateDirectory($directory) | Out-Null }
[IO.File]::WriteAllText($denyAskpass, "@exit /b 1`r`n", $utf8)
# These settings and environment overrides apply only to children. No saved
# configuration, credentials, credential store, SSH key, or agent is changed.
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
[protocol "file"]
    allow = always
'@, $utf8)
$source = if ($SelfTest) { Join-Path $scratch 'source' } else { 'https://github.com/ocornut/imgui.git' }
$sourceKind = if ($SelfTest) { '--local' } else { '--url' }
$phase = 'setup'
$runs = [Collections.Generic.List[object]]::new()
$evidence = [ordered]@{
    passed = $false; started_utc = [DateTime]::UtcNow.ToString('o'); transport = $mode
    source = $source; scratch = $scratch; destination = $target; cancelled_destination = $cancelled
    inspector_sha256 = (Get-FileHash -LiteralPath $inspector -Algorithm SHA256).Hash
    cancel_probe_sha256 = (Get-FileHash -LiteralPath $cancelProbe -Algorithm SHA256).Hash
    runner_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
    authentication = 'disabled'; remote_writes = $false
}

function Quote-WindowsArgument([string]$Value) {
    '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}

function Invoke-Isolated([string]$Executable, [string[]]$Arguments, [switch]$CaptureProgress) {
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
    $values = @{
        HOME = $emptyHome; USERPROFILE = $emptyHome; XDG_CONFIG_HOME = $emptyHome; CURL_HOME = $emptyHome
        GIT_CONFIG_GLOBAL = $config; GIT_CONFIG_SYSTEM = $config; GIT_CONFIG_NOSYSTEM = '1'
        GIT_TEMPLATE_DIR = $hooks; GIT_TERMINAL_PROMPT = '0'; GIT_ASKPASS = $denyAskpass
        SSH_ASKPASS = $denyAskpass; SSH_ASKPASS_REQUIRE = 'never'; GCM_INTERACTIVE = '0'
        GIT_ALLOW_PROTOCOL = $(if ($SelfTest) { 'file' } else { 'https' })
        GIT_NO_LAZY_FETCH = '1'; GIT_NO_REPLACE_OBJECTS = '1'; GIT_OPTIONAL_LOCKS = '0'
        ROCKET_LOG_DIR = $logs; LC_ALL = 'C'
    }
    foreach ($key in $values.Keys) { $start.EnvironmentVariables[$key] = $values[$key] }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $started = $false
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $lines = [Collections.Generic.List[object]]::new()
    $writer = $null
    $lastDisplayed = -5000L
    try {
        if ($CaptureProgress) { $writer = [IO.StreamWriter]::new($eventsPath, $true, $utf8) }
        [void]$process.Start(); $started = $true
        $process.StandardInput.Close()
        $readers = @{ stdout = $process.StandardOutput; stderr = $process.StandardError }
        $pending = @{ stdout = $process.StandardOutput.ReadLineAsync(); stderr = $process.StandardError.ReadLineAsync() }
        while ($pending.Count -gt 0) {
            foreach ($stream in @('stdout', 'stderr')) {
                while ($pending.ContainsKey($stream) -and $pending[$stream].IsCompleted) {
                    $line = $pending[$stream].GetAwaiter().GetResult()
                    if ($null -eq $line) { $pending.Remove($stream); break }
                    if ($lines.Count -ge 10000) { throw 'Acceptance child exceeded the output limit.' }
                    $event = [pscustomobject]@{ phase = $phase; stream = $stream; elapsed_ms = $watch.ElapsedMilliseconds; process_running = (-not $process.HasExited); text = $line }
                    $lines.Add($event)
                    if ($CaptureProgress) {
                        $writer.WriteLine(($event | ConvertTo-Json -Compress))
                        if ($line -match '^Clone phase:|^CANCEL_|^PASS |^Cancellation requested' -or ($line -match '^Clone Receiving:' -and $watch.ElapsedMilliseconds - $lastDisplayed -ge 5000)) {
                            Write-Host ('[{0} +{1}ms] {2}' -f $phase, $watch.ElapsedMilliseconds, $line)
                            $lastDisplayed = $watch.ElapsedMilliseconds
                        }
                    }
                    $pending[$stream] = $readers[$stream].ReadLineAsync()
                }
            }
            if ($watch.Elapsed.TotalSeconds -gt 630) { throw 'Acceptance child exceeded its time limit.' }
            if ($pending.Count -gt 0) { Start-Sleep -Milliseconds 10 }
        }
        if (-not $process.WaitForExit(5000)) { throw 'Acceptance child closed its pipes without exiting.' }
        $runs.Add([ordered]@{ phase = $phase; exit = $process.ExitCode; elapsed_ms = $watch.ElapsedMilliseconds })
        return [pscustomobject]@{
            exit = $process.ExitCode; events = $lines.ToArray(); ms = $watch.ElapsedMilliseconds
            stdout = (@($lines | Where-Object stream -eq 'stdout' | ForEach-Object text) -join "`n")
            stderr = (@($lines | Where-Object stream -eq 'stderr' | ForEach-Object text) -join "`n")
        }
    } finally {
        if ($null -ne $writer) { $writer.Dispose() }
        try {
            if ($started -and -not $process.HasExited) {
                # Terminate only the owned child on a harness fault. Rocket's
                # normal cancellation path must finish without this fallback.
                & (Join-Path $env:WINDIR 'System32\taskkill.exe') /PID $process.Id /T /F 2>&1 | Out-Null
                if (-not $process.WaitForExit(5000)) { throw 'Owned acceptance child did not stop.' }
            }
        } finally { $process.Dispose() }
    }
}

function Invoke-Git([string[]]$Arguments, [string]$At = $scratch) {
    $output = Invoke-Isolated $gitPath (@('-C', $At) + $Arguments)
    if ($output.exit -ne 0) { throw "Git verification failed during $phase (exit $($output.exit)); no raw transport output was saved." }
    return $output.stdout.TrimEnd("`r", "`n")
}

function Read-RemoteHead {
    $text = Invoke-Git @('ls-remote', '--symref', '--', $source, 'HEAD')
    $head = [regex]::Match($text, '(?m)^([a-f0-9]{40}|[a-f0-9]{64})\tHEAD$')
    $branch = [regex]::Match($text, '(?m)^ref: (refs/heads/[^\r\n]+)\tHEAD$')
    if (-not $head.Success -or -not $branch.Success) { throw 'Remote did not advertise a usable default HEAD.' }
    [pscustomobject]@{ oid = $head.Groups[1].Value; branch = $branch.Groups[1].Value }
}

try {
    if ($SelfTest) {
        [IO.Directory]::CreateDirectory($source) | Out-Null
        Invoke-Git @('init', '--initial-branch=main') -At $source | Out-Null
        Invoke-Git @('config', 'user.name', 'Rocket Clone Acceptance') -At $source | Out-Null
        Invoke-Git @('config', 'user.email', 'rocket@example.invalid') -At $source | Out-Null
        [IO.File]::WriteAllText((Join-Path $source '.gitattributes'), "*.cpp text eol=crlf`n", $utf8)
        [IO.File]::WriteAllText((Join-Path $source 'imgui.cpp'), "Synthetic local acceptance fixture only.`n", $utf8)
        # Incompressible local blobs keep real Receiving progress observable.
        $random = [Security.Cryptography.RandomNumberGenerator]::Create()
        try {
            $bytes = New-Object byte[] (256KB)
            for ($i = 0; $i -lt 64; $i++) {
                $random.GetBytes($bytes)
                [IO.File]::WriteAllBytes((Join-Path $source ('payload-{0:D2}.bin' -f $i)), $bytes)
            }
        } finally { $random.Dispose() }
        Invoke-Git @('add', '--all', '--') -At $source | Out-Null
        Invoke-Git @('commit', '-m', 'Synthetic clone acceptance') -At $source | Out-Null
    }
    $phase = 'remote-before'
    $before = Read-RemoteHead
    $evidence.remote_before = $before
    $phase = 'complete-clone'
    $complete = Invoke-Isolated $inspector @('clone', $sourceKind, $source, $target) -CaptureProgress
    if ($complete.exit -ne 0) { throw 'Clone failed; inspect cli-events.jsonl and diagnostic phase outcomes.' }
    foreach ($message in @('Clone phase: Cloning','Clone phase: Verifying','Clone phase: Publishing','Clone phase: Opening','Clone command: succeeded','Clone verification: succeeded','Clone publication: succeeded')) {
        if (-not ($complete.stdout + "`n" + $complete.stderr).Contains($message)) { throw "Missing clone phase/result: $message" }
    }
    $receiving = @($complete.events | Where-Object { $_.text -cmatch '^Clone Receiving: ([0-9]+) / Some\(([0-9]+)\) objects$' })
    $live = @($receiving | Where-Object { $_.process_running })
    if ($live.Count -lt 2 -or @($live.text | Select-Object -Unique).Count -lt 2) { throw 'Live changing transfer progress was not observed.' }
    $evidence.progress = [ordered]@{ receiving_events = $receiving.Count; live_receiving_events = $live.Count; first_ms = $live[0].elapsed_ms; last_ms = $live[-1].elapsed_ms; completion_ms = $complete.ms; checkout_events = @($complete.events | Where-Object { $_.text -like 'Clone checkout:*' }).Count }
    $phase = 'verify-checkout'
    $head = Invoke-Git @('rev-parse', 'HEAD') -At $target
    if ($head -cne (Invoke-Git @('rev-parse', '@{u}') -At $target)) { throw 'HEAD/upstream mismatch.' }
    if ((Invoke-Git @('status', '--porcelain=v1', '--untracked-files=all') -At $target) -ne '') { throw 'Published checkout is not clean.' }
    if ((Invoke-Git @('config', '--get', 'remote.origin.url') -At $target) -cne $source) { throw 'Origin URL mismatch.' }
    # Compare the canonical Git blob, applying the repository's text attributes.
    # Raw CRLF checkout bytes legitimately differ from the stored LF blob on Windows.
    # No filter driver is configured in the isolated configuration.
    $blob = Invoke-Git @('rev-parse', 'HEAD:imgui.cpp') -At $target
    if ($blob -cne (Invoke-Git @('hash-object', '--path=imgui.cpp', '--', 'imgui.cpp') -At $target)) { throw 'Working file does not match the committed blob after Git attribute normalization.' }
    if ($SelfTest -and $blob -ceq (Invoke-Git @('hash-object', '--no-filters', '--', 'imgui.cpp') -At $target)) { throw 'Local self-test did not exercise the CRLF/LF distinction.' }
    Invoke-Git @('fsck', '--connectivity-only', '--no-dangling') -At $target | Out-Null
    if (@(Get-ChildItem -LiteralPath $scratch -Directory -Force -Filter '.rocket-clone-*').Count -ne 0) { throw 'Successful clone left a staging workspace.' }
    $evidence.head = $head
    Write-Host 'PASS complete clone, changing live progress, clean checkout, upstream, normalized blob, and object connectivity'

    $phase = 'cancel-during-receiving'
    $cancel = Invoke-Isolated $cancelProbe @($sourceKind, $source, $cancelled) -CaptureProgress
    if ($cancel.exit -ne 0 -or -not $cancel.stdout.Contains('PASS cancellation after receiving progress')) { throw 'Active-transfer cancellation was not proven; inspect cli-events.jsonl.' }
    $trigger = [regex]::Match($cancel.stdout, 'CANCEL_REQUEST receiving_current=(\d+) receiving_total=(\d+) elapsed_ms=(\d+)')
    $stop = [regex]::Match($cancel.stdout, 'CANCEL_REPORT stop_ms=(\d+) destination_absent=true workspace_retained=true verification=skipped publication=skipped')
    if (-not $trigger.Success -or -not $stop.Success) { throw 'Cancellation evidence is incomplete.' }
    $retained = @(Get-ChildItem -LiteralPath $scratch -Directory -Force -Filter '.rocket-clone-*')
    if ((Test-Path -LiteralPath $cancelled) -or $retained.Count -ne 1) { throw 'Cancellation destination/workspace mismatch.' }
    $evidence.cancellation = [ordered]@{ receiving_current = [long]$trigger.Groups[1].Value; receiving_total = [long]$trigger.Groups[2].Value; requested_ms = [long]$trigger.Groups[3].Value; stop_ms = [long]$stop.Groups[1].Value; retained_workspace = $retained[0].FullName; destination_absent = $true }

    $phase = 'existing-destination'
    $indexBefore = (Get-FileHash -LiteralPath (Join-Path $target '.git\index')).Hash
    $fileBefore = (Get-FileHash -LiteralPath (Join-Path $target 'imgui.cpp')).Hash
    $rejected = Invoke-Isolated $inspector @('clone', $sourceKind, $source, $target) -CaptureProgress
    if ($rejected.exit -ne 1 -or -not $rejected.stderr.Contains('[clone_destination_exists]')) { throw 'Existing destination was not clearly rejected.' }
    if ((Get-FileHash -LiteralPath (Join-Path $target '.git\index')).Hash -cne $indexBefore -or (Get-FileHash -LiteralPath (Join-Path $target 'imgui.cpp')).Hash -cne $fileBefore -or (Invoke-Git @('rev-parse', 'HEAD') -At $target) -cne $head) { throw 'Existing destination changed.' }
    if (@(Get-ChildItem -LiteralPath $scratch -Directory -Force -Filter '.rocket-clone-*').Count -ne 1) { throw 'Preflight rejection allocated another workspace.' }
    Write-Host 'PASS existing destination rejected; HEAD, index, and working file preserved'

    $phase = 'remote-after'
    $after = Read-RemoteHead
    $evidence.remote_after = $after
    if ($before.oid -cne $after.oid -or $before.branch -cne $after.branch) { throw 'Upstream default HEAD changed during the run; exact remote comparison is inconclusive.' }
    if ($head -cne $after.oid -or (Invoke-Git @('symbolic-ref', 'HEAD') -At $target) -cne $after.branch) { throw 'Clone differs from the advertised default HEAD.' }

    $phase = 'diagnostics'
    $records = @(); $logText = ''
    foreach ($file in @(Get-ChildItem -LiteralPath $logs -File -Filter '*.jsonl')) {
        $logText += [IO.File]::ReadAllText($file.FullName)
        foreach ($line in [IO.File]::ReadAllLines($file.FullName)) { $records += ($line | ConvertFrom-Json -ErrorAction Stop) }
    }
    if ($records.Count -eq 0 -or @($records | Where-Object { $_.schema -ne 1 -or $_.dropped_before -ne 0 }).Count) { throw 'Missing, invalid, or dropped diagnostic events.' }
    foreach ($start in @($records | Where-Object { $_.event -in @('request_start','step_start','git_start') })) {
        if (@($records | Where-Object { $_.event -ceq $start.event.Replace('_start','_end') -and $_.session -ceq $start.session -and $_.id -eq $start.id }).Count -ne 1) { throw 'Unpaired diagnostic span.' }
    }
    $requests = @($records | Where-Object event -eq 'request_end')
    if ($requests.Count -ne 3 -or @($requests | Where-Object outcome -eq 'ok').Count -ne 1 -or @($requests | Where-Object outcome -eq 'cancelled').Count -ne 1 -or @($requests | Where-Object failure_code -eq 'clone_destination_exists').Count -ne 1) { throw 'Request success/cancellation/rejection outcomes are unclear.' }
    $outcomes = @($records | Where-Object event -eq 'clone_outcome')
    if ($outcomes.Count -ne 2) { throw 'Expected exactly two attempted clones.' }
    $success = @($outcomes | Where-Object { $_.clone_command -eq 'succeeded' -and $_.verification -eq 'succeeded' -and $_.publication -eq 'succeeded' -and $_.reopen -eq 'succeeded' -and $_.cleanup -eq 'succeeded' -and -not $_.error_diagnostics })
    $stopped = @($outcomes | Where-Object { $_.clone_command -eq 'failed' -and $_.verification -eq 'skipped' -and $_.publication -eq 'skipped' -and $_.reopen -eq 'skipped' -and $_.cleanup -eq 'skipped' })
    if ($success.Count -ne 1 -or $stopped.Count -ne 1) { throw 'Clone phase outcomes are incomplete.' }
    foreach ($outcome in $outcomes) {
        if (@($records | Where-Object { $_.session -ceq $outcome.session -and $_.event -eq 'git_start' -and $_.operation -eq 'clone' }).Count -ne 1) { throw 'Clone command missing or automatically retried.' }
    }
    foreach ($privateValue in @('ROCKET_PRIVATE', $scratch, $source, $head, 'imgui.cpp', 'Authorization:', 'ghp_', 'github_pat_')) {
        if ($logText.Contains($privateValue)) { throw 'Repository metadata or credential-shaped content entered diagnostics.' }
    }
    $evidence.diagnostic_records = $records.Count
    $evidence.dropped_events = 0
    $evidence.passed = $true
    Write-Host 'PASS paired diagnostic spans, explicit cancelled/skipped outcomes, failure codes, and log privacy'
} catch {
    $evidence.failure = $_.Exception.Message
    throw
} finally {
    $evidence.finished_utc = [DateTime]::UtcNow.ToString('o')
    $evidence.final_phase = $phase
    $evidence.runs = @($runs.ToArray())
    $evidence | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $resultPath -Encoding UTF8
    Write-Host "Results: $resultPath"
}
Write-Host "Public clone acceptance passed ($mode). Diagnostics: $logs"
Write-Host "Timestamped CLI/probe transcript (includes local paths and public repository metadata): $eventsPath"
