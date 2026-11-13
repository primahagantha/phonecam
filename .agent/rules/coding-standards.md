# Coding Standards & Guidelines

This document establishes the code quality standards, safety constraints, UI design rules, and development practices for the PhoneCam codebase.

---

## 1. Rust Language Standards

### Toolchain & Quality Checks
- **Rust Edition**: 2021 edition (Rust 1.85+).
- **Formatting**: All code must conform to `rustfmt`:
  ```powershell
  cargo fmt --all -- --check
  ```
- **Clippy**: All code must be free of warnings under pedantic checks:
  ```powershell
  cargo clippy --workspace --all-targets -- -D warnings
  ```

### Memory Safety & `unsafe` Boundaries
- **Minimization**: Avoid `unsafe` whenever possible.
- **Justification**: Every `unsafe` block must be accompanied by an explanatory `// SAFETY:` comment detailing:
  1. The exact invariants that the caller guarantees.
  2. Why memory aliasing, alignment, and data races are impossible.
- **Allowed Areas**:
  - `phonecam-framebus`: Memory-mapping Windows file views and pointer arithmetic for the 68-byte packed header.
  - `phonecam-app`: Win32 FFI (`Shell_NotifyIcon`, `ShowWindow`, message loops).
  - `phonecam-android-core`: JNI boundaries between JVM and Rust.

### Error Handling
- **No Uncontrolled Panics**: Never use `.unwrap()` or `.expect()` inside socket loops, video decoding pipelines, or framebus update paths.
- **Library Crates**: Use `thiserror` to define strongly-typed, exhaustive error enums (`ProtocolError`, `FramebusError`, `MediaError`).
- **Application Binary**: Use `anyhow::Result` with `.context("...")` in `phonecam-app` to provide descriptive error trails.

### Zero-Allocation Media Hot Path
- The frame processing loop handles up to 60 frames per second (over 124 MB/s of raw uncompressed video).
- **Rules for Hot Paths**:
  - Zero heap allocations (`Vec::new()`, `String::from()`, `format!()`) on a per-frame basis.
  - Pre-allocate frame buffers during stream initialization or upon resolution renegotiation.
  - Use in-place conversions for YUV420P to NV12 pitch padding and BGRA color conversions.

### Concurrency & Thread Synchronization
- Prefer `parking_lot::Mutex` and `parking_lot::RwLock` over standard library primitives for reduced overhead.
- **Deadlock Prevention**: Never hold a lock across an asynchronous await, socket read/write, or blocking channel send.
- Use `crossbeam_channel` for low-latency thread communication.

---

## 2. UI & Design System Rules (GPUI)

### The Single Status Headline Rule
- The status bar presents **one single headline** answering whether the video is flowing.
- Priority order enforced by `theme::rules::summarise_status`:
  1. `service_unreachable` (Fatal network/host error)
  2. `error` (Last attempt failed)
  3. `no_phone` (No device connected)
  4. `connecting` (Handshake in progress)
  5. `reconnecting` (Link lost, awaiting recovery)
  6. `streaming` (Video flowing)
  7. `ready` (Connected and idle)

### Color Palette Discipline
- **Action Accent (`#3ba3ff`)**: Reserved strictly for interactive controls, selections, and primary actions. It means *"click here"* or *"active tab"*, NOT *"healthy"*.
- **Live / Healthy (`#10b981`)**: Reserved exclusively for active streaming and verified live frame transmission. A registered but idle virtual camera must never be colored green.
- **Error / Danger (`#ef4444`)**: Reserved for connection failures, port conflicts, or auth rejections.
- **Warning / Degraded (`#f59e0b`)**: Used for thermal clamps, packet drops, or quality step-downs.
- **Contrast Floor**: All text-to-background combinations must meet or exceed WCAG 2.1 AA (4.5:1 contrast ratio).

### Tabular Numbers (`tnum`)
- Every dynamic numerical readout (FPS, bitrate, RTT latency, drop percentage, pairing countdown) must use `numeric_font()` with OpenType tabular figures (`tnum`).
- Proportional fonts cause fluctuating string widths that produce visible text jitter on every second tick.

### Single Preview Surface
- The preview image handle must persist for the window lifetime.
- GPUI pulls the latest texture on animation frames rather than having frames pushed into a recreatable surface.

### Native Tray & Window Lifecycle
- Closing the window (`WM_CLOSE`) must call `ShowWindow(SW_HIDE)` to minimize to the tray, keeping the virtual camera driver active.
- Process termination must only occur when the user explicitly clicks "Quit" in the system tray menu.

---

## 3. Android & JNI Standards

- **Kotlin Conventions**: Adhere to official Kotlin coding standards and Android architecture guidelines.
- **Camera2 Lifecycle**: Always unbind and release camera devices when switching sensors or when the background streaming service stops.
- **MediaCodec Efficiency**: Feed hardware encoder input surfaces directly from Camera2 targets where possible, or cycle direct byte buffers without GC allocations.
- **JNI Hygiene**: Release all local references (`DeleteLocalRef`) in JNI loops and handle Java exceptions promptly before returning to native code.

---

## 4. Logging & Privacy Rules

- **Zero Cleartext Credentials**:
  - Never log pairing tokens, QR strings, device secrets, or HMAC digests.
  - When referencing secrets in debug logs, log `sha256(secret)[0:8]` as a correlator.
  - Never log raw video payload buffers; log only frame indices, timestamps, and byte counts.
- **Mandatory Redaction**:
  - Every log sink must run `redact()`.
  - The redactor automatically strips any contiguous sequence of 32 or more lowercase hexadecimal characters.

---

## 5. Commit Standards

All commits must follow the Conventional Commits specification:
```
<type>(<scope>): <short description in imperative mood>

[optional body explaining rationale]
```
Allowed scopes: `(protocol)`, `(framebus)`, `(media)`, `(core)`, `(ui)`, `(app)`, `(android)`, `(vcam)`, `(deps)`.
