# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [1.0.0] - 2026-11-13

### Initial Native Rust Rewrite

This milestone represents a complete ground-up rewrite of PhoneCam, migrating from the legacy Electron desktop application to a single-process native Rust and GPUI architecture.

### Added
- **Native GPUI Desktop Application**:
  - Replaced Chromium and Electron with a native Rust binary powered by Zed's GPU-accelerated **GPUI** framework.
  - Reduced idle memory consumption by 78% (from ~180 MB down to ~40 MB).
  - Reduced cold startup time to sub-200 milliseconds.
  - Packaged standalone Windows NSIS installer (`PhoneCam-Setup-1.0.0.exe`) weighing under 10 MB.
- **Single In-Process H.264 Decoding**:
  - Integrated Cisco's `openh264` directly into the runtime pipeline.
  - Eliminated legacy fragmented-MP4 temporary disk-muxing, cutting frame decode latency by ~24 milliseconds per batch.
  - Decodes H.264 Annex-B bitstream once, feeding NV12 directly to shared memory and BGRA to the GPUI window texture.
- **Windows Shared Memory Virtual Camera (Framebus)**:
  - High-throughput seqlock writer publishing frames to memory-mapped buffer (`C:\ProgramData\PhoneCam\framebuffer-v2.bin`).
  - Binary-compatible 68-byte header maintaining zero-copy IPC with `PhoneCamVirtualCamera.dll` running inside Windows `svchost.exe -k Camera`.
  - Windows Media Foundation virtual camera registration utility (`phonecam-vcam-register.exe`).
- **Android Native Rust JNI Core**:
  - Introduced `phonecam-android-core` Rust crate cross-compiled for Android ABIs (`arm64-v8a`, `armeabi-v7a`, `x86_64`) via `cargo-ndk`.
  - Seamless bridging between Android Camera2 / `MediaCodec` hardware encoders and desktop protocol.
- **Dual Connection Modes**:
  - **USB Cable**: Automated ADB reverse port forwarding (`adb reverse tcp:48721 tcp:48721`), ensuring zero-configuration, low-jitter wired connections with active phone charging.
  - **Wi-Fi LAN**: Automated UDP broadcast beacon discovery on port `48720` and fast QR-code pairing.
- **Hardware Controls & Streaming Features**:
  - Support for 60 FPS and 30 FPS at 480p, 720p, 1080p, and 4K resolutions.
  - Real-time continuous digital zoom.
  - Remote flash / torchlight toggling.
  - Horizontal mirroring / flip and 90°/180°/270° hardware rotation.
- **Dynamic Adaptive Bitrate (ABR) Ladder**:
  - Intelligent quality scaling (`1080p60 -> 1080p30 -> 720p30 -> 480p30`) based on RTT, packet loss, and phone thermal metrics.
  - Rapid step-down protection with hysteresis (10s drop dwell, 30s climb dwell) and instant thermal downgrade.
- **Native Windows Tray Integration**:
  - Dedicated message-only Win32 window owning `Shell_NotifyIcon` tray lifecycle.
  - Close-to-tray background operation ensuring virtual camera streams remain uninterrupted when the window is closed.
- **Accessibility & Design Rules**:
  - Enforced WCAG 2.1 AA 4.5:1 minimum contrast across all UI surfaces.
  - Tabular figures (`tnum`) for FPS, bitrate, and latency readouts to eliminate visual jitter.
  - Single priority-ordered status headline system (`summarise_status`).
- **Comprehensive Test Coverage**:
  - Over 230 automated unit and integration tests covering wire framing, HMAC crypto, seqlock synchronization, color conversion, and full runtime socket orchestration.

### Changed
- Replaced multi-process Electron architecture (Renderer + Main + C++ Core) with a unified, single-process Rust binary.
- Replaced internal TCP loopback IPC (port 48723) with zero-cost crossbeam channels.
- Modernized pairing protocol to use strictly increasing client nonces to mathematically prevent replay attacks.
- Persisted paired device authorizations in `%APPDATA%\PhoneCam\devices.json`.

### Removed
- Removed Chromium, Node.js, and all Electron packaging dependencies.
- Removed legacy disk-muxed fragmented MP4 capture pipeline.
- Removed dual-decode WebCodecs canvas rendering path.

### Fixed
- Fixed race condition where rapid network degradation caused the quality ladder to drop multiple rungs before the phone confirmed the requested rung.
- Fixed virtual camera registration checks by validating actual device enumerability via `--status`.
- Fixed false positive warnings by cleanly distinguishing platform ADB availability (`adbAvailable`) from physical USB device connection (`available`).
- Fixed device persistence bugs where restarting the desktop invalidated previously paired devices.

### Security
- 100% offline and local: zero telemetry, zero analytics, zero cloud dependencies.
- Ephemeral 128-bit CSPRNG pairing tokens with strict 5-minute timeout.
- HMAC-SHA256 authenticated session control.
- Automated regex-based log sanitization stripping 32- and 64-character hex strings to prevent credential exposure.
