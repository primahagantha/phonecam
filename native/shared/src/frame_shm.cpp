#include "phonecam/frame_shm.hpp"

#include <aclapi.h>
#include <sddl.h>
#include <windows.h>

#pragma comment(lib, "advapi32.lib")
namespace phonecam::shm {
namespace {
constexpr size_t kHeaderBytes = sizeof(FrameHeader);

/**
 * Why the last `FrameWriter::Open` failed, for the caller to log.
 *
 * "couldn't create the shared frame buffer" on its own is unactionable: the
 * failure can be a file permission, a mapping that another process still holds,
 * or an ACL that could not be applied, and those have different fixes. The
 * string is set at the point of failure, where the error code is still live.
 */
inline std::string& LastError() {
  static std::string message;
  return message;
}

size_t SectionBytes(uint32_t width, uint32_t height) {
  return kHeaderBytes + Nv12Size(width, height);
}
bool GrantFrameServerRead(const std::wstring& path) {
  // The Media Foundation Frame Server normally runs as LocalService, while
  // the core runs in the interactive user session. Do not rely on the
  // inherited ProgramData ACL for this cross-session handoff.
  //
  // LocalService gets read *and* write, not read only: the reader count in the
  // shared header is the one field the camera writes, and interlocked-updating it
  // through a read-only mapping is an access violation that kills the Frame
  // Server on the first live frame. Frame pixels are still written only by this
  // process; the write access exists for the four-byte counter.
  PSECURITY_DESCRIPTOR descriptor = nullptr;
  if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
          L"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)(A;;GRGW;;;LS)",
          SDDL_REVISION_1, &descriptor, nullptr)) {
    LastError() = "the frame buffer ACL could not be built (error " +
                  std::to_string(GetLastError()) + ")";
    return false;
  }
  const BOOL applied = SetFileSecurityW(
      path.c_str(), DACL_SECURITY_INFORMATION, descriptor);
  const DWORD error = applied == TRUE ? 0 : GetLastError();
  LocalFree(descriptor);
  if (applied != TRUE) {
    LastError() = "the frame buffer ACL could not be applied (error " +
                  std::to_string(error) + "); the Frame Server would not be able to read it";
    return false;
  }
  return true;
}


}

FrameWriter::~FrameWriter() { Close(); }

std::string LastWriterError() { return LastError(); }

