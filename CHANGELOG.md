# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

---

## [1.5.1] - 2026-11-27

### Changed
- **Full Optical Studio Logo Redesign & Cross-Platform Icon Sync**:
  - Redesigned the PhoneCam brand mark as a precision concentric optical studio lens inside a matte obsidian squircle with sapphire-cobalt multi-coated glass and an emerald live tally dot, generated via reproducible `scripts/generate-icons.py` (`assets/logo.svg`, `assets/logo.png` 1024px, `assets/playstore-icon-512.png`, multi-size Windows `assets/icon.ico` & `dist/unpacked/icon.ico` 16–256px).
  - Synced Android launcher icons with dedicated Adaptive Icon VectorDrawables (`ic_launcher_background.xml`, `ic_launcher_foreground.xml` inside the `66/108dp` safe zone, Android 13+ `ic_launcher_monochrome.xml`, `ic_launcher_round.xml`) plus legacy `mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}` PNGs.
  - Added Windows `.exe` embedded resource icon support (`crates/phonecam-app/app.rc` + `build.rs`) and refreshed `assets/hero-showcase.png` & `README.md`.

---

## [1.5.0] - 2026-11-27

### Added
- **Adaptive Android Studio UI/UX Redesign & Undistorted 16:9 Camera Preview** (`9b28ca8`):
  - Redesigned the Android companion app with a dark obsidian Studio shell: cutout-safe left `NavigationRail`, compact `StudioTopBar`, subtle danger `Stop` action, and independently scrollable `ConnectionCard` in landscape, plus clean portrait ergonomics and `CameraPreviewTransformTest` unit coverage.
  - Fixed camera preview stretching ("gepeng") across portrait and landscape orientations by computing aspect-ratio-preserving `TextureView` transforms (`FitCenter` 16:9) from sensor orientation, display rotation, and stream dimensions.
- **Opt-In GitHub Update Checker (Desktop & Android)** (`6073fc4`, `29715d7`, `af2ade9`):
  - Added user-initiated GitHub Releases update checkers (`crates/phonecam-app/src/updater.rs`, `apps/android/.../update/GitHubUpdateChecker.kt`) with SemVer 2.0.0 comparison, inline release changelog preview, EN/ID Android localization, and strict `https://github.com/primahagantha/phonecam` URL validation. Strictly opt-in: never executes on startup or background timers.
- **Bundled Typography & Studio UI Overhaul** (`29715d7`, `af2ade9`):
  - Embedded Inter (`Regular`, `Medium`, `SemiBold`, `Bold`) and JetBrains Mono (`Regular`, `Medium`) fonts under `crates/phonecam-ui/assets/fonts/` and registered them at startup via GPUI `text_system().add_fonts()`, eliminating DirectWrite `No matching font found` errors.
  - Polished both Desktop GPUI and Android Jetpack Compose dark obsidian Studio UI with refined spacing, bordered control wells, legible typography, and improved action ergonomics.
- **Android Release Signing Helper & APK Signature Recovery** (`6073fc4`, `af2ade9`):
  - Added `scripts/setup-android-signing.ps1` to generate a persistent 4096-bit RSA PKCS12 release keystore outside the repository and configure GitHub Actions secrets.
  - Added automatic `INSTALL_FAILED_UPDATE_INCOMPATIBLE` recovery in desktop USB APK installation (`crates/phonecam-app/src/adb.rs`).
- **Live Demo Media & README Refresh**:
  - Added live desktop + Android showcase (`assets/hero-showcase.png`), real streaming motion clip (`assets/demo.gif`), competitor comparison table, and FAQ in `README.md`.

### Fixed & Optimized
- **Desktop UI Layout & Release-APK USB Auto-Connect**:
  - Fixed GPUI/Taffy zero-width `.truncate()` text collapse (`...`) across `Toggle`, `key_value`, pairing dialog, USB device card, gesture, and resolution preset rows (`crates/phonecam-ui/src/views/widgets.rs`, `panels.rs`), and capitalized the disconnected paired-phone panel heading (`Paired phone`).
  - Added non-exported service detection (`is_service_not_exported`) in `crates/phonecam-app/src/adb.rs` so USB auto-connect cleanly falls back to launching exported `MainActivity` on hardened release APKs where `CameraStreamService` sets `android:exported="false"`.
- **60 FPS Constrained High-Speed Camera2 Session** (`6073fc4`):
  - Fixed Android Camera2 60 FPS high-speed capture setup by omitting mismatched preview surfaces, preferring `[60,60]` / `[30,60]` / `[30,120]` AE FPS ranges, and resolving the largest high-speed resolution `<=` the requested preset.
