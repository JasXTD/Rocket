# Build test helpers from checked-in Rust sources; no downloads or dependencies.
[CmdletBinding()]
param([string]$OutputDirectory = (Join-Path $PSScriptRoot 'bin'))
$ErrorActionPreference = 'Stop'
$compiler = (Get-Command rustc -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$outputRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
$helpers = @(
    @{ Source='auto-bisect/test-runner.rs'; Target='auto-bisect/test-runner.exe' },
    @{ Source='commit-switch/git-relay.rs'; Target='commit-switch/git.exe' },
    @{ Source='file-history-preview/git-relay.rs'; Target='file-history/git.exe' },
    @{ Source='patch-series/git-relay.rs'; Target='patch-series/git.exe' },
    @{ Source='productivity/git-relay.rs'; Target='productivity/git.exe' }
)
foreach ($helper in $helpers) {
    $source = Join-Path $PSScriptRoot $helper.Source
    $destination = Join-Path $outputRoot $helper.Target
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
    & $compiler --edition=2024 -C debuginfo=0 -o $destination $source
    if ($LASTEXITCODE -ne 0) { throw "Helper compilation failed: $($helper.Source)" }
}
# These consumers use the same read-pause and before/after gate protocols.
# Keep their expected executable paths without storing duplicate compiled files.
foreach ($alias in @(
    @{ Source='file-history/git.exe'; Target='diff-review/git.exe' },
    @{ Source='productivity/git.exe'; Target='workflows/git.exe' }
)) {
    $destination = Join-Path $outputRoot $alias.Target
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
    Copy-Item -LiteralPath (Join-Path $outputRoot $alias.Source) -Destination $destination
}
"Built seven local test helpers in $outputRoot"