bool FrameWriter::Open(const std::wstring& path, uint32_t width, uint32_t height) {
  Close();
  LastError().clear();
  if (width == 0 || height == 0) {
    LastError() = "the frame size is zero";
    return false;
  }
  CreateDirectoryW(L"C:\\ProgramData\\PhoneCam", nullptr);
  const size_t bytes = SectionBytes(width, height);
  HANDLE file = CreateFileW(path.c_str(), GENERIC_READ | GENERIC_WRITE,
      FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) {
    LastError() = "the frame buffer file could not be opened (error " +
                  std::to_string(GetLastError()) + ")";
    return false;
  }

  // Grow only, never shrink.
  //
  // The virtual camera DLL lives in the Frame Server process and keeps its own
  // view of this file open for as long as the camera is in use. Windows refuses
  // to truncate a file that another process has mapped (ERROR_USER_MAPPED_FILE),
  // so shrinking it here — which is what a resolution step *down* does — made
  // this Open fail, and the core then had no frame buffer at all: a permanent
  // placeholder after the adaptive ladder moved one rung down. Leaving the file
  // at its largest size costs a few unused megabytes and lets the reader's
  // existing mapping stay valid; the header's capacity is what bounds a read,
  // not the file size.
  LARGE_INTEGER existing{};
  if (!GetFileSizeEx(file, &existing)) {
    LastError() = "the frame buffer size could not be read (error " +
                  std::to_string(GetLastError()) + ")";
    CloseHandle(file);
    return false;
  }
  if (static_cast<uint64_t>(existing.QuadPart) < bytes) {
    LARGE_INTEGER size{};
    size.QuadPart = static_cast<LONGLONG>(bytes);
    if (!SetFilePointerEx(file, size, nullptr, FILE_BEGIN) || !SetEndOfFile(file)) {
      // ERROR_USER_MAPPED_FILE lands here: another process still has a view of
      // this file, so it cannot be extended. Worth naming, because the fix is
      // to stop the consumer that holds it, not to retry.
      LastError() = "the frame buffer could not be grown to " + std::to_string(bytes) +
                    " bytes (error " + std::to_string(GetLastError()) + ")";
      CloseHandle(file);
      return false;
    }
  }
  if (!GrantFrameServerRead(path)) {
    CloseHandle(file);
    return false;
  }
  // The mapping object is created at the file's *current* size (0,0 means "as
  // large as the file"), not at `bytes`.
  //
  // The file only ever grows, so after a resolution step down it is still sized
  // for the larger stream that ran before. Creating the mapping at the smaller
  // `bytes` and then mapping the whole file would fail outright — a view can
  // never be larger than the mapping object it comes from — and that is a core
  // with no frame buffer at all and a permanent placeholder. The header's
  // `capacity` is what delimits a frame, so a larger view is harmless.
  HANDLE mapping = CreateFileMappingW(file, nullptr, PAGE_READWRITE, 0, 0, nullptr);
  if (mapping == nullptr) {
    LastError() = "the frame buffer could not be mapped (error " +
                  std::to_string(GetLastError()) + ")";
    CloseHandle(file);
    return false;
  }
  LARGE_INTEGER fileSize{};
  if (!GetFileSizeEx(file, &fileSize) || fileSize.QuadPart < static_cast<LONGLONG>(bytes)) {
    LastError() = "the frame buffer is smaller than the frame it must hold";
    CloseHandle(mapping);
    CloseHandle(file);
    return false;
  }
  void* view = MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0,
                             static_cast<size_t>(fileSize.QuadPart));
  if (view == nullptr) {
    LastError() = "the frame buffer view could not be created (error " +
                  std::to_string(GetLastError()) + ")";
    CloseHandle(mapping);
    CloseHandle(file);
    return false;
  }
  backingFile_ = file;
  mapping_ = mapping;
  base_ = static_cast<uint8_t*>(view);
  header_ = reinterpret_cast<FrameHeader*>(base_);
  width_ = width;
  height_ = height;
  header_->magic = kMagic;
  header_->layoutVersion = kLayoutVersion;
  header_->width = width;
  header_->height = height;
  header_->strideY = width;
  header_->strideUv = width;
  header_->capacity = Nv12Size(width, height);
  header_->ptsUs = 0;
  header_->frameIndex = 0;
  header_->state = FrameState::kWaiting;
  header_->mirror = 0;
  header_->rotation = 0;
  header_->seq = 0;
  header_->payloadBytes = 0;
  // `readers` is deliberately not written here. It is a live count maintained by
  // `FrameReader::Open`/`Close` in the shared header, and the writer only
  // recreates the header when it reopens the buffer — at which point a reader
  // that is still attached is still attached. Resetting it would make the UI's
  // "N using it" drop to zero on a resolution change.
  frameIndex_ = 0;
  return true;
}

void FrameWriter::Close() {
  if (base_ != nullptr) { UnmapViewOfFile(base_); base_ = nullptr; header_ = nullptr; }
  if (mapping_ != nullptr) { CloseHandle(static_cast<HANDLE>(mapping_)); mapping_ = nullptr; }
  if (backingFile_ != nullptr) { CloseHandle(static_cast<HANDLE>(backingFile_)); backingFile_ = nullptr; }
  width_ = 0;
  height_ = 0;
}

void FrameWriter::SetTransform(bool mirror, uint32_t rotation) {
  if (header_ == nullptr) return;
  InterlockedIncrement(reinterpret_cast<volatile LONG*>(&header_->seq));
  header_->mirror = mirror ? 1u : 0u;
  header_->rotation = rotation % 360u;
  InterlockedIncrement(reinterpret_cast<volatile LONG*>(&header_->seq));
}

