param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][ValidateSet('x64', 'x86')][string]$Architecture,
    [Parameter(Mandatory)][string]$Version
)
$ErrorActionPreference = 'Stop'
$resolved = (Resolve-Path -LiteralPath $Path).Path
$bytes = [IO.File]::ReadAllBytes($resolved)
$pe = [BitConverter]::ToInt32($bytes, 0x3c)
if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550) { throw 'Invalid PE signature' }
$expectedMachine = if ($Architecture -eq 'x64') { 0x8664 } else { 0x14c }
if ([BitConverter]::ToUInt16($bytes, $pe + 4) -ne $expectedMachine) { throw 'Wrong PE architecture' }
$sections = [BitConverter]::ToUInt16($bytes, $pe + 6)
$optional = $pe + 24
$table = $optional + [BitConverter]::ToUInt16($bytes, $pe + 20)
$expectedMagic = if ($Architecture -eq 'x64') { 0x20b } else { 0x10b }
if ([BitConverter]::ToUInt16($bytes, $optional) -ne $expectedMagic) { throw 'Wrong PE optional header' }
if ([BitConverter]::ToUInt16($bytes, $optional + 68) -ne 2) { throw 'Expected Windows GUI subsystem' }
$directory = $optional + $(if ($Architecture -eq 'x64') { 112 } else { 96 })
if ([BitConverter]::ToUInt32($bytes, $directory + 13 * 8) -ne 0) {
    throw 'Delay imports require an explicit dependency review'
}
function Convert-Rva([uint32]$Rva) {
    for ($i = 0; $i -lt $sections; $i++) {
        $section = $table + $i * 40
        $address = [BitConverter]::ToUInt32($bytes, $section + 12)
        $size = [BitConverter]::ToUInt32($bytes, $section + 16)
        if ($Rva -ge $address -and $Rva -lt [uint64]$address + $size) {
            return [int]($Rva - $address + [BitConverter]::ToUInt32($bytes, $section + 20))
        }
    }
    throw "Invalid file RVA: $Rva"
}
$offset = Convert-Rva ([BitConverter]::ToUInt32($bytes, $directory + 8))
$imports = @()
while (($nameRva = [BitConverter]::ToUInt32($bytes, $offset + 12)) -ne 0) {
    $start = Convert-Rva $nameRva
    $end = $start
    while ($end -lt $bytes.Length -and $bytes[$end] -ne 0) { $end++ }
    if ($end -eq $bytes.Length) { throw 'Unterminated import name' }
    $imports += [Text.Encoding]::ASCII.GetString($bytes, $start, $end - $start).ToLowerInvariant()
    $offset += 20
}
# Explicit list: a new dependency must be reviewed rather than silently shipped.
$systemDlls = @(
    'kernel32.dll', 'gdi32.dll', 'user32.dll', 'uxtheme.dll', 'shell32.dll',
    'comctl32.dll', 'oleaut32.dll', 'ntdll.dll', 'ole32.dll', 'pdh.dll',
    'version.dll', 'advapi32.dll', 'winhttp.dll', 'gdiplus.dll', 'powrprof.dll',
    'comdlg32.dll', 'combase.dll', 'dwmapi.dll', 'bcryptprimitives.dll',
    'wtsapi32.dll',
    'api-ms-win-core-synch-l1-2-0.dll', 'api-ms-win-core-winrt-l1-1-0.dll'
)
foreach ($dll in $imports) {
    if ($dll -notin $systemDlls) { throw "Unreviewed runtime import: $dll" }
}
$info = [Diagnostics.FileVersionInfo]::GetVersionInfo($resolved)
if ($info.ProductVersion -ne $Version) { throw "Product version mismatch: $($info.ProductVersion)" }
if ($info.OriginalFilename -ne "IdleTrigger-$Architecture.exe") { throw 'OriginalFilename mismatch' }
$core = ($Version.TrimStart('v') -split '[-+]')[0]
$numeric = if ($core -match '^\d+\.\d+\.\d+$') { "$core.0" } else { '0.0.0.0' }
if ($info.FileVersion -ne $numeric) { throw "Fixed version mismatch: $($info.FileVersion), expected $numeric" }
$text = [Text.Encoding]::UTF8.GetString($bytes)
foreach ($marker in 'Microsoft.Windows.Common-Controls', 'PerMonitorV2', 'True/PM') {
    if (!$text.Contains($marker)) { throw "Missing embedded manifest marker: $marker" }
}
if ($text.Contains('IDLETRIGGER_DEVTOOLS')) { throw 'Diagnostic switches found in release artifact' }
Write-Output "Verified $Architecture $Version ($($bytes.Length) bytes): $($imports -join ', ')"
