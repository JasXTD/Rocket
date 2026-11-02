param([string]$Workspace = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)))
$ErrorActionPreference = 'Stop'
$root = Join-Path $Workspace ('work\reference-ui-' + [guid]::NewGuid().ToString('N'))
$repo = Join-Path $root 'Rocket'
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
    $git = (Get-Command git.exe).Source
    function FixtureGit([string[]]$GitArgs) {
        $result = & $git -C $repo @GitArgs 2>&1
        if ($LASTEXITCODE -ne 0) { throw "Fixture Git failed: $($GitArgs -join ' '): $result" }
        return ($result -join "`n").Trim()
    }
    FixtureGit @('init','--initial-branch=main','.') | Out-Null
    FixtureGit @('config','user.name','Rocket Developer') | Out-Null
    FixtureGit @('config','user.email','developer@example.invalid') | Out-Null
    FixtureGit @('config','core.autocrlf','false') | Out-Null
    FixtureGit @('config','commit.gpgsign','false') | Out-Null
    FixtureGit @('config','core.hooksPath',(Join-Path $root 'no-hooks')) | Out-Null
    $subjects = @('Add structured local operation logs','Refresh tracking branches after fetch','Add explicit push branch and fetch-first guidance','Guard fast-forward pull against local edits',"Merge branch 'release/0.2'",'Give authentication failures clearer recovery steps','Keep retry available after an interrupted transfer','Add transfer progress and cancellation events',"Merge branch 'feature/clone-progress'",'Cache repository reads between refreshes','Preserve existing files when clone is cancelled')
    $parents = @(@(),@(0),@(1),@(0),@(3,2),@(4),@(4),@(6),@(5,7),@(8),@(8,9))
    $ids = @()
    for ($i=0; $i -lt $subjects.Count; $i++) {
        $env:GIT_AUTHOR_DATE = '2026-10-15T12:' + ('{0:D2}' -f $i) + ':00+08:00'
        $env:GIT_COMMITTER_DATE = $env:GIT_AUTHOR_DATE
        foreach ($file in @('README.md','authentication.rs','transport.rs')) { [IO.File]::WriteAllText((Join-Path $repo $file), "$($subjects[$i])`n") }
        FixtureGit @('add','--all') | Out-Null
        $tree = FixtureGit @('write-tree')
        $arguments = @('commit-tree',$tree,'-m',$subjects[$i])
        foreach ($parent in $parents[$i]) { $arguments += @('-p',$ids[$parent]) }
        $ids += FixtureGit $arguments
    }
    foreach ($entry in @(@('refs/heads/main',10),@('refs/heads/dev',4),@('refs/heads/feature/clone-progress',7),@('refs/heads/feature/read-cache',9),@('refs/heads/release/0.2',2),@('refs/remotes/origin/release/0.2',2),@('refs/remotes/origin/main',10))) {
        FixtureGit @('update-ref',$entry[0],$ids[$entry[1]]) | Out-Null
    }
    FixtureGit @('read-tree','--reset','-u','HEAD') | Out-Null
    $remote = Join-Path $root 'origin.git'
    & $git init --bare --quiet $remote
    if ($LASTEXITCODE -ne 0) { throw 'Unable to initialize local fixture remote' }
    FixtureGit @('remote','add','origin',$remote) | Out-Null
    FixtureGit @('config','branch.main.remote','origin') | Out-Null
    FixtureGit @('config','branch.main.merge','refs/heads/main') | Out-Null
    for ($i=0;$i -lt 5;$i++) { [IO.File]::WriteAllText((Join-Path $repo "change-$i.rs"), "// Local visual-review fixture.`n") }
    $paths = @($repo)
    foreach ($name in @('Bootstrap','ImGui')) {
        $path=Join-Path $root $name
        & $git init --quiet --initial-branch=main $path
        if ($LASTEXITCODE -ne 0) { throw "Unable to initialize $name" }
        $paths += $path
    }
    $process=Start-Process -FilePath (Join-Path $Workspace 'target\debug\examples\reference_probe.exe') -ArgumentList (@(('"'+$output+'"')) + @($paths | ForEach-Object { '"'+$_+'"' })) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $root 'stdout.log') -RedirectStandardError (Join-Path $root 'stderr.log')
    if (-not $process.WaitForExit(210000)) { $process.Kill(); throw "Reference probe timeout: $root" }
    if ($process.ExitCode -ne 0 -or -not (Test-Path (Join-Path $output 'result.json'))) { throw "Reference probe failed: $root" }
    Add-Type -AssemblyName System.Drawing
    foreach ($file in Get-ChildItem -LiteralPath $output -Filter '*.bmp') {
        $image=[Drawing.Image]::FromFile($file.FullName)
        try { $image.Save([IO.Path]::ChangeExtension($file.FullName,'.png'),[Drawing.Imaging.ImageFormat]::Png) } finally { $image.Dispose() }
    }
    Write-Output "Reference review passed: $output"
    Get-Content (Join-Path $output 'result.json')
} finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name,$saved[$name],'Process') }
}
