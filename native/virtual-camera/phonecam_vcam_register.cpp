/**
 * PhoneCam Virtual Camera registration tool.
 *
 * Usage:
 *   phonecam-vcam-register.exe --register
 *   phonecam-vcam-register.exe --unregister
 *   phonecam-vcam-register.exe --status
 */

#include <windows.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mfvirtualcamera.h>
#include <wchar.h>

#include <iostream>
#include <string>
#include <vector>

#pragma comment(lib, "mfplat.lib")
#pragma comment(lib, "mfsensorgroup.lib")
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "advapi32.lib")

namespace {

inline constexpr const wchar_t* kClsidVirtualCameraString = L"{B7593649-C791-4966-9321-7B8B1CE54D38}";
inline constexpr const wchar_t* kVirtualCameraFriendlyName = L"PhoneCam Camera";

std::wstring GetModuleDir() {
  wchar_t buffer[MAX_PATH] = {0};
  if (GetModuleFileNameW(nullptr, buffer, MAX_PATH) > 0) {
    std::wstring path(buffer);
    size_t lastSlash = path.find_last_of(L"\\/");
    if (lastSlash != std::wstring::npos) {
      return path.substr(0, lastSlash);
    }
  }
  return L".";
}

std::wstring GetDllPath() {
  std::wstring dir = GetModuleDir();
  std::wstring candidate = dir + L"\\PhoneCamVirtualCamera.dll";
  DWORD attrs = GetFileAttributesW(candidate.c_str());
  if (attrs != INVALID_FILE_ATTRIBUTES && !(attrs & FILE_ATTRIBUTE_DIRECTORY)) {
    return candidate;
  }
  return candidate;
}

/**
 * True when `root\subKey` already points at `dllPath`.
 *
 * Checked before writing so that re-registering an unchanged installation does
 * not require elevation. Rebuilding the DLL in place is the common case, and
 * demanding administrator rights for a no-op write would be a needless barrier —
 * the key only has to change when the path changes or on a first install.
 */
bool ComServerKeyMatches(HKEY root, const std::wstring& subKey, const std::wstring& dllPath) {
  HKEY hKey = nullptr;
  if (RegOpenKeyExW(root, subKey.c_str(), 0, KEY_READ, &hKey) != ERROR_SUCCESS) return false;

  wchar_t existing[MAX_PATH] = {0};
  DWORD type = 0;
  DWORD size = sizeof(existing);
  const LSTATUS status = RegQueryValueExW(hKey, nullptr, nullptr, &type,
                                          reinterpret_cast<BYTE*>(existing), &size);
  RegCloseKey(hKey);

  if (status != ERROR_SUCCESS || type != REG_SZ) return false;

  // Case-insensitive: Windows paths are, and the user may have typed the path
  // with different capitalisation than GetModuleFileNameW returned.
  return _wcsicmp(existing, dllPath.c_str()) == 0;
}

bool WriteComServerKey(HKEY root, const std::wstring& subKey, const std::wstring& dllPath) {
  HKEY hKey = nullptr;
  LSTATUS status = RegCreateKeyExW(
      root, subKey.c_str(), 0, nullptr,
      REG_OPTION_NON_VOLATILE, KEY_WRITE, nullptr, &hKey, nullptr);

  if (status != ERROR_SUCCESS) return false;

  status = RegSetValueExW(hKey, nullptr, 0, REG_SZ,
                          reinterpret_cast<const BYTE*>(dllPath.c_str()),
                          static_cast<DWORD>((dllPath.size() + 1) * sizeof(wchar_t)));
  if (status != ERROR_SUCCESS) {
    RegCloseKey(hKey);
    return false;
  }

  const wchar_t threadingModel[] = L"Both";
  RegSetValueExW(hKey, L"ThreadingModel", 0, REG_SZ,
                 reinterpret_cast<const BYTE*>(threadingModel),
                 sizeof(threadingModel));

  RegCloseKey(hKey);
  return true;
}

bool RegisterComServer(const std::wstring& dllPath) {
  std::wstring subKey = L"Software\\Classes\\CLSID\\";
  subKey += kClsidVirtualCameraString;
  subKey += L"\\InprocServer32";

  // HKLM is the one that matters: the Frame Server runs as NT AUTHORITY\LocalService
  // in its own process, so it only resolves COM classes from the machine store.
  // A per-user HKCU registration is invisible to it.
  if (ComServerKeyMatches(HKEY_LOCAL_MACHINE, subKey, dllPath)) {
    std::wcout << L"COM server is already registered machine-wide for this DLL.\n";
    return true;
  }

  if (!WriteComServerKey(HKEY_LOCAL_MACHINE, subKey, dllPath)) {
    // Distinguish "the machine key points at a different DLL" from "there is no
    // key at all". Both surface as an access-denied write, but only the first
    // one is a normal, non-elevated situation: the caller is running a build-tree
    // binary while an installed copy owns the registration. Saying
    // "re-run as administrator" there sends the user to a prompt that would
    // repoint the registration at a developer build, which is not what they want.
    std::wstring existing;
    HKEY hKey = nullptr;
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, subKey.c_str(), 0, KEY_READ, &hKey) == ERROR_SUCCESS) {
      wchar_t value[MAX_PATH] = {0};
      DWORD type = 0;
      DWORD size = sizeof(value);
      if (RegQueryValueExW(hKey, nullptr, nullptr, &type,
                           reinterpret_cast<BYTE*>(value), &size) == ERROR_SUCCESS &&
          type == REG_SZ) {
        existing = value;
      }
      RegCloseKey(hKey);
    }

