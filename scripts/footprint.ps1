param([long]$Limit = 1000000000)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
. "$PSScriptRoot/measure.ps1"
$bytes = Get-WorkspaceBytes $taskRoot
Write-Output ('Workspace: {0:N2} MB ({1} bytes, including .git and target)' -f ($bytes / 1000000), $bytes)
if ($bytes -ge $Limit) { throw 'Workspace exceeds the strict 1 GB limit.' }
if ($bytes -gt 750000000) { Write-Warning 'Above 750 MB: prune intermediate artifacts before another build.' }

