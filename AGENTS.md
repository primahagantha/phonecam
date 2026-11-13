# AGENTS.md — PhoneCam System Architecture & Agent Standards

> **System Identity**: PhoneCam is an ultra-low latency, high-performance open-source webcam system that transforms modern Android devices into professional PC webcams. It replaces legacy Electron/Node.js stacks with a **single, unified native Rust process** leveraging GPUI for hardware-accelerated rendering, an in-process `openh264` decoder, and a direct Windows Shared Memory seqlock interface for Windows Media Foundation virtual camera streaming with zero cloud dependencies.

---

## 1. Non-Negotiable Architectural Invariants

Every agent or contributor modifying or extending PhoneCam must adhere strictly to these architectural invariants:

### 1.1 Single Native Process Architecture
- The desktop application operates as **one single native binary** (`phonecam.exe`).
- Do **not** spawn background daemon processes, helper subprocesses, or loopback HTTP/IPC servers (the legacy port `48723` is completely retired).
- `phonecam-app` coordinates application lifetime, native Win32 window message loops, system tray integration, and the GPUI application runtime.

### 1.2 Single Decode Invariant
- Video bitstreams from the phone are decoded **exactly once** in-process via `openh264` in `phonecam-media`.
- Never decode the bitstream twice.
- Output planar YUV420P frames are transformed simultaneously:
  1. **NV12** (interleaved UV): written directly to the Windows Shared Memory framebus for virtual camera consumption.
  2. **BGRA** (premultiplied): copied directly to the GPUI `RenderImage` texture buffer for the UI preview.
- The UI preview surface and the virtual camera feed reflect the exact same decoded frame; they cannot drift or display divergent states.

### 1.3 Shared Memory Framebus Contract
- Windows Shared Memory backing file: `C:\ProgramData\PhoneCam\framebuffer-v2.bin`.
- Header specification: pinned at exactly **68 bytes** packed (`#[repr(C, packed)]`).
  - `magic`: `0x5043414D` (`PCAM`)
  - `layout_version`: `2`
  - `header_size`: `68`
  - `width`, `height`, `stride_y`, `stride_uv`
  - `frame_index`: monotonic `u64`
  - `timestamp_us`: monotonic timestamp
  - `state`: enum (0 = Uninitialized, 1 = Live, 2 = Disconnected)
  - `seq`: 32-bit seqlock sequence counter (odd = write in progress, even = read consistent)
  - `readers`: 32-bit active reader counter
- **Critical Seqlock Rules**:
  - The writer increments `seq` to an odd number before memory copy, copies NV12 payload into offset 68, and increments `seq` to an even number upon completion.
  - The `readers` field is updated **exclusively** by consumer processes (virtual camera DLL). The writer must **never** zero or reset `readers`.
  - The backing file is **grow-only**. Never truncate the file on resolution downgrade, as Windows rejects truncation while mapped by external processes.

### 1.4 Virtual Camera Driver Isolation
- `PhoneCamVirtualCamera.dll` is instantiated by Windows inside `svchost.exe -k Camera` (running under `NT AUTHORITY\LocalService`).
- Communication occurs exclusively through the memory-mapped framebus file.
- **Fail-Safe Slate**: When link loss occurs, the framebus keeps publishing the last valid frame for 3.0 seconds, followed by a solid black NV12 frame with `state = Disconnected`. Never allow the camera device to unregister or vanish from the host OS.

### 1.5 Unified Transport Invariant (USB & Wi-Fi)
- USB is **not** a secondary code path.
- In both Wi-Fi and USB modes, the phone connects via TCP to port `48721`.
- Over USB, PhoneCam automates `adb reverse tcp:48721 tcp:48721`, allowing the phone to dial `127.0.0.1:48721`.
- Transport, handshake, authentication, control messaging, and video packet parsing are 100% identical.
- USB availability (`usb.available`) and ADB executable presence (`usb.adbAvailable`) are distinct states and must never be conflated.

### 1.6 Authentication & Monotonic Replay Protection
- **Pairing (First Contact)**: 128-bit CSPRNG token displayed in QR code on desktop UI, 5-minute TTL, single-use, requires explicit user approval on desktop.
- Handshake must **peek** at pairing tokens without consuming them when returning `busy` to prevent permanent refusal on transient network congestion.
- **Session Auth**: 256-bit persistent secret stored in `%APPDATA%\PhoneCam\devices.json`. Authenticated via `HMAC-SHA256(secret, serverNonce || clientNonce)`.
- Replay guard: `client_nonce` is a strictly monotonic `u64` persisted across phone reboots; stale or decremented nonces are rejected.