    if (!existing.empty()) {
      std::wcerr << L"ERROR: the machine-wide registration already points at a different DLL:\n"
                 << L"         " << existing << L"\n"
                 << L"         This tool is trying to register:\n"
                 << L"         " << dllPath << L"\n"
                 << L"To change which copy owns the camera, run this tool as administrator.\n"
                 << L"To repair the installed camera instead, run the copy that sits next to\n"
                 << L"that DLL:\n"
                 << L"         \"" << existing.substr(0, existing.find_last_of(L"\\/"))
                 << L"\\phonecam-vcam-register.exe\" --register\n";
      return false;
    }

    std::wcerr << L"ERROR: could not write HKEY_LOCAL_MACHINE\\" << subKey << L".\n"
               << L"The Frame Server runs as a service and can only load COM servers\n"
               << L"registered machine-wide. Re-run this tool as administrator.\n";
    return false;
  }

  // Kept as a convenience for tools that resolve the class in the user's context.
  WriteComServerKey(HKEY_CURRENT_USER, subKey, dllPath);
  return true;
}

bool UnregisterComServer() {
  std::wstring subKey = L"Software\\Classes\\CLSID\\";
  subKey += kClsidVirtualCameraString;

  // The machine-wide CLSID key is deliberately left in place.
  //
  // Removing it is what broke the camera. Disabling the virtual camera is a
  // routine, non-elevated action — the core sends `set_vcam: false` when the
  // stream stops and the toggle is off — but deleting HKLM requires elevation,
  // so the next enable could not put it back and failed with "Windows needs
  // permission to register the camera". Turning a feature off must not destroy
  // the ability to turn it back on.
  //
  // Leaving the key is also harmless: the key alone does not make a camera
  // appear. What applications enumerate is the device entry, which the caller
  // removes through `IMFVirtualCamera::Remove`. `--status` now reports
  // enumerability rather than the key, so a key left behind cannot be mistaken
  // for a working camera.
  RegDeleteTreeW(HKEY_CURRENT_USER, subKey.c_str());
  return true;
}

