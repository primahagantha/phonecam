---
name: phonecam-dev
description: Developer skill reference for AI agents and human contributors to build, test, package, and debug PhoneCam across desktop and Android.
---

# PhoneCam Developer Skill Guide

This skill guide provides a comprehensive reference of commands, workflows, and diagnostic procedures for building, testing, packaging, and debugging the PhoneCam ecosystem across desktop and Android platforms.

---

## 1. Environment & Prerequisites Checklist

Ensure the following tools and environment variables are active in your session:

| Component | Minimum Version | Path / Environment Variable |
| :--- | :--- | :--- |
| **Rust** | 1.85+ | `rustup default stable-x86_64-pc-windows-msvc` |
| **MSVC C++** | VS 2022 Build Tools | Windows SDK 10.0.22621.0+ |
| **Java JDK** | 17 | `JAVA_HOME` pointing to JDK 17 |
| **Android SDK** | 34+ | `ANDROID_HOME` or `ANDROID_SDK_ROOT` |
| **Android NDK** | 28+ | `ANDROID_NDK_HOME` (e.g., `D:\apps\android-sdk\ndk\28.2.13676358`) |
| **cargo-ndk** | Latest | `cargo install cargo-ndk` |
| **NSIS** | 3.08+ | `makensis.exe` in system `PATH` |

---

## 2. Desktop Application Workflows

### Run Desktop in Development Mode
To launch the desktop app directly:
```powershell
cargo run -p phonecam-app
```

### Overriding the Default Port
If legacy PhoneCam (Electron) is running or port 48721 is occupied, launch PhoneCam on an alternate port:
```powershell
$env:PHONECAM_PORT = 48791
cargo run -p phonecam-app
```

### Building the Release Binary
To compile the optimized native binary:
```powershell
cargo build --release -p phonecam-app
```
Binary output: `target\release\phonecam.exe`.

---

## 3. Automated Testing Workflows

### Complete Workspace Test Suite
Run all 230+ tests across the 7 crates:
```powershell
cargo test --workspace
```

### Testing Individual Subsystems
```powershell
# Wire protocol, HMAC-SHA256, and framing tests
cargo test -p phonecam-protocol

# Shared memory seqlock and 68-byte header tests
cargo test -p phonecam-framebus

# H.264 decode, SPS parsing, and color conversions
cargo test -p phonecam-media

# Transport, handshake, and adaptive bitrate ladder
cargo test -p phonecam-core

# UI design system tokens, contrast checks, and layout rules
cargo test -p phonecam-ui

# End-to-end runtime integration tests
cargo test -p phonecam-app --test end_to_end
```

### Android Unit Tests
Run JVM and JNI unit tests in the Android module:
```powershell
cd apps\android
.\gradlew.bat test
cd ..\..
```

---

## 4. Android Native JNI Cross-Compilation

When changes are made to `crates/phonecam-android-core`:

```powershell
# Compiles JNI shared libraries for arm64-v8a, armeabi-v7a, and x86_64
.\scripts\build-android-core.ps1
```

The script builds release libraries and copies them directly into:
`apps\android\app\src\main\jniLibs\<ABI>\libphonecam_android_core.so`

---

## 5. Android APK Compilation & Deployment

```powershell
cd apps\android

# Build Debug APK
.\gradlew.bat assembleDebug
# Output: apps\android\app\build\outputs\apk\debug\app-debug.apk

# Build Release APK
.\gradlew.bat assembleRelease
# Output: apps\android\app\build\outputs\apk\release\app-release.apk

cd ..\..
```

### Deploying APK to Connected Device via ADB
```powershell
adb install -r apps\android\app\build\outputs\apk\debug\app-debug.apk
```

---

## 6. Packaging the Windows NSIS Installer

Ensure the latest release binaries are placed in `dist\unpacked\`:

```powershell
# 1. Copy fresh release binary
Copy-Item target\release\phonecam.exe dist\unpacked\phonecam.exe -Force

# 2. Compile NSIS installer
makensis scripts\installer.nsi
```
Installer output: `dist\installer\PhoneCam-Setup-1.0.0.exe`.

---

## 7. Windows Virtual Camera Management

The virtual camera driver interacts directly with Windows Media Foundation and the Camera Frame Server (`svchost.exe -k Camera`).

> **Note**: Registration and unregistration commands require an **Administrator** PowerShell prompt.

```powershell
# Register the virtual camera with Windows
.\dist\unpacked\phonecam-vcam-register.exe --register

