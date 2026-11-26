<#
.SYNOPSIS
    Generates or reuses a persistent 4096-bit RSA PKCS12 Android release keystore
    outside the Git repository and prepares GitHub Actions signing secrets.

.DESCRIPTION
    Creates a persistent Android signing keystore at
    "%LOCALAPPDATA%\PhoneCam\signing\phonecam-release.jks" (never inside the Git repo)
    if one does not already exist, using JDK's `keytool`.
    Outputs the exact `gh secret set` commands for:
      - ANDROID_KEYSTORE_BASE64
      - ANDROID_KEYSTORE_PASSWORD
      - ANDROID_KEY_ALIAS
      - ANDROID_KEY_PASSWORD

    Pass -ApplyGitHubSecrets to upload all four secrets directly via GitHub CLI (`gh`).

.PARAMETER KeystorePath
    Destination path for the keystore file. Defaults to
    "$env:LOCALAPPDATA\PhoneCam\signing\phonecam-release.jks". Must not be inside the Git repo.

.PARAMETER KeyAlias
    Key alias inside the keystore. Defaults to "phonecam-release".

.PARAMETER ValidityDays
    Certificate validity in days. Defaults to 10000 (~27 years).

.PARAMETER DistinguishedName
    X.500 Distinguished Name for the signing certificate.

.PARAMETER Repo
    Optional GitHub repository (`owner/repo`) passed to `gh secret set --repo`.

.PARAMETER ApplyGitHubSecrets
    Applies the four signing secrets directly to GitHub Actions using `gh secret set`.
#>
[CmdletBinding()]
param(
    [string]$KeystorePath = "",
    [string]$KeyAlias = "phonecam-release",
    [int]$ValidityDays = 10000,
    [string]$DistinguishedName = "CN=PhoneCam Release, OU=Mobile, O=PhoneCam, L=Unknown, ST=Unknown, C=US",
    [string]$Repo = "",
    [switch]$ApplyGitHubSecrets
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-KeytoolPath {
    $cmd = Get-Command keytool -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    $candidates = @()
    if ($env:JAVA_HOME) {
        $candidates += (Join-Path $env:JAVA_HOME "bin\keytool.exe")
    }
    if ($env:ANDROID_STUDIO_JBR) {
        $candidates += (Join-Path $env:ANDROID_STUDIO_JBR "bin\keytool.exe")
    }
    if ($env:ProgramFiles) {
        $candidates += (Join-Path $env:ProgramFiles "Android\Android Studio\jbr\bin\keytool.exe")
        $jdkDirs = @(
            (Join-Path $env:ProgramFiles "Eclipse Adoptium"),
            (Join-Path $env:ProgramFiles "Java"),
            (Join-Path $env:ProgramFiles "Microsoft")
        )
        foreach ($dir in $jdkDirs) {
            if (Test-Path $dir) {
                $found = Get-ChildItem -Path $dir -Filter "keytool.exe" -Recurse -ErrorAction SilentlyContinue |
                    Select-Object -First 1
                if ($found) {
                    $candidates += $found.FullName
                }
            }
        }
    }

    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path $candidate)) {
            return $candidate
        }
    }
    return $null
}

function ConvertFrom-SecureStringPlain([Security.SecureString]$Secure) {
    $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($Secure)
    try {
        return [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
    }
    finally {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    }
}

function Read-ConfirmedPassword([string]$Prompt, [int]$MinLength = 6) {
    while ($true) {
        $first = Read-Host -Prompt $Prompt -AsSecureString
        $firstPlain = ConvertFrom-SecureStringPlain $first
        if ($firstPlain.Length -lt $MinLength) {
            Write-Warning "Password must be at least $MinLength characters long."
            continue
        }
        $second = Read-Host -Prompt "Confirm $Prompt" -AsSecureString
        $secondPlain = ConvertFrom-SecureStringPlain $second
        if ($firstPlain -cne $secondPlain) {
            Write-Warning "Passwords did not match. Please try again."
            continue
        }
        return $firstPlain
    }
}

function Set-GitHubSecret([string]$Name, [string]$Value, [string]$TargetRepo) {
    $ghArgs = @("secret", "set", $Name)
    if (-not [string]::IsNullOrWhiteSpace($TargetRepo)) {
        $ghArgs += @("--repo", $TargetRepo)
    }
    $Value | & gh @ghArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to set GitHub secret '$Name' (exit code $LASTEXITCODE)."
    }
}

$RepoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))

if ([string]::IsNullOrWhiteSpace($KeystorePath)) {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw "LOCALAPPDATA is not set. Specify -KeystorePath pointing outside the Git repository."
    }
    $KeystorePath = Join-Path $env:LOCALAPPDATA "PhoneCam\signing\phonecam-release.jks"
}
$ResolvedKeystorePath = [IO.Path]::GetFullPath($KeystorePath)

# Guard: never allow placing a signing keystore inside the Git repository.
$RepoPrefix = $RepoRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
if ($ResolvedKeystorePath.StartsWith($RepoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to store keystore inside the Git repository ($RepoRoot). Use a secure path outside the repo, such as `%LOCALAPPDATA%\PhoneCam\signing\phonecam-release.jks`."
}

$KeytoolPath = Resolve-KeytoolPath
if (-not $KeytoolPath) {
    throw "JDK 'keytool' was not found on PATH or in JAVA_HOME. Install JDK 17+ (e.g., Eclipse Temurin) and retry."
}

Write-Host "==> PhoneCam Android Persistent Signing Setup" -ForegroundColor Cyan
Write-Host "    keytool  : $KeytoolPath"
Write-Host "    keystore : $ResolvedKeystorePath"
Write-Host "    alias    : $KeyAlias"
Write-Host ""

$storePassword = $null
$keyPassword = $null
$keystoreBase64 = $null

try {
    if (-not (Test-Path $ResolvedKeystorePath)) {
        Write-Host "==> Generating new 4096-bit RSA PKCS12 release keystore..." -ForegroundColor Green
        $keystoreDir = Split-Path -Parent $ResolvedKeystorePath
        if (-not (Test-Path $keystoreDir)) {
            New-Item -ItemType Directory -Path $keystoreDir -Force | Out-Null
        }

        $storePassword = Read-ConfirmedPassword -Prompt "Keystore password (min 6 chars)"
        Write-Host "Press Enter to reuse the keystore password for the key password (recommended for PKCS12)."
        $keyPrompt = Read-Host -Prompt "Key password (optional, Enter to reuse)" -AsSecureString
        $keyPlain = ConvertFrom-SecureStringPlain $keyPrompt
        if ([string]::IsNullOrEmpty($keyPlain)) {
            $keyPassword = $storePassword
        } else {
            if ($keyPlain.Length -lt 6) {
                throw "Key password must be at least 6 characters long."
            }
            $keyPassword = $keyPlain
        }

        $env:PHONECAM_KT_STOREPASS = $storePassword
        $env:PHONECAM_KT_KEYPASS = $keyPassword

        & $KeytoolPath `
            -genkeypair `
            -v `
            -storetype PKCS12 `
            -keystore $ResolvedKeystorePath `
            -alias $KeyAlias `
            -keyalg RSA `
            -keysize 4096 `
            -validity $ValidityDays `
            -dname $DistinguishedName `
            -storepass:env PHONECAM_KT_STOREPASS `
            -keypass:env PHONECAM_KT_KEYPASS

        if ($LASTEXITCODE -ne 0) {
            throw "keytool failed to generate the keystore (exit code $LASTEXITCODE)."
        }

        Write-Host ""
        Write-Host "Created persistent keystore at: $ResolvedKeystorePath" -ForegroundColor Green
        Write-Host "IMPORTANT: Back up this file and its passwords in a password manager. Never commit it to Git." -ForegroundColor Yellow
    } else {
        Write-Host "==> Persistent keystore already exists (preserving existing signing identity)." -ForegroundColor Yellow
        if ($ApplyGitHubSecrets) {
            $storeSecure = Read-Host -Prompt "Existing keystore password" -AsSecureString
            $storePassword = ConvertFrom-SecureStringPlain $storeSecure
            $keySecure = Read-Host -Prompt "Existing key password (Enter to reuse keystore password)" -AsSecureString
            $keyPlain = ConvertFrom-SecureStringPlain $keySecure
            $keyPassword = if ([string]::IsNullOrEmpty($keyPlain)) { $storePassword } else { $keyPlain }

            # Verify keystore + alias before uploading secrets.
            $env:PHONECAM_KT_STOREPASS = $storePassword
            & $KeytoolPath `
                -list `
                -keystore $ResolvedKeystorePath `
                -alias $KeyAlias `
                -storepass:env PHONECAM_KT_STOREPASS | Out-Null
            if ($LASTEXITCODE -ne 0) {
                throw "Could not verify alias '$KeyAlias' in existing keystore with the supplied password."
            }
        }
    }

    $keystoreBytes = [IO.File]::ReadAllBytes($ResolvedKeystorePath)
    $keystoreBase64 = [Convert]::ToBase64String($keystoreBytes)

    $repoFlag = if (-not [string]::IsNullOrWhiteSpace($Repo)) { " --repo $Repo" } else { "" }

    Write-Host ""
    Write-Host "==> GitHub Actions Secrets Setup" -ForegroundColor Cyan
    Write-Host "Run these commands to configure persistent Android APK signing in CI:"
    Write-Host ""
    Write-Host "  [Convert]::ToBase64String([IO.File]::ReadAllBytes(`"$ResolvedKeystorePath`")) | gh secret set ANDROID_KEYSTORE_BASE64$repoFlag"
    Write-Host "  gh secret set ANDROID_KEYSTORE_PASSWORD$repoFlag"
    Write-Host "  gh secret set ANDROID_KEY_ALIAS$repoFlag --body `"$KeyAlias`""
    Write-Host "  gh secret set ANDROID_KEY_PASSWORD$repoFlag"
    Write-Host ""
    Write-Host "For local signed release builds (apps/android), set:"
    Write-Host "  `$env:PHONECAM_KEYSTORE_PATH     = `"$ResolvedKeystorePath`""
    Write-Host "  `$env:PHONECAM_KEY_ALIAS         = `"$KeyAlias`""
    Write-Host "  `$env:PHONECAM_KEYSTORE_PASSWORD = '<your keystore password>'"
    Write-Host "  `$env:PHONECAM_KEY_PASSWORD      = '<your key password>'"
    Write-Host ""

    if ($ApplyGitHubSecrets) {
        if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
            throw "GitHub CLI ('gh') is not installed or not on PATH. Install it from https://cli.github.com/ and run 'gh auth login'."
        }
        & gh auth status | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "GitHub CLI is not authenticated. Run 'gh auth login' first."
        }

        Write-Host "==> Applying signing secrets to GitHub Actions..." -ForegroundColor Green
        Set-GitHubSecret -Name "ANDROID_KEYSTORE_BASE64" -Value $keystoreBase64 -TargetRepo $Repo
        Set-GitHubSecret -Name "ANDROID_KEYSTORE_PASSWORD" -Value $storePassword -TargetRepo $Repo
        Set-GitHubSecret -Name "ANDROID_KEY_ALIAS" -Value $KeyAlias -TargetRepo $Repo
        Set-GitHubSecret -Name "ANDROID_KEY_PASSWORD" -Value $keyPassword -TargetRepo $Repo
        Write-Host "All four GitHub Actions secrets updated successfully." -ForegroundColor Green
    }
}
finally {
    Remove-Item Env:PHONECAM_KT_STOREPASS -ErrorAction SilentlyContinue
    Remove-Item Env:PHONECAM_KT_KEYPASS -ErrorAction SilentlyContinue
    $storePassword = $null
    $keyPassword = $null
    $keystoreBase64 = $null
}
