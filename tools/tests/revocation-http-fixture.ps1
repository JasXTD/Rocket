param([string]$Repository, [string]$ControlDirectory, [switch]$Public)

# Offline validation only. Serve an allowlisted subset of a newly created
# repository's Git directory over dumb HTTP. Bind loopback only; the token is invented.
$ErrorActionPreference = 'Stop'
$listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
$ready = Join-Path $ControlDirectory 'ready.port'
$revoked = Join-Path $ControlDirectory 'revoked'
$stop = Join-Path $ControlDirectory 'stop'
$statsPath = Join-Path $ControlDirectory 'server-counts.json'
$counts = [ordered]@{ anonymous = 0; accepted = 0; rejected = 0 }
$expected = 'Basic ' + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes('rocket-test:ROCKET_FAKE_TOKEN_ONLY_0123456789'))
try {
    $listener.Start()
    [IO.File]::WriteAllText($ready, [string]$listener.LocalEndpoint.Port)
    $deadline = [DateTime]::UtcNow.AddMinutes(3)
    while ([DateTime]::UtcNow -lt $deadline -and -not (Test-Path -LiteralPath $stop)) {
        if (-not $listener.Pending()) { Start-Sleep -Milliseconds 30; continue }
        $client = $listener.AcceptTcpClient()
        try {
            $stream = $client.GetStream()
            $stream.ReadTimeout = 5000
            $stream.WriteTimeout = 5000
            $reader = New-Object IO.StreamReader($stream, [Text.Encoding]::ASCII, $false, 1024, $true)
            $request = $reader.ReadLine()
            $authorization = ''
            $size = 0
            while ($null -ne ($header = $reader.ReadLine()) -and $header -ne '') {
                $size += $header.Length
                if ($size -gt 16384) { throw 'Oversized fixture request' }
                if ($header.StartsWith('Authorization:', [StringComparison]::OrdinalIgnoreCase)) {
                    $authorization = $header.Substring(14).Trim()
                }
            }
            $reader.Dispose()
            $status = '401 Unauthorized'
            $extra = "WWW-Authenticate: Basic realm=`"Rocket offline test`"`r`n"
            $body = [byte[]]@()
            $authenticated = $authorization -ceq $expected -and -not (Test-Path -LiteralPath $revoked)
            if ($authorization -eq '') { $counts.anonymous++ }
            elseif ($authenticated) { $counts.accepted++ }
            else { $counts.rejected++ }
            if ($Public -or $authenticated) {
                $extra = ''
                $status = '404 Not Found'
                if ($request -match '\AGET /rocket-auth-test[.]git/(HEAD|info/refs|objects/info/packs|objects/[0-9a-f]{2}/[0-9a-f]{38})(?:[?][^ ]*)? HTTP/1[.][01]\z') {
                    $file = Join-Path $Repository $Matches[1]
                    if (Test-Path -LiteralPath $file -PathType Leaf) {
                        $status = '200 OK'
                        $body = [IO.File]::ReadAllBytes($file)
                    }
                }
            }
            $response = [Text.Encoding]::ASCII.GetBytes("HTTP/1.1 $status`r`n${extra}Content-Type: text/plain`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n")
            $stream.Write($response, 0, $response.Length)
            $stream.Write($body, 0, $body.Length)
            $stream.Flush()
            $counts | ConvertTo-Json | Set-Content -LiteralPath $statsPath -Encoding ASCII
        } finally { $client.Dispose() }
    }
} finally { $listener.Stop() }
