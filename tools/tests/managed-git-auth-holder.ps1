# Test-only credential data, received through stdin rather than command arguments.
# Restricted before writing; removed in finally when the owning pipe closes.
param([Parameter(Mandatory)][string]$Directory)
$ErrorActionPreference = 'Stop'
$fixtureParent = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../work/mcp-fixtures'))
$fixture = [IO.Path]::GetFullPath((Split-Path -Parent $Directory))
if ((Split-Path -Parent $fixture) -ne $fixtureParent -or
    (Split-Path -Leaf $fixture) -notmatch '^network-https-[a-f0-9]{32}$' -or
    (Split-Path -Leaf $Directory) -ne 'private-auth' -or
    (Test-Path -LiteralPath $Directory)) { throw 'Unexpected credential fixture directory' }
$stream = $null
$file = $null
try {
    $null = New-Item -ItemType Directory -Path $Directory
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetOwner($sid)
    $acl.SetAccessRuleProtection($true, $false)
    $rule = [Security.AccessControl.FileSystemAccessRule]::new(
        $sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
    $acl.AddAccessRule($rule)
    Set-Acl -LiteralPath $Directory -AclObject $acl
    $observed = Get-Acl -LiteralPath $Directory
    if (!$observed.AreAccessRulesProtected -or
        @($observed.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier]) |
          Where-Object IdentityReference -ne $sid).Count -ne 0) {
        throw 'Private credential ACL verification failed'
    }
    $inputLine = [Console]::In.ReadLine()
    if ($null -eq $inputLine -or $inputLine.Length -gt 16384) { throw 'Invalid private input' }
    $bytes = [Convert]::FromBase64String($inputLine)
    $file = Join-Path $Directory 'git-auth.config'
    # Git's Windows config reader does not share DELETE access, so a native
    # delete-on-close handle prevents Git from reading. EOF/finally owns cleanup.
    $stream = [IO.FileStream]::new($file, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite,
        [IO.FileShare]::Read, 4096, [IO.FileOptions]::None)
    $stream.Write($bytes, 0, $bytes.Length)
    $stream.Flush()
    [Array]::Clear($bytes, 0, $bytes.Length)
    $inputLine = $null
    [Console]::Out.WriteLine('READY')
    $null = [Console]::In.ReadLine()
} catch {
    [Console]::Error.WriteLine('Private credential holder failed; input omitted')
    exit 1
} finally {
    if ($null -ne $stream) { $stream.Dispose() }
    if ($null -ne $file -and [IO.File]::Exists($file)) { [IO.File]::Delete($file) }
}
