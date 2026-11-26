/**
 * PhoneCam virtual camera probe.
 *
 * Enumerates the system's video capture devices, then activates the PhoneCam
 * virtual camera and reads frames through a source reader — exactly the path
 * OBS, Chrome or the Windows Camera app take.
 *
 * This is the end-to-end check for the virtual camera: registration, Frame
 * Server activation, stream negotiation and sample delivery. `--self-test`
 * covers the protocol and `e2e-protocol.mjs` covers the transport, but neither
 * touches Media Foundation.
 *
 * Usage:
 *   phonecam-vcam-probe                 list devices, then read 5 frames
 *   phonecam-vcam-probe --list          list devices only
 *   phonecam-vcam-probe --frames 60     read a specific number of frames
 *
 * Exit codes: 0 frames delivered, 1 camera missing or activation failed,
 * 2 the camera activated but produced no frames.
 */

#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mferror.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cwchar>

#pragma comment(lib, "mfplat.lib")
#pragma comment(lib, "mf.lib")
#pragma comment(lib, "mfreadwrite.lib")
#pragma comment(lib, "mfuuid.lib")
#pragma comment(lib, "ole32.lib")

namespace {

constexpr const wchar_t* kDeviceNameMatch = L"PhoneCam";

void PrintHr(const char* what, HRESULT hr) {
  std::printf("  %-42s 0x%08lX\n", what, static_cast<unsigned long>(hr));
}

/** Reads an optional integer command-line argument. */
int ArgumentValue(int argc, char** argv, const char* name, int fallback) {
  for (int i = 1; i + 1 < argc; ++i) {
    if (std::strcmp(argv[i], name) == 0) return std::atoi(argv[i + 1]);
  }
  return fallback;
}

bool HasFlag(int argc, char** argv, const char* name) {
  for (int i = 1; i < argc; ++i) {
    if (std::strcmp(argv[i], name) == 0) return true;
  }
  return false;
}

}  // namespace