bool CheckComServerRegistered() {
  std::wstring subKey = L"Software\\Classes\\CLSID\\";
  subKey += kClsidVirtualCameraString;
  subKey += L"\\InprocServer32";

  // Only the machine-wide key is loadable by the Frame Server.
  HKEY hKey = nullptr;
  if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, subKey.c_str(), 0, KEY_READ, &hKey) == ERROR_SUCCESS) {
    RegCloseKey(hKey);
    return true;
  }
  return false;
}

/**
 * True when a capture device with our friendly name is actually enumerable.
 *
 * The COM key alone is not proof that the camera exists. Registration is two
 * separate things: the machine-wide CLSID key, and a device entry created by
 * `MFCreateVirtualCamera` + `Start`. `--unregister` removes the device entry
 * (and the core calls it on every stream stop when the toggle is off) while the
 * CLSID key can survive, so a key-only check reported "PRESENT" for a camera
 * that OBS, NVIDIA Broadcast and even our own probe could not see. The core then
 * read that answer as "already in the desired state" and never re-registered,
 * which is exactly how the camera went missing.
 *
 * Enumerating devices is the only check that answers the question that matters:
 * can an application open this camera right now?
 */
bool DeviceIsEnumerable() {
  HRESULT hr = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
  if (FAILED(hr)) return false;

  IMFAttributes* attributes = nullptr;
  bool found = false;

  hr = MFCreateAttributes(&attributes, 1);
  if (SUCCEEDED(hr) && attributes != nullptr) {
    attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                        MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);

    IMFActivate** devices = nullptr;
    UINT32 count = 0;
    hr = MFEnumDeviceSources(attributes, &devices, &count);
    if (SUCCEEDED(hr) && devices != nullptr) {
      for (UINT32 i = 0; i < count; ++i) {
        wchar_t name[256] = {0};
        UINT32 length = 0;
        if (SUCCEEDED(devices[i]->GetString(
                MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, name, 256, &length))) {
          // The device name carries a suffix in some Windows builds
          // ("PhoneCam Camera (Windows Virtual Camera)"), so match on the stem.
          if (wcsstr(name, kVirtualCameraFriendlyName) != nullptr) found = true;
        }
        devices[i]->Release();
        if (found) break;
      }
      CoTaskMemFree(devices);
    }
    attributes->Release();
  }

  MFShutdown();
  return found;
}

int DoRegister() {
  std::wstring dllPath = GetDllPath();
  std::wcout << L"Registering COM in-proc server for " << dllPath << L" ...\n";
  if (!RegisterComServer(dllPath)) {
    std::wcerr << L"ERROR: Failed to register COM server in registry.\n";
    return 1;
  }
  std::wcout << L"COM server registered machine-wide (HKLM).\n";

  HRESULT hr = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
  if (FAILED(hr)) {
    std::wcerr << L"ERROR: MFStartup failed: 0x" << std::hex << hr << L"\n";
    return 1;
  }

  BOOL supported = FALSE;
  hr = MFIsVirtualCameraTypeSupported(MFVirtualCameraType_SoftwareCameraSource, &supported);
  if (FAILED(hr) || !supported) {
    std::wcerr << L"ERROR: Virtual camera type not supported on this Windows build: 0x"
               << std::hex << hr << L"\n";
    MFShutdown();
    return 1;
  }

  // A surviving CLSID key does not mean the device exists: `--unregister` can
  // remove the device entry while the key stays, and the device is the thing
  // applications enumerate. Creating it is idempotent, so it is always done
  // rather than skipped on the strength of the key alone.
  IMFVirtualCamera* pVirtualCamera = nullptr;
  hr = MFCreateVirtualCamera(
      MFVirtualCameraType_SoftwareCameraSource,
      MFVirtualCameraLifetime_System,
      MFVirtualCameraAccess_CurrentUser,
      kVirtualCameraFriendlyName,
      kClsidVirtualCameraString,
      nullptr,
      0,
      &pVirtualCamera);

  if (FAILED(hr)) {
    std::wcerr << L"ERROR: MFCreateVirtualCamera failed: 0x" << std::hex << hr << L"\n";
    MFShutdown();
    return 1;
  }

  hr = pVirtualCamera->Start(nullptr);
  if (FAILED(hr)) {
    std::wcerr << L"ERROR: IMFVirtualCamera::Start failed: 0x" << std::hex << hr << L"\n";
    pVirtualCamera->Release();
    MFShutdown();
    return 1;
  }

  pVirtualCamera->Release();
  MFShutdown();

  if (DeviceIsEnumerable()) {
    std::wcout << L"Successfully registered '" << kVirtualCameraFriendlyName
               << L"' (device is enumerable).\n";
  } else {
    std::wcerr << L"Registered '" << kVirtualCameraFriendlyName
               << L"', but the device is still not enumerable. Restart the app that\n"
               << L"was looking for it (OBS, NVIDIA Broadcast) so it re-enumerates.\n";
  }
  return 0;
}

