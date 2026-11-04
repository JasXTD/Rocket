# Focused release gates. Do not run alongside Cargo builds or native acceptance.
[CmdletBinding()]
param(
    [switch]$Offline,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$sourceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $sourceRoot ('work/release-checks-' + [guid]::NewGuid().ToString('N'))
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Choose a new results directory; existing evidence is never overwritten.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$report = [ordered]@{ passed = $false; scope = 'Five exact Windows release regressions, sequentially; not a workspace suite'; cases = @() }
Push-Location $sourceRoot
try {
    $buildArgs = @('test', '--locked', '-p', 'rocket-git', '--lib', '--no-run', '--message-format=json', '-j', '2')
    if ($Offline) { $buildArgs += '--offline' }
    & cargo @buildArgs 1> (Join-Path $OutputDirectory 'build.jsonl') 2> (Join-Path $OutputDirectory 'build.stderr.log')
    if ($LASTEXITCODE -ne 0) { throw 'Focused test build failed; see build.stderr.log.' }
    $artifacts = @(Get-Content -LiteralPath (Join-Path $OutputDirectory 'build.jsonl') |
        ForEach-Object { $_ | ConvertFrom-Json } |
        Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'rocket_git' -and $_.profile.test -and $_.executable })
    if ($artifacts.Count -ne 1) { throw 'Expected exactly one rocket-git library test executable.' }
    $testBinary = $artifacts[0].executable
    $report.executable_sha256 = (Get-FileHash -LiteralPath $testBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    # These real I/O/process tests have their original deadlines. Serial execution
    # prevents the acceptance harness itself from competing with their fixtures.
    # No retry, assertion changes, ignored tests or passing empty filters.
    $cases = @(
        'diagnostics::tests::known_contention_recovers_without_repeated_lock_waits',
        'runner::scope_tests::vulnerable_runtime_blocks_http_and_rewrites_before_connecting',
        'runner::scope_tests::vulnerable_runtime_keeps_local_git_working',
        'runner::scope_tests::patched_runtime_keeps_http_transport_available',
        'integration_worker::windows::tests::isolated_worker_echo_limits_and_cancellation_reap_the_process'
    )
    foreach ($case in $cases) {
        $name = $case.Replace('::', '-')
        $stdout = Join-Path $OutputDirectory ($name + '.stdout.log')
        $stderr = Join-Path $OutputDirectory ($name + '.stderr.log')
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $process = Start-Process -FilePath $testBinary -ArgumentList @('--exact', $case, '--nocapture', '--test-threads=1') -WorkingDirectory $sourceRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        try {
            if (-not $process.WaitForExit(120000)) {
                $process.Kill()
                $process.WaitForExit()
                throw "Test process exceeded its outer watchdog: $case"
            }
            $process.WaitForExit()
            $text = [IO.File]::ReadAllText($stdout) + [IO.File]::ReadAllText($stderr)
            $ok = $process.ExitCode -eq 0 -and $text -match 'test result: ok\. 1 passed; 0 failed; 0 ignored;'
            $report.cases += [ordered]@{ name = $case; passed = $ok; exit_code = $process.ExitCode; elapsed_ms = $clock.ElapsedMilliseconds }
            if (-not $ok) { throw "Release gate failed: $case. Original output is retained; no automatic retry." }
            Write-Output ('PASS {0} ({1} ms)' -f $case, $clock.ElapsedMilliseconds)
        } finally { $process.Dispose() }
    }
    $report.passed = $true
} catch {
    $report.error = $_.Exception.Message
    throw
} finally {
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'result.json') -Encoding UTF8
    Pop-Location
    Write-Output "Results: $OutputDirectory"
}
