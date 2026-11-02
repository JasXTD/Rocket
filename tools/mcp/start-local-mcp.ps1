param([string]$ExecutablePath)
$ErrorActionPreference = 'Stop'
$projectPath = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$profilePath = Join-Path $env:LOCALAPPDATA 'Rocket\state\mcp'
$fixturePath = Join-Path $projectPath 'work\mcp-fixtures'
[IO.Directory]::CreateDirectory($fixturePath) | Out-Null
if (-not $ExecutablePath) {
    $ExecutablePath = Join-Path $projectPath 'target\release\rocket-mcp.exe'
    if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
        $manifest = Get-Content -LiteralPath (Join-Path $projectPath 'Cargo.toml') -Raw
        $version = [regex]::Match($manifest, '(?s)\[workspace\.package\].*?version\s*=\s*"([0-9.]+)"').Groups[1].Value
        $ExecutablePath = Join-Path $projectPath "tools\releases\$version\rocket-mcp.exe"
    }
}
if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) { throw 'Build rocket-mcp in target/release or pass -ExecutablePath for a verified bundle.' }
$executablePath = (Resolve-Path -LiteralPath $ExecutablePath).ProviderPath
& $executablePath --init-http $profilePath $fixturePath
if ($LASTEXITCODE -ne 0) { throw 'Local MCP profile initialization failed; existing files preserved.' }
& node (Join-Path $PSScriptRoot 'check-local-mcp.cjs')
if ($LASTEXITCODE -eq 0) { exit 0 }
if ($LASTEXITCODE -ne 2) { throw 'An existing service could not be verified; no second service was started.' }
# Both output streams must leave the caller's pipes: the service outlives this launcher.
$serviceProcess = Start-Process -FilePath $executablePath -ArgumentList @('--http-profile', ('"' + $profilePath + '"')) -WindowStyle Hidden -RedirectStandardError (Join-Path $profilePath 'service.log') -RedirectStandardOutput (Join-Path $profilePath 'service.stdout.log') -PassThru
Write-Output ('Started local MCP process ' + $serviceProcess.Id)
for ($attempt = 0; $attempt -lt 20; $attempt++) {
    & node (Join-Path $PSScriptRoot 'check-local-mcp.cjs')
    if ($LASTEXITCODE -eq 0) { exit 0 }
    if ($serviceProcess.HasExited) { throw 'MCP exited during startup; inspect the profile service.log.' }
    Start-Sleep -Milliseconds 100
}
throw 'MCP startup was not confirmed; inspect status before starting another instance.'
