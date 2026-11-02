param([string]$Operation)

# Test-only Git credential protocol. Never call GCM, save a password, or erase
# a saved credential. The parent supplies a token only in its child's environment.
$ErrorActionPreference = 'Stop'
try {
    if ($Operation -notin @('get', 'store', 'erase')) { exit 0 }
    $fields = @{}
    $length = 0
    while ($null -ne ($line = [Console]::ReadLine()) -and $line -ne '') {
        $length += $line.Length
        if ($length -gt 16384) { exit 1 }
        if ($Operation -eq 'get') {
            $equals = $line.IndexOf('=')
            if ($equals -gt 0) { $fields[$line.Substring(0, $equals)] = $line.Substring($equals + 1) }
        }
    }
    $line = $null
    # Git calls store after success and erase after rejection. Both are no-ops.
    if ($Operation -ne 'get') { exit 0 }
    if (-not $env:ROCKET_REVOCATION_PROTOCOL -or -not $env:ROCKET_REVOCATION_HOST -or
        -not $env:ROCKET_REVOCATION_PATH -or -not $env:ROCKET_REVOCATION_USER) { exit 1 }
    if ($fields['protocol'] -cne $env:ROCKET_REVOCATION_PROTOCOL -or
        $fields['host'] -cne $env:ROCKET_REVOCATION_HOST -or
        $fields['path'] -cne $env:ROCKET_REVOCATION_PATH -or
        ($fields.ContainsKey('username') -and $fields['username'] -cne $env:ROCKET_REVOCATION_USER)) { exit 0 }
    if (-not $env:ROCKET_REVOCATION_TOKEN) { exit 0 }
    # A payload-free marker proves that each fetch requested this test token.
    if ($env:ROCKET_REVOCATION_MARKER) {
        [IO.File]::AppendAllText($env:ROCKET_REVOCATION_MARKER, "get_token`n")
    }
    [Console]::WriteLine('username=' + $env:ROCKET_REVOCATION_USER)
    [Console]::WriteLine('password=' + $env:ROCKET_REVOCATION_TOKEN)
    [Console]::WriteLine()
    exit 0
} catch {
    # No raw exception, input, environment value, or credential in diagnostics.
    exit 1
}
