@echo off
setlocal enabledelayedexpansion

echo ========================================================
echo  PhoneCam - Portable Launcher
echo ========================================================
echo.

:: Switch working directory to script directory
cd /d "%~dp0"

:: 1. Check for primary executable
if not exist "%~dp0phonecam.exe" (
    echo [ERROR] phonecam.exe not found in "%~dp0"!
    echo Please make sure all files were extracted properly.
    echo.
    pause
    exit /b 1
)

:: 2. Check for companion components
if not exist "%~dp0adb.exe" (
    echo [WARNING] adb.exe not found in "%~dp0". USB mode will require system ADB.
)

if not exist "%~dp0PhoneCamVirtualCamera.dll" (
    echo [WARNING] PhoneCamVirtualCamera.dll not found. Virtual camera feature may be unavailable.
)

:: 3. Inform about virtual camera driver if phonecam-vcam-register.exe is present
if exist "%~dp0phonecam-vcam-register.exe" (
    "%~dp0phonecam-vcam-register.exe" --status >nul 2>&1
    if !errorlevel! neq 0 (
        echo [INFO] Virtual camera driver is not yet registered on this system.
        echo To use PhoneCam in Zoom, Discord, OBS or Meet, run:
        echo   install-virtual-camera.bat (as Administrator)
        echo.
    )
)

:: 4. Add current directory to PATH so phonecam can locate bundled adb.exe seamlessly
set "PATH=%~dp0;%PATH%"

echo Launching PhoneCam...
start "" "%~dp0phonecam.exe" %*

exit /b 0
