# PhoneCam Multi-Platform OS Support & Architecture Matrix

This document provides a comprehensive reference for PhoneCam across supported desktop operating systems (Windows 11, Windows 10 Build 17763+, Linux distributions, and macOS) as well as the Android client multi-ABI architecture.

---

## 1. Operating System Support Matrix

| Platform | Minimum Version | Target Architectures | Video Output Pipeline | Transport | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Windows 11** | Build 22621 (22H2)+ | x86_64, aarch64 | Windows Media Foundation Virtual Camera (`MFCreateVirtualCamera`) | USB (ADB) & Wi-Fi | **Tier 1 (Production)** |
| **Windows 10** | Version 1809 (Build 17763)+ | x86_64 | DirectShow COM Virtual Camera Filter (`phonecam-vcam-register.exe`) | USB (ADB) & Wi-Fi | **Tier 1 (Production)** |
| **Linux (Ubuntu/Debian)** | Ubuntu 20.04 LTS / Debian 11+ | x86_64, aarch64 | `v4l2loopback` kernel module (`/dev/videoX`) | USB (ADB) & Wi-Fi | **Supported** |
| **Linux (Fedora/DNF)** | Fedora 38+ / RHEL 9+ | x86_64, aarch64 | `v4l2loopback` kernel module (`/dev/videoX`) | USB (ADB) & Wi-Fi | **Supported** |
| **Linux (Arch Linux)** | Rolling (kernel 5.15+) | x86_64 | `v4l2loopback-dkms` kernel module (`/dev/videoX`) | USB (ADB) & Wi-Fi | **Supported** |
| **macOS** | macOS 12.3 (Monterey)+ | Apple Silicon (arm64), Intel (x86_64) | CoreMediaIO DAL Plugin / Camera Extension | USB (ADB) & Wi-Fi | **Supported** |
| **Android Client** | Android 8.0 (API 26)+ | arm64-v8a, armeabi-v7a, x86_64 | Camera2 API + Hardware `MediaCodec` H.264 | Wi-Fi LAN & USB Loopback | **Tier 1 (Production)** |

---

## 2. Windows 10 (Version 1809+, Build 17763+) Validation

### Architecture & Capabilities
Windows 10 Version 1809 (October 2018 Update, OS Build 17763) is the foundational minimum supported Windows version.

1. **Desktop Companion Application**:
   - Compiles and executes natively with GPUI DirectX 11 / Direct3D 11 hardware-accelerated rendering.
   - Decodes H.264 Annex-B bitstreams in-process via Cisco `openh264`, eliminating external runtime dependencies.
   - System Tray integration is driven by Win32 `Shell_NotifyIconW` with a dedicated hidden message-only window (`HWND`), ensuring icon click events and close-to-tray behaviors work properly.
   - Dual connection modes (USB via ADB reverse tunnel and Wi-Fi via UDP discovery & QR pairing) operate with identical zero-copy performance.

2. **Virtual Camera Implementation on Windows 10**:
   - While Windows 11 provides the newer `MFCreateVirtualCamera` system API, Windows 10 utilizes the DirectShow COM Source Filter mechanism (`PhoneCamVirtualCamera.dll`).
   - Registration is performed via `phonecam-vcam-register.exe`:
     ```cmd
     :: Must be run from an elevated (Administrator) Command Prompt
     phonecam-vcam-register.exe --register
     ```
   - Registration verifies COM class entries (`CLSID_VideoInputDeviceCategory`), device enumeration, and Frame Server availability.
   - Verifiable status can be checked at any time:
     ```cmd
     phonecam-vcam-register.exe --status
     ```

3. **Portable Release Bundle**:
   - For environments where system installation is prohibited or portable use is preferred, `dist/unpacked/` delivers a self-contained execution package:
     - `run-portable.bat`: Runs local directory sanity checks, ensures bundled `adb.exe` is in `PATH`, verifies `PhoneCamVirtualCamera.dll` presence, and starts `phonecam.exe` without installation.
     - `install-virtual-camera.bat`: One-click Administrator privilege elevation wrapper to register the virtual camera driver.
     - `uninstall-virtual-camera.bat`: One-click uninstaller for driver cleanup.

---

## 3. Linux Building & Usage Workflows

PhoneCam's core (`phonecam-core`, `phonecam-protocol`, `phonecam-media`, `phonecam-framebus`, and `phonecam-ui`) builds natively on modern Linux systems.

### Package Prerequisites by Distribution

#### Debian / Ubuntu / Linux Mint
```bash
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    pkg-config \
    libfontconfig1-dev \
    libasound2-dev \
    libx11-dev \
    libxcursor-dev \
    libxrandr-dev \
    libxi-dev \
    libssl-dev \
    v4l2loopback-dkms \
    v4l-utils \
    android-tools-adb
```

