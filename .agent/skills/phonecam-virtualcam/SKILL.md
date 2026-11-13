---
name: phonecam-virtualcam
description: Virtual camera implementations across Windows (MFCreateVirtualCamera & DirectShow COM), Linux (v4l2loopback), and macOS (CoreMediaIO DAL plugin).
---

# PhoneCam Virtual Camera Architecture & Implementation Skill Guide

This skill guide provides a comprehensive technical reference for PhoneCam's virtual camera subsystems across Windows, Linux, and macOS, detailing operating-system driver contracts, shared memory synchronization, registration lifecycles, and cross-platform capture integration.

---

## 1. System Architecture & Driver Isolation

A virtual camera allows consumer applications (OBS Studio, Google Meet, Zoom, Microsoft Teams, Discord, native Camera apps) to access PhoneCam as if it were a physical UVC hardware camera.

```
┌─────────────────────────────────────────────────────────────┐
│                    PhoneCam Desktop Process                 │
│   Network Receiver ──► openh264 Decoder ──► NV12 Framebus   │
└───────────────────────────────┬─────────────────────────────┘
                                │ Memory-Mapped File (Seqlock)
                                ▼
┌─────────────────────────────────────────────────────────────┐
│               Host Operating System Camera Subsystem        │
│                                                             │
│  [Windows] Windows Camera Frame Server (svchost.exe -k)     │
│            └── PhoneCamVirtualCamera.dll (MF / DirectShow)  │
│                                                             │
│  [Linux]   v4l2loopback Kernel Module                       │
│            └── /dev/video10 (V4L2 Device Node)              │
│                                                             │
│  [macOS]   CoreMediaIO Camera Extension / DAL Plugin        │
│            └── CMIOExtensionProvider (CMSampleBuffer NV12)   │
└───────────────────────────────┬─────────────────────────────┘
                                │ Native OS Camera API
                                ▼
┌─────────────────────────────────────────────────────────────┐
│                      Client Applications                    │
│             OBS Studio / Zoom / Meet / Teams / Browsers     │
└─────────────────────────────────────────────────────────────┘
```

### Architectural Principles
1. **Process Isolation**: The camera driver runs inside the operating system's camera server process (`svchost.exe -k Camera` on Windows, kernel space on Linux, or isolated daemon on macOS). It cannot access PhoneCam's internal heap, sockets, or threads.
2. **Zero-IPC Overhead**: Video data flows via high-speed, zero-copy shared memory synchronized through lock-free seqlocks.
3. **Fail-Safe Slate**: If the network connection drops, the virtual camera never disappears. It holds the last frame for 3.0 seconds, then outputs a solid black frame with `state = Disconnected`.

---

## 2. Windows Implementation (Media Foundation & DirectShow)

### 2.1 Media Foundation Virtual Camera (`MFCreateVirtualCamera`)
Modern Windows (Windows 11 Build 22621+ / Version 22H2+) introduces native software camera source registration through Media Foundation:
- **API Function**: `MFCreateVirtualCamera`
- **Camera Type**: `MFVirtualCameraType_SoftwareCameraSource`
- **Lifetime**: `MFVirtualCameraLifetime_System`
- **Access**: `MFVirtualCameraAccess_AllUsers`
- **Host Process**: Windows Camera Frame Server (`svchost.exe -k Camera`) running under the `NT AUTHORITY\LocalService` account.
- **CLSID**: Registered in Windows device catalog under `KSCATEGORY_VIDEO_CAMERA`.

### 2.2 DirectShow COM Fallback
For legacy 32-bit applications and older Windows installations (Windows 10 Build 19041–19045):
- **Filter Base**: DirectShow `CSource` and `CSourceStream` COM filter.
- **Registration**: Registered in `CLSID_VideoInputDeviceCategory` (`{8606A210-D43D-11CF-BD16-003555000000}`).
- **Pin Formats**: Advertises `MEDIASUBTYPE_NV12` and `MEDIASUBTYPE_YUY2` output pins.
- **Registration Command**: `regsvr32.exe /s PhoneCamVirtualCameraDS.dll`.

