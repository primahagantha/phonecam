---
name: phonecam-troubleshoot
description: Step-by-step diagnostic guide for Wi-Fi discovery, USB ADB reverse, video encoder fallbacks, and audio sync.
---

# PhoneCam Diagnostic & Troubleshooting Skill Guide

This skill guide provides a systematic, step-by-step diagnostic procedure for resolving connectivity failures, network discovery issues, ADB reverse tunneling errors, video encoding/decoding anomalies, shared memory framebus synchronization stalls, and audio/video synchronization drift.

---

## 1. Fast Triage Decision Tree

When diagnosing an issue, follow this hierarchical triage order:

```
[ Problem Occurs ]
       │
       ▼
[ Step 1: Host Process & Port Availability ]
       ├── Error: "os error 10048 (WSAEADDRINUSE)"
       │     └── Resolution: Override port with $env:PHONECAM_PORT = 48791
       │
       ▼
[ Step 2: Connection Mode & Transport ]
       ├── USB Mode: Inspect ADB daemon and reverse tunnel (adb reverse --list)
       └── Wi-Fi Mode: Check UDP 48720 broadcast & AP isolation; fallback to QR
       │
       ▼
[ Step 3: Authentication & Handshake ]
       ├── Status 1 (BadAuth): Nonce desync or secret mismatch -> Re-pair with QR
       └── Status 2 (Busy): Desktop holds active session -> Disconnect orphan client
       │
       ▼
[ Step 4: Media Pipeline & Decoder ]
       ├── Bitstream not decoding: Missing SPS/PPS headers -> Request keyframe
       └── Heavy stutter / frame drop: Thermal clamp active -> Trigger down-step
       │
       ▼
[ Step 5: Shared Memory Framebus & Virtual Camera ]
       ├── OBS shows black screen: Inspect framebuffer-v2.bin header (magic/state)
       └── Camera missing from device list: Run phonecam-vcam-register.exe --status
```

---

## 2. Wi-Fi Discovery & UDP Broadcast Troubleshooting

### 2.1 Router AP Isolation / Client Isolation
- **Symptom**: Phone cannot discover desktop over Wi-Fi despite both being on the same local router.
- **Root Cause**: Many consumer and enterprise routers enable **AP Isolation** (client-to-client isolation), which drops all local multicast and broadcast packets between wireless clients.
- **Diagnostics**:
  - Test ping from phone to desktop IP address (using an Android terminal app or Termux).
  - If pings fail, client isolation is active.
- **Remediation**:
  1. Disable AP Isolation in your router's wireless settings.
  2. If router settings are inaccessible, connect phone via **USB mode** (100% immune to network isolation).
  3. Or use **Manual IP Connect**: Enter the desktop's LAN IP address in the phone app or scan the pairing QR code.

### 2.2 Windows Firewall Blocking UDP 48720 or TCP 48721
- **Symptom**: UDP beacons sent by the desktop never arrive at the phone.
- **Root Cause**: Windows assigns the network interface to the **Public** profile instead of **Private**, blocking incoming unsolicited UDP/TCP packets.
- **Inspection & Fix**:
  ```powershell
  # Check current network profile category
  Get-NetConnectionProfile

  # Set Wi-Fi connection to Private profile
  Set-NetConnectionProfile -InterfaceAlias "Wi-Fi" -NetworkCategory Private

  # Add explicit Windows Defender Firewall rules for PhoneCam
  New-NetFirewallRule -DisplayName "PhoneCam Service (TCP 48721)" -Direction Inbound -Protocol TCP -LocalPort 48721 -Action Allow
  New-NetFirewallRule -DisplayName "PhoneCam Discovery (UDP 48720)" -Direction Inbound -Protocol UDP -LocalPort 48720 -Action Allow
  ```

### 2.3 Multiple Network Interfaces (Virtual Adapters)
- **Symptom**: Desktop advertises an unreachable IP (e.g., WSL `172.x.x.x` or Hyper-V/VirtualBox `192.168.56.x` instead of your real Wi-Fi `192.168.1.x`).
- **Remediation**:
  - PhoneCam scans interfaces for non-loopback, non-virtual IPv4 addresses. Ensure your primary Wi-Fi adapter has a higher metric priority, or specify the correct IP manually in the phone app.

### 2.4 Port Conflict (`WSAEADDRINUSE / Error 10048`)
- **Symptom**: Application fails to start or logs: `Address already in use (os error 10048)`.
- **Root Cause**: A legacy PhoneCam instance or another process has already bound TCP port `48721`.
- **Remediation**:
  ```powershell
  # Identify process holding port 48721
  Get-NetTCPConnection -LocalPort 48721 | Select-Object LocalAddress, LocalPort, OwningProcess

  # Launch PhoneCam on an alternate port
  $env:PHONECAM_PORT = 48791
  cargo run -p phonecam-app
  ```