#### Fedora / RHEL / CentOS Stream (DNF)
```bash
sudo dnf install -y \
    gcc \
    gcc-c++ \
    pkgconf-pkg-config \
    fontconfig-devel \
    alsa-lib-devel \
    libX11-devel \
    libXcursor-devel \
    libXrandr-devel \
    libXi-devel \
    openssl-devel \
    v4l2loopback \
    v4l-utils \
    android-tools
```

#### Arch Linux / Manjaro (Pacman)
```bash
sudo pacman -S --needed \
    base-devel \
    fontconfig \
    alsa-lib \
    libx11 \
    libxcursor \
    libxrandr \
    libxi \
    openssl \
    v4l2loopback-dkms \
    v4l-utils \
    android-tools
```

### Configuring `v4l2loopback` for Virtual Camera
To create the virtual video device for consumer applications (OBS Studio, Discord, Chrome, Zoom):

```bash
# Load module with device label and fixed device node
sudo modprobe v4l2loopback devices=1 video_nr=10 card_label="PhoneCam" exclusive_caps=1
```

To persist across reboots:
```bash
# /etc/modules-load.d/v4l2loopback.conf
v4l2loopback

# /etc/modprobe.d/v4l2loopback.conf
options v4l2loopback devices=1 video_nr=10 card_label="PhoneCam" exclusive_caps=1
```

### Compiling on Linux
```bash
# Build desktop companion
cargo build --release -p phonecam-app

# Run all workspace unit tests
cargo test --workspace
```

---

## 4. macOS Building & Usage Workflows

### Prerequisites
- macOS 12.3 (Monterey) or newer (macOS 13+ Ventura / macOS 14+ Sonoma recommended).
- Xcode Command Line Tools: `xcode-select --install`
- Android platform tools: `brew install android-platform-tools`
- Rust target support:
  ```bash
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  ```

### Building for Apple Silicon & Intel
```bash
# Apple Silicon (M1 / M2 / M3 / M4)
cargo build --release --target aarch64-apple-darwin -p phonecam-app

# Intel x86_64 Macs
cargo build --release --target x86_64-apple-darwin -p phonecam-app

# Create Universal 2 binary
lipo -create -output target/release/phonecam-universal \
    target/aarch64-apple-darwin/release/phonecam \
    target/x86_64-apple-darwin/release/phonecam
```

### Virtual Camera Pipeline on macOS
- Rendering is handled natively by GPUI's Metal backend with vsync-aligned presenting.
- Virtual camera feed is integrated via CoreMediaIO DAL plugin (`/Library/CoreMediaIO/Plug-Ins/DAL/PhoneCam.plugin`) or modern Camera Extension (`CMIOExtensionProvider`).

---

## 5. Android Multi-ABI Packaging Configuration

The Android application is configured in `apps/android/app/build.gradle.kts` to build optimized per-architecture APKs alongside a universal APK.

### Target Architectures
- **`arm64-v8a`**: Primary 64-bit ARM architecture. Targeted by all modern Android smartphones released since 2016. Produces lean, high-performance binary code with full NEON vectorization.
- **`armeabi-v7a`**: Legacy 32-bit ARM architecture. Supported for older test devices and budget hardware.
- **`x86_64`**: 64-bit x86 architecture. Crucial for Android Studio emulators and ChromeOS Android subsystem testing.

### Gradle Configuration
```kotlin
android {
    defaultConfig {
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }
    }

    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a", "armeabi-v7a", "x86_64")
            isUniversalApk = true
        }
    }
}
```

### Build Outputs (`apps/android/app/build/outputs/apk/release/`)
| Artifact Name | ABI / Target | Approx. Size | Purpose |
| :--- | :--- | :--- | :--- |
| `app-arm64-v8a-release.apk` | 64-bit ARM | ~2.36 MB | Optimized for 99%+ of modern consumer smartphones |
| `app-armeabi-v7a-release.apk` | 32-bit ARM | ~2.14 MB | Optimized for legacy 32-bit hardware |
| `app-x86_64-release.apk` | 64-bit x86 | ~2.34 MB | Fast native testing in emulators & ChromeOS |
| `app-universal-release.apk` | All ABIs bundled | ~3.75 MB | Standalone sideloading, bundled in portable zip |

### Verification Commands
```powershell
# In apps/android/
.\gradlew.bat test              # JVM and native mock unit tests
.\gradlew.bat assembleRelease   # Compiles all ABI splits and universal APK
```
