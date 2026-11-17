# CLAUDE.md — PhoneCam Engineering & Style Guide

This file provides essential commands, architectural standards, code style rules, and commit conventions for Claude Code and AI agents working in the PhoneCam repository.

---

## 1. Build, Lint & Test Commands

### 1.1 Quick Reference
```powershell
# Fast compilation check across the entire workspace
cargo check --workspace

# Run complete automated test suite (413+ tests across 7 crates)
cargo test --workspace

# Run tests with output for a specific crate
cargo test -p phonecam-protocol -- --nocapture
cargo test -p phonecam-framebus -- --nocapture
cargo test -p phonecam-media -- --nocapture
cargo test -p phonecam-core -- --nocapture
cargo test -p phonecam-ui -- --nocapture
cargo test -p phonecam-app --test end_to_end

# Formatting check
cargo fmt --all -- --check

# Strict Clippy linter
cargo clippy --workspace --all-targets -- -D warnings
```

### 1.2 Desktop Application
```powershell
# Run in development mode
cargo run -p phonecam-app

# Run on custom port (e.g. if 48721 is occupied)
$env:PHONECAM_PORT = 48791
cargo run -p phonecam-app

# Build optimized native release binary (target\release\phonecam.exe)
cargo build --release -p phonecam-app
```

### 1.3 Android & JNI
```powershell
# Cross-compile Rust JNI libraries for Android ABIs (arm64-v8a, armeabi-v7a, x86_64)
.\scripts\build-android-core.ps1

# Run Android JVM unit tests
cd apps\android
.\gradlew.bat test
cd ..\..

# Build Android Release APKs
cd apps\android
.\gradlew.bat assembleRelease
cd ..\..
```

### 1.4 Packaging
```powershell
# Copy release binary and compile NSIS installer (dist\installer\PhoneCam-Setup-1.0.0.exe)
Copy-Item target\release\phonecam.exe dist\unpacked\phonecam.exe -Force
makensis scripts\installer.nsi
```

---

## 2. Core Architectural Invariants

PhoneCam enforces critical non-negotiable architectural invariants:

1. **Single Native Process**: Desktop app runs as a single unified process. Never spawn daemon subprocesses or background HTTP/IPC servers (port 48723 is retired).
2. **Single Decode Invariant**: In-process `openh264` decodes incoming H.264 bitstream exactly once. Decoded YUV420P frames are converted in-place to **NV12** (Windows Shared Memory framebus for virtual camera) and **premultiplied BGRA** (GPUI window preview). The preview and virtual camera can never diverge.
3. **68-Byte Framebus Header Contract**:
   - Backing file: `C:\ProgramData\PhoneCam\framebuffer-v2.bin` (grow-only, never truncate).
   - Packed 68-byte header: `magic = 0x5043414D` (`PCAM`), `layout_version = 2`, `header_size = 68`.
   - Seqlock synchronization: writer increments sequence counter to **odd** before writing NV12 payload at offset 68, and **even** when complete.
   - Reader count (`readers`): updated exclusively by virtual camera DLL readers; writer must **never** zero or reset it.
