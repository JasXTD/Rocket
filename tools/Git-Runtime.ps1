# Shared first-party packaging checks. This script never downloads or executes Git.
function Get-RocketGitSpec {
    [pscustomobject]@{
        Version = '2.56.1.windows.1'
        ManifestSha256 = '094e647a706b68349c44dcabd0fb4863cf00d55de39814355e6dba3b8af3c13a'
        ArchiveSha256 = '9dd04a608ceb5f6ab32a73709e0ecb159d211da71b697c45ff81f6dd0c6b49c7'
    }
}
function Get-RocketGitInventory {
    param([Parameter(Mandatory)][string]$Manifest)
    $spec = Get-RocketGitSpec
    $item = Get-Item -LiteralPath $Manifest -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $item.Length -gt 8MB) { throw 'Invalid bundled Git inventory file.' }
    $stream = [IO.File]::OpenRead($item.FullName)
    try {
        if ($stream.Length -gt 8MB -or (Get-FileHash -InputStream $stream -Algorithm SHA256).Hash.ToLowerInvariant() -cne $spec.ManifestSha256) { throw 'Bundled Git inventory is not the reviewed version.' }
        $stream.Position = 0
        $reader = [IO.StreamReader]::new($stream)
        try { $lines = $reader.ReadToEnd().TrimEnd([char]10,[char]13) -split '\r?\n' } finally { $reader.Dispose() }
    } finally { $stream.Dispose() }
    if ($lines[0] -cne "ROCKET-MANAGED-GIT-1`t$($spec.Version)") { throw 'Unsupported bundled Git inventory.' }
    $entries = [Collections.Generic.Dictionary[string,object]]::new([StringComparer]::OrdinalIgnoreCase)
    $bytes = 0L
    foreach ($line in $lines | Select-Object -Skip 1) {
        $parts = $line.Split("`t")
        if ($parts.Count -ne 3 -or $parts[0] -notmatch '^[0-9a-f]{64}$' -or $parts[1] -notmatch '^\d+$') { throw 'Malformed bundled Git inventory record.' }
        $relative = $parts[2]
        if ([IO.Path]::IsPathRooted($relative) -or $relative.Contains(':') -or $relative.Contains('\') -or $relative -match '[\x00-\x1f\x7f]' -or ($relative -split '/') -contains '..' -or ($relative -split '/') -contains '.' -or ($relative -split '/') -contains '') { throw 'Unsafe bundled Git inventory path.' }
        $entry = [pscustomobject]@{ Path=$relative; Hash=$parts[0]; Bytes=[long]$parts[1] }
        $bytes += $entry.Bytes
        if ($entries.ContainsKey($relative) -or $entries.Count -ge 20000 -or $bytes -gt 2GB) { throw 'Duplicate or oversized bundled Git inventory.' }
        $entries.Add($relative,$entry)
    }
    if (-not $entries.ContainsKey('cmd/git.exe')) { throw 'Bundled Git executable missing from inventory.' }
    [pscustomobject]@{ Spec=$spec; Entries=$entries; Bytes=$bytes }
}
function Test-RocketGitRuntime {
    param([Parameter(Mandatory)][string]$RuntimeRoot, [Parameter(Mandatory)]$Inventory)
    $root = [IO.Path]::GetFullPath($RuntimeRoot).TrimEnd('\','/')
    $item = Get-Item -LiteralPath $root -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Bundled Git root must be a normal directory.' }
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $folders = [Collections.Generic.List[string]]::new()
    $pending = [Collections.Generic.Stack[string]]::new(); $pending.Push($root)
    while ($pending.Count) {
        $directory = $pending.Pop()
        foreach ($file in Get-ChildItem -LiteralPath $directory -Force) {
            if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Bundled Git contains a link or reparse point.' }
            $relative = $file.FullName.Substring($root.Length+1).Replace('\','/')
            if ($file.PSIsContainer) {
                if ($folders.Count -gt 80000) { throw 'Too many bundled Git directories.' }
                $folders.Add($relative); $pending.Push($file.FullName); continue
            }
            if (-not $Inventory.Entries.ContainsKey($relative) -or -not $seen.Add($relative)) { throw "Unexpected bundled Git file: $relative" }
            $expected = $Inventory.Entries[$relative]
            if ($file.Length -ne $expected.Bytes -or (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne $expected.Hash) { throw "Bundled Git file changed: $relative" }
        }
    }
    if ($seen.Count -ne $Inventory.Entries.Count) { throw 'Bundled Git files are missing.' }
    [pscustomobject]@{ Root=$root; Directories=$folders; Files=$seen.Count; Bytes=$Inventory.Bytes; Version=$Inventory.Spec.Version }
}
