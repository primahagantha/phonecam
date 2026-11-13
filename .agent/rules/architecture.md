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
