<p align="center">
  <img src="assets/logo.png" alt="PhoneCam Logo" width="112" height="112" />
</p>

<h1 align="center">PhoneCam</h1>

<p align="center">
  <strong>Free, open-source, watermark-free Android-to-PC webcam in native Rust + GPUI — no 480p paywall, no ads, ~50 MB RAM.</strong>
</p>

<p align="center">
  <a href="https://github.com/primahagantha/phonecam/actions/workflows/ci.yml"><img src="https://github.com/primahagantha/phonecam/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/primahagantha/phonecam/releases"><img src="https://img.shields.io/badge/release-v1.3.0%20%7C%201.4.0--nightly.1-blue.svg" alt="Release v1.3.0 | 1.4.0-nightly.1" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License" /></a>
  <img src="https://img.shields.io/badge/platform-Windows%2011%20x64%20%7C%20Android%208%2B-lightgrey.svg" alt="Platform: Windows 11 x64 | Android 8+" />
  <img src="https://img.shields.io/badge/rust-1.85%2B-orange.svg" alt="Rust 1.85+" />
</p>

> **Preview / Demo Asset Placeholder:** No demo capture is committed yet. Drop a desktop + virtual-camera recording at `assets/demo.gif` (or `assets/screenshot.png`) and link it here once captured.

---

## Quick Start (Windows 11 + Android)

