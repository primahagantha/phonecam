# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [1.2.1] - 2026-11-15

### Pro Media Pipeline & Touchless Controls

This release introduces native pure Rust computer vision face tracking, OBSBOT-style hand gesture controls, studio audio streaming with an LED VU meter, unlocked 60 FPS capture, dynamic capability probing, and dedicated pro controls.

### Added
- **Pure Rust Computer Vision Face Detection & Smart Auto-Framing**:
  - Implemented real-time face detection on premultiplied BGRA buffers (`phonecam-media::face_detect`) in pure Rust with zero external ML runtimes (no ONNX/TensorFlow).
  - Guaranteed zero dynamic memory allocations on the 60 FPS hot path using pre-allocated static scratch buffers.
  - Added YCbCr skin chrominance segmentation, candidate component clustering, geometric aspect ratio validation, and Haar-like facial contrast verification.
  - Enhanced `AutoFramingTracker` with a 1.5% spatial deadband to eliminate micro-jitter, aspect-ratio preserving dynamic cropping (`& !1`), and temporal exponential smoothing ($\alpha = 0.12$).
  - Added tracking lock/pause functionality to pin camera framing in place during demonstrations.
  - Added dedicated on/off switch toggle in Capture and Preferences panels (`SideEffect::SetAutoFraming`).
- **OBSBOT-Style Hand Gesture Recognition**:
  - Implemented high-precision, zero-allocation hand gesture detector (`phonecam-media::gesture`) in pure Rust.
  - Multi-feature classification: horizontal projection analysis, finger peak prominence, valley depth thresholds, and palm base solidity ratios.
  - Debounce filter (consecutive frame verification) and refractory cooldown timer to eliminate accidental re-triggers.
  - Full 6-gesture repertoire:
    - ✋ **Open Palm**: Toggle tracking lock / unlock.
    - ✌ **Peace / V-Sign**: Smoothly toggle zoom between 1.0x wide-angle and 2.0x close-up.
    - ☝ **Index Pointing Up**: Set zoom to 1.5x medium presentation scale.
    - 🤙 **Shaka / Call**: Reset framing coordinates to center `(0.50, 0.50)` and reset zoom to 1.0x uncropped.
    - 🤏 **Pinch / OK**: Engage 2.2x close-up macro focal framing.
    - 👍 **Thumbs Up**: Quick confirmation trigger and zoom toggle.
  - Real-time translucent HUD gesture feedback chips rendered atop the GPUI preview stage.
  - Dedicated "Hand Gesture Controls Guide & Customization" reference card with master on/off toggle.
- **Low-Latency Microphone Audio Streaming with 16-Segment LED VU Meter**:
  - Direct 16-bit uncompressed PCM microphone audio capture (48,000 Hz / 16,000 Hz) via Android `AudioRecord`.
  - Multiplexed in unified TCP connection over `ChannelRole::Audio = 3` alongside H.264 video (`ChannelRole::Video = 1`) and control frames (`ChannelRole::Control = 2`).
  - Sub-millisecond timestamp alignment (`timestamp_us`) ensuring perfect lip-sync.
  - 16-segment dynamic LED VU level meter in Capture and Diagnostics panels (10 Green nominal segments, 4 Amber headroom segments, 2 Red clipping segments) with live dBFS and RMS telemetry.
  - Dedicated on/off switch toggles in desktop and mobile app.
- **Unlocked 60 FPS Camera2 & Robust Anti-Flicker**:
  - Resolved Camera2 30 FPS locking by dynamically inspecting `CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES` to select ranges with `upper == 60` (`[60, 60]` or `[30, 60]`).
  - Configured hardware `MediaCodec` with both `KEY_FRAME_RATE = 60` and `KEY_CAPTURE_RATE = 60`.
  - Added mains frequency antibanding / anti-flicker (`Auto`, `50Hz`, `60Hz`, `Off`) protecting against cyclic AC lighting hum, preserved seamlessly across physical lens transitions.
- **Dynamic Resolution Capabilities**:
  - Implemented dynamic hardware sensor capability discovery via `StreamConfigurationMap`.
  - Dynamically detects and advertises supported device resolutions up to 4K (`3840x2160` @ 30 FPS), 1080p @ 60 FPS, 720p @ 60 FPS, and 480p @ 60 FPS.
  - Desktop UI dynamically formats supported resolutions into selectable cards with aspect ratio chips and detailed resolution badges (`res_detail`).
- **Dedicated On/Off Toggles for All Pro Features**:
  - Added dedicated switch toggles for Auto-Framing, Hand Gestures, Audio Streaming, Bokeh Blur, and Color LUTs.
  - Settings persisted across restarts in `%APPDATA%\PhoneCam\desktop-settings.json`.

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
