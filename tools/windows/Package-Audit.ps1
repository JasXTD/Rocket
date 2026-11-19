# Bind the inventory to the bytes the privacy checker actually examined.
function Read-PackageAudit([string]$Path, [string]$Summary) {
    $receipt = $Summary | ConvertFrom-Json
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($receipt.passed -ne $true -or [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)) -ine $receipt.reportSha256) {
        throw 'Package privacy audit report changed or failed.'
    }
    [Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-Json
}
