========================================================================
                     PHONECAM v1.4.0-nightly.1
  Free, Open-Source Android-to-PC Webcam Bridge in Native Rust + GPUI
========================================================================

CONTENTS OF THIS PACKAGE:
-------------------------
1. phonecam.exe                  - Native Rust desktop app (GPUI + openh264)
2. run-portable.bat              - Portable launcher (checks vcam + sets ADB PATH)
3. PhoneCam.apk                  - Android Camera2 + MediaCodec H.264 client app
4. PhoneCamVirtualCamera.dll     - Prebuilt Windows Media Foundation vcam DLL
5. phonecam-vcam-register.exe    - Virtual camera registration helper
6. install-virtual-camera.bat    - Register Virtual Camera (Run as Administrator)
7. uninstall-virtual-camera.bat  - Unregister Virtual Camera (Run as Administrator)
8. adb.exe / AdbWinApi*.dll      - Bundled ADB tools for USB cable mode

QUICK SETUP GUIDE:
------------------
Step 1: Register the Windows Virtual Camera (One-Time Setup)
  - Right-click `install-virtual-camera.bat` and choose "Run as administrator".
  - This registers "PhoneCam Camera" via Windows Media Foundation so OBS,
    Zoom, Discord, Teams, and browsers can select it.

Step 2: Install PhoneCam on your Android phone
  - Copy `PhoneCam.apk` to your phone and install it (allow "Install unknown apps"),
    OR connect via USB with USB Debugging enabled and click "Install app on phone"
    in the PhoneCam desktop Setup tab.
  - Note: If `PhoneCam.apk` is missing or empty in a dev build, download the latest
    APK from https://github.com/primahagantha/phonecam/releases

Step 3: Allow Firewall Ports (Wi-Fi Mode) & Connect
  - USB: Plug in your phone with USB Debugging ON. PhoneCam runs
    `adb reverse tcp:48721 tcp:48721` automatically.
  - Wi-Fi: Ensure PC and phone are on the same LAN. If Windows Defender Firewall
    prompts for `phonecam.exe`, allow Private networks (TCP 48721 & UDP 48720).
    Scan the QR code in the desktop app from the Android app.

SYSTEM REQUIREMENTS & CURRENT STATUS:
-------------------------------------
* Windows:
  - Windows 11 (Build 22621 / 22H2 or newer, x64) is required for the
    Windows Media Foundation Virtual Camera (`MFCreateVirtualCamera`).
  - Windows 10 (Build 17763+, x64) can run the desktop preview window on
    DirectX 11, but virtual camera output (DirectShow) is on the Roadmap.
* GPU: DirectX 11 / Direct3D 11 compatible graphics adapter.
* Android: Android 8.0 (Oreo, API 26) or newer.
* Roadmap: Windows 10 DirectShow, Linux v4l2loopback writer (experimental),
  macOS CoreMediaIO, Virtual Microphone audio routing, and iOS.
========================================================================
