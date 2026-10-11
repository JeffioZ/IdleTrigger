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

Push-Location -LiteralPath $repoRoot
try {
    New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
    foreach ($arch in $architectures) {
        $targetTriple = if ($arch -eq 'x64') { 'x86_64-pc-windows-msvc' } else { 'i686-pc-windows-msvc' }
        # Ignore per-tool target-directory overrides for this build entry point.
        & cargo build --locked --release --target $targetTriple --target-dir $cacheDir
        if ($LASTEXITCODE -ne 0) {
            throw "Cargo build failed for $arch (exit $LASTEXITCODE). Executables in dist were not replaced."
        }
    }
    # Publish only after every requested architecture compiled successfully.
    foreach ($arch in $architectures) {
        $targetTriple = if ($arch -eq 'x64') { 'x86_64-pc-windows-msvc' } else { 'i686-pc-windows-msvc' }
        $source = Join-Path $cacheDir "$targetTriple/release/IdleTrigger.exe"
        $destination = Join-Path $outputDir "IdleTrigger-$arch.exe"
        Copy-Item -LiteralPath $source -Destination $destination -Force
        Write-Output "Built: $destination"
    }
} finally {
    Pop-Location
}
