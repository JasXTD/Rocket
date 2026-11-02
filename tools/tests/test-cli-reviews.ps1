# Focused real rocket-cli -> adjacent rocket.exe launch check. No accounts/network.
param([switch]$Baseline, [switch]$CloseDuringStartup, [switch]$StrictClose,
    [switch]$Minimized, [switch]$InPlace, [string]$UiExecutable, [string]$ResultPath,
    [ValidateRange(1,120)][int]$CloseTimeoutSeconds = 10)
$ErrorActionPreference = 'Stop'
$taskProject = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$taskRoot = Join-Path $taskProject ('work/cli-reviews-' + [guid]::NewGuid().ToString('N'))
$taskRepo = Join-Path $taskRoot '資料 repo'
$taskCli = Join-Path $taskProject 'target/release/rocket-cli.exe'
New-Item -ItemType Directory -Path $taskRepo -Force | Out-Null
# This script runs in a disposable PowerShell process; no persistent environment changes.
Get-ChildItem Env: | Where-Object Name -Match '^(GIT_|SSH_)|_PROXY$' | ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
$env:GIT_CONFIG_GLOBAL = Join-Path $taskRoot 'global.config'
$env:GIT_CONFIG_SYSTEM = Join-Path $taskRoot 'system.config'
$env:GIT_CONFIG_NOSYSTEM = '1'
$env:GIT_ALLOW_PROTOCOL = 'file'
$env:GIT_TERMINAL_PROMPT = '0'
$env:GIT_TEMPLATE_DIR = Join-Path $taskRoot 'template'
New-Item -ItemType Directory -Path $env:GIT_TEMPLATE_DIR | Out-Null
[IO.File]::WriteAllText($env:GIT_CONFIG_GLOBAL, '')
[IO.File]::WriteAllText($env:GIT_CONFIG_SYSTEM, '')
function Invoke-FixtureGit([string[]]$Arguments) {
    $taskOutput = & git -C $taskRepo @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw ($taskOutput -join "`n") }
    return ($taskOutput -join "`n")
}
Invoke-FixtureGit @('init','-b','main') | Out-Null
Invoke-FixtureGit @('config','user.name','Review Fixture') | Out-Null
Invoke-FixtureGit @('config','user.email','fixture@example.invalid') | Out-Null
Invoke-FixtureGit @('config','commit.gpgsign','false') | Out-Null
[IO.File]::WriteAllText((Join-Path $taskRepo '表示.rs'), "fn main() {}`n")
Invoke-FixtureGit @('add','.') | Out-Null
Invoke-FixtureGit @('commit','-m','Unicode review fixture') | Out-Null
$taskHead = Invoke-FixtureGit @('rev-parse','HEAD')
$taskConfigHash = (Get-FileHash -LiteralPath (Join-Path $taskRepo '.git/config')).Hash
$taskFileHash = (Get-FileHash -LiteralPath (Join-Path $taskRepo '表示.rs')).Hash
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class RocketReviewWindows {
    private delegate bool Callback(IntPtr handle, IntPtr arg);
    [DllImport("user32.dll")] private static extern bool EnumWindows(Callback callback, IntPtr arg);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr handle, out uint id);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr handle);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr handle, StringBuilder text, int count);
    [DllImport("user32.dll")] private static extern bool PostMessage(IntPtr handle, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr handle);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr handle, int command);
    [DllImport("kernel32.dll", SetLastError=true)] private static extern bool GetExitCodeProcess(IntPtr handle, out uint code);
    public static uint ExitCode(IntPtr handle) {
        uint code;
        if (!GetExitCodeProcess(handle, out code)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        return code;
    }
    public static Dictionary<long,string> List(uint process) {
        var found = new Dictionary<long,string>();
        EnumWindows((handle,arg) => { uint id; GetWindowThreadProcessId(handle,out id);
            if(id == process && IsWindowVisible(handle)) {
                var text = new StringBuilder(1024); GetWindowText(handle,text,text.Capacity);
                var title = text.ToString();
                // Winit's zero-sized event target has WS_VISIBLE but no title.
                // Closing it destroys repaint delivery, not an application window.
                if (title == "Rocket" || title.StartsWith("Rocket review")) found.Add(handle.ToInt64(),title);
            }
            return true; },IntPtr.Zero);
        return found;
    }
    public static void Close(uint process) {
        foreach(var pair in List(process)) PostMessage(new IntPtr(pair.Key),0x0010,IntPtr.Zero,IntPtr.Zero);
    }
    public static void CloseMain(uint process) {
        foreach(var pair in List(process)) if (!pair.Value.StartsWith("Rocket review")) {
            var window = new IntPtr(pair.Key);
            ShowWindow(window,9); SetForegroundWindow(window);
            PostMessage(window,0x0010,IntPtr.Zero,IntPtr.Zero);
        }
    }
    public static void Minimize(uint process) {
        foreach(var pair in List(process)) if (!pair.Value.StartsWith("Rocket review")) ShowWindow(new IntPtr(pair.Key),6);
    }
    public static void CloseMainInPlace(uint process) {
        foreach(var pair in List(process)) if (!pair.Value.StartsWith("Rocket review")) PostMessage(new IntPtr(pair.Key),0x0010,IntPtr.Zero,IntPtr.Zero);
    }
}
'@
$taskReport = [ordered]@{ passed = $false; root = $taskRoot; cases = @(); cli_sha256 = (Get-FileHash -LiteralPath $taskCli).Hash.ToLowerInvariant(); ui_sha256 = (Get-FileHash -LiteralPath (Join-Path $taskProject 'target/release/rocket.exe')).Hash.ToLowerInvariant() }
$taskReport.loaded_before_close = !$CloseDuringStartup
$taskReport.native_close_followups = @()
if($Baseline) {
    if (!$UiExecutable) { $UiExecutable = Join-Path $taskProject 'tools/releases/0.82.0/rocket.exe' }
    $UiExecutable = (Resolve-Path -LiteralPath $UiExecutable).Path
    $taskReport.ui_sha256 = (Get-FileHash -LiteralPath $UiExecutable).Hash.ToLowerInvariant()
    $taskReport.ui_executable = $UiExecutable
}
$taskReport.close_ms = @()
$taskReport.strict_close = [bool]$StrictClose
$taskReport.minimized = [bool]$Minimized
$taskOwnedProcess = $null
try {
    foreach ($taskMode in $(if($Baseline){@('baseline')}else{@('commit','file','blame')})) {
        $env:LOCALAPPDATA = Join-Path $taskRoot ('profile-' + $taskMode)
        New-Item -ItemType Directory -Path $env:LOCALAPPDATA | Out-Null
        $taskArgs = @('--repo',$taskRepo,'--json','review',$(if ($taskMode -eq 'commit') {'commit'} else {'file'}),'HEAD')
        if ($taskMode -ne 'commit') { $taskArgs += @('--path','表示.rs') }
        if ($taskMode -eq 'blame') { $taskArgs += '--blame' }
        if($Baseline) {
            $taskExpectedExecutable = $UiExecutable
            $taskStart = [Diagnostics.ProcessStartInfo]::new($taskExpectedExecutable)
            $taskStart.UseShellExecute = $false
            $taskStart.ArgumentList.Add($taskRepo)
            $taskOwnedProcess = [Diagnostics.Process]::Start($taskStart)
        } else {
            $taskJson = (& $taskCli @taskArgs | Out-String)
            if ($LASTEXITCODE -ne 0) { throw $taskJson }
            $taskReply = $taskJson | ConvertFrom-Json
            if (-not $taskReply.data.launched -or $taskReply.data.commit -ne $taskHead -or $taskReply.data.mode -ne $taskMode) { throw "Incorrect CLI result: $taskJson" }
            [IO.File]::WriteAllText((Join-Path $taskRoot ($taskMode + '.json')), $taskJson)
            $taskOwnedProcess = Get-Process -Id $taskReply.data.pid
            $taskExpectedExecutable = Join-Path $taskProject 'target/release/rocket.exe'
        }
        if ($taskOwnedProcess.Path -ne $taskExpectedExecutable) { throw 'Unexpected launch executable' }
        # Keep a query handle before exit; Get-Process did not start this child.
        $taskProcessHandle = $taskOwnedProcess.Handle
        $taskDeadline = [DateTime]::UtcNow.AddSeconds(25)
        do {
            $taskWindows = [RocketReviewWindows]::List($taskOwnedProcess.Id)
            if($Baseline -and $taskWindows.Count -ge 1) { break }
            if (@($taskWindows.Values | Where-Object { $_ -like 'Rocket review*' }).Count -eq 1 -and $taskWindows.Count -ge 2) { break }
            if ($taskOwnedProcess.HasExited) { throw 'Rocket exited before its native review appeared' }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $taskDeadline)
        if (!$Baseline -and @($taskWindows.Values | Where-Object { $_ -like 'Rocket review*' }).Count -ne 1) { throw 'Native review window did not appear' }
        if(!$CloseDuringStartup) {
            $taskLog=Join-Path $env:LOCALAPPDATA 'Rocket/logs/rocket.jsonl'
            $taskDeadline=[DateTime]::UtcNow.AddSeconds(25)
            do {
                $taskRecords=if(Test-Path -LiteralPath $taskLog){Get-Content -LiteralPath $taskLog | ForEach-Object {$_ | ConvertFrom-Json}}
                $taskCompletedSave=@($taskRecords | Where-Object {$_.event -eq 'request_end' -and $_.operation -eq 'save_state' -and $_.outcome -eq 'ok'}).Count -gt 0
                $taskStarted=@($taskRecords | Where-Object event -eq 'request_start').Count
                $taskEnded=@($taskRecords | Where-Object event -eq 'request_end').Count
                if($taskCompletedSave -and $taskStarted -eq $taskEnded){break}
                Start-Sleep -Milliseconds 100
            } while([DateTime]::UtcNow -lt $taskDeadline)
            if(!$taskCompletedSave -or $taskStarted -ne $taskEnded){throw 'Initial reads and profile save did not settle'}
        }
        # Closing the main window must also dispose its still-open review.
        if ($Minimized) { [RocketReviewWindows]::Minimize($taskOwnedProcess.Id); Start-Sleep -Milliseconds 200 }
        $taskCloseWatch = [Diagnostics.Stopwatch]::StartNew()
        if ($Minimized -or $InPlace) { [RocketReviewWindows]::CloseMainInPlace($taskOwnedProcess.Id) }
        else { [RocketReviewWindows]::CloseMain($taskOwnedProcess.Id) }
        if (-not $taskOwnedProcess.WaitForExit($CloseTimeoutSeconds * 1000)) {
            # Preserve failure evidence. Follow-up requests are cleanup, never
            # evidence that the first native close succeeded.
            $taskReport.native_close_followups += $taskMode
            $taskReport.stalled_windows = @([RocketReviewWindows]::List($taskOwnedProcess.Id).Values)
            if ($StrictClose) { throw 'First native Close stalled; single-close acceptance failed' }
            Write-Output "KNOWN ISSUE ${taskMode}: first native Close stalled; sending a second Close to this owned test process"
            [RocketReviewWindows]::Close($taskOwnedProcess.Id)
            if (-not $taskOwnedProcess.WaitForExit(5000)) { throw 'Owned window did not close after the recorded lifecycle failure' }
        }
        $taskReport.close_ms += [math]::Round($taskCloseWatch.Elapsed.TotalMilliseconds)
        $taskExitCode = [RocketReviewWindows]::ExitCode($taskProcessHandle)
        if ($taskExitCode -ne 0) { throw "Review exit: $taskExitCode" }
        $taskOwnedProcess = $null
        $taskReport.cases += $(if ($Baseline) { 'Native main window: first Close exited zero' } else { "CLI ${taskMode}: JSON result, pinned commit and separate native window; first Close exited zero" })
        Write-Output ('PASS ' + $taskReport.cases[-1])
    }
    if ((Invoke-FixtureGit @('rev-parse','HEAD')) -ne $taskHead -or (Invoke-FixtureGit @('status','--porcelain')) -ne '' -or (Get-FileHash -LiteralPath (Join-Path $taskRepo '.git/config')).Hash -ne $taskConfigHash -or (Get-FileHash -LiteralPath (Join-Path $taskRepo '表示.rs')).Hash -ne $taskFileHash) { throw 'Fixture state changed' }
    $taskReport.passed = $true
} catch { $taskReport.error = $_.Exception.Message; throw }
finally {
    if ($taskOwnedProcess -and -not $taskOwnedProcess.HasExited) {
        [RocketReviewWindows]::Close($taskOwnedProcess.Id)
        if (-not $taskOwnedProcess.WaitForExit(5000)) { $taskReport.forced_cleanup = $true; $taskOwnedProcess.Kill($true); $taskReport.passed = $false }
    }
    $taskResult = if($ResultPath){[IO.Path]::GetFullPath($ResultPath)}else{Join-Path $taskProject $(if($Baseline){'work/results/reviews-0.83/baseline-quit.json'}else{'work/results/reviews-0.83/cli-native-latest.json'})}
    New-Item -ItemType Directory -Path (Split-Path -Parent $taskResult) -Force | Out-Null
    $taskResultJson = $taskReport | ConvertTo-Json -Depth 6
    [IO.File]::WriteAllText($taskResult, $taskResultJson)
    [IO.File]::WriteAllText((Join-Path $taskRoot 'result.json'), $taskResultJson)
    Write-Output "Evidence: $taskRoot"
}