### 2.3 Shared Memory Framebus Contract
Communication between `phonecam.exe` and `PhoneCamVirtualCamera.dll` is mediated through:
`C:\ProgramData\PhoneCam\framebuffer-v2.bin`

#### Packed Header Layout (Exactly 68 Bytes)
```rust
#[repr(C, packed)]
pub struct FramebusHeader {
    pub magic: u32,          // 0x5043414D ("PCAM")
    pub layout_version: u32, // 2
    pub header_size: u32,    // 68
    pub width: u32,          // Frame width (e.g. 1920)
    pub height: u32,         // Frame height (e.g. 1080)
    pub stride_y: u32,       // Y plane stride (bytes)
    pub stride_uv: u32,      // UV interleaved plane stride (bytes)
    pub frame_index: u64,    // Monotonic frame counter
    pub timestamp_us: u64,   // Frame timestamp (microseconds)
    pub state: u32,          // 0 = Uninitialized, 1 = Live, 2 = Disconnected
    pub seq: u32,            // Seqlock counter (odd = writing, even = stable)
    pub readers: u32,        // Monotonic counter of active reader instances
    pub reserved: [u8; 12],  // Reserved for future extensions
}
```

#### Seqlock Writing & Reading Protocol
1. **Writer Algorithm**:
   - Store `seq + 1` (odd number) with atomic release semantics.
   - Update header metadata (`width`, `height`, `stride`, `frame_index`, `timestamp_us`, `state`).
   - Copy raw NV12 pixel payload to buffer at offset 68.
   - Store `seq + 2` (even number) with atomic release semantics.
2. **Reader Algorithm**:
   - Load `seq0`. If odd, spin or yield until even.
   - Snapshot header metadata.
   - Copy NV12 payload from offset 68.
   - Load `seq1`. If `seq0 != seq1`, payload was torn during copy; re-read.
3. **Critical Invariants**:
   - The writer must **never** write to or reset `readers`. Only consumer DLLs increment/decrement `readers`.
   - The backing file is **grow-only**. Never truncate the file on resolution downgrade, as Windows returns `ERROR_USER_MAPPED_FILE` (0x4C8) if mapped by an external process.

### 2.4 Registration Management CLI (`phonecam-vcam-register.exe`)
The registrar executable manages COM registration and system catalog enumeration:

```powershell
# Register the virtual camera (requires Administrator privileges)
phonecam-vcam-register.exe --register

# Query current registration and enumeration state
phonecam-vcam-register.exe --status

# Unregister virtual camera and remove COM entries
phonecam-vcam-register.exe --unregister
```

> **Importance of `--status`**: Simply checking the exit code of `--register` is insufficient. `--status` executes `MFEnumDeviceSources` to guarantee that the camera is actually visible in the Windows device topology.

---

## 3. Linux Implementation (`v4l2loopback`)

On Linux, PhoneCam integrates with the standard Video4Linux2 subsystem via the `v4l2loopback` kernel module.

### 3.1 Kernel Module Setup
```bash
# Install v4l2loopback kernel module (Debian/Ubuntu)
sudo apt-get install -y v4l2loopback-dkms v4l2loopback-utils

# Load module creating dedicated virtual camera device
sudo modprobe v4l2loopback \
  devices=1 \
  video_nr=10 \
  card_label="PhoneCam Virtual Camera" \
  exclusive_caps=1
```
- `devices=1`: Create a single loopback device node.
- `video_nr=10`: Pins the node to `/dev/video10`.
- `exclusive_caps=1`: Prevents WebRTC/Chrome from misidentifying the device as both an output and capture device.