4. **Virtual Camera Isolation**: `PhoneCamVirtualCamera.dll` runs in `svchost.exe -k Camera` under `NT AUTHORITY\LocalService`. On disconnect, framebus keeps publishing last frame for 3.0s, followed by solid black NV12 frame with `state = Disconnected`. Never unregister device.
5. **Unified Transport (USB & Wi-Fi)**: Phone always connects via TCP to port 48721. USB mode executes `adb reverse tcp:48721 tcp:48721` to bridge `127.0.0.1:48721`. Handshake, auth, control, and video protocols are identical.
6. **Authentication & Replay Guard**: First-time pairing uses a 128-bit CSPRNG token (5 min TTL) with explicit desktop UI approval. Session auth uses persistent 256-bit device secret and `HMAC-SHA256(secret, serverNonce || clientNonce)`. Strictly monotonic `client_nonce` protects against replays.
7. **Adaptive Bitrate Ladder**: `1080p60 -> 1080p30 -> 720p30 -> 480p30` with hysteresis (10s down / instant thermal, 30s clean up). Pending lock prevents skipping rungs.
8. **Pure-Rust Computer Vision Face Detection (`phonecam-media::face_detect`)**:
   - Zero dynamic heap allocations on the 60 FPS hot path using pre-allocated scratch grids (`MAX_GRID_W = 160`, `MAX_GRID_H = 120`, `MAX_GRID_CELLS = 19200`).
   - Pipeline: Normalized YCbCr skin chrominance segmentation $\to$ connected component clustering into `CandidateRegion`s $\to$ aspect ratio ($0.65 \le \text{AR} \le 1.35$) & minimum area filtering $\to$ Haar-like contrast polarity verification between upper eye-line and cheek zones.
   - Cinematic PTZ Smoothing: Center target $(X_c, Y_c) \in [0.15, 0.85]$ stabilized via $1.5\%$ spatial deadband hysteresis, exponential temporal moving average ($\alpha = 0.12$, user zoom $1.0\times - 2.5\times$), and aspect-ratio preserving crop (`CropRect`) with even-pixel boundary alignment (`& !1`).
   - Tracking Lock / Pause: Instantly freezes the PTZ crop window in place via UI toggle or the ✋ Open Palm hand gesture.
9. **OBSBOT-Style Touchless Hand Gesture Recognition (`phonecam-media::gesture`)**:
   - Real-time zero-allocation classifier evaluating horizontal projection histograms (`proj_x`), finger peak prominence, valley depth thresholds, and palm base solidity ratio relative to detected face anchor.
   - Debounce & Refractory Filter: Requires candidate gesture to hold for 3 consecutive frames before confirming trigger; enforces a 45-frame (~0.75-1.5s) refractory cooldown to prevent duplicate triggers.
   - 6 Distinct Gestures:
     - ✋ **Open Palm**: Tracking Lock / Unlock (toggles `tracking_locked` state).
     - ✌ **Peace / V-Sign**: Toggle Zoom (smooth interpolation between $1.0\times \leftrightarrow 2.0\times$).
     - ☝ **Pointing Up (Index)**: Medium Zoom ($1.5\times$ presentation framing).
     - 🤙 **Shaka / Call**: Framing & Zoom Reset (recenters $(0.5, 0.5)$ and returns to $1.0\times$).
     - 🤏 **Pinch / OK**: Macro Close-Up ($2.2\times$ detail zoom).
     - 👍 **Thumbs Up**: Quick Confirm / Alternate Zoom toggle.
   - Live Stage HUD Chips: Confirmed gestures trigger non-blocking translucent HUD chip overlays (`StageContent`) over the live GPUI preview stage.
10. **Multiplexed Low-Latency Audio Streaming & 16-Segment LED VU Meter**:
    - Transmitted over unified TCP port `48721` with `ChannelRole::Audio = 3` alongside Control (`1`) and Video (`2`).
    - Uncompressed 16-bit linear PCM (48,000 Hz primary, 16,000 Hz fallback) in 2,048-byte chunks (~21.3 ms) with microsecond timestamp synchronization (`timestamp_us`) aligned with video frames to eliminate lip-sync drift.
    - 16-Segment Dynamic LED VU Meter: Real-time RMS and dBFS calculation embedded in UI panels with 10 Green nominal segments ($-60\text{ dBFS}$ to $-12\text{ dBFS}$), 4 Amber headroom segments ($-12\text{ dBFS}$ to $-3\text{ dBFS}$), and 2 Red digital clipping segments ($> -3\text{ dBFS}$).
11. **Unlocked 60 FPS Camera2 & Robust Anti-Flicker**:
    - Automatically selects AE Target FPS ranges where `upper == 60` (`[60, 60]`, `[30, 60]`) on repeating requests.
    - Configures `MediaCodec` hardware encoder with both `KEY_FRAME_RATE = 60` and `KEY_CAPTURE_RATE = 60`.
    - Antibanding mode (`Auto`, `50Hz`, `60Hz`, `Off`) dynamically selected and preserved across physical lens switching (`0.5x`, `1.0x`, `3.0x`, `Front`) and stream reconnects.
