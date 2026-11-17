# Architecture Invariants & Rules

This document specifies the non-negotiable architectural invariants and system boundaries of PhoneCam. Any modification or addition to this codebase must adhere strictly to these principles.

---

## 1. Single Native Process Architecture

- **Invariant**: The PhoneCam desktop application runs as a **single, unified native process**.
- **Rule**: Do not introduce child processes, background daemon subprocesses, or loopback HTTP/IPC servers (such as the legacy port 48723).
- **Structure**:
  - `phonecam-app` owns the application lifetime, Win32 event loop, and GPUI runtime.
  - `phonecam-core` manages network listeners, peer authentication, and the session state machine.
  - `phonecam-media` decodes H.264 bitstreams in-process.
  - `phonecam-framebus` writes decoded frames directly into Windows Shared Memory.
  - `phonecam-ui` renders the interface via GPUI on DirectX 11 / Direct3D 11.

---

## 2. Single Decode Invariant

- **Invariant**: The incoming H.264 bitstream is decoded **exactly once**.
- **Rule**: Never spawn multiple decoders or re-decode video streams for separate display and broadcast targets.
- **Data Flow**:
  - The single in-process `openh264` decoder emits planar YUV420P frames.
  - Planar frames are transformed into:
    1. **NV12** (interleaved UV): written directly into the shared memory framebus for the Windows Virtual Camera.
    2. **BGRA** (premultiplied): copied directly to the GPUI `RenderImage` texture handle.
- **Preview Independence**: The UI preview surface displays the exact frame rendered by the camera driver; it cannot drift or disagree with the virtual camera feed.

---

## 3. Shared Memory Framebus Contract

- **Header Specification**:
  - The shared memory buffer header is pinned at exactly **68 bytes** packed (`#[repr(C, packed)]`).
  - Required fields:
    - `magic`: `0x5043414D` (`PCAM`)
    - `layout_version`: `2`
    - `header_size`: `68`
    - `width`, `height`, `stride_y`, `stride_uv`
    - `frame_index`: monotonic `u64`
    - `timestamp_us`: monotonic timestamp
    - `state`: enum (0 = Unitialized, 1 = Live, 2 = Disconnected)
    - `seq`: 32-bit seqlock sequence counter
    - `readers`: 32-bit active reader counter
- **Seqlock Rules**:
  - The writer increments `seq` to an **odd** integer before beginning memory copy.
  - The writer copies the NV12 frame into the payload area (offset 68).
  - The writer increments `seq` to an **even** integer upon completion.
  - **Critical Rule**: The `readers` field is written **exclusively** by external camera reader processes. The writer must NEVER reset or zero `readers`.
- **Memory File Invariant**:
  - Backing file path: `C:\ProgramData\PhoneCam\framebuffer-v2.bin`.
  - The file is **grow-only**. Never truncate the backing file during a resolution downgrade, as Windows will reject truncation while external processes have mapped the section.

---

## 4. Virtual Camera Driver Separation

- **Hosting Environment**: `PhoneCamVirtualCamera.dll` is instantiated by Windows inside `svchost.exe -k Camera` (running under `NT AUTHORITY\LocalService`).
- **Isolation**: The driver cannot access the desktop process's private memory heap or thread state.
- **Communication Guarantee**: All communication between the desktop application and the virtual camera occurs strictly through the memory-mapped framebus file.
- **Fail-Safe Slate**: If the phone drops connection, the framebus must continue publishing the last valid frame for 3.0 seconds, followed by a black NV12 frame with `state = Disconnected`. Never allow the camera device to vanish from the operating system.

---

## 5. Transport & Unified USB Invariant

- **Rule**: USB is **not** a secondary code path.
- **Implementation**:
  - On Wi-Fi LAN: The phone connects directly to the desktop's LAN IP address on port `48721`.
  - On USB: PhoneCam invokes `adb reverse tcp:48721 tcp:48721`. The phone dials `127.0.0.1:48721`, which is forwarded over the USB cable to the desktop socket.
  - The entire handshake, authentication, control messaging, and video packet pipeline is 100% identical across Wi-Fi and USB.
- **Status Separation**:
  - `usb.adbAvailable`: boolean reflecting whether `adb.exe` exists and is executable.
  - `usb.available`: boolean reflecting whether a physical Android device is connected and authorized.
  - These two states must never be collapsed or conflated.

---

## 6. Authentication & Replay Protection

- **First-Time Pairing**:
  - Initiated via a 128-bit CSPRNG token rendered in a QR code on the desktop screen.
  - Tokens expire after 5 minutes and are single-use.
  - Handshake must **peek** at the token before consuming it when returning `busy` to prevent permanent pairing failure on transient congestion.
  - A human must explicitly approve pairing on the desktop UI.
