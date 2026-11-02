# Builds a matching unsigned bundle. Requires the existing Rust/MSVC toolchain.
[CmdletBinding()]
param(
    [ValidateSet('release', 'debug')][string]$Profile = 'release',
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
$buildRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$buildTarget = Join-Path $buildRoot 'target'
$oldWorkerHash = [Environment]::GetEnvironmentVariable('ROCKET_LUA_WORKER_SHA256', 'Process')
Push-Location $buildRoot
try {
    # Fix the target directory; Cargo target overrides would change its layout.
    if ($env:CARGO_BUILD_TARGET) { throw 'Unset CARGO_BUILD_TARGET for this native Windows build.' }
    if ([Environment]::OSVersion.Platform -ne 'Win32NT') { throw 'The Lua sandbox requires Windows.' }
    $common = @('build', '--locked', '--target-dir', $buildTarget)
    if ($Offline) { $common += '--offline' }
    if ($Profile -eq 'release') { $common += '--release' }
    & cargo @common -p rocket-integrations --features runtime --bin rocket-lua-host
    if ($LASTEXITCODE -ne 0) { throw 'Lua worker build failed.' }
    $worker = Join-Path $buildTarget "$Profile/rocket-lua-host.exe"
    $expected = (Get-FileHash -LiteralPath $worker -Algorithm SHA256).Hash.ToLowerInvariant()
    $env:ROCKET_LUA_WORKER_SHA256 = $expected
    # Exclude the worker's package from binary targets so the file just hashed
    # cannot be relinked while building the hosts that embed its digest.
    & cargo @common --workspace --exclude rocket-integrations --bins --features rocket-integrations/runtime
    if ($LASTEXITCODE -ne 0) { throw 'Rocket host build failed.' }
    if ((Get-FileHash -LiteralPath $worker -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
        throw 'Worker changed during the host build. Rebuild the complete bundle.'
    }
    if (Test-Path -LiteralPath (Join-Path $buildRoot 'runtime/git/2.56.0.windows.4') -PathType Container) {
        & (Join-Path $PSScriptRoot 'Assemble-GitRuntime.ps1') -Destination (Join-Path $buildTarget $Profile)
    }
    Write-Output "Built unsigned $Profile bundle in $buildTarget/$Profile"
    Write-Output "Embedded Lua worker SHA-256: $expected"
} finally {
    [Environment]::SetEnvironmentVariable('ROCKET_LUA_WORKER_SHA256', $oldWorkerHash, 'Process')
    Pop-Location
}
