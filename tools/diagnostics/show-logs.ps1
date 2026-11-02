param(
    [string]$Directory,
    [string]$Session,
    [Nullable[long]]$Request,
    [ValidateRange(1, 500)][int]$Tail = 50,
    [switch]$WarningsOnly,
    [switch]$Detailed
)
$ErrorActionPreference = 'Stop'
if (-not $Directory) {
    $Directory = if ($env:ROCKET_LOG_DIR) { $env:ROCKET_LOG_DIR } else { Join-Path $env:LOCALAPPDATA 'Rocket\logs' }
}
Write-Output "Diagnostic directory: $Directory"
$records = @(
    foreach ($name in @('rocket.4.jsonl', 'rocket.3.jsonl', 'rocket.2.jsonl', 'rocket.1.jsonl', 'rocket.jsonl')) {
        $path = Join-Path $Directory $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
        try {
            if ((Get-Item -LiteralPath $path).Length -gt 1MB) {
                Write-Warning "Skipping oversized log file: $name"
                continue
            }
            $lines = [IO.File]::ReadAllLines($path)
        } catch {
            Write-Warning "Could not read $name (it may have rotated); rerun the viewer."
            continue
        }
        foreach ($line in $lines) {
            try {
                $record = $line | ConvertFrom-Json -ErrorAction Stop
                if ($record.schema -ne 1) { continue }
                if ($Session -and $record.session -cne $Session) { continue }
                if ($null -ne $Request -and $record.request -ne $Request) { continue }
                if ($WarningsOnly -and $record.level -notin @('warn', 'error') -and $record.dropped_before -eq 0) { continue }
                $time = [DateTimeOffset]::FromUnixTimeMilliseconds([long]$record.time_unix_ms).UtcDateTime
                [pscustomobject]@{
                    TimeUTC = $time.ToString('yyyy-MM-dd HH:mm:ss.fff')
                    Session = $record.session
                    Request = $record.request
                    Event = $record.event
                    Span = $record.id
                    Operation = $record.operation
                    Step = $record.step
                    ms = $record.elapsed_ms
                    StopCleanupMs = if ($null -ne $record.cancel_to_stop_us) { [double]$record.cancel_to_stop_us / 1000 } else { $null }
                    Outcome = $record.outcome
                    Exit = $record.exit_code
                    Failure = $record.failure_code
                    RemoteOutcome = $record.remote_outcome
                    Objects = $record.objects
                    ObjectBytes = $record.object_bytes
                    ErrorDiagnostics = $record.error_diagnostics
                    FetchOutcome = $record.fetch_outcome
                    UpdateOutcome = $record.update_outcome
                    UpdateKind = $record.update_kind
                    Verification = $record.verification
                    CloneCommand = $record.clone_command
                    Publication = $record.publication
                    Reopen = $record.reopen
                    Cleanup = $record.cleanup
                    Dropped = $record.dropped_before
                    Sequence = $record.seq
                }
            } catch {
                Write-Warning "Skipped an incomplete or invalid record in $name."
            }
        }
    }
)
if ($records.Count -eq 0) {
    Write-Output 'No matching records. Run an inspect command with logging enabled first.'
    return
}
$selected = $records | Sort-Object TimeUTC, Session, Sequence | Select-Object -Last $Tail
if ($Detailed) {
    $selected | Format-List TimeUTC, Session, Request, Span, Event, Operation, Step, ms, StopCleanupMs, Outcome, Exit, Failure, RemoteOutcome, Objects, ObjectBytes, ErrorDiagnostics, FetchOutcome, UpdateOutcome, UpdateKind, Verification, CloneCommand, Publication, Reopen, Cleanup, Dropped
} else {
    $selected | Format-Table TimeUTC, Session, Request, Event, Operation, Step, ms, Outcome, Exit, Failure, RemoteOutcome, FetchOutcome, UpdateOutcome, Dropped -AutoSize -Wrap
}
