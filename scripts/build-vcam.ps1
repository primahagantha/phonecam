<#
.SYNOPSIS
    Builds PhoneCamVirtualCamera.dll and phonecam-vcam-register.exe from C++ source
    and stages them into dist/unpacked/.
#>
[CmdletBinding()]
param(
    [ValidateSet("Release", "Debug")]
    [string]$Configuration = "Release",
    [switch]$SkipStage
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $RepoRoot
try {
    if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        throw "cmake is required on PATH to build the Windows Media Foundation virtual camera."
    }

    $buildDir = Join-Path $RepoRoot "target\vcam-build"

    Write-Host "==> Configuring CMake project in native/ -> target/vcam-build (x64)..."
    & cmake -S (Join-Path $RepoRoot "native") -B $buildDir -A x64
    if ($LASTEXITCODE -ne 0) {
        throw "CMake configure failed with exit code $LASTEXITCODE"
    }

    Write-Host "==> Building virtual camera targets ($Configuration)..."
    & cmake --build $buildDir --config $Configuration
    if ($LASTEXITCODE -ne 0) {
        throw "CMake build failed with exit code $LASTEXITCODE"
    }

    $candidateDirs = @(
        (Join-Path $buildDir "bin\$Configuration"),
        (Join-Path $buildDir "virtual-camera\$Configuration")
    )

    $artifacts = @("PhoneCamVirtualCamera.dll", "phonecam-vcam-register.exe")
    $resolved = @{}
    foreach ($artifact in $artifacts) {
        $found = $null
        foreach ($dir in $candidateDirs) {
            $candidate = Join-Path $dir $artifact
            if (Test-Path $candidate -PathType Leaf) {
                $found = $candidate
                break
            }
        }
        if (-not $found) {
            throw "Expected build artifact '$artifact' not found in: $($candidateDirs -join ', ')"
        }
        $resolved[$artifact] = $found
        $item = Get-Item $found
        Write-Host "    Built $($item.FullName) ($($item.Length) bytes)"
    }

    if (-not $SkipStage) {
        $stageDir = Join-Path $RepoRoot "dist\unpacked"
        New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
        foreach ($artifact in $artifacts) {
            $dest = Join-Path $stageDir $artifact
            Copy-Item -Path $resolved[$artifact] -Destination $dest -Force
            Write-Host "    Staged $dest"
        }
    }
}
finally {
    Pop-Location
}
