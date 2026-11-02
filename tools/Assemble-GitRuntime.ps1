[CmdletBinding()]
param([Parameter(Mandatory)][string]$Destination)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Git-Runtime.ps1')
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $project 'crates/rocket-git/src/managed_git.manifest'
$inventory = Get-RocketGitInventory -Manifest $manifest
$source = Join-Path $project "runtime/git/$($inventory.Spec.Version)"
if (-not (Test-Path -LiteralPath $source -PathType Container)) { throw 'Reviewed portable Git is missing. See runtime/git/README.md; this tool does not download software.' }
$reviewed = Test-RocketGitRuntime -RuntimeRoot $source -Inventory $inventory
$destinationRoot = [IO.Path]::GetFullPath($Destination)
$destinationItem = Get-Item -LiteralPath $destinationRoot -Force
if (-not $destinationItem.PSIsContainer -or ($destinationItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Bundle destination must be a normal directory.' }
$parent = Join-Path $destinationRoot 'git'
if (Test-Path -LiteralPath $parent) {
    $item = Get-Item -LiteralPath $parent -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Existing Git bundle path is not a normal directory.' }
} else { [void][IO.Directory]::CreateDirectory($parent) }
$target = Join-Path $parent $inventory.Spec.Version
$targetManifest = Join-Path $parent 'manifest.tsv'
if (Test-Path -LiteralPath $targetManifest) { [void](Get-RocketGitInventory -Manifest $targetManifest) }
if (Test-Path -LiteralPath $target) {
    [void](Test-RocketGitRuntime -RuntimeRoot $target -Inventory $inventory)
} else {
    $stage = Join-Path $parent ('.' + $inventory.Spec.Version + '-' + [Guid]::NewGuid().ToString('N') + '.partial')
    if (Test-Path -LiteralPath $stage) { throw 'Runtime staging path already exists.' }
    [void][IO.Directory]::CreateDirectory($stage)
    foreach ($directory in $reviewed.Directories) { [void][IO.Directory]::CreateDirectory((Join-Path $stage $directory)) }
    foreach ($entry in $inventory.Entries.Values) { [IO.File]::Copy((Join-Path $source $entry.Path),(Join-Path $stage $entry.Path),$false) }
    [void](Test-RocketGitRuntime -RuntimeRoot $stage -Inventory $inventory)
    $prefix = [IO.Path]::GetFullPath($parent).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    foreach ($path in @($stage,$target)) {
        if (-not [IO.Path]::GetFullPath($path).StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)) { throw 'Runtime publication would leave its bundle folder.' }
    }
    if (Test-Path -LiteralPath $target) { throw 'Another process published the runtime; staged copy retained.' }
    [IO.Directory]::Move($stage,$target)
}
if (-not (Test-Path -LiteralPath $targetManifest)) { [IO.File]::Copy($manifest,$targetManifest,$false) }
Write-Output "Bundled Git $($inventory.Spec.Version): $($reviewed.Files) pinned files, $($reviewed.Bytes) bytes. Existing files were not overwritten."
