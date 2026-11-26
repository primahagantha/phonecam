# PhoneCam Platform Support & Roadmap

This document reflects the **verified current status** of each platform and output pipeline in PhoneCam, separating what works today from experimental work and future roadmap targets.

---

## 1. Platform Support Matrix

| Platform | Minimum Version | Architecture | Video Output Pipeline | Transport | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Windows 11** | Build 22621 (22H2)+ | `x86_64` | Windows Media Foundation Virtual Camera (`MFCreateVirtualCamera`) | USB (ADB) & Wi-Fi | **Supported** |
| **Android** | Android 8.0 (API 26)+ | `arm64-v8a`, `armeabi-v7a`, `x86_64` | Camera2 API + Hardware `MediaCodec` H.264 | USB (ADB) & Wi-Fi | **Supported** |
| **Linux** | Ubuntu 20.04+ / Debian 11+ / Fedora 38+ / Arch | `x86_64` | `v4l2loopback` (writer in progress; `framebus` currently Windows-only) | USB (ADB) & Wi-Fi | **Experimental / In Progress** |
| **Windows 10** | Build 17763 (1809)+ | `x86_64` | DirectShow COM virtual camera (desktop preview runs on D3D11; no vcam yet) | USB (ADB) & Wi-Fi | **Roadmap** |
| **macOS** | macOS 12.3 (Monterey)+ | `arm64`, `x86_64` | CoreMediaIO camera extension (core crates compile; no vcam/app release) | USB (ADB) & Wi-Fi | **Roadmap** |
| **iOS** | — | `arm64` | Native iOS capture client | — | **Roadmap** |

> **Virtual Camera Binary Note:** On Windows 11, `dist/unpacked/PhoneCamVirtualCamera.dll` and `dist/unpacked/phonecam-vcam-register.exe` are currently shipped as prebuilt Media Foundation binaries pending C++ source cleanup and open-sourcing.

---

## 2. Windows 11 (`x86_64`) — Supported

### Requirements
- **OS**: Windows 11 Build `22621` (22H2) or newer (`x86_64`). Required for `MFCreateVirtualCamera` used by `PhoneCamVirtualCamera.dll`.
- **GPU**: DirectX 11 / Direct3D 11 compatible GPU for GPUI hardware-accelerated preview rendering.
- **USB Mode**: Bundled `adb.exe`, `AdbWinApi.dll`, and `AdbWinUsbApi.dll` are included in the installer and portable zip. USB Debugging must be enabled on the Android device.
- **Firewall**: Inbound TCP `48721` and UDP `48720` are needed for Wi-Fi pairing/streaming (configured automatically by the NSIS installer).

### Virtual Camera Registration
The NSIS installer (`PhoneCam-Setup-<version>.exe`) registers the virtual camera automatically. For the portable zip (`PhoneCam-<version>-windows-x64.zip`):
1. Right-click `install-virtual-camera.bat` and select **Run as administrator** (runs `phonecam-vcam-register.exe --register`).
2. Verify registration at any time with:
   ```cmd
   phonecam-vcam-register.exe --status
   ```
3. Launch via `run-portable.bat` or `phonecam.exe`.
4. To unregister later, run `uninstall-virtual-camera.bat` as administrator.

### Windows 10 Note
The GPUI desktop companion runs on Windows 10 (`1809` / Build `17763`+) with Direct3D 11 preview, **but virtual camera registration requires Windows 11 (`MFCreateVirtualCamera`)**. A DirectShow COM virtual camera driver for Windows 10 is on the Roadmap.

---

## 3. Android (`arm64-v8a`, `armeabi-v7a`, `x86_64`) — Supported

The Android client (`apps/android`) uses Camera2 + hardware `MediaCodec` H.264 encoding paired with `phonecam-android-core` (Rust JNI) for framing and HMAC-SHA256 authentication.

### Target ABIs & Outputs (`apps/android/app/build.gradle.kts`)
- `arm64-v8a`: Modern 64-bit ARM devices (recommended).
- `armeabi-v7a`: Older 32-bit ARM devices.
- `x86_64`: Android emulators and ChromeOS.
- Universal APK (`PhoneCam.apk`): Bundles all three ABIs for sideloading.

### Build & Test Commands
```powershell
# Compile Rust JNI libraries (requires ANDROID_NDK_HOME and cargo-ndk)
.\scripts\build-android-core.ps1

# Run JVM unit tests and build release APKs
cd apps\android
.\gradlew.bat test
.\gradlew.bat assembleRelease
```

---

## 4. Linux (`x86_64`) — Experimental / In Progress

The Rust core crates (`phonecam-protocol`, `phonecam-core`, `phonecam-media`, `phonecam-framebus`, `phonecam-android-core`) and `phonecam-app` compile on Linux, and `scripts/setup-linux.sh` installs build dependencies (`vulkan`, `x11`/`wayland`, `alsa`, `adb`) and loads `v4l2loopback` at `/dev/video10`.

**Current limitation:** `crates/phonecam-framebus/src/writer.rs` currently stubs non-Windows targets to return `Err(WriterError::NotWindows)`. Live virtual camera frame output to `/dev/video10` via `v4l2loopback` is not yet wired, so Linux virtual camera streaming is **experimental / in progress**.

---

## 5. macOS, iOS & Virtual Microphone — Roadmap

- **macOS (`CoreMediaIO`)**: `phonecam-protocol`, `phonecam-core`, and `phonecam-media` compile and pass unit tests on macOS in CI, but no `CMIOExtensionProvider` virtual camera or packaged macOS app exists yet.
- **Virtual Microphone Audio Routing**: PhoneCam currently streams Android microphone PCM over `ChannelRole::Audio = 3` to drive the desktop **Audio Monitor (16-segment LED VU meter)**. Routing audio to an OS Virtual Microphone or WASAPI loopback endpoint is on the Roadmap.
- **iOS**: Planned for a future release after Windows/Linux desktop stabilization.