void FrameWriter::Publish(const uint8_t* data, size_t bytes, uint64_t ptsUs, FrameState state) {
  if (header_ == nullptr || data == nullptr || bytes != header_->capacity) return;
  InterlockedIncrement(reinterpret_cast<volatile LONG*>(&header_->seq));
  std::memcpy(base_ + kHeaderBytes, data, bytes);
  header_->ptsUs = ptsUs;
  header_->frameIndex = ++frameIndex_;
  header_->state = state;
  header_->payloadBytes = static_cast<uint32_t>(bytes);
  InterlockedIncrement(reinterpret_cast<volatile LONG*>(&header_->seq));
}

void FrameWriter::PublishState(FrameState state) {
  if (header_ == nullptr) return;
  InterlockedIncrement(reinterpret_cast<volatile LONG*>(&header_->seq));
  header_->state = state;
  header_->frameIndex = ++frameIndex_;
  InterlockedIncrement(reinterpret_cast<volatile LONG*>(&header_->seq));
}

FrameReader::~FrameReader() { Close(); }

bool FrameReader::Open(const std::wstring& path) {
  Close();
  // The header carries a live reader count, so the reader needs a writable
  // mapping to maintain it. LocalService has read/write on the buffer the core
  // creates, but a buffer created by an older core (or by a policy that only
  // granted read) must still be readable: the camera showing pictures matters
  // more than the count, so fall back to a read-only view and simply do not count
  // this reader. Writing the counter through a read-only mapping is what used to
  // fault the Frame Server on the first live frame.
  HANDLE file = CreateFileW(path.c_str(), GENERIC_READ | GENERIC_WRITE,
      FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
  bool writable = file != INVALID_HANDLE_VALUE;
  if (!writable) {
    file = CreateFileW(path.c_str(), GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
  }
  if (file == INVALID_HANDLE_VALUE) return false;
  HANDLE mapping = CreateFileMappingW(file, nullptr, writable ? PAGE_READWRITE : PAGE_READONLY,
                                      0, 0, nullptr);
  if (mapping == nullptr && writable) {
    // The file opened for writing but cannot be mapped writable (a read-only
    // volume, or a race with the writer recreating it). Show pictures anyway.
    CloseHandle(file);
    writable = false;
    file = CreateFileW(path.c_str(), GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file != INVALID_HANDLE_VALUE) {
      mapping = CreateFileMappingW(file, nullptr, PAGE_READONLY, 0, 0, nullptr);
    }
  }
  if (mapping == nullptr) {
    if (file != INVALID_HANDLE_VALUE) CloseHandle(file);
    return false;
  }
  void* view = MapViewOfFile(mapping, writable ? FILE_MAP_ALL_ACCESS : FILE_MAP_READ, 0, 0, 0);
  if (view == nullptr) { CloseHandle(mapping); CloseHandle(file); return false; }
  auto* header = reinterpret_cast<FrameHeader*>(view);

  // The header lives at the start of the file, so it can only be read once the
  // view covers it. A mapping shorter than one header means the writer is
  // mid-recreation and this attempt is simply too early.
  MEMORY_BASIC_INFORMATION info{};
  if (VirtualQuery(view, &info, sizeof(info)) == 0 ||
      info.RegionSize < sizeof(FrameHeader)) {
    UnmapViewOfFile(view); CloseHandle(mapping); CloseHandle(file); return false;
  }

  if (header->magic != kMagic || header->layoutVersion != kLayoutVersion) {
    UnmapViewOfFile(view); CloseHandle(mapping); CloseHandle(file); return false;
  }

  backingFile_ = file;
  mapping_ = mapping;
  base_ = static_cast<uint8_t*>(view);
  header_ = header;
  name_ = path;
  mappedBytes_ = info.RegionSize;
  writable_ = writable;
  // Count this reader in the shared header, so the core can report how many
  // apps are using the camera. Written last, after the reader is fully
  // established, and paired with the decrement in Close. Only a writable view
  // can write it; see the comment above.
  if (writable_) {
    InterlockedIncrement(reinterpret_cast<volatile LONG*>(&header_->readers));
  }
  return true;
}

void FrameReader::Close() {
  if (writable_ && header_ != nullptr) {
    // The view is still mapped here, so the header is writable; decrement before
    // unmapping. A reader that never counted itself has nothing to undo.
    InterlockedDecrement(reinterpret_cast<volatile LONG*>(&header_->readers));
  }
  writable_ = false;
  if (base_ != nullptr) { UnmapViewOfFile(base_); base_ = nullptr; header_ = nullptr; }
  if (mapping_ != nullptr) { CloseHandle(static_cast<HANDLE>(mapping_)); mapping_ = nullptr; }
  if (backingFile_ != nullptr) { CloseHandle(static_cast<HANDLE>(backingFile_)); backingFile_ = nullptr; }
  name_.clear();
  mappedBytes_ = 0;
}

bool FrameReader::IsStale() const {
  if (header_ == nullptr) return false;
  const uint64_t needed = static_cast<uint64_t>(kHeaderBytes) + header_->capacity;
  return needed > mappedBytes_;
}

uint32_t FrameReader::MappedCapacity() const {
  if (header_ == nullptr || mappedBytes_ <= kHeaderBytes) return 0;
  const size_t capacity = mappedBytes_ - kHeaderBytes;
  return capacity > 0xFFFFFFFFu ? 0xFFFFFFFFu : static_cast<uint32_t>(capacity);
}

FrameReader::Snapshot FrameReader::Peek() const {
  Snapshot snapshot;
  if (header_ == nullptr) return snapshot;
  for (int attempt = 0; attempt < 4; ++attempt) {
    const uint32_t before = header_->seq;
    if (before & 1u) continue;
    snapshot.width = header_->width;
    snapshot.height = header_->height;
    snapshot.ptsUs = header_->ptsUs;
    snapshot.frameIndex = header_->frameIndex;
    snapshot.state = header_->state;
    snapshot.mirror = header_->mirror != 0;
    snapshot.rotation = header_->rotation;
    if (before == header_->seq) return snapshot;
  }
  return snapshot;
}

bool FrameReader::HasNewFrame(uint64_t lastFrameIndex) const {
  return header_ != nullptr && header_->frameIndex != lastFrameIndex;
}

bool FrameReader::Read(std::vector<uint8_t>* out, Snapshot* snapshot) {
  if (header_ == nullptr || out == nullptr) return false;
  const uint32_t capacity = header_->capacity;
  if (header_->width == 0 || header_->height == 0 || capacity == 0) return false;

  // The copy below must stay inside the view. A header whose capacity is larger
  // than the mapping means the writer recreated the file at a bigger size and
  // this reader is still attached to the old one; reading it would fault inside
  // the Frame Server. `IsStale` lets the caller reopen instead.
  if (static_cast<uint64_t>(kHeaderBytes) + capacity > mappedBytes_) return false;

  out->resize(capacity);
  for (int attempt = 0; attempt < 64; ++attempt) {
    const uint32_t before = header_->seq;
    if (before & 1u) { YieldProcessor(); continue; }
    std::memcpy(out->data(), base_ + kHeaderBytes, capacity);
    Snapshot local;
    local.width = header_->width;
    local.height = header_->height;
    local.ptsUs = header_->ptsUs;
    local.frameIndex = header_->frameIndex;
    local.state = header_->state;
    local.mirror = header_->mirror != 0;
    local.rotation = header_->rotation;
    if (before == header_->seq) {
      if (snapshot != nullptr) *snapshot = local;
      lastGood_ = local;
      lastBuffer_ = *out;
      haveLastGood_ = true;
      return true;
    }
  }
  if (haveLastGood_ && lastBuffer_.size() == capacity) {
    *out = lastBuffer_;
    if (snapshot != nullptr) *snapshot = lastGood_;
    return true;
  }
  return false;
}

}  // namespace phonecam::shm
