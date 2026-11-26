# PhoneCam Platform Support & Roadmap

This document reflects the **verified current status** of each platform and output pipeline in PhoneCam, separating what works today from experimental work and future roadmap targets.

---

## 1. Platform Support Matrix

| Platform | Minimum Version | Architecture | Video Output Pipeline | Transport | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Windows 11** | Build 22621 (22H2)+ | `x86_64` | Open-source C++ Media Foundation Virtual Camera (`MFCreateVirtualCamera`) | USB (ADB) & Wi-Fi | **Supported** |
| **Android** | Android 8.0 (API 26)+ | `arm64-v8a`, `armeabi-v7a`, `x86_64` | Camera2 API + Hardware `MediaCodec` H.264 | USB (ADB) & Wi-Fi | **Supported** |
| **Linux** | Ubuntu 20.04+ / Debian 11+ / Fedora 38+ / Arch | `x86_64` | `v4l2loopback` NV12 `FrameWriter` (`VIDIOC_S_FMT` + `write` to `/dev/video10` / `PHONECAM_V4L2_DEVICE`) | USB (ADB) & Wi-Fi | **Experimental** (CI-compiled; live Linux verification pending) |
| **Windows 10** | Build 17763 (1809)+ | `x86_64` | DirectShow COM virtual camera (desktop preview runs on D3D11; no vcam yet) | USB (ADB) & Wi-Fi | **Roadmap** |
| **macOS** | macOS 12.3 (Monterey)+ | `arm64`, `x86_64` | CoreMediaIO camera extension (core crates compile; no vcam/app release) | USB (ADB) & Wi-Fi | **Roadmap** |
| **iOS** | — | `arm64` | Native iOS capture client | — | **Roadmap** |

> **100% Open-Source Virtual Camera:** The Windows Media Foundation C++ source code (`native/virtual-camera/` and `native/shared/`) is open source in this repository. No prebuilt `.dll` or `.exe` binaries are tracked in Git — run `.\scripts\build-vcam.ps1` to compile `PhoneCamVirtualCamera.dll` and `phonecam-vcam-register.exe`, and `.\scripts\fetch-adb.ps1` to stage official Google ADB tools into `dist/unpacked/`.

---

## 2. Windows 11 (`x86_64`) — Supported

### Requirements
- **OS**: Windows 11 Build `22621` (22H2) or newer (`x86_64`). Required for `MFCreateVirtualCamera` used by `PhoneCamVirtualCamera.dll`.
- **GPU**: DirectX 11 / Direct3D 11 compatible GPU for GPUI hardware-accelerated preview rendering.
- **USB Mode**: Official Google `adb.exe`, `AdbWinApi.dll`, and `AdbWinUsbApi.dll` (staged via `scripts/fetch-adb.ps1` for releases and portable zips). USB Debugging must be enabled on the Android device.
- **Firewall**: Inbound TCP `48721` and UDP `48720` are needed for Wi-Fi pairing/streaming (configured automatically by the NSIS installer).

### Building Native Virtual Camera & Staging ADB
```powershell
# Compile PhoneCamVirtualCamera.dll & phonecam-vcam-register.exe from native/ via CMake
.\scripts\build-vcam.ps1

# Stage adb.exe, AdbWinApi.dll, AdbWinUsbApi.dll from local Android SDK or Google platform-tools
.\scripts\fetch-adb.ps1
```

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

## 4. Linux (`x86_64`) — Experimental

The Rust workspace including `phonecam-app` and `phonecam-ui` compiles on Ubuntu Linux in CI (`cargo check -p phonecam-app -p phonecam-ui`), and `scripts/setup-linux.sh` installs build dependencies (`vulkan`, `x11`/`wayland`, `alsa`, `adb`) and loads `v4l2loopback` at `/dev/video10`.

`crates/phonecam-framebus/src/writer.rs` (`#[cfg(target_os = "linux")]`) implements `v4l2loopback` output using `VIDIOC_S_FMT` (`V4L2_PIX_FMT_NV12`) and `libc::write` targeting `PHONECAM_V4L2_DEVICE` (defaulting to `/dev/video10`). Live end-to-end runtime verification on a physical Linux desktop with `v4l2loopback` loaded is still pending.

---

## 5. Audio Routing, macOS & iOS — Status & Roadmap

- **Windows Audio Routing (`waveOut` — Supported)**: `crates/phonecam-app/src/audio_out.rs` routes incoming 16-bit mono PCM (`48000 Hz` / `16000 Hz`, sample rate reported via `PhoneState.audioSampleRate`) through WinMM `waveOut` to a selected playback device (`"default"` or named devices such as VB-Audio Virtual Cable `"CABLE Input"`), while defaulting to `monitor_only` (VU meter only) to prevent speaker feedback.
- **Native Kernel Virtual Microphone Driver (Roadmap)**: An OS-level kernel virtual microphone capture driver (exposing a standalone Windows mic device without VB-Audio Virtual Cable) remains on the Roadmap.
- **macOS (`CoreMediaIO`)**: `phonecam-protocol`, `phonecam-core`, and `phonecam-media` compile and pass unit tests on macOS in CI, but no `CMIOExtensionProvider` virtual camera or packaged macOS app exists yet.
- **iOS**: Planned for a future release after Windows/Linux desktop stabilization.
