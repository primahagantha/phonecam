@echo off
echo ========================================================
echo  PhoneCam - Unregister Windows Virtual Camera Driver
echo ========================================================
echo.
net session >nul 2>&1
if %errorLevel% neq 0 (
    echo [ERROR] This script requires Administrator privileges.
    echo Please right-click this .bat file and select "Run as administrator".
    pause
    exit /b 1
)

echo Unregistering PhoneCam Virtual Camera...
"%~dp0phonecam-vcam-register.exe" --unregister

echo Done!
pause