- **End-to-End Video Latency Reduction** (`6073fc4`, `5b45f07`, `af2ade9`):
  - Cut glass-to-glass streaming latency via `TCP_NODELAY`, bounded 128 KB TCP send/receive buffers, Android `H264Encoder` `KEY_OPERATING_RATE` + 2 ms dequeue timeout, 3-frame drop-oldest video channel queue, low-latency desktop decode/frame-drop pacing, and a zero-allocation `PhoneCamVirtualCamera.dll` shared-memory reader.

---

## [1.4.0] - 2026-11-26

### Added
- **Open-Source C++ Virtual Camera & Build Automation** (`ba724b4`):
  - Open-sourced the full C++17 Windows Media Foundation virtual camera and shared memory source under `native/virtual-camera/` and `native/shared/` (matching `phonecam-framebus` 68-byte seqlock layout).
  - Added `scripts/build-vcam.ps1` (CMake x64 Release build for `PhoneCamVirtualCamera.dll` & `phonecam-vcam-register.exe`) and `scripts/fetch-adb.ps1` (stages official Google `adb.exe`, `AdbWinApi.dll`, `AdbWinUsbApi.dll`).
  - Untracked prebuilt `.dll`/`.exe`/`.apk` blobs from Git and wired C++ vcam builds + Linux full-app checks into CI and release workflows.
- **Windows `waveOut` Audio Playback Routing** (`11b0ce7`):
  - Added `crates/phonecam-app/src/audio_out.rs` using WinMM `waveOut` with an 8-slot pre-allocated ring buffer to route 16-bit mono PCM audio to VB-Audio Virtual Cable (`CABLE Input`) or Windows playback devices while keeping `monitor_only` as the feedback-safe default.
  - Added `audioSampleRate` reporting in `PhoneState` / `AudioStreamer.kt` and a playback device selector under Audio Monitor & Output (noting native kernel virtual mic driver remains on the Roadmap).
- **Linux `v4l2loopback` Writer** (`ac3cd42`):
  - Implemented `#[cfg(target_os = "linux")]` NV12 output (`VIDIOC_S_FMT` + `libc::write` to `/dev/video10` or `PHONECAM_V4L2_DEVICE`) in `crates/phonecam-framebus/src/writer.rs`.

### Fixed
- **Protocol, Core & Media Correctness** (`6450e0b`, `ac3cd42`):
  - Fixed `PhoneStats.dropped` wire mismatch (`f64`), cross-thread `now_us()` `OnceLock` clock, `AdaptiveQuality` timeout handling, `StatsWindow::take` bitrate scaling (`* 8.0`), partial TCP frame timeout desync, and bounded `write_all` retries with TCP keepalive.
  - Added zero-alloc separable $O(W \cdot H)$ portrait blur (`EffectsScratch`), BT.709 HD / BT.601 SD color conversion, `bgra_to_nv12` parity so Labs effects render to the Virtual Camera, bilinear scaling, SPS/PPS IDR-only prepending, and `LocalFree` RAII guard in `apply_acl`.
- **Android Security, Camera2 & UI** (`7deb699`):
  - Locked down `CameraStreamService` (`android:exported="false"` in release + internal `SecureRandom` token), acquired `PARTIAL_WAKE_LOCK` + `WIFI_MODE_FULL_LOW_LATENCY`, reported physical device `orientation`, fixed OLED blackout touch-trap & landscape layout, extracted localized strings, and wired release keystore signing.
- **Desktop UI, Runtime, Tray & Installer** (`a100a2a`, `1aa24e1`, `cf31417`):
  - Overhauled IA with single-source controls, collapsible `Experimental (Labs)` card, interactive sliders, segmented selectors, bundled font fallbacks with tabular figures, logarithmic dBFS VU meter, keyboard `FocusHandle` navigation, and removed `unsafe` `RenderImage` reclamation.
  - Wired `minimize_to_tray`, `auto_connect_usb`, `auto_start_stream`, `auto_rotate`, ADB worker thread offloading, tray Start/Stop Stream, `windows_addresses()` overflow resize, `install_phone_app()` exe-relative APK lookup, single-publish 3s fail-safe latch, and NSIS Windows Defender Firewall rules (`TCP 48721`, `UDP 48720`).
- **Documentation & Repo Hygiene**:
  - Updated `README.md` (<250 lines), `docs/`, and `dist/unpacked/README-QUICKSTART.txt` to accurately reflect open-source C++ virtual camera builds, Windows `waveOut` audio routing, and current platform/roadmap status.

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