### 3.2 Udev Rules for Non-Root Access
Create `/etc/udev/rules.d/99-phonecam-vcam.rules`:
```udev
KERNEL=="video[0-9]*", SUBSYSTEM=="video4linux", ATTR{name}=="PhoneCam Virtual Camera", MODE="0666", GROUP="video"
```
Apply rules:
```bash
sudo udevadm control --reload-rules && sudo udevadm trigger
```

### 3.3 Frame Injection Pipeline
In Linux builds, `phonecam-framebus` or a bridge utility writes raw frames directly to the loopback file descriptor:
1. Open device: `int fd = open("/dev/video10", O_RDWR);`
2. Negotiate format via `ioctl(fd, VIDIOC_S_FMT, &v4l2_fmt)`:
   - Pixel format: `V4L2_PIX_FMT_NV12` or `V4L2_PIX_FMT_YUV420`.
   - Resolution: matching incoming stream dimensions.
3. Write frame: `write(fd, frame_ptr, frame_size);`.

---

## 4. macOS Implementation (CoreMediaIO DAL & CMIOExtension)

### 4.1 Modern macOS 12.3+ (`CMIOExtensionProvider`)
Modern macOS deprecates legacy plugins in favor of DriverKit CoreMediaIO Extensions:
- **Framework**: `CoreMediaIO.framework`
- **Extension Type**: `CMIOExtensionProvider`
- **Sandbox**: Fully sandboxed, non-root user daemon.
- **Interface Structure**:
  - `CMIOExtensionDeviceSource`: Declares device identity, vendor name, and localized device title.
  - `CMIOExtensionStreamSource`: Declares stream capabilities (`kCMIOExtensionStreamDirectionSource`) and supported formats.
- **Pixel Buffer Delivery**:
  Delivers `CMSampleBufferRef` wrapping a `CVPixelBufferRef`:
  - Format: `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange` (NV12 planar format).
  - Timing: Monotonic `CMTime` timestamps synchronized with `pts_us`.

### 4.2 Legacy macOS DAL Plugin (`PhoneCamDAL.plugin`)
For macOS installations prior to 12.3:
- **Location**: `/Library/CoreMediaIO/Plug-Ins/DAL/PhoneCamDAL.plugin`
- **Mechanism**: In-process dynamic library loaded into consumer processes.
- **IPC**: Reads POSIX shared memory (`/tmp/phonecam_framebus`) created by PhoneCam desktop app.
- **Limitation**: Apple Hardened Runtime in newer versions of Safari and FaceTime blocks loading third-party DAL plugins unless SIP is relaxed or `CMIOExtension` is used.

---

## 5. Diagnostic & Verification Procedures

### 5.1 Verification Checklist
| Target Application | Verification Method | Expected Result |
| :--- | :--- | :--- |
| **OBS Studio** | Add Source -> Video Capture Device -> "PhoneCam Camera" | Immediate video feed without frame stutter |
| **Chrome / Edge** | `chrome://settings/content/camera` | "PhoneCam Camera" listed in dropdown |
| **Windows Camera** | Launch native app (`Win + S -> Camera`) | Switch camera cycles to PhoneCam |
| **VLC Media Player** | Media -> Open Capture Device -> DirectShow | Select "PhoneCam Camera", plays live |

### 5.2 Common Issues & Resolutions
1. **OBS Shows Solid Black Screen**:
   - Check if `framebuffer-v2.bin` has `state = 1`.
   - Verify `magic == 0x5043414D` and `layout_version == 2`.
   - Restart the Windows Camera Frame Server:
     ```powershell
     Stop-Service -Name "CapabilityAccessManagerService" -Force
     Start-Service -Name "CapabilityAccessManagerService"
     ```
2. **Camera Not Listed in Applications**:
   - Execute `phonecam-vcam-register.exe --status`.
   - Re-run `phonecam-vcam-register.exe --register` from an elevated prompt.
3. **Color Inversion (Red/Blue swapped)**:
   - Ensure the virtual camera receives planar NV12. (BGRA conversion is reserved strictly for the GPUI preview surface).