### 1.7 Adaptive Bitrate & Quality Ladder
- Hierarchy: `1080p60 -> 1080p30 -> 720p30 -> 480p30`.
- Hysteresis: Downward step requires 10 seconds of persistent latency/packet drops; upward step requires 30 continuous seconds of clean statistics.
- Instant down-step triggered upon critical thermal alerts from the phone.
- `AdaptiveQuality::pending` locks ladder transitions until the phone confirms operating at the requested rung.
- The ladder can never step above the user's configured ceiling.

### 1.8 Session State Machine
- Phases: `Idle -> Connecting -> Connected -> Streaming -> Reconnecting`.
- **Invariant**: The `Streaming` phase is entered **only** when at least one decoded frame has been written to the framebus. A live socket or phone-side `streaming: true` flag is necessary but not sufficient.

---

## 2. Workspace & Crate Organization

The PhoneCam workspace contains 7 crates and 1 native Android project:

```
phonecam-rust/
├── Cargo.toml                       # Workspace root definition
├── Cargo.lock
├── crates/
│   ├── phonecam-protocol/           # Frozen wire contract, framing, HMAC-SHA256, packet types
│   ├── phonecam-framebus/           # Windows shared memory, 68B seqlock header, NV12 buffer
│   ├── phonecam-media/              # openh264 in-process decoder, SPS parsing, BGRA/NV12 converters
│   ├── phonecam-core/               # TCP socket listeners, UDP discovery, auth, adaptive ladder
│   ├── phonecam-ui/                 # GPUI application views, design tokens, preview viewport
│   ├── phonecam-app/                # Windows native binary, Win32 tray, ADB supervisor, runtime
│   └── phonecam-android-core/       # Rust JNI cdylib compiled for Android ABIs
├── apps/
│   └── android/                     # Kotlin Android application (Camera2 + MediaCodec H.264)
├── dist/                            # Unpacked distribution staging and NSIS installer target
├── scripts/                         # Build and packaging automation scripts
└── .agent/
    ├── rules/                       # Core architectural and coding rules
    └── skills/                      # Agent capability skills and domain references
```

### Detailed Crate Responsibilities
| Crate | Responsibility | Key Modules / Boundaries |
| :--- | :--- | :--- |
| `phonecam-protocol` | Wire protocol v1, binary framing, crypto | `framing.rs`, `messages.rs`, `lib.rs` |
| `phonecam-framebus` | Shared memory seqlock buffer writer & reader | `writer.rs`, `reader.rs`, `nv12.rs`, `lib.rs` |
| `phonecam-media` | Video decoding, color conversion, pacer | `decoder.rs`, `h264.rs`, `convert.rs`, `pacer.rs` |
| `phonecam-core` | Connection listener, auth state machine, ladder | `session.rs`, `transport.rs`, `discovery.rs`, `quality.rs` |
| `phonecam-ui` | GPUI renderer, design system, stage & rail | `theme.rs`, `stage.rs`, `rail.rs`, `status_bar.rs` |
| `phonecam-app` | Windows binary entrypoint, tray, ADB | `main.rs`, `runtime.rs`, `tray.rs`, `adb.rs`, `vcam.rs` |
| `phonecam-android-core` | JNI interface for Android streaming engine | `jni.rs`, `lib.rs` |

---

## 3. Build, Lint & Test Commands

### 3.1 Rust Workspace Commands
```powershell
# Check workspace compilation
cargo check --workspace

# Run full automated test suite (230+ tests across 7 crates)
cargo test --workspace

# Run individual crate test suites
cargo test -p phonecam-protocol
cargo test -p phonecam-framebus
cargo test -p phonecam-media
cargo test -p phonecam-core
cargo test -p phonecam-ui
cargo test -p phonecam-app --test end_to_end

# Code formatting check
cargo fmt --all -- --check

# Clippy linter with strict warning rejection
cargo clippy --workspace --all-targets -- -D warnings
```

### 3.2 Running the Desktop Application
```powershell
# Development run
cargo run -p phonecam-app

# Run on alternate port (if port 48721 is occupied)
$env:PHONECAM_PORT = 48791
cargo run -p phonecam-app

# Build optimized release binary
cargo build --release -p phonecam-app
```

### 3.3 Android & JNI Commands
```powershell
# 1. Compile native Rust JNI libraries for Android ABIs (arm64-v8a, armeabi-v7a, x86_64)
.\scripts\build-android-core.ps1

# 2. Run Android JVM unit tests
cd apps\android
.\gradlew.bat test
cd ..\..

# 3. Assemble Android APKs (Universal + Per-ABI splits)
cd apps\android
.\gradlew.bat assembleRelease
cd ..\..
```

