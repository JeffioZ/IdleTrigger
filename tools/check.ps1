#requires -Version 7.0

[CmdletBinding()]
param(
    [switch]$Full,
    [switch]$Vulncheck
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false
Set-StrictMode -Version Latest
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

function Invoke-NativeCheck {
    param(
        [string]$Name,
        [scriptblock]$Command
    )

    Write-Host "==> $Name"
    & $Command
    $succeeded = $?
    $exitCode = $LASTEXITCODE
    if (-not $succeeded -or $exitCode -ne 0) {
        if ($null -eq $exitCode) { $exitCode = 1 }
        throw "FAILED: $Name failed with exit code $exitCode."
    }
}

Push-Location $repoRoot
try {
    Invoke-NativeCheck "go mod verify" { go mod verify }

    Write-Host "==> gofmt -l ."
    $unformatted = @(gofmt -l .)
    $gofmtSucceeded = $?
    $gofmtExitCode = $LASTEXITCODE
    if (-not $gofmtSucceeded -or $gofmtExitCode -ne 0) {
        if ($null -eq $gofmtExitCode) { $gofmtExitCode = 1 }
        throw "FAILED: gofmt failed with exit code $gofmtExitCode."
    }
    if ($unformatted.Count -ne 0) {
        Write-Host "FAILED: unformatted files:"
        $unformatted | ForEach-Object { Write-Host $_ }
        throw "FAILED: gofmt found $($unformatted.Count) unformatted file(s)."
    }

    Invoke-NativeCheck "git diff HEAD --check" { git diff HEAD --check }
    if ($Full) {
        Invoke-NativeCheck "go test -count=1 ./..." { go test -count=1 ./... }
    } else {
        Invoke-NativeCheck "go test -short -count=1 ./..." { go test -short -count=1 ./... }
    }
    Invoke-NativeCheck "go vet ./..." { go vet ./... }
    Invoke-NativeCheck "build dependency boundaries" { & (Join-Path $PSScriptRoot 'check-build-boundaries.ps1') }

    $golangciSkipped = $false
    if ($Full) {
        Invoke-NativeCheck "go test -count=1 -tags devtools ./..." { go test -count=1 -tags devtools ./... }
        Invoke-NativeCheck "go test -count=1 -tags tools ./..." { go test -count=1 -tags tools ./... }
        Invoke-NativeCheck "go vet -tags devtools ./..." { go vet -tags devtools ./... }

        $golangci = Get-Command golangci-lint -ErrorAction SilentlyContinue
        $golangciSkipped = $null -eq $golangci
        if ($golangciSkipped) {
            Write-Host "SKIPPED: golangci-lint is not installed."
        } else {
            Invoke-NativeCheck "golangci-lint" { & $golangci.Source run --timeout=5m --disable=errcheck ./... }
            $errcheckTargets = @(
                './internal/config/...'
                './internal/automation/...'
                './internal/app/...'
                './internal/platform/windows/autostart/...'
                './internal/platform/windows/gdiplus/...'
                './internal/platform/windows/hotkey/...'
                './internal/platform/windows/ipc/...'
                './internal/platform/windows/singleinstance/...'
            )
            Invoke-NativeCheck "errcheck (core)" {
                & $golangci.Source run --timeout=5m --default=none --enable=errcheck @errcheckTargets
            }
        }
    } else {
        Write-Host "NOT RUN: extended build-tag checks and golangci-lint (pass -Full to run them)."
    }

    if ($Vulncheck) {
        Invoke-NativeCheck "govulncheck v1.1.4" { go run golang.org/x/vuln/cmd/govulncheck@v1.1.4 ./... }
    } else {
        Write-Host "NOT RUN: govulncheck (pass -Vulncheck to run it)."
    }

    if ($Full -and -not $golangciSkipped -and $Vulncheck) {
        Write-Host "All requested checks passed."
    } elseif ($Full) {
        Write-Host "Full checks passed; see skipped or not-run checks above."
    } else {
        Write-Host "Quick checks passed; see not-run checks above."
    }
} finally {
    Pop-Location
}
