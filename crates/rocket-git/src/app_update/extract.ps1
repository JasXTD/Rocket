$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
try {
    $lines = [Console]::In.ReadToEnd().TrimEnd("`r","`n").Split("`n")
    if ($lines.Count -ne 2) { throw 'Invalid input' }
    $archivePath = [IO.Path]::GetFullPath($lines[0].TrimEnd("`r"))
    $outputPath = [IO.Path]::GetFullPath($lines[1].TrimEnd("`r"))
    if (Test-Path -LiteralPath $outputPath) { throw 'Output exists' }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        if ($archive.Entries.Count -gt 20000) { throw 'Too many entries' }
        $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        $items = [Collections.Generic.List[object]]::new()
        [long]$total = 0
        foreach ($entry in $archive.Entries) {
            $relative = $entry.FullName
            $parts = $relative.TrimEnd('/').Split('/')
            if (-not $relative -or $relative.Contains('\') -or $relative -match '[\x00-\x1f:<>"|?*]' -or [IO.Path]::IsPathRooted($relative) -or $parts -contains '..' -or $parts -contains '.' -or $parts -contains '') { throw 'Unsafe archive path' }
            foreach ($part in $parts) { if ($part -match '[. ]$|^(?i:con|prn|aux|nul|conin\$|conout\$|com[0-9¹²³]|lpt[0-9¹²³])(?:\.|$)') { throw 'Windows path alias' } }
            # Unix symlinks/devices and Windows reparse attributes are never extracted.
            $kind = ($entry.ExternalAttributes -shr 16) -band 61440
            if ($kind -notin @(0,16384,32768) -or ($entry.ExternalAttributes -band 1024)) { throw 'Archive link or special entry' }
            if (-not $seen.Add($relative.TrimEnd('/'))) { throw 'Duplicate entry' }
            $full = [IO.Path]::GetFullPath((Join-Path $outputPath $relative))
            if (-not $full.StartsWith($outputPath.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Archive path escape' }
            $total += $entry.Length
            if ($entry.Length -gt 268435456 -or $total -gt 1073741824) { throw 'Inflated size limit' }
            $items.Add([pscustomobject]@{ entry=$entry; path=$full; directory=$relative.EndsWith('/') })
        }
        [IO.Directory]::CreateDirectory($outputPath) | Out-Null
        foreach ($item in $items) {
            if ($item.directory) { [IO.Directory]::CreateDirectory($item.path) | Out-Null; continue }
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($item.path)) | Out-Null
            $inputStream=$item.entry.Open()
            $outputStream=[IO.File]::Open($item.path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
            try {
                $buffer=New-Object byte[] 65536
                [long]$written=0
                while (($count=$inputStream.Read($buffer,0,$buffer.Length)) -gt 0) {
                    $written+=$count
                    if ($written -gt $item.entry.Length) { throw 'Inflated length mismatch' }
                    $outputStream.Write($buffer,0,$count)
                }
                if ($written -ne $item.entry.Length) { throw 'Truncated archive entry' }
            } finally { $inputStream.Dispose(); $outputStream.Dispose() }
        }
    } finally { $archive.Dispose() }
    [Console]::Out.WriteLine('Extracted')
} catch { [Console]::Error.WriteLine('Update archive extraction failed'); exit 1 }
