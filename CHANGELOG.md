# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Changed
- **Documentation & Repo Hygiene**: Rewrote `README.md`, cleaned `docs/`, updated `dist/unpacked/README-QUICKSTART.txt`, and aligned all platform/feature claims (Windows 11 Media Foundation virtual camera, Linux `v4l2loopback` experimental status, Audio Monitor vs Roadmap virtual mic, and Labs visual effects) with verified codebase behavior.
- **Installer & CI/CD**:
  - Added automatic Windows Defender Firewall rules (TCP `48721`, UDP `48720`) on install and removal on uninstall in `scripts/installer.nsi`.
  - Tightened Windows CI clippy checks (`cargo clippy --workspace --all-targets -- -D warnings`) and added Linux checks for `phonecam-framebus`.
  - Added optional Android release keystore decoding from GitHub Secrets, optional `cargo-ndk` JNI build when `ANDROID_NDK_HOME` is present, and `SHA256SUMS.txt` checksum generation in `.github/workflows/release.yml`.

---

## [1.4.0-nightly.1] - 2026-11-20

### Added
- **Android Instant Preview & OLED Blackout Mode** (`0307fd3`):
  - Added instant preview toggle and pure-black OLED ambient blackout mode on Android, eliminating view borders for full-bleed 0-nit blackout rendering.
- **Release Pipeline**:
  - Bumped workspace and Android app versions to `1.4.0-nightly.1` for prerelease validation (`dd0a04d`, `99aa6e5`).

---

## [1.3.0] - 2026-11-20

### Fixed
- **Media & Encoder Stability** (`cd55d26`, `b181425`):
  - Enforced constant-bitrate (CBR) low-latency encoding, reduced frame flush delay, and stabilized Camera2 auto-exposure framerate selection.
  - Improved camera noise reduction, continuous autofocus behavior, and thermal pipeline resilience.
- **UI & Pairing Reliability** (`53be04c`):
  - Improved Android battery charging detection in status telemetry, hardened QR pairing stage handling, and expanded status strip unit tests.

---

## [1.2.1] - 2026-11-15

### Pro Media Pipeline & Touchless Controls

This release introduces pure Rust computer vision face tracking, OBSBOT-style hand gesture controls, audio monitoring with an LED VU meter, Camera2 60 FPS range probing, dynamic capability negotiation, and modular effect toggles.

### Added
- **Pure Rust Computer Vision Face Detection & Smart Auto-Framing** (Labs):
  - Implemented heuristic face detection on premultiplied BGRA buffers (`phonecam-media::face_detect`) in pure Rust with zero external ML runtimes (no ONNX/TensorFlow).
  - Added YCbCr skin chrominance segmentation, candidate component clustering, geometric aspect ratio validation, and Haar-like facial contrast verification.
  - Added `AutoFramingTracker` with a 1.5% spatial deadband, aspect-ratio preserving dynamic cropping (`& !1`), temporal exponential smoothing ($\alpha = 0.12$), and tracking lock/pause state.
- **Hand Gesture Recognition** (Labs):
  - Implemented hand gesture classifier (`phonecam-media::gesture`) evaluating horizontal projection histograms, finger peak prominence, valley depth thresholds, and palm base solidity ratios with debounce and cooldown filtering.
  - Added 6 gestures (Open Palm, Peace/V-Sign, Index Pointing Up, Shaka/Call, Pinch/OK, Thumbs Up) and translucent HUD gesture chips on the GPUI preview stage.
- **Audio Monitor with 16-Segment LED VU Meter**:
  - Added 16-bit uncompressed PCM microphone capture (48,000 Hz / 16,000 Hz) via Android `AudioRecord` multiplexed over `ChannelRole::Audio = 3`.
  - Added a 16-segment LED VU level meter in the desktop UI (10 Green nominal, 4 Amber headroom, 2 Red clipping segments).
- **Camera2 AE Range & Anti-Flicker**:
  - Added Camera2 `CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES` inspection and mains frequency antibanding / anti-flicker (`Auto`, `50Hz`, `60Hz`, `Off`) preserved across lens transitions.
- **Dynamic Resolution Capabilities**:
  - Added hardware sensor capability discovery via `StreamConfigurationMap` with dynamic resolution cards in the desktop UI.
- **Persistent Settings**:
  - Persisted desktop preferences and toggles in `%APPDATA%\PhoneCam\desktop-settings.json`.

---

## [1.0.0] - 2026-11-13

### Initial Native Rust Rewrite

This milestone represents a ground-up rewrite of PhoneCam, migrating from the legacy Electron desktop application to a single-process native Rust and GPUI architecture.

### Added
- **Native GPUI Desktop Application**:
  - Replaced Chromium and Electron with a unified native Rust binary powered by Zed's GPU-accelerated **GPUI** framework.
  - Packaged standalone Windows NSIS installer (`PhoneCam-Setup-1.0.0.exe`).
- **Single In-Process H.264 Decoding**:
  - Integrated `openh264` directly into the runtime pipeline, eliminating legacy fragmented-MP4 temporary disk-muxing (~24 ms batch delay).
  - Decodes H.264 Annex-B bitstreams once, feeding NV12 to Windows shared memory and premultiplied BGRA to the GPUI preview texture.
- **Windows Shared Memory Virtual Camera (Framebus)**:
  - High-throughput seqlock writer publishing frames to `C:\ProgramData\PhoneCam\framebuffer-v2.bin` with a pinned 68-byte packed header for `PhoneCamVirtualCamera.dll` running inside `svchost.exe -k Camera`.
  - Virtual camera registration utility (`phonecam-vcam-register.exe`).
- **Android Native Rust JNI Core**:
  - Introduced `phonecam-android-core` Rust crate cross-compiled for Android ABIs (`arm64-v8a`, `armeabi-v7a`, `x86_64`) via `cargo-ndk`.
- **Dual Connection Modes**:
  - **USB Cable**: Automated ADB reverse port forwarding (`adb reverse tcp:48721 tcp:48721`).
  - **Wi-Fi LAN**: Automated UDP broadcast beacon discovery on port `48720` and QR-code pairing.
- **Dynamic Adaptive Bitrate (ABR) Ladder**:
  - Quality scaling (`1080p60 -> 1080p30 -> 720p30 -> 480p30`) with hysteresis (10s drop dwell, 30s climb dwell) and instant thermal downgrade.
- **Native Windows Tray Integration**:
  - Message-only Win32 window owning `Shell_NotifyIcon` tray lifecycle with close-to-tray background persistence.

### Changed
- Replaced multi-process Electron architecture with a single native Rust process and crossbeam channels (retiring TCP `48723`).
- Persisted paired device authorizations in `%APPDATA%\PhoneCam\devices.json` with strictly increasing `u64` client nonces.

### Security
- 100% offline and local: zero telemetry, zero analytics, zero cloud dependencies.
- Ephemeral 128-bit CSPRNG pairing tokens with 5-minute TTL, `HMAC-SHA256` session authentication, and automated log hex redaction.