12. **Dynamic Capabilities Negotiation**:
    - Sensor capabilities probed dynamically via Android `SCALER_STREAM_CONFIGURATION_MAP` and hardware `CodecCapabilities` supporting up to 4K@30, 1080p@60, 720p@60 without hardcoded tables.
    - Dynamically updates desktop UI preset cards and resolution detail chips (`res_detail`).
13. **Double-Buffered Frame Ring & Bounded Memory Management**:
    - Runtime Double-Buffering: Video decode loop in `phonecam-app` runtime utilizes a ping-pong buffer pair (`scratch.bgra` and `scratch.spare`) ensuring the rendering mailbox never reads a buffer actively being written.
    - GPU Texture Atlas Eviction Safety: In `phonecam-ui` (`AppView`), the preview image handle holds `preview_image` and `previous_preview_image` across consecutive frames, preventing flickering and race conditions during GPU texture atlas eviction.
    - Explicit Buffer Reclamation: Backing allocations are reclaimed with original capacity via `reclaim_render_image_buffer` (`std::mem::forget(render_img)` followed by `Vec::from_raw_parts`) when exclusive Arc ownership permits, preventing heap churn and VRAM leaks.
    - Memory consumption remains strictly bounded and flat during prolonged 60 FPS streaming sessions.
14. **Privacy, Anonymity & Path Generality Contract (AGENTS.md Rule 1.11)**:
    - Strictly zero hardcoded personal paths (`C:\Users\...`, `/home/...`), personal usernames, developer machine drive letters (`D:\...`), or specific hardware device serials.
    - Standard dynamic resolution contract:
      - Device Serial: `ANDROID_SERIAL` env var $\to$ `adb get-serialno` $\to$ parse `adb devices`. Tests skip gracefully if no device is detected.
      - ADB Path: `PHONECAM_ADB_PATH` env var $\to$ `dist/unpacked/adb.exe` $\to$ system `PATH` $\to$ standard SDK locations.
      - Artifact Directory: `PHONECAM_ARTIFACT_DIR` env var $\to$ `%TEMP%\phonecam-artifacts`.
      - Service Ports: `PHONECAM_PORT` (default: `48721` TCP / `48720` UDP).
    - Permissible fixed paths are standard OS contracts only: `C:\ProgramData\PhoneCam\framebuffer-v2.bin` and `C:\Program Files\PhoneCam`.

---

## 3. Code Style & Engineering Guidelines

### 3.1 Rust Standards
- **Edition**: Rust 2021 (1.85+).
- **Zero-Allocation Hot Path**: Zero heap allocations (`Vec::new()`, `String::from()`, `format!()`) in per-frame media loops. Pre-allocate buffers at initialization and resolution changes. Applies strictly to video decoders, color converters, `phonecam-media::face_detect` static grid processing (`MAX_GRID_CELLS = 19200`), and `phonecam-media::gesture` projection histograms.
- **`unsafe` Blocks**: Must have an explicit `// SAFETY:` comment documenting caller invariants and data-race prevention. Restricted to `phonecam-framebus` (memory mapping), `phonecam-app` (Win32 API), `phonecam-ui` (buffer reclamation via `reclaim_render_image_buffer`), and `phonecam-android-core` (JNI).
- **Concurrency**: Use `parking_lot` primitives (`Mutex`, `RwLock`) and `crossbeam_channel`. Never hold a mutex lock across an `.await`, channel send, or socket I/O.
- **Double-Buffering & Bounded Memory**: Use ping-pong double-buffering (`scratch.bgra` / `scratch.spare`) and retain previous preview frame (`previous_preview_image`) for GPU texture atlas eviction. Reclaim image allocations with capacity intact to eliminate heap churn.

### 3.2 Error Handling
- **No Unchecked Panics**: Never use `.unwrap()` or `.expect()` inside socket loops, video decoders, or framebus update paths.
- **Libraries**: Use `thiserror` for typed, exhaustive domain errors (`ProtocolError`, `FramebusError`, `MediaError`).
- **Application Binary**: Use `anyhow::Result` with contextual messages (`.context("...")`) in `phonecam-app`.