---

## 3. USB ADB Reverse Tunnel Troubleshooting

### 3.1 USB State Machine Separation
PhoneCam strictly tracks two independent USB status indicators:
- `usb.adbAvailable`: Whether `adb.exe` is found in `PATH` or standard SDK paths and can execute.
- `usb.available`: Whether a physical Android device is connected, authorized, and responsive.

Never confuse a missing ADB binary with a disconnected phone.

### 3.2 Step-by-Step ADB Verification
```powershell
# 1. Verify ADB detects the phone
adb devices -l
```
Possible outputs:
- `List of devices attached` (empty): Bad USB cable (charge-only cable), USB debugging disabled in Developer Options, or missing OEM USB drivers.
- `xxxxxxxx    unauthorized`: Phone screen is locked or USB debugging prompt was not accepted. **Unlock phone and tap "Always allow from this computer"**.
- `xxxxxxxx    device`: Device is fully authorized and ready.

### 3.3 Verifying & Repairing Reverse Sockets
PhoneCam bridges port `48721` from phone loopback to PC:
```powershell
# Inspect active reverse socket mappings
adb reverse --list

# Expected output:
# (reverse) tcp:48721 tcp:48721

# If missing or broken, recreate the tunnel:
adb reverse tcp:48721 tcp:48721

# If conflicting reverse tunnels exist:
adb reverse --remove-all
adb reverse tcp:48721 tcp:48721
```

### 3.4 Remotely Triggering Android Camera Service via ADB
If the phone screen is not directly accessible, remotely start the streaming service:
```powershell
adb shell am start-foreground-service `
  -n com.phonecam.camera.debug/com.phonecam.camera.service.CameraStreamService `
  -a com.phonecam.camera.action.CONNECT_MANUAL `
  --es host 127.0.0.1 `
  --ei port 48721
```

---

## 4. Video Encoder & Hardware Fallbacks

### 4.1 Android MediaCodec Encoder Failures
- **Symptom**: Phone connects, but video stream terminates immediately or phone app crashes.
- **Root Cause**: Device hardware encoder (`OMX.qcom.video.encoder.avc` or `c2.android.avc.encoder`) rejected the requested resolution or framerate (e.g. 1080p60 on low-tier chipsets).
- **Diagnostics**:
  ```powershell
  adb logcat -v time -s MediaCodec:E PhoneCam:V
  ```
- **Remediation**:
  - The phone client automatically falls back to software AVC encoder (`c2.android.avc.encoder` or `OMX.google.h264.encoder`) or clamps the resolution to `720p30`.
  - In desktop UI: Lower the capture resolution ceiling in the Rail -> Capture tab to `720p30` or `480p30`.

### 4.2 Missing SPS/PPS Parameter Sets
- **Symptom**: Desktop receives video packets, but `openh264` outputs zero frames (`no picture decoded`).
- **Root Cause**: Decoder joined mid-stream and has not yet received Sequence Parameter Sets (SPS) or Picture Parameter Sets (PPS) NAL units.
- **Remediation**:
  - PhoneCam automatically requests an instantaneous IDR frame via `DesktopToPhone::RequestKeyframe`.
  - In Android client, ensure `MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME` is dispatched upon connection.

### 4.3 Thermal Clamping & Quality Ladder Drops
- **Symptom**: Stream drops from 1080p60 to 720p30 without packet loss.
- **Root Cause**: Phone emitted a critical thermal alert (`thermal: "critical"`).
- **Behavior**: The adaptive bitrate ladder intentionally bypasses the standard 10-second dwell time on thermal emergencies, dropping rungs immediately to prevent hardware thermal shutdown.

### 4.4 60 FPS Camera2 AE Range Troubleshooting (`upper == 60` vs 30 FPS Cap)
- **Symptom**: Desktop requests 60 FPS, but frame telemetry indicates incoming FPS is capped at 30 FPS, or capture session creation fails.
- **Root Cause**:
  1. Camera sensor hardware does not support 60 FPS in its Auto-Exposure target ranges (`CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES`).
  2. Many budget or mid-tier Android sensors support 60 FPS exclusively via **Constrained High Speed Session** (`highSpeedVideoSizes`), which forbids simultaneous preview surfaces (`previewSurface == null`).
