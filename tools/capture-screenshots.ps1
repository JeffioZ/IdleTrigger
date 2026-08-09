#requires -Version 7.0

[CmdletBinding()]
param(
    [ValidateSet('Readme', 'Review')]
    [string]$CaptureSet = 'Readme',
    [string]$OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
Set-StrictMode -Version Latest
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = if ($CaptureSet -eq 'Readme') { Join-Path $repoRoot 'docs\images' } else { Join-Path $repoRoot 'dist\ui-review' }
}
$outputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$temporaryDirectory = Join-Path (Join-Path $repoRoot 'dist') ('.IdleTrigger-screenshot-build-' + $PID)
$captureDirectory = Join-Path $temporaryDirectory 'captured'
$exePath = Join-Path $temporaryDirectory 'IdleTrigger-x64-screenshot.exe'
$reviewFilePrefixes = @('control-panel', 'automation-manager', 'automation-editor', 'process-picker', 'settings', 'settings-theme', 'settings-app')
$files = if ($CaptureSet -eq 'Readme') {
    @('control-panel-en-light.png', 'control-panel-en-dark.png', 'control-panel-zh-CN-light.png', 'control-panel-zh-CN-dark.png')
} else {
    @('light', 'dark') | ForEach-Object {
        $theme = $_
        @('en', 'zh-CN') | ForEach-Object {
            $language = $_
            $reviewFilePrefixes | ForEach-Object { "$_-$language-$theme.png" }
        }
    }
}
$previousEnvironment = @{
    CGO_ENABLED = [Environment]::GetEnvironmentVariable('CGO_ENABLED', 'Process')
    GOARCH = [Environment]::GetEnvironmentVariable('GOARCH', 'Process')
    GOCACHE = [Environment]::GetEnvironmentVariable('GOCACHE', 'Process')
}

function Get-PngSize([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 24) { throw "PNG is too small: $Path" }
    $signature = [byte[]](0x89,0x50,0x4e,0x47,0x0d,0x0a,0x1a,0x0a)
    for ($i = 0; $i -lt $signature.Length; $i++) { if ($bytes[$i] -ne $signature[$i]) { throw "Invalid PNG signature: $Path" } }
    $width = ([int]$bytes[16] -shl 24) -bor ([int]$bytes[17] -shl 16) -bor ([int]$bytes[18] -shl 8) -bor [int]$bytes[19]
    $height = ([int]$bytes[20] -shl 24) -bor ([int]$bytes[21] -shl 16) -bor ([int]$bytes[22] -shl 8) -bor [int]$bytes[23]
    if ($width -le 0 -or $height -le 0) { throw "Invalid PNG dimensions: $Path" }
    return ,@($width, $height)
}

try {
    New-Item -ItemType Directory -Force -Path $temporaryDirectory | Out-Null
    Push-Location $repoRoot
    try {
        $env:CGO_ENABLED = '0'
        $env:GOARCH = 'amd64'
        $env:GOCACHE = Join-Path $temporaryDirectory 'gocache'
        go build -tags devtools -trimpath -ldflags '-s -w -H windowsgui -X github.com/JeffioZ/idletrigger/internal/version.Value=screenshot' -o $exePath ./cmd/idletrigger
        if ($LASTEXITCODE -ne 0) { throw "go build failed with exit code $LASTEXITCODE" }
        $setFlag = if ($CaptureSet -eq 'Readme') { '--readme-set' } else { '--review-set' }
        $arguments = @('screenshot', $setFlag, '--output', ('"' + $captureDirectory + '"'))
        $process = Start-Process -FilePath $exePath -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "screenshot command failed with exit code $($process.ExitCode)" }
    } finally { Pop-Location }

    $sizes = @{}
    foreach ($name in $files) {
        $path = Join-Path $captureDirectory $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing screenshot: $path" }
        $size = Get-PngSize $path
        $sizes[$name] = $size
        Write-Output "$name $($size[0])x$($size[1])"
    }
    if ($CaptureSet -eq 'Readme') {
        if ($sizes['control-panel-en-light.png'][1] -ne $sizes['control-panel-en-dark.png'][1]) { throw 'English light/dark heights differ' }
        if ($sizes['control-panel-zh-CN-light.png'][1] -ne $sizes['control-panel-zh-CN-dark.png'][1]) { throw 'Chinese light/dark heights differ' }
    } else {
        foreach ($language in @('en', 'zh-CN')) {
            foreach ($prefix in $reviewFilePrefixes) {
                $light = $sizes["$prefix-$language-light.png"]
                $dark = $sizes["$prefix-$language-dark.png"]
                if ($light[0] -ne $dark[0] -or $light[1] -ne $dark[1]) { throw "$prefix $language light/dark sizes differ" }
            }
        }
    }

    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    foreach ($name in $files) {
        Copy-Item -LiteralPath (Join-Path $captureDirectory $name) -Destination (Join-Path $outputDirectory $name) -Force
    }
} finally {
    foreach ($name in $previousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
    }

    # Windows may briefly retain a handle to a just-exited GUI executable.
    # Clean synchronously so the script never leaves a detached cleanup
    # process behind.
    $removed = $false
    for ($attempt = 0; $attempt -lt 50 -and (Test-Path -LiteralPath $temporaryDirectory); $attempt++) {
        try {
            Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force -ErrorAction Stop
            $removed = $true
            break
        } catch {
            Start-Sleep -Milliseconds 200
        }
    }
    if ((Test-Path -LiteralPath $temporaryDirectory) -and -not $removed) {
        throw "Unable to remove temporary screenshot directory: $temporaryDirectory"
    }
}
