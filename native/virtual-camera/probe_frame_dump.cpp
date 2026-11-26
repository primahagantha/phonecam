/**
 * Saves one frame from the PhoneCam virtual camera as a PNG.
 *
 * Used to check what a consumer actually sees — specifically that the
 * disconnected placeholder carries readable text rather than a blank rectangle.
 * The NV12 frame is converted to RGB and written as a minimal PNG so the result
 * can be inspected as an image.
 */

#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cwchar>
#include <string>
#include <vector>

#pragma comment(lib, "mfplat.lib")
#pragma comment(lib, "mf.lib")
#pragma comment(lib, "mfreadwrite.lib")
#pragma comment(lib, "mfuuid.lib")
#pragma comment(lib, "ole32.lib")

namespace {

void WriteBigEndian32(std::vector<uint8_t>& out, uint32_t value) {
  out.push_back(static_cast<uint8_t>((value >> 24) & 0xFF));
  out.push_back(static_cast<uint8_t>((value >> 16) & 0xFF));
  out.push_back(static_cast<uint8_t>((value >> 8) & 0xFF));
  out.push_back(static_cast<uint8_t>(value & 0xFF));
}

uint32_t Crc32(const uint8_t* data, size_t length) {
  static uint32_t table[256];
  static bool ready = false;
  if (!ready) {
    for (uint32_t i = 0; i < 256; ++i) {
      uint32_t c = i;
      for (int k = 0; k < 8; ++k) c = (c & 1) ? (0xEDB88320u ^ (c >> 1)) : (c >> 1);
      table[i] = c;
    }
    ready = true;
  }
  uint32_t crc = 0xFFFFFFFFu;
  for (size_t i = 0; i < length; ++i) crc = table[(crc ^ data[i]) & 0xFF] ^ (crc >> 8);
  return crc ^ 0xFFFFFFFFu;
}

void WriteChunk(std::vector<uint8_t>& out, const char type[4], const std::vector<uint8_t>& body) {
  WriteBigEndian32(out, static_cast<uint32_t>(body.size()));
  const size_t crcStart = out.size();
  out.insert(out.end(), type, type + 4);
  out.insert(out.end(), body.begin(), body.end());
  const uint32_t crc = Crc32(out.data() + crcStart, out.size() - crcStart);
  WriteBigEndian32(out, crc);
}

/** Writes an RGB24 buffer as a PNG, using stored (uncompressed) deflate blocks. */
bool WritePng(const char* path, const std::vector<uint8_t>& rgb, uint32_t width, uint32_t height) {
  // Raw scanlines, each prefixed with filter type 0.
  std::vector<uint8_t> raw;
  raw.reserve(static_cast<size_t>(height) * (1 + width * 3));
  for (uint32_t y = 0; y < height; ++y) {
    raw.push_back(0);
    const uint8_t* row = rgb.data() + static_cast<size_t>(y) * width * 3;
    raw.insert(raw.end(), row, row + static_cast<size_t>(width) * 3);
  }

  // zlib stream: header, stored deflate blocks, Adler-32.
  std::vector<uint8_t> z;
  z.push_back(0x78);
  z.push_back(0x01);
  const size_t blockMax = 65535;
  for (size_t offset = 0; offset < raw.size(); offset += blockMax) {
    const size_t length = raw.size() - offset < blockMax ? raw.size() - offset : blockMax;
    const bool last = offset + length >= raw.size();
    z.push_back(last ? 1 : 0);
    z.push_back(static_cast<uint8_t>(length & 0xFF));
    z.push_back(static_cast<uint8_t>((length >> 8) & 0xFF));
    z.push_back(static_cast<uint8_t>(~length & 0xFF));
    z.push_back(static_cast<uint8_t>((~length >> 8) & 0xFF));
    z.insert(z.end(), raw.begin() + offset, raw.begin() + offset + length);
  }
  uint32_t a = 1, b = 0;
  for (uint8_t byte : raw) {
    a = (a + byte) % 65521;
    b = (b + a) % 65521;
  }
  WriteBigEndian32(z, (b << 16) | a);

  std::vector<uint8_t> png = {0x89, 'P', 'N', 'G', 0x0D, 0x0A, 0x1A, 0x0A};
  std::vector<uint8_t> ihdr;
  WriteBigEndian32(ihdr, width);
  WriteBigEndian32(ihdr, height);
  ihdr.push_back(8);   // bit depth
  ihdr.push_back(2);   // colour type: truecolour
  ihdr.push_back(0);
  ihdr.push_back(0);
  ihdr.push_back(0);
  WriteChunk(png, "IHDR", ihdr);
  WriteChunk(png, "IDAT", z);
  WriteChunk(png, "IEND", {});

  FILE* file = nullptr;
  if (fopen_s(&file, path, "wb") != 0 || file == nullptr) return false;
  fwrite(png.data(), 1, png.size(), file);
  fclose(file);
  return true;
}

uint8_t Clamp(double value) {
  if (value < 0.0) return 0;
  if (value > 255.0) return 255;
  return static_cast<uint8_t>(value);
}

/** Converts one NV12 frame to RGB24 using the Rec.601 matrix the encoder assumes. */
std::vector<uint8_t> Nv12ToRgb(const std::vector<uint8_t>& nv12, uint32_t width, uint32_t height) {
  std::vector<uint8_t> rgb(static_cast<size_t>(width) * height * 3);
  const uint8_t* yPlane = nv12.data();
  const uint8_t* uvPlane = nv12.data() + static_cast<size_t>(width) * height;

  for (uint32_t y = 0; y < height; ++y) {
    for (uint32_t x = 0; x < width; ++x) {
      const double Y = yPlane[static_cast<size_t>(y) * width + x];
      const size_t uvIndex = static_cast<size_t>(y / 2) * width + (x / 2) * 2;
      const double U = uvPlane[uvIndex] - 128.0;
      const double V = uvPlane[uvIndex + 1] - 128.0;

      const size_t out = (static_cast<size_t>(y) * width + x) * 3;
      rgb[out + 0] = Clamp(Y + 1.402 * V);
      rgb[out + 1] = Clamp(Y - 0.344136 * U - 0.714136 * V);
      rgb[out + 2] = Clamp(Y + 1.772 * U);
    }
  }
  return rgb;
}

}  // namespace