- **Inspection**:
  ```powershell
  # Query Camera2 characteristics via ADB
  adb shell dumpsys media.camera | Select-String -Pattern "CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES|SCALER_STREAM_CONFIGURATION_MAP"
  ```
- **Architectural Handling in PhoneCam**:
  - `CameraCapabilityProbe` queries `aeRanges.any { it.upper >= 60 }`.
  - When `hasAe60` is true, PhoneCam requests standard session with `CONTROL_AE_TARGET_FPS_RANGE` set to `[30, 60]`.
  - **Exposure Headroom Rule**: PhoneCam avoids pinning `[60, 60]`. Pinning `[60, 60]` forbids exposure lengthening, causing freezing/underexposure indoors. `[30, 60]` allows the camera to adapt exposure under dim lighting while delivering 60 FPS in normal lighting.
  - When `hasAe60` is false and high-speed sizes are unavailable, PhoneCam safely clamps the target FPS to 30.

### 4.5 Back Camera Anti-Flicker Reapplication on Lens Switch
- **Symptom**: Switching between rear camera lenses (e.g. 1.0x wide to 0.5x ultra-wide or 3.0x telephoto) introduces visible 50 Hz / 60 Hz rolling light bands or flickering brightness under indoor lighting.
- **Root Cause**:
  - In Android Camera2, switching physical lenses or recreating the capture session resets `CaptureRequest.CONTROL_AE_ANTIBANDING_MODE` to hardware defaults (often `OFF` or device default) unless explicitly re-applied.
- **Remediation**:
  - `CameraController` retains `savedAntibanding = configuredAntibanding` during lens transitions.
  - After recreation of the repeating request builder, it explicitly invokes:
    ```kotlin
    val targetMode = resolveAntibandingMode(chars, configuredAntibanding)
    builder.set(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE, targetMode)
    ```
  - Mode resolution:
    - `"50hz"` $\rightarrow$ `CONTROL_AE_ANTIBANDING_MODE_50HZ` (falls back to `AUTO`).
    - `"60hz"` $\rightarrow$ `CONTROL_AE_ANTIBANDING_MODE_60HZ` (falls back to `AUTO`).
    - `"auto"` $\rightarrow$ `CONTROL_AE_ANTIBANDING_MODE_AUTO` (dynamic flicker detection).

---

## 5. Shared Memory Framebus & Virtual Camera Diagnostics

### 5.1 Inspecting the Framebus File
Backing file location: `C:\ProgramData\PhoneCam\framebuffer-v2.bin`

Use PowerShell to verify header validity and streaming health:
```powershell
$bytes = [System.IO.File]::ReadAllBytes("C:\ProgramData\PhoneCam\framebuffer-v2.bin")
$magic = [System.Text.Encoding]::ASCII.GetString($bytes[0..3])
$version = [System.BitConverter]::ToUInt32($bytes, 4)
$headerSize = [System.BitConverter]::ToUInt32($bytes, 8)
$width = [System.BitConverter]::ToUInt32($bytes, 12)
$height = [System.BitConverter]::ToUInt32($bytes, 16)
$state = [System.BitConverter]::ToUInt32($bytes, 44)
$seq = [System.BitConverter]::ToUInt32($bytes, 48)
$readers = [System.BitConverter]::ToUInt32($bytes, 52)

[PSCustomObject]@{
    Magic = $magic          # Must be "PCAM"
    Version = $version      # Must be 2
    HeaderSize = $headerSize# Must be 68
    Resolution = "$width x $height"
    State = $state          # 1 = Live, 2 = Disconnected
    Sequence = $seq         # Must be even when stable
    ActiveReaders = $readers# Incremented by OBS/Zoom
}
```

### 5.2 Virtual Camera Driver Re-Registration
If OBS Studio or web browsers do not list "PhoneCam Camera":
```powershell
# Run from Administrator PowerShell:
# 1. Check enumerable status
.\dist\unpacked\phonecam-vcam-register.exe --status

# 2. Re-register COM server and Media Foundation source
.\dist\unpacked\phonecam-vcam-register.exe --register

# 3. Restart Windows Camera Frame Server if COM registration is stale
Stop-Service -Name "CapabilityAccessManagerService" -Force
Start-Service -Name "CapabilityAccessManagerService"
```

### 5.3 Double-Buffered GPU Texture Memory Leak Diagnosis
- **Symptom**:
  - The desktop process (`phonecam.exe`) gradually increases GPU VRAM consumption and system memory (Private Working Set) over hours of streaming, eventually causing DirectX 11 out-of-memory errors or application crashes.
