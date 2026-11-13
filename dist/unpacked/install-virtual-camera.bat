@echo off
echo ========================================================
echo  PhoneCam - Register Windows Virtual Camera Driver
echo ========================================================
echo.
echo Requesting administrator privileges...
net session >nul 2>&1
if %errorLevel% neq 0 (
    echo [ERROR] This script requires Administrator privileges.
    echo Please right-click this .bat file and select "Run as administrator".
    pause
    exit /b 1
)

echo Registering PhoneCam Virtual Camera...
"%~dp0phonecam-vcam-register.exe" --register

echo.
echo Checking Virtual Camera status...
"%~dp0phonecam-vcam-register.exe" --status

echo.
echo Done! PhoneCam Camera is now available in OBS, Discord, Zoom, and Windows Apps.
pause