- **Session Authentication**:
  - Established via a persistent 256-bit secret stored in `%APPDATA%\PhoneCam\devices.json`.
  - Authenticated via `HMAC-SHA256(secret, serverNonce || clientNonce)`.
  - The phone persists a strictly monotonic `clientNonce` counter. Connections with replayed or out-of-order nonces are rejected immediately.

---

## 7. Adaptive Bitrate & Quality Ladder

- **Ladder Hierarchy**: `1080p60 -> 1080p30 -> 720p30 -> 480p30`.
- **Hysteresis Timing**:
  - Downward steps require 10 seconds of persistent latency/packet drop, or an **instant** drop on critical thermal notifications from the phone.
  - Upward steps require 30 continuous seconds of clean, zero-drop statistics.
- **Pending Guard**:
  - When a quality downgrade is dispatched, `AdaptiveQuality::pending` locks the ladder until the phone explicitly reports operating at the new rung. Multiple rungs must never be skipped in a single transition cycle.
- **Ceiling Invariant**:
  - The ladder cannot step above the user's manually configured capture resolution.

---

## 8. Session Phase State Machine

```
      ┌──────────┐
      │   Idle   │
      └────┬─────┘
           │ Phone TCP Dial
           ▼
     ┌───────────┐
     │Connecting │
     └─────┬─────┘
           │ Handshake & Auth OK
           ▼
     ┌───────────┐
     │ Connected │
     └─────┬─────┘
           │ First Decoded Frame Published
           ▼
     ┌───────────┐        Link Loss        ┌──────────────┐
     │ Streaming │────────────────────────►│ Reconnecting │
     └─────┬─────┘                         └──────┬───────┘
           │ Explicit Stop                        │ Phone Returns
           ▼                                      ▼
     ┌───────────┐                         ┌──────────────┐
     │ Connected │                         │  Streaming   │
     └───────────┘                         └──────────────┘
```

- **Invariant**: The `streaming` phase is entered **only** when at least one decoded frame has been published to the framebus. A valid socket connection or phone-side `streaming: true` flag is necessary but not sufficient.

---

## 9. Pure-Rust Computer Vision & Zero-Allocation PTZ Face Tracking

- **Pure-Rust In-Process CV**:
  - Face detection and hand gesture recognition execute 100% locally in Rust in `phonecam-media` without external OpenCV, Python, or C++ dependencies.
  - Operates directly on premultiplied BGRA buffers with **zero dynamic heap allocations** on the 60 FPS hot path.
  - Scratch buffers (`skin_mask`, `filtered_mask`, `luma`, `visited`, `queue`, `candidates`) are pre-allocated at detector creation (`MAX_GRID_CELLS = 160 x 120`).
- **Detection Algorithm**:
  1. Downscaled grid analysis preserving aspect ratio.
  2. Skin-chrominance segmentation using fixed-point BT.601 integer math (`is_skin_rgb`).
  3. Morphological 3x3 noise filter.
  4. Breadth-First Search (BFS) connected-component blob extraction.
  5. Facial geometry verification with Haar-like vertical luma contrast (darker eye sockets vs forehead and cheeks) and bilateral symmetry scoring.
- **PTZ Auto-Framing Tracking Contract**:
  - `AutoFramingTracker` calculates aspect-ratio preserving digital crops centered on the detected subject.
  - **Deadband Hysteresis**: Applies a **1.5% deadband** to eliminate micro-jitter and small head movements.
  - **Temporal Exponential Smoothing**: Applies smoothing factor $\alpha = 0.12$ ($x_t = x_{t-1} \cdot (1 - \alpha) + x_{\text{target}} \cdot \alpha$) clamped to sensor boundaries.
  - **Loss Recovery**: Holds previous face position with exponential confidence decay for up to 6 frames (~100-200ms) to bridge transient occlusions.

---

## 10. 6 OBSBOT Hand Gestures & Interactive Stage HUD

- **Gesture Vocabulary**:
  - `OpenPalm` (✋): Toggles target tracking lock / pause.
  - `PeaceV` (✌): Toggles zoom level (1.0x <-> 2.0x).
  - `IndexPoint` (☝): Sets zoom to 1.5x medium zoom.
  - `ShakaCall` (🤙): Resets framing to center (0.50, 0.50) and zoom to 1.0x.
  - `PinchOk` (🤏): Sets zoom to 2.2x close-up macro focal target.
  - `ThumbsUp` (👍): Quick confirm / toggles zoom.
- **Classification Rules**:
  - **Valley Depth Check**: Distinct finger peaks in vertical projection require a valley dip of at least 25% relative to neighboring peak values (`min_valley <= min_peak * 0.75`).
  - **O(1) Enclosed Loop Scanner**: Pinch/OK gesture detects an internal circular void between thumb and index finger using row-wise (`first_skin_x`, `last_skin_x`) and column-wise (`first_skin_y`, `last_skin_y`) bounding projections.
  - **Palm Base Solidity Ratio**: OpenPalm and fist gestures verify `lower_fill >= 0.35` across the lower hand half.
  - **Face Masking**: Face bounding box (expanded by 4% margin) is masked out from skin detection to eliminate false-positive hand detections on cheeks.
