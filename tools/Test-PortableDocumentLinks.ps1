# Check the documentation entry points as a reader receives them in the ZIP.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Archive)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $Archive).ProviderPath)
try {
    $missing = [Collections.Generic.List[string]]::new()
    $documents = @('README.md', 'SECURITY.md', 'PRIVACY.md', 'docs/Windows-installer.md', 'docs/Windows-MSIX.md')
    $documents += @($zip.Entries.FullName | Where-Object { $_ -match '^docs/releases/\d+\.\d+\.\d+\.md$' })
    foreach ($name in $documents) {
        $entry = $zip.GetEntry($name)
        if (-not $entry) { $missing.Add($name); continue }
        $base = [Uri]::new("file:///$name")
        $reader = [IO.StreamReader]::new($entry.Open())
        try { $markdown = $reader.ReadToEnd() } finally { $reader.Dispose() }
        foreach ($match in [regex]::Matches($markdown, '\]\((?<target><[^>]+>|[^\s)]+)')) {
            $target = $match.Groups['target'].Value.Trim('<', '>')
            if ($target -match '^(?:[a-z][a-z0-9+.-]*:|//)') { continue }
            $target = ($target -split '[?#]', 2)[0]
            if (-not $target) { continue }
            $relative = [Uri]::UnescapeDataString([Uri]::new($base, $target).AbsolutePath.TrimStart('/'))
            if (-not $zip.GetEntry($relative)) { $missing.Add("$name -> $target") }
        }
    }
    if ($missing.Count) { throw ("Missing linked files in portable ZIP: " + (($missing | Sort-Object -Unique) -join ', ')) }
} finally { $zip.Dispose() }
Write-Output 'Portable ZIP documentation links verified.'
