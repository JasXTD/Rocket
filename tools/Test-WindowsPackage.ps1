# Windows PowerShell 5.1+; no Node, Rust or installed Git required.
# Runs only the supplied, already-reviewed package and synthetic local fixtures.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Bundle,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [string]$EnvironmentLabel = 'Existing Windows host; isolated child profile'
)
$ErrorActionPreference = 'Stop'
$utf8 = New-Object Text.UTF8Encoding($false)
$source = (Resolve-Path -LiteralPath $Bundle).ProviderPath
$root = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $root) { throw 'Choose a new output directory; no existing files are replaced.' }
if ($root.StartsWith($source.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Results must be outside the accepted package.' }
$ancestor = [IO.Directory]::GetParent($root)
while ($null -ne $ancestor) {
    if (Test-Path -LiteralPath (Join-Path $ancestor.FullName '.git')) {
        throw 'Use a new system-temp directory outside any Git checkout; repository initialization intentionally refuses nested repositories.'
    }
    $ancestor = $ancestor.Parent
}
New-Item -ItemType Directory -Path $root | Out-Null
$report = [ordered]@{
    passed = $false; environment = $EnvironmentLabel
    os_version = [Environment]::OSVersion.Version.ToString()
    powershell_version = $PSVersionTable.PSVersion.ToString()
    fresh_os_proven = $false
    scope = 'Local packaged CLI, GUI startup and localhost MCP; no hosting credentials or external remotes'
    cases = @(); measurements = [ordered]@{}
}
$script:owned = New-Object Collections.Generic.List[Diagnostics.Process]
$script:requestId = 0
$script:state = $null
$script:service = $null
$script:token = $null
$script:port = 0
$app = Join-Path $root 'Rocket portable'
$repo = Join-Path $root ('repo ' + [char]0x7814 + [char]0x7A76)
$profile = Join-Path $root 'mcp-profile'
$fixtureHome = Join-Path $root 'home'
$empty = Join-Path $root 'empty.config'
$branch = 'rocket-ui-test/package'
$commandIndex = 0
function Assert-That([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Pass([string]$Name) { $report.cases += $Name; Write-Output "PASS $Name" }
function Assert-AppLocalRuntime([Diagnostics.Process]$Process, [string]$Label) {
    $Process.Refresh()
    $modules = @($Process.Modules | Where-Object { $_.ModuleName -ieq 'vcruntime140.dll' })
    Assert-That ($modules.Count -eq 1 -and $modules[0].FileName -ieq (Join-Path $app 'vcruntime140.dll')) "$Label did not load its app-local Visual C++ runtime."
    $report.measurements[$Label + '_vc_runtime'] = $modules[0].FileName
}
function Save-Utf8([string]$Path, [string]$Text) { [IO.File]::WriteAllText($Path, $Text, $utf8) }
function Quote-Argument([string]$Value) {
    # Windows CommandLineToArgvW escaping, including trailing backslashes.
    '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}
function Start-Owned([string]$Executable, [string[]]$Arguments, [bool]$Interactive = $false) {
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $Executable
    $info.Arguments = ($Arguments | ForEach-Object { Quote-Argument $_ }) -join ' '
    $info.WorkingDirectory = $root
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    # The ordinary GUI is the interactive application being tested. Helpers
    # and the separately controlled MCP host stay hidden.
    $info.WindowStyle = if ($Interactive) { [Diagnostics.ProcessWindowStyle]::Normal } else { [Diagnostics.ProcessWindowStyle]::Hidden }
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = $utf8
    $info.StandardErrorEncoding = $utf8
    $info.EnvironmentVariables.Clear()
    foreach ($key in $childEnvironment.Keys) { $info.EnvironmentVariables[$key] = $childEnvironment[$key] }
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $info
    Assert-That ($process.Start()) 'Child did not start.'
    $script:owned.Add($process)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    [pscustomobject]@{ Process = $process; Stdout = $stdout; Stderr = $stderr }
}
function Run-Child([string]$Executable, [string[]]$Arguments, [int]$TimeoutSeconds = 120) {
    $child = Start-Owned $Executable $Arguments
    if (-not $child.Process.WaitForExit($TimeoutSeconds * 1000)) {
        $child.Process.Kill()
        $child.Process.WaitForExit()
        throw 'Owned test command exceeded its outer watchdog.'
    }
    $child.Process.WaitForExit()
    [pscustomobject]@{ Code = $child.Process.ExitCode; Out = $child.Stdout.Result; Err = $child.Stderr.Result }
}
function Invoke-PackageCli([string[]]$Arguments, [bool]$Success = $true) {
    $script:commandIndex++
    $result = Run-Child (Join-Path $app 'rocket-cli.exe') (@('--json', '--log-dir', (Join-Path $root 'cli-logs')) + $Arguments)
    Save-Utf8 (Join-Path $root ("cli-{0}.json" -f $script:commandIndex)) $result.Out
    $json = $result.Out | ConvertFrom-Json
    Assert-That ($json.schema_version -eq 1) 'CLI did not return schema_version=1 JSON.'
    Assert-That (($result.Code -eq 0) -eq $Success -and $json.ok -eq $Success) ('Unexpected CLI outcome: ' + $result.Err + $result.Out)
    $json
}
function Git([string[]]$Arguments) {
    $result = Run-Child $script:git (@('-C', $repo) + $Arguments)
    Assert-That ($result.Code -eq 0) ('Fixture Git failed: ' + $result.Err)
    $result.Out.Trim()
}
function Http([string]$Method, [string]$Route, $Body = $null) {
    $request = [Net.HttpWebRequest]::Create("http://127.0.0.1:$script:port$Route")
    $request.Proxy = $null
    $request.Method = $Method
    $request.Timeout = 30000
    $request.ReadWriteTimeout = 30000
    $request.KeepAlive = $false
    $request.Headers['Authorization'] = 'Bearer ' + $script:token
    $request.Headers['MCP-Protocol-Version'] = '2025-11-25'
    $request.Accept = 'application/json, text/event-stream'
    if ($null -ne $Body) {
        $bytes = $utf8.GetBytes(($Body | ConvertTo-Json -Depth 24 -Compress))
        $request.ContentType = 'application/json'
        $request.ContentLength = $bytes.Length
        $stream = $request.GetRequestStream()
        try { $stream.Write($bytes, 0, $bytes.Length) } finally { $stream.Dispose() }
    } elseif ($Method -eq 'POST') { $request.ContentLength = 0 }
    $response = $request.GetResponse()
    try {
        $inputStream = $response.GetResponseStream()
        $buffer = New-Object byte[] 8192
        $memory = New-Object IO.MemoryStream
        try {
            while (($read = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                if ($memory.Length + $read -gt 32MB) { throw 'MCP response exceeded its bound.' }
                $memory.Write($buffer, 0, $read)
            }
            $utf8.GetString($memory.ToArray())
        } finally { $memory.Dispose(); $inputStream.Dispose() }
    } finally { $response.Dispose() }
}
function Rpc([string]$Method, $Parameters) {
    $script:requestId++
    $value = (Http POST '/mcp' @{ jsonrpc = '2.0'; id = $script:requestId; method = $Method; params = $Parameters }) | ConvertFrom-Json
    Assert-That ($value.id -eq $script:requestId) 'MCP response ID mismatch.'
    if ($value.error) { throw ('MCP error: ' + $value.error.message) }
    $value.result
}
function Tool([string]$Name, $Arguments = @{}) {
    $result = Rpc 'tools/call' @{ name = $Name; arguments = $Arguments }
    if ($result.isError) { throw ('MCP tool refused: ' + (($result.content | ForEach-Object { $_.text }) -join ' ')) }
    if ($null -ne $result.structuredContent) { return $result.structuredContent }
    $result
}
function Wait-State([scriptblock]$Predicate, [string]$Description, [int]$Seconds = 60) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-That (-not $script:service.Process.HasExited) 'Owned MCP service exited unexpectedly.'
        $script:state = Tool 'rocket_state'
        if ($script:state.startup_error) { throw $script:state.startup_error }
        if (& $Predicate $script:state) { return }
        Start-Sleep -Milliseconds 60
    } while ($watch.Elapsed.TotalSeconds -lt $Seconds)
    throw "State deadline: $Description"
}
function Settle {
    Wait-State { param($s) -not $s.busy -and -not $s.input_pending -and -not (@($s.tabs | Where-Object { $_.loading }).Count) } 'UI settled'
}
function Click([string]$Target) {
    Settle
    Assert-That (@($script:state.controls | Where-Object { $_.id -eq $Target -and $_.enabled }).Count -eq 1) "Missing/disabled control: $Target"
    Tool 'rocket_click' @{ target = $Target; expected_tab = $script:state.active_tab } | Out-Null
    Settle
}
function Capture([string]$Name) {
    $result = Rpc 'tools/call' @{ name = 'rocket_capture'; arguments = @{} }
    Assert-That (-not $result.isError) 'Native capture failed.'
    $image = @($result.content | Where-Object { $_.type -eq 'image' })[0]
    Assert-That ($null -ne $image) 'Native capture did not contain an image.'
    [IO.File]::WriteAllBytes((Join-Path $root ($Name + '.png')), [Convert]::FromBase64String($image.data))
}
try {
    foreach ($folder in @($fixtureHome, "$fixtureHome/AppData/Local", "$fixtureHome/AppData/Roaming", "$root/tmp", "$root/hooks")) {
        New-Item -ItemType Directory -Path $folder -Force | Out-Null
    }
    Save-Utf8 $empty ''
    Save-Utf8 (Join-Path $root 'rocket-private-state') ('synthetic-content-only-v1' + [char]10)
    $system = [Environment]::GetFolderPath('System')
    $windows = [IO.Directory]::GetParent($system).FullName
    $childEnvironment = @{
        SystemRoot = $windows; WINDIR = $windows; COMSPEC = (Join-Path $system 'cmd.exe')
        PATH = ($system + ';' + (Join-Path $system 'WindowsPowerShell/v1.0'))
        HOME = $fixtureHome; USERPROFILE = $fixtureHome
        LOCALAPPDATA = "$fixtureHome/AppData/Local"; APPDATA = "$fixtureHome/AppData/Roaming"
        TEMP = "$root/tmp"; TMP = "$root/tmp"
        GIT_CONFIG_GLOBAL = $empty; GIT_CONFIG_SYSTEM = $empty; GIT_CONFIG_NOSYSTEM = '1'
        GIT_TEMPLATE_DIR = "$root/hooks"; GIT_TERMINAL_PROMPT = '0'; GCM_INTERACTIVE = 'never'
        GIT_ALLOW_PROTOCOL = 'file'
        # Normal GUI update checks must not reach the internet in this fixture.
        HTTPS_PROXY = 'http://127.0.0.1:1'; HTTP_PROXY = 'http://127.0.0.1:1'
        ALL_PROXY = 'http://127.0.0.1:1'; NO_PROXY = '127.0.0.1,localhost'
    }
    $verifier = Join-Path $source 'tools/Verify-Release.ps1'
    & $verifier -Bundle $source -RequireBundledGit *> (Join-Path $root 'source-verification.log')
    $manifestBefore = (Get-FileHash -LiteralPath (Join-Path $source 'SHA256SUMS.txt') -Algorithm SHA256).Hash
    Copy-Item -LiteralPath $source -Destination $app -Recurse -ErrorAction Stop
    & (Join-Path $app 'tools/Verify-Release.ps1') -Bundle $app -RequireBundledGit *> (Join-Path $root 'copy-verification.log')
    $report.package_manifest_sha256 = $manifestBefore.ToLowerInvariant()
    Pass 'Accepted package and relocated disposable copy pass full inventory verification'
    foreach ($tool in @('git.exe', 'cargo.exe', 'rustc.exe', 'node.exe')) {
        $found = Run-Child (Join-Path $system 'where.exe') @($tool)
        Assert-That ($found.Code -ne 0) "Clean child PATH unexpectedly provides $tool"
    }
    Pass 'Application child PATH has no installed Git, Rust or Node'
    $gitDirectories = @(Get-ChildItem -LiteralPath (Join-Path $app 'git') -Directory)
    Assert-That ($gitDirectories.Count -eq 1) 'Expected one bundled Git runtime.'
    $script:git = Join-Path $gitDirectories[0].FullName 'cmd/git.exe'
    Invoke-PackageCli @('init', $repo, '--branch', $branch) | Out-Null
    Git @('config', 'user.name', 'Rocket Package Fixture') | Out-Null
    Git @('config', 'user.email', 'fixture@example.invalid') | Out-Null
    Git @('config', 'commit.gpgsign', 'false') | Out-Null
    Git @('config', 'core.autocrlf', 'false') | Out-Null
    Save-Utf8 (Join-Path $repo '.git/rocket-ui-fixture') $branch
    $filename = ([string][char]0x8CC7 + [char]0x6599 + ' file.txt')
    Save-Utf8 (Join-Path $repo $filename) ('base' + [char]10)
    Invoke-PackageCli @('--repo', $repo, 'stage', '--', $filename) | Out-Null
    Invoke-PackageCli @('--repo', $repo, 'commit', '--message', 'Synthetic package acceptance') | Out-Null
    $head = Git @('rev-parse', 'HEAD')
    Assert-That ($head -match '^[0-9a-f]{40,64}$') 'Commit was not created.'
    Assert-That ((Git @('status', '--porcelain')) -eq '') 'Fixture did not become clean.'
    Pass 'Bundled Git creates a Unicode-path repository and commits through CLI JSON'
    Invoke-PackageCli @('--repo', (Join-Path $root 'does-not-exist'), 'status') $false | Out-Null
    Pass 'Invalid repository returns a nonzero exit and structured JSON error'
    $workerStart = Run-Child (Join-Path $app 'rocket-lua-host.exe') @()
    $workerError = $workerStart.Out | ConvertFrom-Json
    Assert-That ($workerStart.Code -eq 1 -and $workerError.kind -eq 'error' -and $workerError.code -eq 'sandbox') 'Lua worker did not refuse unsandboxed startup.'
    Pass 'Packaged Lua worker refuses execution outside its sandbox'
    $gui = Start-Owned (Join-Path $app 'rocket.exe') @($repo) $true
    $clock = [Diagnostics.Stopwatch]::StartNew()
    do {
        $gui.Process.Refresh()
        Assert-That (-not $gui.Process.HasExited) 'Ordinary desktop entry point exited during startup.'
        if ($gui.Process.MainWindowHandle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    } while ($clock.Elapsed.TotalSeconds -lt 60)
    Assert-That ($gui.Process.MainWindowHandle -ne [IntPtr]::Zero) 'Ordinary desktop window did not appear.'
    Assert-That ($gui.Process.MainWindowTitle -notmatch 'cannot start') 'Ordinary desktop refused startup.'
    # Window creation precedes startup workers. Closing while integration
    # discovery is still running correctly opens a confirmation dialog.
    # Wait for actual completed requests rather than a fixed startup delay.
    $desktopLog = Join-Path $fixtureHome 'AppData/Local/Rocket/logs/rocket.jsonl'
    $startup = [Diagnostics.Stopwatch]::StartNew()
    $ready = $false
    do {
        Assert-That (-not $gui.Process.HasExited) 'Desktop exited before startup completed.'
        if (Test-Path -LiteralPath $desktopLog) {
            $pending = @{}
            $statusReady = $false
            $integrationsReady = $false
            foreach ($line in [IO.File]::ReadAllLines($desktopLog)) {
                if (-not $line.EndsWith('}')) { continue }
                $record = $line | ConvertFrom-Json
                if ($record.event -eq 'request_start') { $pending[[string]$record.id] = $true }
                if ($record.event -eq 'request_end') {
                    $pending.Remove([string]$record.id)
                    if ($record.operation -eq 'status' -and $record.outcome -eq 'ok') { $statusReady = $true }
                    if ($record.operation -eq 'integration-discover' -and $record.outcome -eq 'ok') { $integrationsReady = $true }
                }
            }
            $ready = $statusReady -and $integrationsReady -and $pending.Count -eq 0
        }
        if (-not $ready) { Start-Sleep -Milliseconds 100 }
    } while (-not $ready -and $startup.Elapsed.TotalSeconds -lt 60)
    Assert-That $ready 'Desktop startup requests did not complete.'
    $report.measurements.desktop_startup_ms = $clock.ElapsedMilliseconds
    # Process caches MainWindowHandle. The first HWND may be winit's startup
    # window; reacquire it after startup before sending Close to the real UI.
    $gui.Process.Refresh()
    Assert-That ($gui.Process.MainWindowTitle -eq 'Rocket') 'Expected the actual Rocket desktop window.'
    Assert-AppLocalRuntime $gui.Process 'desktop'
    Assert-That ($gui.Process.CloseMainWindow()) 'Could not close the owned ordinary desktop window.'
    Assert-That ($gui.Process.WaitForExit(30000)) 'Ordinary desktop did not close normally.'
    Assert-That ($gui.Process.ExitCode -eq 0) 'Ordinary desktop exited with an error.'
    Save-Utf8 (Join-Path $root 'desktop.stderr.log') $gui.Stderr.Result
    Pass 'Ordinary rocket.exe opens and closes from the relocated package'
    $mcp = Join-Path $app 'rocket-mcp.exe'
    $init = Run-Child $mcp @('--init-http', $profile, $root)
    Assert-That ($init.Code -eq 0) ('MCP profile initialization failed: ' + $init.Err)
    $listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $script:port = $listener.LocalEndpoint.Port
    $listener.Stop()
    Save-Utf8 (Join-Path $profile 'settings') ("ROCKET-MCP-1" + [char]10 + $script:port + [char]10 + $root + [char]10)
    $script:token = [IO.File]::ReadAllText((Join-Path $profile 'access-token'))
    $script:service = Start-Owned $mcp @('--http-profile', $profile)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-That (-not $script:service.Process.HasExited) 'MCP service failed to start.'
        try { Http GET '/health' | Out-Null; break } catch {
            if ($watch.Elapsed.TotalSeconds -ge 30) { throw }
            Start-Sleep -Milliseconds 100
        }
    } while ($true)
    Rpc 'initialize' @{ protocolVersion = '2025-11-25'; capabilities = @{}; clientInfo = @{ name = 'Rocket Windows package acceptance'; version = '1' } } | Out-Null
    Http POST '/mcp' @{ jsonrpc = '2.0'; method = 'notifications/initialized' } | Out-Null
    Tool 'rocket_open' @{ path = $repo } | Out-Null
    Wait-State { param($s) @($s.tabs).Count -eq 1 -and $s.git_settings.active_executable -and -not $s.tabs[0].loading } 'Repository opened'
    Settle
    $actualGit = $script:state.git_settings.active_executable -replace '^\\\\\?\\', ''
    Assert-AppLocalRuntime $script:service.Process 'mcp'
    Assert-That ($actualGit -eq $script:git) 'Native UI did not select the relocated bundled Git.'
    Tool 'rocket_resize' @{ width = 1280; height = 800 } | Out-Null
    Settle
    Capture 'package-desktop'
    Pass 'Localhost MCP opens the repository using the packaged Git runtime'
    Click 'view-changes'
    Save-Utf8 (Join-Path $repo $filename) ('base' + [char]10 + 'external edit' + [char]10)
    Save-Utf8 (Join-Path $repo 'keep-untracked.txt') ('preserve me' + [char]10)
    Wait-State { param($s) $s.tabs[0].change_count -eq 2 -and -not $s.busy } 'Automatic refresh of external edits'
    $preserved = (Get-FileHash -LiteralPath (Join-Path $repo $filename) -Algorithm SHA256).Hash
    Pass 'External file changes refresh without losing Unicode or untracked data'
    Click 'menu-view'
    Click 'view-goto'
    Click 'goto-integrations'
    Wait-State { param($s) $s.integrations.playground.open -or @($s.controls | Where-Object id -eq 'int-developer').Count -gt 0 } 'Integrations entry'
    if (-not $script:state.integrations.playground.open) {
        if (-not @($script:state.controls | Where-Object id -eq 'int-open-playground').Count) { Click 'int-developer' }
        Click 'int-open-playground'
    }
    Wait-State { param($s) $s.integrations.playground.loaded -and -not $s.integrations.playground.busy } 'Playground loaded'
    Tool 'rocket_type' @{ target = 'int-play-source'; text = "return function() return ui.text('PORTABLE_LUA_OK') end"; expected_tab = $script:state.active_tab } | Out-Null
    Click 'int-play-run'
    Wait-State { param($s) -not $s.integrations.playground.busy -and ($s.integrations.playground.has_preview -or $s.integrations.playground.error) } 'Sandboxed Lua result'
    Assert-That (-not $script:state.integrations.playground.error -and $script:state.integrations.playground.has_preview) 'Packaged sandboxed Lua execution failed.'
    Assert-That (@($script:state.integrations.playground.text.labels | Where-Object { $_ -eq 'PORTABLE_LUA_OK' }).Count -eq 1) 'Lua preview did not contain the expected result.'
    Pass 'Statically linked Lua worker runs inside the real sandbox through localhost MCP'
    Click 'view-changes'
    Click 'menu-terminal'
    Click 'terminal-new'
    Wait-State { param($s) $s.terminal.running -and $s.terminal.tail -match 'PS ' } 'Packaged terminal startup'
    $terminalPid = $script:state.terminal.pid
    Tool 'rocket_type' @{ target = 'terminal-command'; text = "Write-Output ('ROCKET_' + 'TERMINAL_OK')"; expected_tab = $script:state.active_tab } | Out-Null
    Settle
    Click 'terminal-send'
    Wait-State { param($s) $s.terminal.tail -match 'ROCKET_TERMINAL_OK' } 'PowerShell command output'
    Assert-That ($script:state.terminal.pid -eq $terminalPid) 'Terminal command replaced the shell.'
    Tool 'rocket_resize' @{ width = 780; height = 620 } | Out-Null
    Settle
    Capture 'package-compact-terminal'
    Pass 'Packaged terminal helper executes a command at desktop and compact sizes'
    Assert-That ((Git @('rev-parse', 'HEAD')) -eq $head) 'Read-only native checks changed HEAD.'
    Assert-That ((Get-FileHash -LiteralPath (Join-Path $repo $filename) -Algorithm SHA256).Hash -eq $preserved) 'Native checks changed external edits.'
    Assert-That ([IO.File]::ReadAllText((Join-Path $repo 'keep-untracked.txt')) -eq ('preserve me' + [char]10)) 'Untracked content changed.'
    $logs = Tool 'rocket_logs' @{ limit = 1000 }
    Assert-That ($script:state.dropped_events -eq 0) 'Native diagnostics dropped events.'
    Assert-That (($logs | ConvertTo-Json -Depth 24 -Compress) -notmatch 'ROCKET_TERMINAL_OK|PORTABLE_LUA_OK|preserve me|external edit') 'Diagnostics contain private fixture payload.'
    Save-Utf8 (Join-Path $root 'diagnostics.json') ($logs | ConvertTo-Json -Depth 24)
    Tool 'rocket_close' | Out-Null
    Wait-State { param($s) $s.window -in @('closed', 'not_started') -or $s.dialog.kind -eq 'quit' } 'Native window closure'
    if ($script:state.dialog.kind -eq 'quit') {
        Tool 'rocket_click' @{ target = 'dialog-submit'; expected_tab = $script:state.active_tab } | Out-Null
        Wait-State { param($s) $s.window -in @('closed', 'not_started') } 'Confirmed native closure'
    }
    Http POST '/shutdown' | Out-Null
    Assert-That ($script:service.Process.WaitForExit(15000)) 'Owned MCP service did not stop.'
    Assert-That ($script:service.Process.ExitCode -eq 0) 'Owned MCP service exited with an error.'
    Assert-That ($null -eq (Get-Process -Id $terminalPid -ErrorAction SilentlyContinue)) 'Owned terminal shell survived closure.'
    Save-Utf8 (Join-Path $root 'mcp.stderr.log') $script:service.Stderr.Result
    Pass 'Normal shutdown reaps the terminal, preserves local changes and keeps logs private'
    & (Join-Path $app 'tools/Verify-Release.ps1') -Bundle $app -RequireBundledGit *> (Join-Path $root 'final-verification.log')
    Assert-That ((Get-FileHash -LiteralPath (Join-Path $source 'SHA256SUMS.txt') -Algorithm SHA256).Hash -eq $manifestBefore) 'Accepted source manifest changed.'
    Pass 'Application package remains unchanged after first-run operations'
    $report.passed = $true
} catch {
    $report.error = $_.Exception.Message
    if ($script:state) { Save-Utf8 (Join-Path $root 'failure-state.json') ($script:state | ConvertTo-Json -Depth 24) }
    throw
} finally {
    # Kill only processes started by this runner, never a normal Rocket instance.
    foreach ($process in $script:owned) {
        try {
            if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit(10000) | Out-Null }
        } finally { $process.Dispose() }
    }
    if (Test-Path -LiteralPath (Join-Path $profile 'access-token')) {
        Remove-Item -LiteralPath (Join-Path $profile 'access-token')
    }
    $script:token = $null
    Save-Utf8 (Join-Path $root 'result.json') ($report | ConvertTo-Json -Depth 12)
    Write-Output "Results: $root"
}
