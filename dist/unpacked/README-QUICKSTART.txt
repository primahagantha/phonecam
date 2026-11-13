========================================================================
                         PHONECAM v1.0.0 (RELEASE)
  High-Performance, Ultra-Low Latency Webcam Bridge (Android to PC)
========================================================================

CONTENTS OF THIS PACKAGE:
-------------------------
1. phonecam.exe                  - Native Rust Desktop companion (GPUI + openh264)
2. run-portable.bat              - Portable launcher with directory and dependency checks
3. PhoneCam.apk                  - Android Camera2 + Hardware H.264 client app
4. PhoneCamVirtualCamera.dll     - DirectShow / Media Foundation Virtual Camera Driver
5. phonecam-vcam-register.exe    - Virtual Camera driver registration utility
6. install-virtual-camera.bat    - 1-Click Administrator script to register Virtual Camera
7. uninstall-virtual-camera.bat  - 1-Click Administrator script to unregister Virtual Camera
8. adb.exe / AdbWinApi*.dll      - Bundled Android Debug Bridge tools for USB transport

QUICK SETUP GUIDE:
------------------
Step 1: Install PhoneCam on your Android phone
  - Copy `PhoneCam.apk` to your phone and install it (enable "Install unknown apps").
  - Or connect via USB with USB Debugging enabled, and open phonecam.exe:
    Click "Install app on phone" in the Setup tab.

Step 2: Register Virtual Camera on Windows (One-Time Setup)
  - Right-click `install-virtual-camera.bat` and choose "Run as administrator".
  - This registers the virtual camera so OBS, Discord, Zoom, Meet see "PhoneCam Camera".

Step 3: Connect & Stream
  - WI-FI: Ensure PC and phone are on the same Wi-Fi network. Open PhoneCam on phone,
    tap "Scan QR Code" or enter the IP/Port shown in the desktop app.
  - USB: Connect phone via USB cable with USB Debugging turned ON.
    The desktop app will automatically create a reverse tunnel and launch the session.

SYSTEM REQUIREMENTS & COMPATIBILITY:
------------------------------------
* Windows:
  - Windows 11 (Build 22621+ recommended for native Media Foundation Virtual Camera).
  - Windows 10 (Version 1809 / Build 17763 or newer):
    Desktop companion app runs with 100% full feature set (low-latency preview, audio,
    control, USB/Wi-Fi). Virtual camera driver uses COM / DirectShow registration.
* GPU: DirectX 11 / Direct3D 11 compatible graphics card.
* Android: Android 8.0 (Oreo) or newer (API 26+).
* Linux (Arch, Debian, Ubuntu, Fedora):
  - Supported via native Linux build and v4l2loopback kernel module.
========================================================================