- **Debounce & Refractory Cooldown**:
  - **Debounce**: 3 consecutive identical candidate frames required before triggering.
  - **Refractory Cooldown**: 45 frames (~0.75-1.5s at 30-60 FPS) lockout following a trigger to prevent action chatter.
- **Stage HUD Feedback**:
  - Triggered gestures display transient HUD chips overlaid on the video stage preview with gesture iconography and animated confirmation badges.

---

## 11. Multiplexed Audio Streaming (Channel Role 3)

- **Channel Role 3**:
  - PhoneCam multiplexes audio over the unified TCP transport alongside Video (`ChannelRole::Video = 2`) and Control (`ChannelRole::Control = 1`).
  - Wire identifier: `ChannelRole::Audio = 3`.
  - Toggled via `DesktopToPhone::SetAudioStream { enabled: bool }` (JSON: `{"type": "setAudioStream", "enabled": true}`).
- **Audio Payload Format**:
  - Uncompressed 16-bit linear PCM at 48,000 Hz or 44,100 Hz mono/stereo.
  - Desktop calculates live audio volume levels (RMS / peak metering) for real-time display in the status bar and stage meters.
- **Generation Counter Invariant**:
  - Desktop runtime tracks monotonic `audio_generation: Arc<AtomicU64>`.
  - Stale audio channels from previous connection attempts gracefully terminate upon detecting an incremented generation counter, preventing race conditions and thread leaks.

---

## 12. Dynamic Capabilities Negotiation

- **Android Probe via `StreamConfigurationMap`**:
  - Phone client inspects Camera2 `CameraCharacteristics` and `StreamConfigurationMap` upon startup.
  - Gathers available sensor IDs, facing (`front`, `back`, `external`), lens types (ultra-wide, wide, telephoto), supported output resolutions, and available auto-exposure target FPS ranges (`CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES`).
- **Capabilities Exchange**:
  - Phone dispatches `PhoneToDesktop::Capabilities` with `CapabilitiesPayload` containing `DeviceCapabilities` and `CameraCapability` lists over the control channel.
  - Desktop UI dynamically populates resolution selectors, framerate toggles (unlocking 60 FPS when supported by the hardware sensor), and multi-lens switchers (`0.5x`, `1.0x`, `3.0x`, `Front`) without static hardcoding.

---

## 13. Double-Buffered Texture Recycling & GPU Leak Prevention

- **GPUI Texture Atlas Invariant**:
  - Recreating GPU textures every frame at 60 FPS without eviction leaks texture atlas VRAM in Direct3D 11.
- **Double-Buffered Management**:
  - `AppView` maintains `preview_image` (active frame) and `previous_preview_image` (frame from previous tick).
  - When a new frame arrives, the frame from 2 frames ago is reclaimed:
    1. Explicitly evicted from the GPUI window texture cache via `window.drop_image(frame_2_ago)`.
    2. Buffer payload is recycled back into the application's memory pool via `reclaim_render_image_buffer`.
  - Prevents flickering by ensuring the currently rendered frame is never unmapped while the GPU draws, while strictly eliminating VRAM leaks over long streaming sessions.

---

## 14. Privacy, Anonymity & Path Generality Invariant

- **Invariant**: Zero hardcoded personal paths, user home directories, developer drive letters, or single-device hardware serial numbers.
- **Rule**:
  - Source code, tests, build scripts, markdown files, and CI workflows must **never** hardcode user directories (`C:\Users\...`, `/home/...`), personal usernames, developer machine drive letters (`D:\...`), or specific hardware device serials (e.g. `79134849`).
  - Standard dynamic resolution patterns must be used:
    - **Device Serials**: Dynamically resolved via `ANDROID_SERIAL` env var, falling back to `adb get-serialno` / `adb devices`, or graceful test skipping (`println!("SKIPPED: No Android device detected")`).
    - **ADB Executable**: Dynamically resolved via `PHONECAM_ADB_PATH`, workspace relative staging (`dist/unpacked/adb.exe`), or system `PATH`.
    - **Test Artifacts**: Output to `PHONECAM_ARTIFACT_DIR`, falling back to `std::env::temp_dir().join("phonecam-artifacts")`.
    - **Android NDK**: Dynamically discovered via `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT`, or standard SDK search paths.
- **System Exceptions**: The only permissible hardcoded absolute system paths are standardized OS-level contracts: the Windows shared memory backing file (`C:\ProgramData\PhoneCam\framebuffer-v2.bin`) and default Windows installation paths (`C:\Program Files\PhoneCam`).

