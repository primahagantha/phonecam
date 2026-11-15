# PhoneCam Pro Features & Architecture Manual

> Comprehensive technical reference and developer documentation for the advanced camera, video processing, audio, and power-saving features in PhoneCam Pro.

---

## 1. Executive Summary

PhoneCam Pro elevates the PhoneCam open-source webcam suite into a broadcast-grade streaming instrument. Unlike cloud-dependent solutions, **PhoneCam Pro runs 100% locally on your own hardware** with zero telemetry, zero cloud server dependencies, and zero subscriptions.

Every pro feature is engineered around our **single native process, zero-copy, single-decode** philosophy:
- Video frames decoded via in-process `openh264` are converted directly into premultiplied BGRA for rendering and NV12 for the virtual camera.
- Color LUTs, portrait background blur, and digital auto-framing transformations execute in-place with zero dynamic memory allocations on the active video hot path.
- The companion Android application leverages native Camera2 NDK and hardware `MediaCodec` encoders with direct physical lens switching and sensor-level manual controls.

```mermaid
flowchart TD
    subgraph AndroidDevice["Android Phone (Camera2 + MediaCodec)"]
        Sensors["Physical Multi-Lens (0.5x, 1.0x, 3.0x, Front)"]
        Manuals["Hardware AE / AWB / AF Controls"]
        OLED["Pure-Black OLED Ambient Display"]
        AudioCap["Audio Record (PCM 48kHz / ChannelRole::Audio)"]
        H264Enc["Hardware H.264 Encoder (Annex-B)"]
        
        Sensors --> Manuals --> H264Enc
        OLED -.->|"Power & Thermal Saver"| Sensors
    end

    subgraph Transport["Unified Link (USB ADB / Wi-Fi LAN)"]
        TCPStream["TCP 48721 Length-Prefixed Stream"]
        H264Enc --> TCPStream
        AudioCap --> TCPStream
    end

    subgraph DesktopProcess["PhoneCam Desktop (Single In-Process Engine)"]
        TCPRx["Protocol Receiver & HMAC Replay Guard"]
        OpenH264["Single In-Process openh264 Decoder"]
        
        subgraph ProMediaPipeline["Pro Video Effects Engine (In-Place BGRA)"]
            LUT["5 Studio Color LUT Presets"]
            Blur["Portrait Bokeh Background Blur (SIMD Box + Hermite Feather)"]
            AutoFrame["AI Auto-Framing (Exponential Smoothing Pan/Zoom)"]
            
            OpenH264 --> AutoFrame --> Blur --> LUT
        end
        
        AudioEngine["Low-Latency Audio Engine / Virtual Audio"]
        Framebus["Windows Shared Memory (NV12 68B Seqlock)"]
        GPUIPreview["GPUI DirectX 11 Preview Window"]
        
        TCPStream --> TCPRx
        TCPRx --> OpenH264
        TCPRx --> AudioEngine
        LUT -->|"NV12 Conversion"| Framebus
        LUT -->|"Premultiplied BGRA"| GPUIPreview
    end

    subgraph SystemOutputs["Operating System Endpoints"]
        VCam["Windows Virtual Camera (svchost.exe Camera Service)"]
        Mic["Windows Virtual Audio Device / Playback"]
        Apps["OBS Studio / Zoom / Discord / Google Meet / Teams"]
        
        Framebus --> VCam --> Apps
        AudioEngine --> Mic --> Apps
    end
```

---

## 2. Feature Deep-Dive & Architecture

### 2.1 AI Auto-Framing (Smart Center Stage)

**AI Auto-Framing** continuously keeps the human subject centered within the broadcast frame, providing an automated pan, tilt, and zoom experience without moving the physical camera.

- **Tracking & Geometry**:
  - The tracking engine computes normalized subject center coordinates $(X_c, Y_c) \in [0.15, 0.85]$ and target framing rectangles.
  - An aspect-ratio preserving crop rectangle `CropRect { x, y, width, height }` is computed with even-pixel boundary alignment (`& !1`) to maintain color sub-sampling fidelity.
