; PhoneCam NSIS Installer Script
; Modern UI 2
!include "MUI2.nsh"
!include "FileFunc.nsh"

Name "PhoneCam"
OutFile "..\dist\installer\PhoneCam-Setup-1.0.0.exe"
InstallDir "$PROGRAMFILES64\PhoneCam"
InstallDirRegKey HKLM "Software\PhoneCam" "Install_Dir"
RequestExecutionLevel admin

; UI Configuration
!define MUI_ABORTWARNING
!define MUI_ICON "..\dist\unpacked\icon.ico"
!define MUI_UNICON "..\dist\unpacked\icon.ico"

; Pages
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\phonecam.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch PhoneCam"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "PhoneCam Core (required)" SecCore
  SectionIn RO

  ; Kill any running phonecam instances before installing
  DetailPrint "Checking for running instances..."
  nsExec::Exec 'taskkill /F /IM phonecam.exe'

  SetOutPath "$INSTDIR"

  ; Install unpacked binaries and resources
  File "..\dist\unpacked\phonecam.exe"
  File "..\dist\unpacked\PhoneCamVirtualCamera.dll"
  File "..\dist\unpacked\phonecam-vcam-register.exe"
  File "..\dist\unpacked\adb.exe"
  File "..\dist\unpacked\AdbWinApi.dll"
  File "..\dist\unpacked\AdbWinUsbApi.dll"
  File "..\dist\unpacked\icon.ico"

  ; Register Virtual Camera
  DetailPrint "Registering PhoneCam virtual camera..."
  nsExec::ExecToLog '"$INSTDIR\phonecam-vcam-register.exe" --register'

  ; Write installation path into registry
  WriteRegStr HKLM "Software\PhoneCam" "Install_Dir" "$INSTDIR"

  ; Write uninstaller and Add/Remove Programs entry
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhoneCam" "DisplayName" "PhoneCam"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhoneCam" "DisplayVersion" "1.0.0"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhoneCam" "Publisher" "PhoneCam"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhoneCam" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhoneCam" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhoneCam" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhoneCam" "NoRepair" 1
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Shortcuts
  CreateDirectory "$SMPROGRAMS\PhoneCam"
  CreateShortcut "$SMPROGRAMS\PhoneCam\PhoneCam.lnk" "$INSTDIR\phonecam.exe" "" "$INSTDIR\icon.ico" 0
  CreateShortcut "$SMPROGRAMS\PhoneCam\Uninstall PhoneCam.lnk" "$INSTDIR\uninstall.exe" "" "$INSTDIR\icon.ico" 0
  CreateShortcut "$DESKTOP\PhoneCam.lnk" "$INSTDIR\phonecam.exe" "" "$INSTDIR\icon.ico" 0

SectionEnd

Section "Uninstall"
  ; Stop running phonecam
  nsExec::Exec 'taskkill /F /IM phonecam.exe'

  ; Unregister Virtual Camera
  DetailPrint "Unregistering PhoneCam virtual camera..."
  nsExec::ExecToLog '"$INSTDIR\phonecam-vcam-register.exe" --unregister'

  ; Remove shortcuts
  Delete "$DESKTOP\PhoneCam.lnk"
  Delete "$SMPROGRAMS\PhoneCam\PhoneCam.lnk"
  Delete "$SMPROGRAMS\PhoneCam\Uninstall PhoneCam.lnk"
  RMDir "$SMPROGRAMS\PhoneCam"

  ; Remove installed files
  Delete "$INSTDIR\phonecam.exe"
  Delete "$INSTDIR\PhoneCamVirtualCamera.dll"
  Delete "$INSTDIR\phonecam-vcam-register.exe"
  Delete "$INSTDIR\adb.exe"
  Delete "$INSTDIR\AdbWinApi.dll"
  Delete "$INSTDIR\AdbWinUsbApi.dll"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ; Clean registry
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhoneCam"
  DeleteRegKey HKLM "Software\PhoneCam"
SectionEnd