- **Root Cause**:
  - In GPUI on Windows (Direct3D 11), creating a new `gpui::RenderImage` at 30-60 FPS adds textures to the window texture atlas.
  - If previous frame handles are retained or dropped without explicit atlas eviction, stale textures accumulate in VRAM.
- **Remediation & Architecture**:
  - `phonecam-ui` implements a strict double-buffered preview contract in `AppView`:
    ```rust
    // Evicts frame from 2 frames ago from Direct3D texture atlas
    if let Some(frame_2_ago) = self.previous_preview_image.take() {
        if let Some(w) = window.as_mut() {
            let _ = w.drop_image(frame_2_ago.clone());
        }
        reclaim_render_image_buffer(frame_2_ago, self.previous_preview_capacity);
    }
    self.previous_preview_image = self.preview_image.take();
    self.preview_image = Some(Arc::new(RenderImage::new(...)));
    ```
  - `window.drop_image()` explicitly purges the GPU texture handle.
  - `reclaim_render_image_buffer` recycles the raw pixel `Vec<u8>` capacity to prevent heap churn.
  - Holding the frame for exactly 1 extra frame (`previous_preview_image`) ensures the GPU finishes drawing the frame before eviction, preventing visible flickering.

---

## 6. Audio Sync & Latency Tuning

### 6.1 Audio/Video Skew Diagnosis
- **Symptom**: Audio leads or lags behind video during live streaming or recording.
- **Root Causes**:
  1. Phone camera hardware pipeline and microphone hardware pipeline operate on independent hardware clocks that drift over time.
  2. Frame pacing buffer in desktop application holding excess video frames.
- **Diagnostics**:
  - Inspect `pts_us` in video headers: verify timestamps advance by ~16,666 µs (60 FPS) or ~33,333 µs (30 FPS).
  - Jitter spikes: If `pts_us` jumps backward or jumps by >100ms, the phone encoder dropped frames under heavy system load.

### 6.2 Latency Tuning Checklist
1. **Prefer USB Connection**: Eliminates 10–40ms of 2.4/5GHz Wi-Fi packet jitter and bufferbloat.
2. **Disable Power Saving on Phone**: Battery saver mode throttles camera sensor processing clock, inducing 50–100ms latency spikes.
3. **Set Constant Framerate (CFR)**: Configure Android Camera2 capture request to `CONTROL_AE_TARGET_FPS_RANGE = [30, 60]`.

### 6.3 Audio Recording Permission & Foreground Service Configuration
- **Symptom**:
  - Enabling audio streaming produces no sound, or phone logcat logs:
    `AudioStreamer: android.permission.RECORD_AUDIO not granted; aborting`
  - Android application crashes with `SecurityException: Foreground service type microphone not declared`.
- **Root Cause**:
  1. User denied microphone permission on Android.
  2. Android 14+ (API 34+) strictly requires foreground services capturing audio to declare `android:foregroundServiceType="camera|microphone"` in `AndroidManifest.xml`.
- **Diagnostics**:
  ```powershell
  # Check granted permissions
  adb shell pm dump com.phonecam.camera.debug | Select-String -Pattern "RECORD_AUDIO"
  ```
- **Remediation**:
  1. Grant microphone permission manually via ADB if UI prompt was dismissed:
     ```powershell
     adb shell pm grant com.phonecam.camera.debug android.permission.RECORD_AUDIO
     ```
  2. In `apps/android/app/src/main/AndroidManifest.xml`, verify service declaration:
     ```xml
     <service
         android:name=".service.CameraStreamService"
         android:foregroundServiceType="camera|microphone"
         android:exported="false" />
     ```
  3. Ensure `CameraStreamService` starts with `startForeground()` passing a valid notification with audio capabilities.

---

## 7. Diagnostic Bundles & Privacy Verification

When gathering logs for GitHub issues or bug reports:

### 7.1 Collecting Desktop Logs
Desktop logs are stored at:
`%APPDATA%\PhoneCam\logs\phonecam.log`

### 7.2 Collecting Android Logcat
```powershell
adb logcat -d -v time -s PhoneCam:V CameraStreamService:V > android-phonecam.log
```

### 7.3 Verifying Credential Redaction
Before sharing logs, verify that no raw secrets were emitted:
```powershell
# Search for unredacted 32+ character hex strings
Select-String -Path android-phonecam.log, "$env:APPDATA\PhoneCam\logs\phonecam.log" -Pattern "[a-f0-9]{32,}"
```
PhoneCam's built-in `redact()` automatically replaces all 32+ hex strings with `[REDACTED]`. The search above should return zero raw secret matches.
