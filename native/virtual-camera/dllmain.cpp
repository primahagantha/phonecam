#include "phonecam_virtual_camera.hpp"
#include "vcam_log.hpp"

#include <windows.h>
#include <shlwapi.h>
#include <string>

static HMODULE g_hModule = nullptr;

BOOL APIENTRY DllMain(HMODULE hModule, DWORD dwReason, LPVOID lpReserved) {
  (void)lpReserved;
  if (dwReason == DLL_PROCESS_ATTACH) {
    g_hModule = hModule;
    DisableThreadLibraryCalls(hModule);
    // No logging here: this runs under the loader lock, and opening a file from
    // DllMain risks deadlocking the process that is loading us.
  }
  return TRUE;
}

STDAPI DllCanUnloadNow() {
  return phonecam::vcam::CanUnloadNow() ? S_OK : S_FALSE;
}

_Check_return_
STDAPI DllGetClassObject(_In_ REFCLSID rclsid, _In_ REFIID riid, _Outptr_ LPVOID* ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;

  if (rclsid == phonecam::vcam::CLSID_PhoneCamVirtualCamera) {
    phonecam::vcam::PhoneCamClassFactory* factory = new (std::nothrow) phonecam::vcam::PhoneCamClassFactory();
    if (!factory) return E_OUTOFMEMORY;
    HRESULT hr = factory->QueryInterface(riid, ppv);
    factory->Release();
    return hr;
  }

  return CLASS_E_CLASSNOTAVAILABLE;
}

STDAPI DllGetActivate(_COM_Outptr_ IMFActivate** ppActivate) {
  if (!ppActivate) return E_POINTER;
  *ppActivate = nullptr;

  phonecam::vcam::PhoneCamActivate* activate = new (std::nothrow) phonecam::vcam::PhoneCamActivate();
  if (!activate) return E_OUTOFMEMORY;

  HRESULT hr = activate->Initialize();
  if (FAILED(hr)) {
    activate->Release();
    return hr;
  }

  *ppActivate = activate;
  return S_OK;
}

static std::wstring GetCurrentDllPath() {
  wchar_t buffer[MAX_PATH] = {0};
  if (GetModuleFileNameW(g_hModule, buffer, MAX_PATH) > 0) {
    return std::wstring(buffer);
  }
  return L"";
}

STDAPI DllRegisterServer() {
  std::wstring dllPath = GetCurrentDllPath();
  if (dllPath.empty()) return E_FAIL;

  std::wstring subKey = L"Software\\Classes\\CLSID\\";
  subKey += phonecam::vcam::kClsidVirtualCameraString;
  subKey += L"\\InprocServer32";

  // Register under HKCU (works without admin privileges)
  HKEY hKey = nullptr;
  LSTATUS status = RegCreateKeyExW(
      HKEY_CURRENT_USER, subKey.c_str(), 0, nullptr,
      REG_OPTION_NON_VOLATILE, KEY_WRITE, nullptr, &hKey, nullptr);

  if (status == ERROR_SUCCESS) {
    RegSetValueExW(hKey, nullptr, 0, REG_SZ,
                   reinterpret_cast<const BYTE*>(dllPath.c_str()),
                   static_cast<DWORD>((dllPath.size() + 1) * sizeof(wchar_t)));

    const wchar_t threadingModel[] = L"Both";
    RegSetValueExW(hKey, L"ThreadingModel", 0, REG_SZ,
                   reinterpret_cast<const BYTE*>(threadingModel),
                   sizeof(threadingModel));
    RegCloseKey(hKey);
  }

  // Also attempt HKLM if running with administrator privileges
  HKEY hKeyLM = nullptr;
  status = RegCreateKeyExW(
      HKEY_LOCAL_MACHINE, subKey.c_str(), 0, nullptr,
      REG_OPTION_NON_VOLATILE, KEY_WRITE, nullptr, &hKeyLM, nullptr);

  if (status == ERROR_SUCCESS) {
    RegSetValueExW(hKeyLM, nullptr, 0, REG_SZ,
                   reinterpret_cast<const BYTE*>(dllPath.c_str()),
                   static_cast<DWORD>((dllPath.size() + 1) * sizeof(wchar_t)));

    const wchar_t threadingModel[] = L"Both";
    RegSetValueExW(hKeyLM, L"ThreadingModel", 0, REG_SZ,
                   reinterpret_cast<const BYTE*>(threadingModel),
                   sizeof(threadingModel));
    RegCloseKey(hKeyLM);
  }

  return S_OK;
}

STDAPI DllUnregisterServer() {
  std::wstring subKey = L"Software\\Classes\\CLSID\\";
  subKey += phonecam::vcam::kClsidVirtualCameraString;

  RegDeleteTreeW(HKEY_CURRENT_USER, subKey.c_str());
  RegDeleteTreeW(HKEY_LOCAL_MACHINE, subKey.c_str());

  return S_OK;
}
