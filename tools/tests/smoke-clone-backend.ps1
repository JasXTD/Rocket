$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inspector = Join-Path $workspace 'target\debug\examples\inspect.exe'
if (-not (Test-Path -LiteralPath $inspector -PathType Leaf)) {
    throw 'Build first: cargo build --offline --locked -p rocket-git --example inspect'
}
$gitPath = (Get-Command git.exe -CommandType Application | Select-Object -First 1).Source
$scratch = Join-Path (Join-Path $workspace 'work') ('smoke-clone-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch | Out-Null
$seed = Join-Path $scratch 'ROCKET_PRIVATE_source'
$target = Join-Path $scratch 'ROCKET_PRIVATE_clone with spaces'
$logs = Join-Path $scratch 'logs'
$hooks = Join-Path $scratch 'empty-hooks'
$config = Join-Path $scratch 'empty.config'
$transcript = Join-Path $scratch 'cli-output.log'
$resultPath = Join-Path $scratch 'result.json'
$utf8 = New-Object Text.UTF8Encoding($false)
foreach ($path in @($seed, $hooks)) { [IO.Directory]::CreateDirectory($path) | Out-Null }
[IO.File]::WriteAllText($config, '', $utf8)
$runs = New-Object 'System.Collections.Generic.List[object]'
$evidence = [ordered]@{passed=$false; started_utc=[DateTime]::UtcNow.ToString('o'); transport='local-file-only'; scratch=$scratch; inspector_sha256=(Get-FileHash -LiteralPath $inspector -Algorithm SHA256).Hash}
$phase = 'setup'

function Invoke-Git([string[]]$Arguments, [string]$At = $seed) {
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $gitPath -C $At @Arguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previous }
    if ($actual -ne 0) { throw "Fixture Git failed ($actual): $Arguments`n$($output -join "`n")" }
    return ($output -join "`n")
}
function Invoke-Clone([string[]]$Arguments, [int]$ExpectedExit, [string]$ExpectedText) {
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $inspector clone @Arguments 2>&1)
        $actual = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previous }
    $text = $output -join "`n"
    [IO.File]::AppendAllText($transcript, "PHASE: $phase`n$text`n", $utf8)
    $runs.Add([ordered]@{phase=$phase; exit=$actual; expected_exit=$ExpectedExit})
    if ($actual -ne $ExpectedExit -or -not $text.Contains($ExpectedText)) { throw "Unexpected clone result during $phase. Inspect $transcript" }
    Write-Host "PASS $phase (exit $actual)"
    return $text
}
function Quote-Shell([string]$Path) { return "'" + $Path.Replace('\','/').Replace("'", "'\''") + "'" }

# These environment changes are restricted to this smoke process and its children.
# No network protocol, saved credential, real repository, or GitHub ref is used.
$names = @('ROCKET_LOG','ROCKET_LOG_DIR','GIT_CONFIG_GLOBAL','GIT_CONFIG_SYSTEM','GIT_CONFIG_NOSYSTEM','GIT_ALLOW_PROTOCOL','GIT_TEMPLATE_DIR','GIT_TERMINAL_PROMPT')
$names += @(Get-ChildItem Env: | Where-Object { $_.Name -match '^(GIT_|SSH_)|_PROXY$' } | ForEach-Object { $_.Name })
$names = @($names | Select-Object -Unique)
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    foreach ($name in $names) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
    $env:ROCKET_LOG_DIR = $logs
    $env:GIT_CONFIG_GLOBAL = $config
    $env:GIT_CONFIG_SYSTEM = $config
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_ALLOW_PROTOCOL = 'file'
    $env:GIT_TEMPLATE_DIR = $hooks
    $env:GIT_TERMINAL_PROMPT = '0'
    Invoke-Git @('init','--initial-branch=main') | Out-Null
    foreach ($setting in @(@('user.name','Rocket Clone Smoke'), @('user.email','rocket@example.invalid'), @('core.autocrlf','false'), @('core.hooksPath',$hooks), @('commit.gpgsign','false'))) {
        Invoke-Git @('config',$setting[0],$setting[1]) | Out-Null
    }
    [IO.File]::WriteAllText((Join-Path $seed 'file'), "ROCKET_PRIVATE_CONTENT`n", $utf8)
    Invoke-Git @('add','file') | Out-Null
    Invoke-Git @('commit','-m','ROCKET_PRIVATE_COMMIT') | Out-Null
    $originalHead = Invoke-Git @('rev-parse','HEAD')
    $originalIndex = (Get-FileHash -LiteralPath (Join-Path $seed '.git\index')).Hash
    [IO.File]::WriteAllText((Join-Path $seed 'file'), "local edits stay here`n", $utf8)
    $phase = 'local-clone'
    $text = Invoke-Clone @('--local',$seed,$target) 0 'Clone published at'
    foreach ($label in @('Clone phase: Cloning','Clone phase: Verifying','Clone phase: Publishing','Clone phase: Opening','Clone Receiving:','Clone verification: succeeded')) {
        if (-not $text.Contains($label)) { throw "Missing progress/result: $label" }
    }
    if ((Invoke-Git @('rev-parse','HEAD') -At $target) -cne $originalHead -or (Invoke-Git @('rev-parse','@{u}') -At $target) -cne $originalHead) { throw 'Clone HEAD/upstream mismatch.' }
    if ([IO.File]::ReadAllText((Join-Path $target 'file')) -cne "ROCKET_PRIVATE_CONTENT`n") { throw 'Checkout mismatch.' }
    if ((Get-FileHash -LiteralPath (Join-Path $seed '.git\index')).Hash -cne $originalIndex -or [IO.File]::ReadAllText((Join-Path $seed 'file')) -cne "local edits stay here`n") { throw 'Source index/working file changed.' }
    if (@(Get-ChildItem -LiteralPath $scratch -Force -Directory -Filter '.rocket-clone-*').Count) { throw 'Successful clone left an unexpected workspace.' }

    $phase = 'existing-directory'
    Invoke-Clone @('--local',$seed,$target) 1 '[clone_destination_exists]' | Out-Null
    $existingFile = Join-Path $scratch 'existing-file'
    [IO.File]::WriteAllText($existingFile, 'KEEP', $utf8)
    $phase = 'existing-file'
    Invoke-Clone @('--local',$seed,$existingFile) 1 '[clone_destination_exists]' | Out-Null
    if ([IO.File]::ReadAllText($existingFile) -cne 'KEEP') { throw 'Existing file changed.' }
    $empty = Join-Path $scratch 'empty-directory'
    [IO.Directory]::CreateDirectory($empty) | Out-Null
    $phase = 'existing-empty-directory'
    Invoke-Clone @('--local',$seed,$empty) 1 '[clone_destination_exists]' | Out-Null
    if (@(Get-ChildItem -LiteralPath $empty -Force).Count) { throw 'Existing empty directory changed.' }
    $phase = 'invalid-url-credentials'
    $text = Invoke-Clone @('--url','https://user:SMOKE_SENSITIVE_TOKEN@example.invalid/repo',(Join-Path $scratch 'invalid')) 1 '[clone_unsupported_source]'
    if ($text.Contains('SMOKE_SENSITIVE_TOKEN')) { throw 'Credential-shaped input leaked.' }

    $notGit = Join-Path $scratch 'SMOKE_SENSITIVE_not_git'
    [IO.Directory]::CreateDirectory($notGit) | Out-Null
    $phase = 'not-a-repository'
    $text = Invoke-Clone @('--local',$notGit,(Join-Path $scratch 'failed')) 1 '[repository_unavailable]'
    if ($text.Contains('SMOKE_SENSITIVE_not_git') -or -not $text.Contains('Workspace retained for inspection:')) { throw 'Failure privacy/recovery path missing.' }
    $phase = 'explicit-raw-output'
    Invoke-Clone @('--local',$notGit,(Join-Path $scratch 'raw-failed'),'--raw-error') 1 'SMOKE_SENSITIVE_not_git' | Out-Null

    # Synthetic URL rewrites stay local even if the guard regresses. This helper
    # only writes a fixture marker and exits; no server or credentials are involved.
    $protocolMarker = Join-Path $scratch 'forbidden-helper-ran'
    $protocolHelper = Join-Path $scratch 'forbidden-helper.sh'
    [IO.File]::WriteAllText($protocolHelper, "#!/bin/sh`nprintf ran > $(Quote-Shell $protocolMarker)`nexit 1`n", $utf8)
    # git-remote-ext uses percent escaping, not shell quote parsing, for argv.
    $protocolArgument = $protocolHelper.Replace('\','/').Replace('%','%%').Replace(' ','% ')
    $rewrite = 'url.ext::sh ' + $protocolArgument + '.insteadOf'
    Invoke-Git @('config','--file',$config,$rewrite,'https://clone-policy.invalid/repo') | Out-Null
    Invoke-Git @('config','--file',$config,'protocol.ext.allow','always') | Out-Null
    $env:GIT_ALLOW_PROTOCOL = 'file:https:ssh:ext'
    $phase = 'rewritten-helper-refused'
    Invoke-Clone @('--url','https://clone-policy.invalid/repo',(Join-Path $scratch 'rewrite-denied'),'--raw-error') 1 "transport 'ext' not allowed" | Out-Null
    if (Test-Path -LiteralPath $protocolMarker) { throw 'A URL rewrite executed the forbidden transport helper.' }
    Invoke-Git @('config','--file',$config,'--unset-all',$rewrite) | Out-Null
    Invoke-Git @('config','--file',$config,'--unset-all','protocol.ext.allow') | Out-Null
    $env:GIT_ALLOW_PROTOCOL = 'file'
    # Even a supported URL source must not widen this inherited file-only policy.
    $rewrite = 'url.file:///' + $seed.Replace('\','/') + '.insteadOf'
    Invoke-Git @('config','--file',$config,$rewrite,'https://clone-policy.invalid/repo') | Out-Null
    $phase = 'inherited-restriction-preserved'
    Invoke-Clone @('--url','https://clone-policy.invalid/repo',(Join-Path $scratch 'inherited-denied')) 1 '[clone_transport_not_allowed]' | Out-Null
    Invoke-Git @('config','--file',$config,'--unset-all',$rewrite) | Out-Null

    # Real Git pauses in a disposable smudge helper during checkout. The CLI timer
    # requests cancellation through the same task token used by its Ctrl+C handler.
    [IO.File]::WriteAllText((Join-Path $seed '.gitattributes'), "*.dat filter=rocket-clone-smoke`n", $utf8)
    [IO.File]::WriteAllText((Join-Path $seed 'slow.dat'), "synthetic checkout`n", $utf8)
    Invoke-Git @('add','--all','--') | Out-Null
    Invoke-Git @('commit','-m','Local cancellation fixture') | Out-Null
    $marker = Join-Path $scratch 'checkout-started'
    $filter = Join-Path $scratch 'slow-filter.sh'
    [IO.File]::WriteAllText($filter, "#!/bin/sh`nprintf ready > $(Quote-Shell $marker)`nsleep 60`ncat`n", $utf8)
    Invoke-Git @('config','--file',$config,'filter.rocket-clone-smoke.smudge',("sh " + (Quote-Shell $filter))) | Out-Null
    Invoke-Git @('config','--file',$config,'filter.rocket-clone-smoke.clean','cat') | Out-Null
    Invoke-Git @('config','--file',$config,'filter.rocket-clone-smoke.required','true') | Out-Null
    $phase = 'cancel-active-checkout'
    $cancelled = Join-Path $scratch 'cancelled'
    $text = Invoke-Clone @('--local',$seed,$cancelled,'--cancel-after-ms','5000') 1 'operation cancelled'
    if (-not (Test-Path -LiteralPath $marker) -or (Test-Path -LiteralPath $cancelled) -or -not $text.Contains('Workspace retained for inspection:')) { throw 'Cancellation did not reach the active checkout or published a destination.' }

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
    foreach ($code in @('clone_destination_exists','clone_unsupported_source','repository_unavailable','clone_transport_not_allowed')) {
        if (-not $diagnostics.Contains(('"failure_code":"' + $code + '"'))) { throw "Missing failure code $code" }
    }
    if (-not $diagnostics.Contains('"outcome":"cancelled"')) { throw 'Cancellation was not logged.' }
    foreach ($secret in @('ROCKET_PRIVATE','SMOKE_SENSITIVE',$scratch,$originalHead)) {
        if ($diagnostics.Contains($secret)) { throw 'Private fixture payload entered diagnostics.' }
    }
    $outcomes = @($records | Where-Object { $_.event -ceq 'clone_outcome' })
    if ($outcomes.Count -ne 6) { throw 'Expected six attempted clones; preflight refusals must not execute Git clone.' }
    foreach ($outcome in $outcomes) {
        if (@($records | Where-Object { $_.session -ceq $outcome.session -and $_.event -ceq 'git_start' -and $_.operation -ceq 'clone' }).Count -ne 1) { throw 'Clone command missing or retried.' }
    }
    if (@($outcomes | Where-Object { $_.publication -ceq 'succeeded' -and $_.verification -ceq 'succeeded' -and $_.reopen -ceq 'succeeded' -and $_.cleanup -ceq 'succeeded' }).Count -ne 1) { throw 'Published outcome fields were lost.' }
    Write-Host 'PASS source/destination preservation, retained failures, progress, cancellation, paired logs, and log privacy'
    $evidence.passed = $true
    $evidence.requests = $runs.Count
    $evidence.diagnostic_records = $records.Count
    $evidence.clone_attempts = $outcomes.Count
    $evidence.retained_workspaces = @(Get-ChildItem -LiteralPath $scratch -Force -Directory -Filter '.rocket-clone-*' | ForEach-Object { $_.FullName })
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    $evidence.finished_utc = [DateTime]::UtcNow.ToString('o')
    $evidence.final_phase = $phase
    $evidence.runs = @($runs.ToArray())
    $evidence | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $resultPath -Encoding UTF8
}
Write-Host "Clone smoke checks passed. Local fixtures: $scratch"
Write-Host "Results: $resultPath"
Write-Host "Diagnostics: $logs"
Write-Host "CLI transcript (synthetic fixture data): $transcript"