1. **Windows Desktop** — download from [GitHub Releases](https://github.com/primahagantha/phonecam/releases):
   - **Installer (`PhoneCam-Setup-<version>.exe`)**: installs `phonecam.exe`, bundles `adb.exe` + `PhoneCam.apk`, registers the Windows Media Foundation virtual camera, and adds inbound firewall rules (`TCP 48721`, `UDP 48720`).
   - **Portable Zip (`PhoneCam-<version>-windows-x64.zip`)**: extract, right-click `install-virtual-camera.bat` -> **Run as administrator**, then run `run-portable.bat`.
2. **Android App (`PhoneCam.apk`)**:
   - Sideload `PhoneCam.apk` from Releases (Android 8.0 / API 26+), **or** plug in via USB with **USB Debugging** enabled and click **Install app on phone** in the desktop Setup tab.
3. **Connect & Select Camera**:
   - **USB**: plug in your cable (`adb reverse tcp:48721 tcp:48721` runs automatically).
   - **Wi-Fi**: ensure both devices share a LAN and scan the desktop QR code from the Android app.
   - Select **PhoneCam Camera** in OBS Studio, Zoom, Discord, Microsoft Teams, Google Meet, or any browser WebRTC app.

---

## Platform & Feature Status

| Target / Capability | Status | Details |
| :--- | :--- | :--- |
| **Windows 11 (`22621+` x64)** | **Supported** | GPUI (DirectX 11) + `openh264` single decode + Media Foundation virtual camera (`MFCreateVirtualCamera`) via `C:\ProgramData\PhoneCam\framebuffer-v2.bin`. |
| **Android (`API 26+`)** | **Supported** | Camera2 + hardware `MediaCodec` H.264 + `phonecam-android-core` (Rust JNI) over `arm64-v8a`, `armeabi-v7a`, `x86_64`. |
| **USB & Wi-Fi Transport** | **Supported** | Unified TCP `48721` protocol (`adb reverse` on USB; UDP `48720` beacon + QR pairing on Wi-Fi), `HMAC-SHA256` auth, monotonic nonce replay guard. |
| **Linux (`v4l2loopback`)** | **Experimental / In Progress** | Core crates and desktop app build on Linux (`scripts/setup-linux.sh` installs `v4l2loopback` deps), but `phonecam-framebus` currently returns `NotWindows` — `/dev/videoX` frame output is not wired yet. |
| **Windows 10 (DirectShow)** | **Roadmap** | Desktop preview runs on D3D11, but `PhoneCamVirtualCamera.dll` requires Windows 11 `MFCreateVirtualCamera`. DirectShow COM support is planned. |
| **macOS (`CoreMediaIO`)** | **Roadmap** | Core libraries compile on macOS; no CoreMediaIO camera extension or packaged macOS app yet. |
| **Virtual Microphone** | **Roadmap** | Android PCM (`ChannelRole::Audio = 3`) currently drives the desktop UI **Audio Monitor (16-segment LED VU meter)**; OS virtual mic / WASAPI routing is planned. |
| **iOS Client** | **Roadmap** | Not started. |

> **Virtual Camera Binaries Transparency Note:** `dist/unpacked/PhoneCamVirtualCamera.dll` and `dist/unpacked/phonecam-vcam-register.exe` are currently shipped as prebuilt C++ Windows Media Foundation binaries pending C++ source cleanup and open-sourcing. All Rust crates (`crates/`) and the Android client (`apps/android/`) are open source under MIT / Apache-2.0.

---

## Architecture at a Glance

```
Android (Camera2 + MediaCodec H.264 + AudioRecord)
   └──► TCP 48721 (Wi-Fi LAN or USB adb reverse)
           └──► phonecam.exe (Single Rust + GPUI process)
                   ├── openh264 single in-process decode
                   ├── NV12 ──► Shared Memory Seqlock (68B header) ──► PhoneCamVirtualCamera.dll (svchost.exe)
                   ├── BGRA ──► GPUI D3D11 Preview (+ optional Labs effects)
                   └── PCM  ──► Audio Monitor (16-segment LED VU meter)
```

Key invariants (see [docs/architecture.md](docs/architecture.md) for full details):
- **Single-decode pipeline**: H.264 Annex-B is decoded once per frame via `openh264`, avoiding dual-decode CPU overhead and preview/camera drift.
- **Resilient virtual camera**: On disconnect, `phonecam-framebus` holds the last frame for 3 seconds before publishing a disconnected NV12 slate so OBS/Zoom/Meet never lose the camera device.
- **Local-only security**: Zero cloud calls or telemetry. Pairing uses a 128-bit single-use QR token (5-minute TTL), persistent 256-bit secrets (`%APPDATA%\PhoneCam\devices.json`), `HMAC-SHA256`, monotonic replay nonces, and hex-token log redaction.
- **Controls & Labs**: Supports hardware camera switching, manual EV / Kelvin WB / diopter focus, anti-flicker (`50Hz`/`60Hz`/`Auto`), mirror/rotation, pure-black OLED phone mode, and experimental local **Labs** toggles (heuristic YCbCr auto-framing, hand gestures, portrait bokeh, color LUTs). See [docs/pro-features.md](docs/pro-features.md).

---

## Building from Source

### Prerequisites
- Rust toolchain (`1.85+` stable)
- **Windows 11 (x64)**: MSVC Build Tools + Windows SDK (for GPUI / `windows` crate) and [NSIS](https://nsis.sourceforge.io/) (optional, for installer packaging)
- **Android**: JDK 17, Android SDK (API 34), Android NDK + `cargo-ndk` (if rebuilding `.so` JNI libs)

### Desktop (Windows)
```powershell
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p phonecam-app
```

To package the release binary and NSIS installer:
```powershell
cargo build --release -p phonecam-app
Copy-Item target\release\phonecam.exe dist\unpacked\phonecam.exe -Force
makensis scripts\installer.nsi
```

### Android & JNI Core
```powershell
# Optional: rebuild Rust JNI .so libraries for arm64-v8a, armeabi-v7a, x86_64
.\scripts\build-android-core.ps1

# Build and test Android APK
cd apps\android
.\gradlew.bat test
.\gradlew.bat assembleRelease
```

---

## Workspace Layout

| Path | Role |
| :--- | :--- |
| `crates/phonecam-protocol` | Binary framing, control/video messages, `HMAC-SHA256` & log redaction |
| `crates/phonecam-framebus` | Windows shared-memory 68-byte seqlock ring & NV12 writer/reader |
| `crates/phonecam-media` | `openh264` decoder, YUV/NV12/BGRA conversion, Labs CV & effects |
| `crates/phonecam-core` | TCP/UDP transport, pairing/session state machine, adaptive quality ladder |
| `crates/phonecam-ui` | GPUI desktop views, preview stage, control rail, VU meter |
| `crates/phonecam-app` | Windows binary, Win32 system tray, ADB supervisor, runtime orchestration |
| `crates/phonecam-android-core` | Rust JNI library (`cdylib`) shared with the Android app |
| `apps/android` | Kotlin + Jetpack Compose Android client (Camera2 + MediaCodec) |
| `docs/` | [Architecture](docs/architecture.md), [Platform Support](docs/platform-support.md), [Pro & Labs Features](docs/pro-features.md) |

---

## Troubleshooting

- **Wi-Fi discovery / pairing fails**: Ensure PC and phone are on the same subnet (guest Wi-Fi / client isolation blocks UDP `48720` and TCP `48721`). Allow `phonecam.exe` through Windows Defender Firewall on Private networks, or scan the QR code directly.
- **USB not detected**: Enable **Developer options -> USB debugging** on Android and accept the RSA debugging prompt on the phone screen.
- **Camera missing in Zoom/OBS/Discord**: Verify registration as Administrator via `phonecam-vcam-register.exe --status` (or re-run `install-virtual-camera.bat`). Requires Windows 11 Build `22621+`.
- **Port conflict (`48721`)**: Override the service port via `$env:PHONECAM_PORT = "48791"`.

---

## Contributing & License

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) and [CHANGELOG.md](CHANGELOG.md). Commits follow [Conventional Commits](https://www.conventionalcommits.org/).

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option (see [LICENSE](LICENSE)).
