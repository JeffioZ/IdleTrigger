[CmdletBinding()]
param(
    [ValidateSet('x64', 'x86', 'all')]
    [string]$Architecture = 'x64'
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$cacheDir = Join-Path $repoRoot 'target'
$outputDir = Join-Path $repoRoot 'dist'
$architectures = if ($Architecture -eq 'all') { @('x64', 'x86') } else { @($Architecture) }
$publishFiles = @()
$published = @()
$publishId = [Guid]::NewGuid().ToString('N')
$buildLock = $null

Push-Location -LiteralPath $repoRoot
try {
    # Hold a repository-local exclusive handle through build, publish and
    # rollback. Keep the empty lock file: deleting it would race other callers.
    New-Item -ItemType Directory -Path $cacheDir -Force | Out-Null
    try {
        $buildLock = [IO.File]::Open(
            (Join-Path $cacheDir '.idletrigger-build.lock'),
            [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    } catch {
        throw "Cannot acquire the build lock. Another build may be running; retry after it finishes. $($_.Exception.Message)"
    }
    New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
    foreach ($arch in $architectures) {
        $targetTriple = if ($arch -eq 'x64') { 'x86_64-pc-windows-msvc' } else { 'i686-pc-windows-msvc' }
        # Ignore per-tool target-directory overrides for this build entry point.
        & cargo build --locked --release --target $targetTriple --target-dir $cacheDir
        if ($LASTEXITCODE -ne 0) {
            throw "Cargo build failed for $arch (exit $LASTEXITCODE). Executables in dist were not replaced."
        }
    }
    # Stage complete files on the destination volume before changing either EXE.
    foreach ($arch in $architectures) {
        $targetTriple = if ($arch -eq 'x64') { 'x86_64-pc-windows-msvc' } else { 'i686-pc-windows-msvc' }
        $source = Join-Path $cacheDir "$targetTriple/release/IdleTrigger.exe"
        $destination = Join-Path $outputDir "IdleTrigger-$arch.exe"
        $entry = [pscustomobject]@{
            Destination = $destination
            Staged = "$destination.$publishId.new"
            Backup = "$destination.$publishId.bak"
            Existed = [IO.File]::Exists($destination)
        }
        $publishFiles += $entry
        Copy-Item -LiteralPath $source -Destination $entry.Staged
    }
    try {
        foreach ($entry in $publishFiles) {
            if ($entry.Existed) {
                [IO.File]::Replace($entry.Staged, $entry.Destination, $entry.Backup, $true)
            } else {
                [IO.File]::Move($entry.Staged, $entry.Destination)
            }
            $published += $entry
        }
    } catch {
        $publishFailure = $_
        for ($index = $published.Count - 1; $index -ge 0; $index--) {
            $entry = $published[$index]
            try {
                if ($entry.Existed) {
                    [IO.File]::Replace($entry.Backup, $entry.Destination, $null, $true)
                } else {
                    Remove-Item -LiteralPath $entry.Destination
                }
            } catch {
                Write-Warning "Rollback failed for $($entry.Destination). Backup retained at $($entry.Backup): $_"
            }
        }
        throw $publishFailure
    }
    foreach ($entry in $publishFiles) {
        if ([IO.File]::Exists($entry.Backup)) {
            Remove-Item -LiteralPath $entry.Backup -ErrorAction Continue
        }
        Write-Output "Built: $($entry.Destination)"
    }
} finally {
    try {
        foreach ($entry in $publishFiles) {
            if ([IO.File]::Exists($entry.Staged)) {
                Remove-Item -LiteralPath $entry.Staged -ErrorAction Continue
            }
        }
    } finally {
        if ($null -ne $buildLock) { $buildLock.Dispose() }
        Pop-Location
    }
}
