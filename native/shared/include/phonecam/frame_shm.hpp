/** Shared-memory frame transport between PhoneCam Core and the Media Foundation virtual camera. */
#pragma once

#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

namespace phonecam::shm {

inline constexpr uint32_t kMagic = 0x5043414Du;
inline constexpr uint32_t kLayoutVersion = 2;

enum class FrameState : uint32_t {
  kWaiting = 0,
  kLive = 1,
  kStale = 2,
  kDisconnected = 3,
};

#pragma pack(push, 1)
struct FrameHeader {
  uint32_t magic;
  uint32_t layoutVersion;
  uint32_t width;
  uint32_t height;
  uint32_t strideY;
  uint32_t strideUv;
  uint32_t capacity;
  uint64_t ptsUs;
  uint64_t frameIndex;
  FrameState state;
  uint32_t mirror;
  uint32_t rotation;
  volatile uint32_t seq;
  uint32_t payloadBytes;
  /**
   * Readers (the virtual camera DLL) that currently hold the buffer open.
   *
   * `reserved` was never written and never read, so it is repurposed rather than
   * growing the header: the layout stays 68 bytes, so a reader built against the
   * old header still parses this one. Updated with interlocked operations by
   * `FrameReader::Open`/`Close`, and read by the core to report how many apps
   * are using the camera.
   *
   * A reader can only maintain this field when it mapped the file writable. The
   * Frame Server runs as LocalService, which historically had read-only access,
   * so a reader that cannot write maps read-only and does not count itself;
   * writing the counter through a read-only view faults the service on the first
   * live frame. See `FrameReader::Open`.
   */
  uint32_t readers;
};
#pragma pack(pop)

static_assert(sizeof(FrameHeader) == 68, "FrameHeader layout changed; bump kLayoutVersion");

inline uint32_t Nv12Size(uint32_t width, uint32_t height) {
  return width * height + width * ((height + 1) / 2);
}

class FrameWriter {
 public:
  FrameWriter() = default;
  ~FrameWriter();
  FrameWriter(const FrameWriter&) = delete;
  FrameWriter& operator=(const FrameWriter&) = delete;

  bool Open(const std::wstring& name, uint32_t width, uint32_t height);
  void Close();
  bool IsOpen() const { return base_ != nullptr; }
  void Publish(const uint8_t* data, size_t bytes, uint64_t ptsUs, FrameState state);
  void PublishState(FrameState state);
  void SetTransform(bool mirror, uint32_t rotation);
  uint32_t width() const { return width_; }
  uint32_t height() const { return height_; }

  /**
   * How many readers (the virtual camera DLL) currently hold the buffer open.
   *
   * Each `FrameReader` increments the header's reader count on open and
   * decrements it on close, so this is a real measurement rather than a field
   * that is always zero. It is what the UI shows as "N using it".
   */
  uint32_t ReaderCount() const {
    return header_ != nullptr ? header_->readers : 0;
  }

 private:
  FrameHeader* header_ = nullptr;
  uint8_t* base_ = nullptr;
  void* mapping_ = nullptr;
  void* backingFile_ = nullptr;
  uint32_t width_ = 0;
  uint32_t height_ = 0;
  uint64_t frameIndex_ = 0;
};

class FrameReader {
 public:
  FrameReader() = default;
  ~FrameReader();
  FrameReader(const FrameReader&) = delete;
  FrameReader& operator=(const FrameReader&) = delete;

  bool Open(const std::wstring& name);
  void Close();
  bool IsOpen() const { return base_ != nullptr; }

  /**
   * True when the mapping no longer describes the buffer the writer is using.
   *
   * The core recreates this file when the stream's resolution changes, which the
   * adaptive ladder does on its own. A reader that kept the mapping it opened at
   * startup would still be attached to the old, smaller section: it would read a
   * header announcing the new capacity and then copy that many bytes out of a
   * mapping that does not cover them, which faults inside the Frame Server.
   * Detecting it here is what lets the caller reopen instead of crashing.
   */
  bool IsStale() const;

  /** Capacity the mapping was created with, in NV12 bytes. */
  uint32_t MappedCapacity() const;

  struct Snapshot {
    uint32_t width = 0;
    uint32_t height = 0;
    uint64_t ptsUs = 0;
    uint64_t frameIndex = 0;
    FrameState state = FrameState::kWaiting;
    bool mirror = false;
    uint32_t rotation = 0;
  };

  bool Read(std::vector<uint8_t>* out, Snapshot* snapshot);
  bool HasNewFrame(uint64_t lastFrameIndex) const;
  Snapshot Peek() const;

 private:
  FrameHeader* header_ = nullptr;
  uint8_t* base_ = nullptr;
  void* mapping_ = nullptr;
  void* backingFile_ = nullptr;
  std::wstring name_;
  Snapshot lastGood_;
  std::vector<uint8_t> lastBuffer_;
  bool haveLastGood_ = false;

  /** Bytes the view covers, recorded at Open. Reads never go past this. */
  size_t mappedBytes_ = 0;

  /**
   * True when this reader could map the file writable and counted itself.
   *
   * Only then may `Close` decrement the header's reader count: a read-only
   * reader never incremented it, and a decrement through a read-only view is the
   * same fault the increment would be.
   */
  bool writable_ = false;
};

inline std::wstring SectionName() {
  // The Windows Frame Server may run as LocalService in another session. A
  // file-backed mapping avoids session namespaces while keeping the frame
  // buffer local to this machine.
  return L"C:\\ProgramData\\PhoneCam\\framebuffer-v2.bin";
}

/**
 * Why the last `FrameWriter::Open` failed.
 *
 * A frame buffer that cannot be created is the difference between a working
 * camera and a permanent "DEVICE NOT CONNECTED", and the possible causes — a
 * permission on ProgramData, a mapping another process still holds, an ACL that
 * could not be applied — have different fixes. Callers log this instead of a
 * generic message so the failure is diagnosable from the core's own output.
 */
std::string LastWriterError();

}  // namespace phonecam::shm