### 3.4 Packaging the Windows Installer
```powershell
# 1. Copy release binary to unpacked staging
Copy-Item target\release\phonecam.exe dist\unpacked\phonecam.exe -Force

# 2. Compile NSIS installer
makensis scripts\installer.nsi
# Output: dist\installer\PhoneCam-Setup-1.0.0.exe
```

---

## 4. Coding Conventions & Quality Guidelines

### 4.1 Zero-Allocation Media Hot Path
- The video processing loop operates at up to 60 FPS (>124 MB/s raw pixel throughput).
- **Zero dynamic allocations** (`Vec::new()`, `String::from()`, `format!()`) inside per-frame processing loops.
- Pre-allocate frame buffers during stream configuration or resolution changes.
- Perform color format transformations (YUV420P -> NV12 / BGRA) using in-place strided copies.

### 4.2 Safe & Documented `unsafe` Code
- Every `unsafe` block must be accompanied by an explicit `// SAFETY:` comment explaining:
  1. The specific invariants guaranteed by the caller.
  2. Why memory safety, pointer alignment, and data-race prevention are ensured.
- Permitted exclusively in:
  - `phonecam-framebus`: Windows shared memory pointer arithmetic.
  - `phonecam-app`: Win32 native APIs (`Shell_NotifyIcon`, `ShowWindow`).
  - `phonecam-android-core`: JNI boundaries with the JVM.

### 4.3 Error Handling Discipline
- **Never panic** (`unwrap()`, `expect()`) inside network event loops or frame rendering pipelines.
- Library crates must use `thiserror` for typed, exhaustive domain errors (`ProtocolError`, `FramebusError`, `MediaError`).
- Binary application crate (`phonecam-app`) uses `anyhow::Result` with `.context("...")` to provide clear debugging context.

### 4.4 GPUI & UI Ergonomics
- GPUI `RenderImage` requires **premultiplied BGRA** color data. Outputting standard RGBA results in red/blue color channel swaps.
- Dynamic numerical readouts (FPS, Bitrate, Latency RTT, Nonce) must utilize tabular figures (`tnum`) via `numeric_font()` to eliminate text jitter.
- The status bar must adhere to the **Single Status Headline Rule**, displaying exactly one status headline following priority ordering (`service_unreachable` > `error` > `no_phone` > `connecting` > `reconnecting` > `streaming` > `ready`).
- Window close minimizes to the system tray (`ShowWindow(SW_HIDE)`) to ensure the virtual camera driver remains active for consuming applications.

### 4.5 Logging & Privacy Discipline
- **Mandatory Redaction**: All log sinks invoke `redact()` to sanitize any sequence of 32 or more consecutive hex characters.
- Never log pairing tokens, QR strings, device secrets, or HMAC digests in plaintext.
- If referencing credentials in debug logs, log only `sha256(secret)[0:8]`.
- Log packet counts, timestamps, and byte lengths; never dump raw payload buffers.

---

## 5. Semantic Commits & Pull Request Standards

All commits must follow the [Conventional Commits](https://www.conventionalcommits.org/) specification enforced by CI commitlint:

```
<type>(<scope>): <short imperative description>

[optional body explaining rationale]
```

- **Types**: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `style`, `build`, `ci`, `chore`.
- **Scopes**: `(protocol)`, `(framebus)`, `(media)`, `(core)`, `(ui)`, `(app)`, `(android)`, `(vcam)`, `(deps)`.
- **Validation**: Ensure `cargo test --workspace` passes and `cargo clippy --workspace --all-targets -- -D warnings` reports zero warnings prior to submitting changes.

---

## 6. Release Gatekeeper & Contributor Transparency

### 6.1 Gated Release Publishing (`release-gate.json`)
- Routine commits and feature pushes to `master` do **not** publish releases automatically.
- Automated release builds and GitHub Releases are gated by `release-gate.json`:
  ```json
  {
    "release_enabled": false,
    "version": "1.0.0",
    "approved_by": "RegenadeJester"
  }
  ```
- To authorize a public multi-platform release:
  1. Change `"release_enabled"` to `true`.
  2. Verify all tests pass (`cargo test --workspace` & `./gradlew test`).
  3. Commit and push. GitHub Actions validates the gate and compiles the full multi-platform matrix (Windows NSIS + portable ZIP, Linux tarball, Android universal & multi-ABI APKs).

### 6.2 Contributor Avatars & Release Notes
- GitHub release notes are configured via `.github/release.yml` and synthesized via `scripts/generate-release-notes.py`.
- Every release automatically includes:
  - Categorized changelog entries linked to commits and PRs.
  - Transparent contributor table displaying each author's live GitHub profile avatar (`https://github.com/<username>.png`), username, commit count, and profile link.