### 3.3 UI & GPUI Guidelines
- **Color Format**: GPUI `RenderImage` requires **premultiplied BGRA**. Emitting standard RGBA results in inverted red/blue channels.
- **Tabular Figures**: Dynamic numerical readouts (FPS, latency, bitrate, nonces) must use `numeric_font()` with OpenType `tnum` to prevent text jitter.
- **Single Status Headline**: Status bar shows exactly one headline based on strict priority (`service_unreachable` > `error` > `no_phone` > `connecting` > `reconnecting` > `streaming` > `ready`).
- **Live Stage HUD Overlays**: Gesture confirmations and status notifications render non-blocking translucent HUD chips (`StageContent`) over the live stage viewport with automatic timeout fades.
- **16-Segment LED VU Meter**: Render dynamic 16-segment audio level indicator with color-coded thresholds (10 Green nominal, 4 Amber headroom, 2 Red clipping) and tabular dBFS readouts.
- **Texture Double-Buffering & Safe Dropping**: Retain `previous_preview_image` for safe GPU texture atlas replacement; reclaim buffers via `reclaim_render_image_buffer` to eliminate memory leaks and allocation pauses.
- **Window Lifecycle**: Window close (`WM_CLOSE`) minimizes to the system tray (`ShowWindow(SW_HIDE)`), ensuring the virtual camera driver remains active.

### 3.4 Android & JNI
- **Camera2 Lifecycle**: Ensure camera devices and sessions are cleanly released on stop or camera switch.
- **MediaCodec**: Use hardware encoder input surface or reuse direct byte buffers to eliminate garbage collection pauses.
- **JNI Hygiene**: Release all local references (`DeleteLocalRef`) in JNI loops and clear pending Java exceptions before returning.

### 3.5 Privacy, Paths & Logging Rules
- **Rule 1.11 Privacy & Path Generality**: Strictly zero hardcoded personal paths (`C:\Users\...`, `/home/...`), personal usernames, developer machine drive letters (`D:\...`), or device serials.
- **Dynamic Resolution Contract**: Always resolve device serials dynamically via `ANDROID_SERIAL` env var (or `adb get-serialno` / `adb devices`), ADB path via `PHONECAM_ADB_PATH`, artifact directory via `PHONECAM_ARTIFACT_DIR` (fallback to `%TEMP%\phonecam-artifacts`), and service port via `PHONECAM_PORT`. Tests must skip gracefully when no physical device is detected.
- **Mandatory Redaction**: All log sinks apply `redact()`, stripping any continuous hex sequence of 32 or more characters.
- **Zero Cleartext Credentials**: Never log pairing tokens, QR strings, device secrets, or HMAC digests. Use `sha256(secret)[0:8]` for log correlation.
- Never dump raw video payload byte buffers.

---

## 4. Git & Semantic Commit Conventions

PhoneCam strictly enforces Conventional Commits via commitlint:

```
<type>(<scope>): <imperative summary>

[optional detailed body]
```

### 4.1 Allowed Types
- `feat`: User-facing features or capability additions
- `fix`: Defect resolution or bug fixes
- `perf`: Performance optimizations and allocation reductions
- `refactor`: Structural changes without feature or bug changes
- `test`: Adding or correcting tests
- `docs`: Documentation updates
- `style`: Formatting, whitespace, semicolon changes
- `build`: Cargo dependencies, Gradle configuration, build scripts
- `ci`: CI workflows and GitHub Actions configurations
- `chore`: Repository maintenance or tooling adjustments

### 4.2 Allowed Scopes
- `(protocol)`: Wire format, framing, HMAC-SHA256, packet models
- `(framebus)`: Shared memory, seqlock synchronization, NV12 conversion
- `(media)`: openh264 in-process decoder, SPS parsing, pacer, color matrix
- `(core)`: Transport listeners, handshake, adaptive bitrate ladder
- `(ui)`: GPUI views, design tokens, preview stage, status bar
- `(app)`: Windows binary, Win32 tray integration, ADB supervisor, runtime
- `(android)`: Android Kotlin application, Camera2 pipeline, MediaCodec
- `(vcam)`: Virtual camera driver, registration tool, installer integration
- `(deps)`: Dependency updates