int main(int argc, char** argv) {
  const int wantedFrames = ArgumentValue(argc, argv, "--frames", 5);
  const bool listOnly = HasFlag(argc, argv, "--list");

  HRESULT hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  if (FAILED(hr)) {
    std::printf("CoInitializeEx failed: 0x%08lX\n", static_cast<unsigned long>(hr));
    return 1;
  }

  hr = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
  if (FAILED(hr)) {
    std::printf("MFStartup failed: 0x%08lX\n", static_cast<unsigned long>(hr));
    CoUninitialize();
    return 1;
  }

  IMFAttributes* attributes = nullptr;
  hr = MFCreateAttributes(&attributes, 1);
  if (FAILED(hr)) {
    MFShutdown();
    CoUninitialize();
    return 1;
  }
  attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                      MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);

  IMFActivate** devices = nullptr;
  UINT32 deviceCount = 0;
  hr = MFEnumDeviceSources(attributes, &devices, &deviceCount);
  attributes->Release();

  if (FAILED(hr)) {
    std::printf("MFEnumDeviceSources failed: 0x%08lX\n", static_cast<unsigned long>(hr));
    MFShutdown();
    CoUninitialize();
    return 1;
  }

  std::printf("Video capture devices: %u\n", deviceCount);
  IMFActivate* target = nullptr;
  for (UINT32 i = 0; i < deviceCount; ++i) {
    WCHAR* name = nullptr;
    UINT32 nameLength = 0;
    devices[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &name, &nameLength);
    std::printf("  [%u] %ls\n", i, name ? name : L"<unnamed>");
    if (name != nullptr && std::wcsstr(name, kDeviceNameMatch) != nullptr && target == nullptr) {
      target = devices[i];
      target->AddRef();
    }
    CoTaskMemFree(name);
  }

  int exitCode = 0;
  if (target == nullptr) {
    std::printf("\nRESULT: no PhoneCam camera found.\n"
                "Register it first: phonecam-vcam-register.exe --register (elevated).\n");
    exitCode = 1;
  } else if (listOnly) {
    std::printf("\nRESULT: PhoneCam camera present.\n");
  } else {
    std::printf("\nActivating PhoneCam Camera...\n");

    IMFMediaSource* source = nullptr;
    hr = target->ActivateObject(IID_PPV_ARGS(&source));
    PrintHr("ActivateObject", hr);

    IMFSourceReader* reader = nullptr;
    if (SUCCEEDED(hr)) {
      hr = MFCreateSourceReaderFromMediaSource(source, nullptr, &reader);
      PrintHr("MFCreateSourceReaderFromMediaSource", hr);
    }

    if (SUCCEEDED(hr)) {
      IMFMediaType* outputType = nullptr;
      MFCreateMediaType(&outputType);
      outputType->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
      outputType->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12);
      hr = reader->SetCurrentMediaType(static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM),
                                       nullptr, outputType);
      PrintHr("SetCurrentMediaType(NV12)", hr);
      outputType->Release();
    }

    int frames = 0;
    if (SUCCEEDED(hr)) {
      // The Frame Server may take a few ticks to produce the first sample, so
      // allow more reads than frames requested before giving up.
      const int maxReads = wantedFrames * 6 + 30;
      for (int i = 0; i < maxReads && frames < wantedFrames; ++i) {
        DWORD streamIndex = 0;
        DWORD flags = 0;
        LONGLONG timestamp = 0;
        IMFSample* sample = nullptr;

        hr = reader->ReadSample(static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM), 0,
                                &streamIndex, &flags, &timestamp, &sample);
        if (FAILED(hr)) {
          std::printf("  ReadSample failed: 0x%08lX\n", static_cast<unsigned long>(hr));
          break;
        }
        if ((flags & MF_SOURCE_READERF_ENDOFSTREAM) != 0) {
          std::printf("  end of stream\n");
          break;
        }
        if (sample == nullptr) continue;  // stream tick

        DWORD length = 0;
        long long meanLuma = -1;
        IMFMediaBuffer* buffer = nullptr;
        if (SUCCEEDED(sample->GetBufferByIndex(0, &buffer)) && buffer != nullptr) {
          buffer->GetCurrentLength(&length);
          // Ground-truth latency needs content, not timestamps: the reader
          // re-stamps samples on its own clock, so wall-vs-ts says nothing.
          // Mean luma of the Y plane turns a torch toggle into a visible
          // step; the wall ms at which that step arrives in the consumer is
          // the end-to-end delay.
          BYTE* data = nullptr;
          DWORD maxLen = 0, curLen = 0;
          if (SUCCEEDED(buffer->Lock(&data, &maxLen, &curLen)) && data != nullptr) {
            if (curLen >= 2 * 1024) {  // enough bytes for a sampled mean
              long long sum = 0;
              long long n = 0;
              for (DWORD p = 0; p + 1 < curLen; p += 16) {
                sum += data[p];
                ++n;
              }
              meanLuma = n > 0 ? sum / n : -1;
            }
            buffer->Unlock();
          }
          buffer->Release();
        }
        sample->Release();

        ++frames;
        // Absolute wall clock so the number can be subtracted directly from a
        // control-command timestamp taken in another process (drive.mjs prints
        // Date.now() when it sends set_torch).
        FILETIME ft = {};
        GetSystemTimeAsFileTime(&ft);
        const ULARGE_INTEGER epoch100ns = {{ft.dwLowDateTime, ft.dwHighDateTime}};
        const long long epochMs =
            static_cast<long long>((epoch100ns.QuadPart - 116444736000000000ULL) / 10000);
        std::printf("  frame %d: epoch_ms=%lld luma=%lld bytes=%lu\n", frames, epochMs,
                    meanLuma, static_cast<unsigned long>(length));
      }
    }

    if (frames > 0) {
      std::printf("\nRESULT: %d frame(s) delivered.\n", frames);
    } else {
      std::printf("\nRESULT: camera activated but delivered no frames.\n");
      exitCode = 2;
    }

    if (reader != nullptr) reader->Release();
    if (source != nullptr) source->Release();
  }

  if (target != nullptr) target->Release();
  for (UINT32 i = 0; i < deviceCount; ++i) devices[i]->Release();
  CoTaskMemFree(devices);
  MFShutdown();
  CoUninitialize();
  return exitCode;
}