- **Dual-Axis Exponential Smoothing**:
  - Rather than snapping immediately to new bounding boxes, the camera smoothly pans and zooms using temporal exponential smoothing:
    $$\vec{C}_{t} = \vec{C}_{t-1} + (\vec{T}_{t} - \vec{C}_{t-1}) \cdot \alpha$$
    where default smoothing factor $\alpha = 0.12$ and default zoom factor is $1.20\times$ (user-configurable between $1.0\times$ and $2.5\times$).
  - Clamping prevents the crop window from exceeding frame boundaries under rapid subject movement.
- **Toggle Control**:
  - Controlled via dedicated on/off toggle switch in the desktop UI Capture tab (`SideEffect::SetAutoFraming(bool)`). State is persisted to `desktop-settings.json`.

---

### 2.2 Low-Latency Audio Streaming

PhoneCam Pro streams crystal-clear audio from the Android smartphone's onboard microphone array directly to your PC, eliminating the need for an external USB desk microphone.

- **Protocol Transport**:
  - Transmitted over the unified TCP stream using `ChannelRole::Audio = 3` alongside the video stream (`ChannelRole::Video = 1`) and control messaging (`ChannelRole::Control = 2`).
  - Framing uses standard length-prefixed protocol packets, maintaining strict synchronization with video timestamps (`timestamp_us`).
- **Audio Specifications**:
  - Format: High-fidelity uncompressed 16-bit PCM or Opus, 48 kHz sampling rate, mono or stereo.
  - Jitter buffering maintains end-to-end audio/video lip-sync under varying network conditions.
- **Desktop Routing**:
  - Audio packets are fed into a low-latency ring buffer connected to Windows WASAPI / DirectSound audio rendering or a virtual audio cable for consumption in OBS, Zoom, or Discord.
- **Toggle Control**:
  - Toggled via the "Stream Audio" switch under Capture and Setup tabs (`SideEffect::SetAudioStream(bool)`).

---

### 2.3 Portrait Bokeh Blur (Background Blur)

The **Portrait Bokeh Blur** engine applies an artistic, shallow depth-of-field effect directly to the live webcam feed, softening distracting room backgrounds while keeping the presenter in crisp focus.

- **Separable 2D Box Blur Algorithm**:
  - Video frames are processed in-place using a two-pass horizontal and vertical box blur.
  - Box radius is mapped to preset intensity:
    - **Soft**: Radius 3 px (subtle camera depth).
    - **Studio**: Radius 6 px (professional prime-lens portrait feel).
    - **Dreamy**: Radius 10 px (heavy, cinematic background abstraction).
  - An additional continuous amount slider ($0.0$ to $1.0$) scales the blend weight.
- **Hermite Smoothstep Subject Mask**:
  - A centered elliptical portrait mask preserves the presenter's face and upper torso:
    $$d = \sqrt{\left(\frac{x - x_c}{r_x}\right)^2 + \left(\frac{y - y_c}{r_y}\right)^2}$$
  - Transition feathering is computed with a smooth cubic Hermite polynomial ($3t^2 - 2t^3$) between $d \in [0.85, 1.40]$, avoiding harsh halo artifacts around hair or shoulders.
- **Toggle & Controls**:
  - On/Off toggle switch + intensity preset pills (`Soft`, `Studio`, `Dreamy`) and blend intensity slider in the Capture panel (`SideEffect::SetPortraitBlur(bool, f32)`).

---

### 2.4 Studio Color LUT Presets

PhoneCam Pro includes 5 studio-tuned color grading Look-Up Table (LUT) presets applied directly to premultiplied BGRA buffers with zero dynamic memory allocation.

| Preset | Visual Characteristics | Best Used For |
| :--- | :--- | :--- |
| **Natural** | Identity calibration, natural sensor fidelity, neutral skin tones | Day-to-day meetings, realistic true-to-life capture |
| **Warm Studio** | Elevated red/warm spectrum, $+10\%$ contrast, subtle golden shadow lift | Dim home-office lighting, warm desk lamps, flattering skin warmth |
| **Cool Corporate** | Crisp blue tones, lifted shadows, neutral whites, high clarity | Executive presentations, daylight-balanced offices, clean modern look |
| **Vibrant Streamer** | $+35\%$ saturation boost with ITU-R BT.601 luma preservation, punchy colors | Gaming streams, YouTube live streaming, vibrant backdrops & RGB setups |
| **Monochrome** | High-contrast black & white luminance filter via BT.601 fixed-point math | Artistic broadcasts, dramatic lighting, black-and-white presentations |