int main(int argc, char** argv) {
  const char* outputPath = argc > 1 ? argv[1] : "frame.png";

  // Optional sample count. The Frame Server delivers whatever the source has
  // when a consumer attaches, and a source that has only just been activated
  // can hand over the placeholder as its first sample even though live frames
  // follow a moment later. Reading several and keeping the last one is what
  // makes this tool answer "what does the camera show *now*" rather than "what
  // did it show in the first millisecond".
  int wantedSamples = 1;
  if (argc > 2) {
    const int parsed = std::atoi(argv[2]);
    if (parsed > 0 && parsed <= 600) wantedSamples = parsed;
  }

  HRESULT hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  if (FAILED(hr)) { std::printf("CoInitializeEx failed\n"); return 1; }
  hr = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
  if (FAILED(hr)) { std::printf("MFStartup failed: 0x%08lX\n", static_cast<unsigned long>(hr)); return 1; }

  IMFAttributes* attributes = nullptr;
  MFCreateAttributes(&attributes, 1);
  attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);

  IMFActivate** devices = nullptr;
  UINT32 count = 0;
  hr = MFEnumDeviceSources(attributes, &devices, &count);
  attributes->Release();
  if (FAILED(hr)) { std::printf("enumeration failed\n"); return 1; }

  IMFActivate* target = nullptr;
  for (UINT32 i = 0; i < count; ++i) {
    WCHAR* name = nullptr;
    UINT32 nameLength = 0;
    devices[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &name, &nameLength);
    if (name != nullptr && std::wcsstr(name, L"PhoneCam") != nullptr && target == nullptr) {
      target = devices[i];
      target->AddRef();
    }
    CoTaskMemFree(name);
  }
  if (target == nullptr) { std::printf("PhoneCam camera not found\n"); return 1; }

  IMFMediaSource* source = nullptr;
  hr = target->ActivateObject(IID_PPV_ARGS(&source));
  if (FAILED(hr)) { std::printf("activate failed: 0x%08lX\n", static_cast<unsigned long>(hr)); return 1; }

  IMFSourceReader* reader = nullptr;
  hr = MFCreateSourceReaderFromMediaSource(source, nullptr, &reader);
  if (FAILED(hr)) { std::printf("reader failed: 0x%08lX\n", static_cast<unsigned long>(hr)); return 1; }

  IMFMediaType* outputType = nullptr;
  MFCreateMediaType(&outputType);
  outputType->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
  outputType->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12);
  reader->SetCurrentMediaType(static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM), nullptr, outputType);
  outputType->Release();

  std::vector<uint8_t> nv12;
  uint32_t width = 0;
  uint32_t height = 0;
  int captured = 0;

  for (int attempt = 0; attempt < 200 + wantedSamples * 40 && captured < wantedSamples; ++attempt) {
    DWORD streamIndex = 0, flags = 0;
    LONGLONG timestamp = 0;
    IMFSample* sample = nullptr;
    hr = reader->ReadSample(static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM), 0,
                            &streamIndex, &flags, &timestamp, &sample);
    if (FAILED(hr)) { std::printf("ReadSample failed: 0x%08lX\n", static_cast<unsigned long>(hr)); break; }
    if (sample == nullptr) {
      // A stream tick: the Frame Server is still starting up. Reading again in a
      // tight loop would spin through every attempt before a frame exists.
      Sleep(25);
      continue;
    }

    IMFMediaType* current = nullptr;
    if (SUCCEEDED(reader->GetCurrentMediaType(static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM), &current))) {
      MFGetAttributeSize(current, MF_MT_FRAME_SIZE, &width, &height);
      current->Release();
    }

    IMFMediaBuffer* buffer = nullptr;
    if (SUCCEEDED(sample->GetBufferByIndex(0, &buffer)) && buffer != nullptr) {
      BYTE* data = nullptr;
      DWORD maxLength = 0, currentLength = 0;
      if (SUCCEEDED(buffer->Lock(&data, &maxLength, &currentLength))) {
        nv12.assign(data, data + currentLength);
        captured += 1;
        buffer->Unlock();
      }
      buffer->Release();
    }
    sample->Release();
  }

  if (nv12.empty() || width == 0 || height == 0) {
    std::printf("no frame captured\n");
    return 1;
  }

  const std::vector<uint8_t> rgb = Nv12ToRgb(nv12, width, height);
  if (!WritePng(outputPath, rgb, width, height)) {
    std::printf("could not write %s\n", outputPath);
    return 1;
  }

  std::printf("wrote %s (%ux%u, %zu bytes)\n", outputPath, width, height, nv12.size());

  reader->Release();
  source->Release();
  target->Release();
  for (UINT32 i = 0; i < count; ++i) devices[i]->Release();
  CoTaskMemFree(devices);
  MFShutdown();
  CoUninitialize();
  return 0;
}
