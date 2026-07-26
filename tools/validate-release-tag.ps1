#requires -Version 7.0

[CmdletBinding()]
param(
    [string]$Tag,
    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$semVerPattern = '^v(?<major>0|[1-9][0-9]*)\.(?<minor>0|[1-9][0-9]*)\.(?<patch>0|[1-9][0-9]*)(?:-(?<prerelease>(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+(?<build>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$'

function ConvertFrom-ReleaseTag {
    param([Parameter(Mandatory)][string]$Value)

    $match = [regex]::Match($Value, $semVerPattern)
    if (-not $match.Success) {
        throw "Release tag '$Value' is not strict SemVer in the form vMAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]."
    }

    $version = $Value.Substring(1)
    $windowsParts = [regex]::Matches($version, '[0-9]+') | Select-Object -First 4
    foreach ($part in $windowsParts) {
        $number = [uint32]0
        if (-not [uint32]::TryParse($part.Value, [ref]$number) -or $number -gt [uint16]::MaxValue) {
            throw "Release tag '$Value' contains a Windows version component outside 0..65535."
        }
    }
    return $version
}

if ($SelfTest) {
    $valid = @(
        "v0.0.0",
        "v1.9.5",
        "v1.2.3-alpha",
        "v1.2.3-alpha.1",
        "v1.2.3-0.3.7",
        "v1.2.3-x.7.z.92",
        "v1.2.3+build.001",
        "v1.2.3-beta+exp.sha.5114f85",
        "v65535.65535.65535-rc.65535"
    )
    $invalid = @(
        "1.2.3",
        "v01.2.3",
        "v1.02.3",
        "v1.2.03",
        "v1.2",
        "v1.2.3-",
        "v1.2.3+",
        "v1.2.3-alpha..1",
        "v1.2.3-01",
        "v1.2.3-alpha_beta",
        "v1.2.3-alpha.١",
        'v1.2.3";Write-Output PWN;#',
        "v65536.0.0",
        "v1.2.3-rc.65536"
    )

    foreach ($value in $valid) {
        try {
            $null = ConvertFrom-ReleaseTag -Value $value
        } catch {
            throw "Expected valid release tag '$value': $_"
        }
    }
    foreach ($value in $invalid) {
        try {
            $null = ConvertFrom-ReleaseTag -Value $value
        } catch {
            continue
        }
        throw "Expected invalid release tag '$value' to be rejected."
    }
    Write-Output "release tag validation self-test passed"
    return
}

if ([string]::IsNullOrWhiteSpace($Tag)) {
    throw "Tag is required unless -SelfTest is used."
}
ConvertFrom-ReleaseTag -Value $Tag