- **Off Toggle**:
  - Users can select **None / Off** to bypass color grading entirely and stream the raw sensor output.
- **Toggle & Controls**:
  - Dropdown and pill selectors in the Capture panel dispatching `SideEffect::SetLutFilter(LutPreset)`.

---

### 2.5 Pure-Black OLED Ambient Display Mode

Streaming for hours with a phone screen turned on at full brightness leads to excessive thermal buildup, rapid battery depletion, and annoying screen glare reflecting off the user's face or monitors.

PhoneCam Pro introduces the **Pure-Black OLED Ambient Display Mode**:

- **True-Black 0-Nit Pixel Shutoff**:
  - Renders `#000000` (100% black) across the entire AMOLED/OLED panel.
  - OLED pixels physically power off, drawing 0 milliamps and emitting 0 nits of light.
  - Reduces phone surface temperatures by up to **8°C to 12°C** compared to running live viewfinder previews.
- **Anti-Burn-In Digital Clock Drift**:
  - Displays a dim, minimalist clock at 20% white opacity (`#33FFFFFF`) along with a subtle live dot badge (`● LIVE`) and battery level gauge.
  - Automatically shifts its coordinates $(X_{offset}, Y_{offset})$ every minute using a subtle cyclical offset formula to prevent static burn-in on sensitive OLED panels.
- **Tap-to-Peek Interaction**:
  - Tapping the black screen immediately transitions to full on-screen controls, connection metrics, and camera settings.
  - Tapping again instantly returns to pure-black ambient silence.
- **Selectable Screen Modes**:
  - **Keep Screen On**: Normal viewfinder display.
  - **Dim Screen**: Viewfinder at minimum system backlight.
  - **Pure-Black Ambient**: True-black OLED power-saving mode.

---

### 2.6 Multi-Lens Optical Camera Switching

Modern Android phones feature advanced multi-camera arrays (Ultra-Wide, Wide, Telephoto, Front). PhoneCam Pro allows switching between these physical lenses on the fly without interrupting the active stream or disconnecting the virtual camera.

- **Physical Lens Detection**:
  - Android client queries `CameraCharacteristics` and `CameraManager.cameraIdList` to classify physical sensors:
    - **Ultra-Wide (0.5x)**: Focal length $<24\text{mm}$ equivalent or optical zoom $\le 0.6\times$.
    - **Wide (1.0x)**: Primary main sensor ($24\text{mm} - 50\text{mm}$ equivalent).
    - **Telephoto (3.0x)**: Dedicated optical zoom sensor ($>50\text{mm}$ equivalent, $2\times - 5\times$ zoom).
    - **Front**: Front-facing selfie camera.
- **Seamless Lens Handoff**:
  - Camera sessions transition cleanly using Camera2 `switchLens()`, preserving stream settings, encoder bitrates, and client state.
- **Desktop UI**:
  - Fast-action selector pills (`0.5x`, `1.0x`, `3.0x`, `Front`) in the Capture panel dispatching `DesktopToPhone::SetCamera`.

---

### 2.7 Pro Manual Camera Controls

Take complete control over sensor exposure, color temperature, and focal plane with manual camera adjustments:

- **Exposure Compensation (EV)**:
  - Adjust exposure bias from $-4.0\text{ EV}$ to $+4.0\text{ EV}$ in fractional steps.
  - Automatically queries `CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE` and clamps inputs to the physical device's sensor limits.
- **Manual Kelvin White Balance**:
  - Switch between Auto White Balance (AWB) and precision Kelvin temperature selection ($2000\text{K} - 8000\text{K}$).
  - Controls `CONTROL_AWB_MODE` and calculates manual R/G/B color channel gains.
