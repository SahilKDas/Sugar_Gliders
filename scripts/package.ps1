param([Alias('Profile')][ValidateSet('dev','release')][string]$BuildProfile='dev',[switch]$SkipBuild)
$ErrorActionPreference='Stop'
$taskRoot=(Resolve-Path (Split-Path $PSScriptRoot -Parent)).Path
Set-Location -LiteralPath $taskRoot
if (-not $SkipBuild) { & "$PSScriptRoot/build.ps1" -Action build -Profile $BuildProfile }
$profileDirectory=if ($BuildProfile -eq 'dev') { 'debug' } else { 'release' }
$source=Join-Path $taskRoot "target/$profileDirectory/maple-client.exe"
if (-not (Test-Path -LiteralPath $source)) { throw 'Build the Windows client before packaging.' }
$dist=Join-Path $taskRoot 'dist'
New-Item -ItemType Directory -Path $dist -Force | Out-Null
$destination=Join-Path $dist 'Maple.exe'
$copyRequired=-not (Test-Path -LiteralPath $destination)
if (-not $copyRequired) { $copyRequired=(Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash }
if ($copyRequired) {
    try { Copy-Item -LiteralPath $source -Destination $destination -Force }
    catch { throw "Unable to replace Maple.exe. Close the running app before packaging a changed binary. $($_.Exception.Message)" }
}
$binary=[IO.File]::ReadAllBytes($destination)
$peOffset=[BitConverter]::ToInt32($binary,0x3c)
$sectionCount=[BitConverter]::ToUInt16($binary,$peOffset+6)
$optionalSize=[BitConverter]::ToUInt16($binary,$peOffset+20)
$optional=$peOffset+24
if ([BitConverter]::ToUInt16($binary,$optional) -ne 0x20b) { throw 'Expected a PE32+ executable.' }
function Convert-Rva([uint32]$Rva) {
    for ($section=0;$section -lt $sectionCount;$section++) {
        $entry=$optional+$optionalSize+40*$section
        $virtualSize=[BitConverter]::ToUInt32($binary,$entry+8)
        $virtualAddress=[BitConverter]::ToUInt32($binary,$entry+12)
        $rawSize=[BitConverter]::ToUInt32($binary,$entry+16)
        $rawOffset=[BitConverter]::ToUInt32($binary,$entry+20)
        if ($Rva -ge $virtualAddress -and $Rva -lt $virtualAddress+[Math]::Max($virtualSize,$rawSize)) { return [int]($rawOffset+$Rva-$virtualAddress) }
    }
    throw "Invalid PE RVA $Rva"
}
$importRva=[BitConverter]::ToUInt32($binary,$optional+120)
$imports=@()
if ($importRva -ne 0) {
    $descriptor=Convert-Rva $importRva
    while ([BitConverter]::ToUInt32($binary,$descriptor+12) -ne 0) {
        $offset=Convert-Rva ([BitConverter]::ToUInt32($binary,$descriptor+12))
        $end=$offset
        while ($binary[$end] -ne 0) { $end++ }
        $imports += [Text.Encoding]::ASCII.GetString($binary,$offset,$end-$offset)
        $descriptor+=20
    }
}
$systemDirectory=Join-Path $env:SystemRoot 'System32'
foreach ($dependency in $imports) {
    if ($dependency -match '^(opus|libopus|vcruntime|msvcp|ucrtbase)' -or
        ($dependency -notmatch '^(api-ms-win-|ext-ms-win-)' -and -not (Test-Path -LiteralPath (Join-Path $systemDirectory $dependency)))) {
        throw "Non-system DLL dependency: $dependency"
    }
}
Write-Output ('Packaged: {0} ({1:N2} MB)' -f $destination, ((Get-Item -LiteralPath $destination).Length/1000000))
Write-Output ('Verified OS-only DLL imports: ' + ($imports -join ', '))
Get-FileHash -LiteralPath $destination -Algorithm SHA256 | Select-Object Hash
& "$PSScriptRoot/footprint.ps1"
