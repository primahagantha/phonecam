# CLAUDE.md — PhoneCam Engineering & Style Guide

This file provides essential commands, architectural standards, code style rules, and commit conventions for Claude Code and AI agents working in the PhoneCam repository.

---

## 1. Build, Lint & Test Commands

### 1.1 Quick Reference
```powershell
# Fast compilation check across the entire workspace
cargo check --workspace

# Run complete automated test suite (230+ tests across 7 crates)
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

---

## 3. Code Style & Engineering Guidelines

### 3.1 Rust Standards
- **Edition**: Rust 2021 (1.85+).
- **Zero-Allocation Hot Path**: Zero heap allocations (`Vec::new()`, `String::from()`, `format!()`) in per-frame media loops. Pre-allocate buffers at initialization and resolution changes.
- **`unsafe` Blocks**: Must have an explicit `// SAFETY:` comment documenting caller invariants and data-race prevention. Restricted to `phonecam-framebus` (memory mapping), `phonecam-app` (Win32 API), and `phonecam-android-core` (JNI).
- **Concurrency**: Use `parking_lot` primitives (`Mutex`, `RwLock`) and `crossbeam_channel`. Never hold a mutex lock across an `.await`, channel send, or socket I/O.

### 3.2 Error Handling
- **No Unchecked Panics**: Never use `.unwrap()` or `.expect()` inside socket loops, video decoders, or framebus update paths.
- **Libraries**: Use `thiserror` for typed, exhaustive domain errors (`ProtocolError`, `FramebusError`, `MediaError`).
- **Application Binary**: Use `anyhow::Result` with contextual messages (`.context("...")`) in `phonecam-app`.

### 3.3 UI & GPUI Guidelines
- **Color Format**: GPUI `RenderImage` requires **premultiplied BGRA**. Emitting standard RGBA results in inverted red/blue channels.
- **Tabular Figures**: Dynamic numerical readouts (FPS, latency, bitrate, nonces) must use `numeric_font()` with OpenType `tnum` to prevent text jitter.
- **Single Status Headline**: Status bar shows exactly one headline based on strict priority (`service_unreachable` > `error` > `no_phone` > `connecting` > `reconnecting` > `streaming` > `ready`).
- **Window Lifecycle**: Window close (`WM_CLOSE`) minimizes to the system tray (`ShowWindow(SW_HIDE)`), ensuring the virtual camera driver remains active.

### 3.4 Android & JNI
- **Camera2 Lifecycle**: Ensure camera devices and sessions are cleanly released on stop or camera switch.
- **MediaCodec**: Use hardware encoder input surface or reuse direct byte buffers to eliminate garbage collection pauses.
- **JNI Hygiene**: Release all local references (`DeleteLocalRef`) in JNI loops and clear pending Java exceptions before returning.

### 3.5 Privacy & Logging Rules
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

## 5. Release Gatekeeper & Contributor Avatars

- **Release Gate (`release-gate.json`)**: Commits to `master` do **not** trigger public releases automatically. Automated GitHub release publishing runs only when `"release_enabled": true` in `release-gate.json`.
- **Release Notes Generator (`scripts/generate-release-notes.py`)**: Automatically categorizes commits and builds an author table complete with GitHub avatar images (`https://github.com/<user>.png`) and links.
- **Enabling a Release**: Set `"release_enabled": true` in `release-gate.json`, commit, and push. GitHub Actions checks the gate, executes the multi-platform matrix (Windows, Linux, Android multi-ABI), and publishes the tagged release.