- **Manual Diopter Focus (Macro Focus)**:
  - Transition from Continuous Video Autofocus (`CONTROL_AF_MODE_CONTINUOUS_VIDEO`) to manual diopter distance control (`LENS_FOCUS_DISTANCE`).
  - Focus distance calibrated in diopters ($0.0$ for infinity to $10.0+$ diopters for extreme close-up macro shots).
- **Anti-Flicker Mode**:
  - Toggle between `Auto`, `50Hz` (Europe, Asia, Australia), and `60Hz` (North America) mains frequency anti-banding to eliminate rolling horizontal bars caused by fluorescent or LED room lights.

---

### 2.8 USB Auto-Connect & Background ADB Supervisor

PhoneCam Pro makes tethered streaming frictionless:

- **Background ADB Supervisor**:
  - The desktop runtime monitors USB device connection events asynchronously.
  - Automatically executes `adb reverse tcp:48721 tcp:48721` whenever a supported Android device is detected with USB debugging enabled.
- **Auto-Connect on USB Plug**:
  - When enabled, connecting your phone via USB immediately connects the streaming session without requiring the user to click "Start Stream" on the phone or PC.
- **Minimize to Tray on Window Close**:
  - Closing the desktop window minimizes the process to the Windows system notification tray (`Shell_NotifyIcon`), keeping the virtual camera feed alive and active for OBS, Zoom, and Discord.

---

## 3. Privacy Principles & Zero-Telemetry Guarantee

PhoneCam is built from first principles as an **offline-first, zero-cloud, privacy-preserving application**:

1. **Strict Zero-Telemetry Invariant**:
   - There are **no analytics**, **no user tracking**, **no crash reporting pings**, and **no phone-home network requests**.
   - PhoneCam never connects to any external cloud API, remote server, or third-party endpoint.
2. **Local-Only Media Processing**:
   - 100% of video decoding, audio decoding, color grading, background blurring, and auto-framing math runs entirely on your local CPU/GPU.
   - Decoded pixels never leave your machine; they are written directly to local Windows shared memory (`framebuffer-v2.bin`) accessible only to the local OS camera server.
3. **Cryptographic Local Pairing**:
   - First-time device pairing requires scanning an ephemeral 128-bit CSPRNG QR token that expires in 5 minutes and requires explicit manual approval on the desktop.
   - Subsequent sessions authenticate via `HMAC-SHA256` using local secrets stored in `%APPDATA%\PhoneCam\devices.json`.
4. **Automated Sensitive Token Redaction**:
   - All internal logging facilities automatically redact any contiguous 32-character or 64-character hexadecimal tokens from log sinks, preventing accidental credential leakage.

---

## 4. Testing Procedures & Environment Variables

PhoneCam includes a comprehensive physical device and integration test harness in `crates/phonecam-app/tests/live_device_test.rs`.

### Environment Variables

The test suite and development scripts utilize standard environment variables for cross-platform and multi-device portability:

| Environment Variable | Description | Default Fallback Behavior |
| :--- | :--- | :--- |
| `ANDROID_SERIAL` | Target Android device serial number (e.g. `export ANDROID_SERIAL=12345678`) | Auto-detects via `adb get-serialno` or first device in `adb devices`; skips gracefully if no device is connected. |
| `PHONECAM_ADB_PATH` | Path to the Android Debug Bridge executable | Checks `PATH` for `adb`, then falls back to relative `dist/unpacked/adb.exe`. |
| `PHONECAM_ARTIFACT_DIR` | Destination directory for test screenshots and captured video frames | Defaults to `%TEMP%\phonecam-artifacts` (or `/tmp/phonecam-artifacts`). |
| `PHONECAM_PORT` | Custom TCP/UDP service port for desktop listener | Defaults to `48721` (TCP) and `48720` (UDP). |

### Running Tests

```powershell
# 1. Run all workspace unit tests (230+ tests across all 7 crates)
cargo test --workspace

# 2. Run Android JVM unit tests
cd apps/android
.\gradlew.bat test
cd ../..

# 3. Run physical live device end-to-end integration test
# (Requires Android phone connected via USB with USB debugging enabled)
$env:ANDROID_SERIAL = "your_device_serial"
cargo test -p phonecam-app --test live_device_test -- --ignored --nocapture

# 4. Code quality & lint checks
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```
