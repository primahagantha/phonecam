<#
.SYNOPSIS
    Builds phonecam-android-core as a Windows host DLL (for Gradle JVM unit tests)
    and Android JNI shared libraries (.so) for arm64-v8a, armeabi-v7a, and x86_64.
#>
[CmdletBinding()]
param(
    [switch]$DebugBuild,
    [switch]$SkipHostDll
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $RepoRoot
try {
    if (-not $env:ANDROID_NDK_HOME -or -not (Test-Path $env:ANDROID_NDK_HOME)) {
        $candidateRoots = @(
            $env:ANDROID_NDK_ROOT,
            $(if ($env:ANDROID_HOME) { Join-Path $env:ANDROID_HOME "ndk" }),
            $(if ($env:ANDROID_SDK_ROOT) { Join-Path $env:ANDROID_SDK_ROOT "ndk" }),
            $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "Android\Sdk\ndk" })
        ) | Where-Object { $_ -and (Test-Path $_) }

        foreach ($root in $candidateRoots) {
            $latest = Get-ChildItem -Path $root -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
            if ($latest) {
                $env:ANDROID_NDK_HOME = $latest.FullName
                break
            }
        }
    }
    if (-not $env:ANDROID_NDK_HOME -or -not (Test-Path $env:ANDROID_NDK_HOME)) {
        throw "ANDROID_NDK_HOME is not set or not found. Please set ANDROID_NDK_HOME to your Android NDK installation directory."
    }

    $cargoNdkVersion = & cargo ndk --version 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-ndk is required (cargo install cargo-ndk): $cargoNdkVersion"
    }

    $targetProfileDir = if ($DebugBuild) { "debug" } else { "release" }

    if (-not $SkipHostDll) {
        Write-Host "==> Building Windows host DLL ($targetProfileDir)..."
        if ($DebugBuild) {
            & cargo build -p phonecam-android-core
        } else {
            & cargo build -p phonecam-android-core --release
        }
        if ($LASTEXITCODE -ne 0) {
            throw "Windows host DLL build failed with exit code $LASTEXITCODE"
        }
        $dllPath = Join-Path $RepoRoot "target\$targetProfileDir\phonecam_android_core.dll"
        if (-not (Test-Path $dllPath)) {
            throw "Expected host DLL missing: $dllPath"
        }
        $dllItem = Get-Item $dllPath
        Write-Host "    Built $($dllItem.FullName) ($($dllItem.Length) bytes)"
    }

    $jniLibsDir = Join-Path $RepoRoot "apps\android\app\src\main\jniLibs"
    Write-Host "==> Building Android JNI libraries via cargo-ndk ($targetProfileDir)..."
    if ($DebugBuild) {
        & cargo ndk `
            -t arm64-v8a `
            -t armeabi-v7a `
            -t x86_64 `
            -o $jniLibsDir `
            build -p phonecam-android-core
    } else {
        & cargo ndk `
            -t arm64-v8a `
            -t armeabi-v7a `
            -t x86_64 `
            -o $jniLibsDir `
            build -p phonecam-android-core --release
    }
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-ndk build failed with exit code $LASTEXITCODE"
    }

    $abis = @("arm64-v8a", "armeabi-v7a", "x86_64")
    foreach ($abi in $abis) {
        $soPath = Join-Path $jniLibsDir "$abi\libphonecam_android_core.so"
        if (-not (Test-Path $soPath)) {
            throw "Expected JNI library missing for ${abi}: $soPath"
        }
        $soItem = Get-Item $soPath
        Write-Host "    Built $($soItem.FullName) ($($soItem.Length) bytes)"
    }
}
finally {
    Pop-Location
}
