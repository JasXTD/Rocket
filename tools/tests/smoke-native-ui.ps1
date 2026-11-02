param([string]$Workspace = 'C:\Users\Tommy\Desktop\Rocket')
$ErrorActionPreference = 'Stop'
$root = Join-Path $Workspace ('work\native-ui-' + [guid]::NewGuid().ToString('N'))
$repo = Join-Path $root 'Rocket-Demo'
$output = Join-Path $root 'review'
New-Item -ItemType Directory -Path $repo,$output,(Join-Path $root 'no-hooks') | Out-Null
[IO.File]::WriteAllText((Join-Path $root 'empty.config'), '')
$saved = @{}
foreach ($name in @('GIT_CONFIG_GLOBAL','GIT_CONFIG_SYSTEM','GIT_CONFIG_NOSYSTEM','GIT_ALLOW_PROTOCOL','GIT_TERMINAL_PROMPT','GIT_TEMPLATE_DIR','GIT_AUTHOR_DATE','GIT_COMMITTER_DATE')) { $saved[$name] = [Environment]::GetEnvironmentVariable($name,'Process') }
try {
    $env:GIT_CONFIG_GLOBAL = Join-Path $root 'empty.config'
    $env:GIT_CONFIG_SYSTEM = $env:GIT_CONFIG_GLOBAL
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_ALLOW_PROTOCOL = 'file'
    $env:GIT_TERMINAL_PROMPT = '0'
    $env:GIT_TEMPLATE_DIR = Join-Path $root 'no-hooks'
    $env:GIT_AUTHOR_DATE = '2026-10-15T12:30:00+08:00'
    $env:GIT_COMMITTER_DATE = $env:GIT_AUTHOR_DATE
    $git = (Get-Command git.exe).Source
    function Invoke-FixtureGit([string[]]$Arguments) {
        & $git -C $repo @Arguments 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Fixture Git failed: $($Arguments -join ' ')" }
    }
    Invoke-FixtureGit @('init','--initial-branch=main','.')
    Invoke-FixtureGit @('config','user.name','Rocket Developer')
    Invoke-FixtureGit @('config','user.email','developer@example.invalid')
    Invoke-FixtureGit @('config','core.autocrlf','false')
    Invoke-FixtureGit @('config','commit.gpgsign','false')
    Invoke-FixtureGit @('config','core.hooksPath',(Join-Path $root 'no-hooks'))
    New-Item -ItemType Directory -Path (Join-Path $repo 'src') | Out-Null
    [IO.File]::WriteAllText((Join-Path $repo 'README.md'), "# Rocket`nA synthetic repository for native UI validation.`n")
    [IO.File]::WriteAllText((Join-Path $repo 'src/backend.rs'), "pub fn refresh() -> bool {`n    true`n}`n")
    Invoke-FixtureGit @('add','--all')
    Invoke-FixtureGit @('commit','-m','Create the Rocket workspace')
    foreach ($subject in @('Preserve paths exactly as returned by Git','Separate staged and working tree changes','Pin history pages to a commit','Keep diagnostics free of private payloads')) {
        [IO.File]::AppendAllText((Join-Path $repo 'README.md'), "$subject`n")
        Invoke-FixtureGit @('add','--all'); Invoke-FixtureGit @('commit','-m',$subject)
    }
    Invoke-FixtureGit @('switch','-c','feature/clone-progress')
    [IO.File]::WriteAllText((Join-Path $repo 'src/clone.rs'), "pub fn clone_progress() {`n    // Progress belongs to the current phase.`n}`n")
    Invoke-FixtureGit @('add','--all'); Invoke-FixtureGit @('commit','-m','Report clone progress by transfer phase')
    [IO.File]::AppendAllText((Join-Path $repo 'src/clone.rs'), "// Preserve existing destination files.`n")
    Invoke-FixtureGit @('add','--all'); Invoke-FixtureGit @('commit','-m','Preserve existing files when clone is cancelled')
    Invoke-FixtureGit @('switch','main')
    [IO.File]::WriteAllText((Join-Path $repo 'src/history.rs'), "pub fn history() {`n    // Follow real parents, including merges.`n}`n")
    Invoke-FixtureGit @('add','--all'); Invoke-FixtureGit @('commit','-m','Draw commit ancestry from parent IDs')
    Invoke-FixtureGit @('merge','--no-ff','-m','Merge clone progress into the backend','feature/clone-progress')
    Invoke-FixtureGit @('branch','feature/native-ui')
    $remote = Join-Path $root 'upstream.git'
    & $git init --bare --initial-branch=main $remote 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Bare repository creation failed' }
    Invoke-FixtureGit @('remote','add','origin',$remote)
    Invoke-FixtureGit @('push','--set-upstream','origin','main')
    [IO.File]::AppendAllText((Join-Path $repo 'src/backend.rs'), "// Request cancellation without blocking the UI.`n")
    [IO.File]::WriteAllText((Join-Path $repo 'src/app.rs'), "pub fn render() {`n    // Native Rust widgets connected to the Git backend.`n}`n")
    [IO.File]::AppendAllText((Join-Path $repo 'README.md'), "Ready for the native desktop app.`n")
    $probe = Join-Path $Workspace 'target\debug\examples\ui_probe.exe'
    $process = Start-Process -FilePath $probe -ArgumentList @(('"'+$repo+'"'),('"'+$output+'"')) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $root 'probe.stdout.log') -RedirectStandardError (Join-Path $root 'probe.stderr.log')
    if (-not $process.WaitForExit(210000)) { $process.Kill(); throw 'UI probe exceeded its timeout' }
    if ($process.ExitCode -ne 0 -or -not (Test-Path (Join-Path $output 'native-result.json'))) { throw "UI probe failed. Inspect $root\probe.stderr.log" }
    Add-Type -AssemblyName System.Drawing
    foreach ($file in Get-ChildItem -LiteralPath $output -Filter '*.bmp') {
        $bitmap = [Drawing.Image]::FromFile($file.FullName)
        try { $bitmap.Save([IO.Path]::ChangeExtension($file.FullName,'.png'),[Drawing.Imaging.ImageFormat]::Png) } finally { $bitmap.Dispose() }
    }
    Write-Output "Native UI checks passed. Review: $output"
    Get-Content (Join-Path $output 'native-result.json')
} finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name,$saved[$name],'Process') }
}
