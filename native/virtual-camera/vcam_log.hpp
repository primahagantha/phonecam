/**
 * Diagnostics for the virtual camera DLL.
 *
 * The DLL is loaded in-process by the Frame Server, so there is no console to
 * write to and stdout would be lost. Logging is therefore off by default and
 * enabled by setting PHONECAM_VCAM_LOG to a file path; without it every call
 * below is a single atomic load and a branch.
 *
 * This exists because failures here are otherwise silent: when the Frame Server
 * cannot activate the source, the only external symptom is an HRESULT from an
 * API call in another process.
 */

#pragma once

#include <windows.h>
#include <share.h>

#include <cstdio>
#include <cstring>

namespace phonecam::vcam {

inline void VcamLog(const char* message) {
  // Opened with deny-none sharing: the Frame Server keeps the DLL loaded for as
  // long as the camera is in use, so a log that could not be read while held
  // would be unreadable exactly when it matters.
  static FILE* sink = []() -> FILE* {
    wchar_t path[MAX_PATH] = {0};
    const DWORD len = GetEnvironmentVariableW(L"PHONECAM_VCAM_LOG", path, MAX_PATH);
    if (len == 0 || len >= MAX_PATH) return nullptr;
    return _wfsopen(path, L"a", _SH_DENYNO);
  }();

  if (sink == nullptr) return;

  SYSTEMTIME now;
  GetLocalTime(&now);
  std::fprintf(sink, "%02u:%02u:%02u.%03u [pid=%lu] %s\n",
               now.wHour, now.wMinute, now.wSecond, now.wMilliseconds,
               GetCurrentProcessId(), message);
  std::fflush(sink);
}

/** Logs an HRESULT-bearing operation, naming the call and the result. */
inline void VcamLogHr(const char* what, HRESULT hr) {
  char buffer[256];
  std::snprintf(buffer, sizeof(buffer), "%s -> 0x%08lX", what, static_cast<unsigned long>(hr));
  VcamLog(buffer);
}

}  // namespace phonecam::vcam
