<#
.SYNOPSIS
    Locates adb.exe, AdbWinApi.dll, and AdbWinUsbApi.dll from local Android SDK
    platform-tools or downloads the official Google Windows platform-tools zip,
    then stages those 3 files into dist/unpacked/.
#>
[CmdletBinding()]
param(
    [switch]$ForceDownload
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$RequiredFiles = @("adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll")
$PlatformToolsUrl = "https://dl.google.com/android/repository/platform-tools-latest-windows.zip"

function Test-PlatformToolsDir {
    param([string]$Dir)
    if (-not $Dir -or -not (Test-Path $Dir -PathType Container)) {
        return $false
    }
    foreach ($file in $RequiredFiles) {
        if (-not (Test-Path (Join-Path $Dir $file) -PathType Leaf)) {
            return $false
        }
    }
    return $true
}

Push-Location $RepoRoot
try {
    $sourceDir = $null

    if (-not $ForceDownload) {
        $candidateDirs = @(
            $(if ($env:ANDROID_HOME) { Join-Path $env:ANDROID_HOME "platform-tools" }),
            $(if ($env:ANDROID_SDK_ROOT) { Join-Path $env:ANDROID_SDK_ROOT "platform-tools" }),
            $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "Android\Sdk\platform-tools" }),
            (Join-Path $RepoRoot "target\platform-tools\platform-tools")
        ) | Where-Object { $_ }

        foreach ($dir in $candidateDirs) {
            if (Test-PlatformToolsDir -Dir $dir) {
                $sourceDir = $dir
                Write-Host "==> Found Android platform-tools at $sourceDir"
                break
            }
        }
    }

    if (-not $sourceDir) {
        $downloadRoot = Join-Path $RepoRoot "target\platform-tools"
        $zipPath = Join-Path $downloadRoot "platform-tools-latest-windows.zip"
        New-Item -ItemType Directory -Force -Path $downloadRoot | Out-Null

        Write-Host "==> Downloading official Android platform-tools from $PlatformToolsUrl..."
        Invoke-WebRequest -Uri $PlatformToolsUrl -OutFile $zipPath -UseBasicParsing

        Write-Host "==> Extracting platform-tools into $downloadRoot..."
        Expand-Archive -Path $zipPath -DestinationPath $downloadRoot -Force

        $extractedDir = Join-Path $downloadRoot "platform-tools"
        if (-not (Test-PlatformToolsDir -Dir $extractedDir)) {
            throw "Downloaded archive did not contain required files ($($RequiredFiles -join ', ')) in $extractedDir"
        }
        $sourceDir = $extractedDir
    }

    $stageDir = Join-Path $RepoRoot "dist\unpacked"
    New-Item -ItemType Directory -Force -Path $stageDir | Out-Null

    foreach ($file in $RequiredFiles) {
        $src = Join-Path $sourceDir $file
        $dest = Join-Path $stageDir $file
        Copy-Item -Path $src -Destination $dest -Force
        $item = Get-Item $dest
        Write-Host "    Staged $($item.FullName) ($($item.Length) bytes)"
    }
}
finally {
    Pop-Location
}
