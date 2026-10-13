param([ValidateSet('build','test','check','clippy')][string]$Action='build',[Alias('Profile')][ValidateSet('dev','release')][string]$BuildProfile='dev')
$ErrorActionPreference = 'Stop'
$taskRoot = (Resolve-Path (Split-Path $PSScriptRoot -Parent)).Path
Set-Location -LiteralPath $taskRoot
& "$PSScriptRoot/footprint.ps1"
# Workstation workaround: older CMake cannot generate Visual Studio 2026 projects.
if (-not $env:CMAKE_GENERATOR -and (Test-Path 'C:\Program Files\Microsoft Visual Studio\2022')) {
    $env:CMAKE_GENERATOR = 'Visual Studio 17 2022'
}
$arguments = @($Action, '--workspace', '--locked', '-j', '2')
if ($Action -eq 'build') { $arguments = @('build','-p','maple-client','--locked','-j','2','--profile',$BuildProfile) }
if ($Action -eq 'clippy') { $arguments += @('--all-targets','--','-D','warnings') }
$logRoot=Join-Path $taskRoot '.cache'
New-Item -ItemType Directory -Path $logRoot -Force | Out-Null
$outputLog=Join-Path $logRoot 'cargo.stdout.log'
$errorLog=Join-Path $logRoot 'cargo.stderr.log'
$job = Start-Process cargo -ArgumentList $arguments -WorkingDirectory $taskRoot -WindowStyle Hidden -RedirectStandardOutput $outputLog -RedirectStandardError $errorLog -PassThru
while (-not $job.HasExited) {
    $bytes = (Get-ChildItem -LiteralPath $taskRoot -Force -Recurse -File | Measure-Object Length -Sum).Sum
    if ($bytes -gt 750000000) {
        $compilerProcesses=@(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId)
        $ownedIds=[Collections.Generic.HashSet[int]]::new()
        [void]$ownedIds.Add($job.Id)
        do {
            $previousCount=$ownedIds.Count
            foreach ($compiler in $compilerProcesses) {
                if ($ownedIds.Contains([int]$compiler.ParentProcessId)) { [void]$ownedIds.Add([int]$compiler.ProcessId) }
            }
        } while ($ownedIds.Count -ne $previousCount)
        foreach ($compilerId in ($ownedIds | Sort-Object -Descending)) { Stop-Process -Id $compilerId -Force -ErrorAction SilentlyContinue }
        [void]$job.WaitForExit(5000)
        $remaining=@(Get-Process -Id @($ownedIds) -ErrorAction SilentlyContinue)
        if ($remaining.Count -gt 0) { throw 'Compiler jobs could not be stopped; do not prune active artifacts.' }
        $buildCache = Join-Path $taskRoot 'target'
        $resolved = [IO.Path]::GetFullPath($buildCache)
        if ($resolved -ne (Join-Path $taskRoot 'target')) { throw 'Unexpected build cache path' }
        # Keep compiled executables; remove only regenerable intermediate directories.
        foreach ($profileName in @('debug','release')) {
            foreach ($intermediate in @('build','deps','.fingerprint','incremental')) {
                $candidate = Join-Path (Join-Path $resolved $profileName) $intermediate
                if ([IO.Path]::GetFullPath($candidate).StartsWith($resolved + [IO.Path]::DirectorySeparatorChar)) {
                    if (Test-Path -LiteralPath $candidate) { Remove-Item -LiteralPath $candidate -Force -Recurse }
                }
            }
        }
        throw 'Build stopped at 750 MB and intermediates pruned. Use a smaller target selection.'
    }
    Start-Sleep -Milliseconds 500
    $job.Refresh()
}
$job.WaitForExit()
Get-Content -LiteralPath $outputLog
Get-Content -LiteralPath $errorLog
if ($job.ExitCode -ne 0) { throw "Cargo failed with code $($job.ExitCode)" }
& "$PSScriptRoot/footprint.ps1"