int DoUnregister() {
  std::wcout << L"Unregistering '" << kVirtualCameraFriendlyName << L"' ...\n";

  HRESULT hr = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
  if (SUCCEEDED(hr)) {
    IMFVirtualCamera* pVirtualCamera = nullptr;
    hr = MFCreateVirtualCamera(
        MFVirtualCameraType_SoftwareCameraSource,
        MFVirtualCameraLifetime_System,
        MFVirtualCameraAccess_CurrentUser,
        kVirtualCameraFriendlyName,
        kClsidVirtualCameraString,
        nullptr,
        0,
        &pVirtualCamera);

    if (SUCCEEDED(hr) && pVirtualCamera != nullptr) {
      pVirtualCamera->Stop();
      pVirtualCamera->Remove();
      pVirtualCamera->Release();
    }
    MFShutdown();
  }

  UnregisterComServer();
  std::wcout << L"Successfully unregistered '" << kVirtualCameraFriendlyName << L"'.\n";
  return 0;
}

int DoStatus() {
  std::wcout << L"Virtual camera status check for '" << kVirtualCameraFriendlyName << L"':\n";
  const bool comOk = CheckComServerRegistered();
  std::wcout << L"  COM registration: " << (comOk ? L"PRESENT" : L"NOT FOUND") << L"\n";

  HRESULT hr = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
  if (SUCCEEDED(hr)) {
    BOOL supported = FALSE;
    MFIsVirtualCameraTypeSupported(MFVirtualCameraType_SoftwareCameraSource, &supported);
    std::wcout << L"  MF Software Camera Source support: " << (supported ? L"SUPPORTED" : L"UNSUPPORTED") << L"\n";
    MFShutdown();
  }

  // The device entry is what applications enumerate. Reporting only the CLSID
  // key let this tool answer "PRESENT" for a camera nothing could open.
  const bool enumerable = DeviceIsEnumerable();
  std::wcout << L"  Device enumerable: " << (enumerable ? L"YES" : L"NO") << L"\n";

  // Exit code follows the device, because the caller is asking "can an app open
  // this camera", not "is there a registry key".
  return enumerable ? 0 : 1;
}

}  // namespace

int main(int argc, char** argv) {
  if (argc < 2) {
    std::cout << "PhoneCam Virtual Camera Registrar\n"
                 "Usage:\n"
                 "  phonecam-vcam-register.exe --register\n"
                 "  phonecam-vcam-register.exe --unregister\n"
                 "  phonecam-vcam-register.exe --status\n";
    return 0;
  }

  std::string arg = argv[1];
  if (arg == "--register" || arg == "-r") {
    return DoRegister();
  } else if (arg == "--unregister" || arg == "-u") {
    return DoUnregister();
  } else if (arg == "--status" || arg == "-s") {
    return DoStatus();
  } else if (arg == "--help" || arg == "-h") {
    std::cout << "Usage: phonecam-vcam-register.exe [--register | --unregister | --status]\n";
    return 0;
  }

  std::cerr << "Unknown argument: " << arg << "\n";
  return 1;
}