# Check current registration and device enumeration status
.\dist\unpacked\phonecam-vcam-register.exe --status

# Unregister the virtual camera
.\dist\unpacked\phonecam-vcam-register.exe --unregister
```

### Inspecting Shared Memory Framebus
The framebus backing file is maintained at:
`C:\ProgramData\PhoneCam\framebuffer-v2.bin`

When streaming is live:
- Header size: exactly 68 bytes packed.
- Magic bytes: `0x5043414D` (`PCAM`).
- Layout version: `2`.
- `state`: `1` (Live).
- `seq`: Even number when quiescent.
- `readers`: Written exclusively by driver consumers (never reset by writer).

For complete driver architecture across Windows, Linux (`v4l2loopback`), and macOS (`CoreMediaIO`), see [phonecam-virtualcam](file:///d:/production/phonecam-rust/.agent/skills/phonecam-virtualcam/SKILL.md).

---

## 8. ADB & Android Debugging Workflows

### Inspecting Connected Devices
```powershell
adb devices -l
```

### Managing Reverse Sockets
PhoneCam uses `adb reverse` to route port 48721 from the phone to the desktop over USB:
```powershell
# Check existing reverse sockets
adb reverse --list

# Manually forward if needed
adb reverse tcp:48721 tcp:48721

# Clear all reverse sockets
adb reverse --remove-all
```

### Launching Android Streaming Service via ADB
To remotely start the camera streaming service:
```powershell
adb shell am start-foreground-service `
  -n com.phonecam.camera.debug/com.phonecam.camera.service.CameraStreamService `
  -a com.phonecam.camera.action.CONNECT_MANUAL `
  --es host 127.0.0.1 `
  --ei port 48721
```

### Monitoring Android Logcat
Filter logcat for PhoneCam camera and transport events:
```powershell
adb logcat -v time -s PhoneCam:V CameraStreamService:V
```

---

## 9. Troubleshooting Quick Reference

| Issue | Root Cause | Solution |
| :--- | :--- | :--- |
| `Address already in use (os error 10048)` | Port 48721 is bound by another app | Set `$env:PHONECAM_PORT = 48791` before starting PhoneCam |
| `MFCreateVirtualCamera failed` | Windows build earlier than 22621 | Requires Windows 11 Build 22621 or newer; fallback to DirectShow |
| Camera registered but OBS shows black screen | Shared memory seqlock mismatch | Confirm `magic = 0x5043414D` and `layout_version = 2` |
| `adb: device unauthorized` | Android device prompt not accepted | Unlock phone and tap "Always allow from this computer" |
| JNI `UnsatisfiedLinkError` | Missing `.so` in APK | Run `.\scripts\build-android-core.ps1` before Gradle build |

For full step-by-step diagnostic workflows, see [phonecam-troubleshoot](file:///d:/production/phonecam-rust/.agent/skills/phonecam-troubleshoot/SKILL.md).

---

## 10. Specialized Skills Cross-Reference

PhoneCam includes dedicated skill modules for specific development domains:

- **[phonecam-ci](file:///d:/production/phonecam-rust/.agent/skills/phonecam-ci/SKILL.md)**:
  Complete reference for GitHub Actions matrix (`ci.yml`, `release.yml`), multi-platform builds, semantic versioning, and release artifact generation.
- **[phonecam-virtualcam](file:///d:/production/phonecam-rust/.agent/skills/phonecam-virtualcam/SKILL.md)**:
  Exhaustive virtual camera implementations across Windows (`MFCreateVirtualCamera` & DirectShow COM), Linux (`v4l2loopback`), and macOS (`CoreMediaIO` DAL / `CMIOExtension`).
- **[phonecam-protocol](file:///d:/production/phonecam-rust/.agent/skills/phonecam-protocol/SKILL.md)**:
  Deep guide for binary framing, 40-byte greeting, 20-byte client handshake, HMAC-SHA256 crypto, JSON payloads, replay guard, and diagnostics.
- **[phonecam-troubleshoot](file:///d:/production/phonecam-rust/.agent/skills/phonecam-troubleshoot/SKILL.md)**:
  Step-by-step diagnostic procedures for Wi-Fi discovery, USB ADB reverse tunnels, hardware encoder fallbacks, thermal clamps, and audio sync.