### 4.3 Examples
```bash
feat(media): implement zero-copy planar YUV420P to NV12 stride conversion
fix(core): preserve pairing token on busy response to allow retry
perf(framebus): eliminate atomic overhead in seqlock read validation
docs(protocol): document monotonic nonce replay protection rules
```

---

## 5. Semantic Versioning 2.0.0 & Automated Semantic Release

PhoneCam strictly enforces the **[Semantic Versioning 2.0.0](https://semver.org/)** specification and automated **[Semantic Release](https://semantic-release.org/)** lifecycle.

### 5.1 SemVer 2.0.0 Principles
A version number follows `MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]`:
- **MAJOR (`X.0.0`)**: Incompatible API changes, breaking wire protocol alterations (`phonecam-protocol`), changing the 68-byte packed seqlock header (`phonecam-framebus`), or breaking Android JNI signatures. Resets MINOR and PATCH to 0.
- **MINOR (`1.X.0`)**: Backward-compatible new functionality (e.g. adding Anti-Flicker mode, unlocking 60 FPS, adding latency telemetry) or deprecating existing features. Resets PATCH to 0.
- **PATCH (`1.1.X`)**: Backward-compatible defect resolutions and bug fixes (e.g. resolving UI text clipping, fixing zoom stepper button truncation, Android Camera2 teardown order).
- **Pre-Release (`-alpha.1`, `-beta.2`, `-rc.1`)**: Must use hyphen and dot-separated identifiers (`1.2.0-alpha.1`). Glued letters (`1.0.0a`) are strictly invalid. Pre-releases rank lower than normal releases.

### 5.2 Conventional Commits to SemVer Mapping
Semantic release analyzes commits between `HEAD` and the previous release tag:
- `fix(...)`, `perf(...)` -> **PATCH** (`1.1.0` -> `1.1.1`)
- `feat(...)` -> **MINOR** (`1.1.0` -> `1.2.0`)
- `feat(...)!`, `fix(...)!`, `BREAKING CHANGE:` -> **MAJOR** (`1.1.0` -> `2.0.0`)
- `docs(...)`, `refactor(...)`, `test(...)`, `chore(...)` -> No release triggered unless paired with feat/fix.

### 5.3 Cross-Platform Version Synchronization Matrix
When bumping version to `X.Y.Z`, all 4 project targets must stay in 100% lockstep:
1. **Rust Workspace**: `Cargo.toml` (`[workspace.package] version = "X.Y.Z"`) -> `cargo check --workspace` to update `Cargo.lock`.
2. **Android Application**: `apps/android/app/build.gradle.kts`:
   - `versionName = "X.Y.Z"`
   - `versionCode = MAJOR * 10000 + MINOR * 100 + PATCH` (strictly monotonic integer for Google Play & APK package manager).
3. **Windows NSIS Installer**: `scripts/installer.nsi`:
   - `OutFile "..\dist\installer\PhoneCam-Setup-X.Y.Z.exe"`
   - `DisplayVersion "X.Y.Z"`
4. **Release Gatekeeper**: `release-gate.json`:
   - `"version": "X.Y.Z"`
   - `"release_enabled": true` (when authorizing release).

---

## 6. Release Gatekeeper & Contributor Avatars

- **Release Gate (`release-gate.json`)**: Commits to `master` do **not** trigger public releases automatically. Automated GitHub release publishing runs only when `"release_enabled": true` in `release-gate.json`.
- **Release Notes Generator (`scripts/generate-release-notes.py`)**: Automatically categorizes commits and builds an author table complete with GitHub avatar images (`https://github.com/<user>.png`) and links.
- **Enabling a Release**: Set `"release_enabled": true` in `release-gate.json`, commit, and push. GitHub Actions checks the gate, executes the multi-platform matrix (Windows, Linux, Android multi-ABI), and publishes the tagged release.
- **Closing the Gate**: Immediately set `"release_enabled": false` after authorizing to prevent accidental re-releases on maintenance commits.

