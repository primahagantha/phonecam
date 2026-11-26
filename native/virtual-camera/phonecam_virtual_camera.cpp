#include "phonecam_virtual_camera.hpp"

#include "phonecam/nv12.hpp"

#include <cstdarg>
#include <cstdio>
#include <chrono>
#include <thread>
#include "vcam_log.hpp"

#include <shlwapi.h>
#include <ksmedia.h>
#include <propvarutil.h>
#include <iostream>

#pragma comment(lib, "mfplat.lib")
#pragma comment(lib, "mfuuid.lib")
#pragma comment(lib, "mf.lib")
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "shlwapi.lib")

namespace phonecam::vcam {

namespace {

static std::atomic<long> g_moduleRefCount{0};



/**
 * Publishes the four attributes the Frame Server needs to identify a stream.
 * Without them activation fails with MF_E_ATTRIBUTENOTFOUND.
 */
HRESULT ApplyDeviceStreamAttributes(IMFAttributes* store) {
  if (store == nullptr) return E_POINTER;

  HRESULT hr = store->SetGUID(MF_DEVICESTREAM_STREAM_CATEGORY, PINNAME_VIDEO_CAPTURE);
  if (SUCCEEDED(hr)) hr = store->SetUINT32(MF_DEVICESTREAM_STREAM_ID, 0);
  if (SUCCEEDED(hr)) hr = store->SetUINT32(MF_DEVICESTREAM_FRAMESERVER_SHARED, 1);
  if (SUCCEEDED(hr)) {
    hr = store->SetUINT32(MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color);
  }
  return hr;
}
HRESULT CreateNV12MediaType(uint32_t width, uint32_t height, uint32_t fps, IMFMediaType** ppType) {
  if (!ppType) return E_POINTER;
  *ppType = nullptr;

  IMFMediaType* pType = nullptr;
  HRESULT hr = MFCreateMediaType(&pType);
  if (FAILED(hr)) return hr;

  hr = pType->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
  if (SUCCEEDED(hr)) hr = pType->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12);
  if (SUCCEEDED(hr)) hr = MFSetAttributeSize(pType, MF_MT_FRAME_SIZE, width, height);
  if (SUCCEEDED(hr)) hr = MFSetAttributeRatio(pType, MF_MT_FRAME_RATE, fps, 1);
  if (SUCCEEDED(hr)) hr = MFSetAttributeRatio(pType, MF_MT_PIXEL_ASPECT_RATIO, 1, 1);
  if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive);
  if (SUCCEEDED(hr)) hr = pType->SetUINT32(MF_MT_ALL_SAMPLES_INDEPENDENT, TRUE);

  if (FAILED(hr)) {
    pType->Release();
    return hr;
  }

  *ppType = pType;
  return S_OK;
}

}  // namespace

void AddRefModule() { g_moduleRefCount.fetch_add(1); }
void ReleaseModule() { g_moduleRefCount.fetch_sub(1); }
bool CanUnloadNow() { return g_moduleRefCount.load() == 0; }

// ============================================================================
// PhoneCamMediaStream
// ============================================================================

PhoneCamMediaStream::PhoneCamMediaStream(PhoneCamMediaSource* source, IMFStreamDescriptor* pDescriptor)
    : source_(source), streamDescriptor_(pDescriptor) {
  AddRefModule();
  if (source_) source_->AddRef();
  if (streamDescriptor_) streamDescriptor_->AddRef();

  MFCreateEventQueue(&eventQueue_);

  // Check offline test pattern environment flag
  wchar_t envBuf[16] = {0};
  if (GetEnvironmentVariableW(L"PHONECAM_VCAM_TEST_PATTERN", envBuf, 16) > 0) {
    forceTestPattern_ = (envBuf[0] == L'1');
  }

  // Pre-generate placeholder
  GenerateDisconnectedNV12(width_, height_, &cachedNv12_);
}

PhoneCamMediaStream::~PhoneCamMediaStream() {
  Shutdown();
  if (streamDescriptor_) {
    streamDescriptor_->Release();
    streamDescriptor_ = nullptr;
  }
  if (source_) {
    source_->Release();
    source_ = nullptr;
  }
  if (eventQueue_) {
    eventQueue_->Release();
    eventQueue_ = nullptr;
  }
  ReleaseModule();
}

STDMETHODIMP PhoneCamMediaStream::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;

  if (riid == IID_IUnknown || riid == IID_IMFMediaEventGenerator || riid == IID_IMFMediaStream) {
    *ppv = static_cast<IMFMediaStream*>(this);
  } else if (riid == IID_IMFMediaStream2) {
    *ppv = static_cast<IMFMediaStream2*>(this);
  } else {
    return E_NOINTERFACE;
  }

  AddRef();
  return S_OK;
}

STDMETHODIMP_(ULONG) PhoneCamMediaStream::AddRef() {
  return refCount_.fetch_add(1) + 1;
}

STDMETHODIMP_(ULONG) PhoneCamMediaStream::Release() {
  ULONG count = refCount_.fetch_sub(1) - 1;
  if (count == 0) {
    delete this;
  }
  return count;
}

STDMETHODIMP PhoneCamMediaStream::BeginGetEvent(IMFAsyncCallback* pCallback, IUnknown* punkState) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  return eventQueue_->BeginGetEvent(pCallback, punkState);
}

STDMETHODIMP PhoneCamMediaStream::GetEvent(DWORD dwFlags, IMFMediaEvent** ppEvent) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  return eventQueue_->GetEvent(dwFlags, ppEvent);
}

STDMETHODIMP PhoneCamMediaStream::EndGetEvent(IMFAsyncResult* pResult, IMFMediaEvent** ppEvent) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  return eventQueue_->EndGetEvent(pResult, ppEvent);
}

STDMETHODIMP PhoneCamMediaStream::QueueEvent(MediaEventType met, REFGUID guidExtendedType, HRESULT hrStatus, const PROPVARIANT* pvValue) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  return eventQueue_->QueueEventParamVar(met, guidExtendedType, hrStatus, pvValue);
}

STDMETHODIMP PhoneCamMediaStream::GetMediaSource(IMFMediaSource** ppMediaSource) {
  if (!ppMediaSource) return E_POINTER;
  *ppMediaSource = nullptr;
  std::lock_guard<std::mutex> lock(mutex_);
  if (!source_) return MF_E_SHUTDOWN;
  *ppMediaSource = source_;
  (*ppMediaSource)->AddRef();
  return S_OK;
}

STDMETHODIMP PhoneCamMediaStream::GetStreamDescriptor(IMFStreamDescriptor** ppStreamDescriptor) {
  if (!ppStreamDescriptor) return E_POINTER;
  *ppStreamDescriptor = nullptr;
  std::lock_guard<std::mutex> lock(mutex_);
  if (!streamDescriptor_) return MF_E_SHUTDOWN;
  *ppStreamDescriptor = streamDescriptor_;
  (*ppStreamDescriptor)->AddRef();
  return S_OK;
}

STDMETHODIMP PhoneCamMediaStream::RequestSample(IUnknown* pToken) {
  // Pace the stream at the negotiated rate. Sleep outside the lock: a state
  // change or a shutdown must never wait on a stream that is only waiting for
  // its next frame slot.
  LONGLONG wait100ns = 0;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    if (!eventQueue_) return MF_E_SHUTDOWN;
    if (streamState_ != MF_STREAM_STATE_RUNNING) return MF_E_INVALIDREQUEST;
    wait100ns = nextSampleDue100ns_ - MFGetSystemTime();
  }
  if (wait100ns > 0) {
    std::this_thread::sleep_for(std::chrono::microseconds(wait100ns / 10));
  }

  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  if (streamState_ != MF_STREAM_STATE_RUNNING) return MF_E_INVALIDREQUEST;
  const HRESULT hr = DeliverFrame(pToken);
  if (SUCCEEDED(hr)) nextSampleDue100ns_ = MFGetSystemTime() + sampleDuration100ns_;
  return hr;
}

STDMETHODIMP PhoneCamMediaStream::SetStreamState(MF_STREAM_STATE value) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (streamState_ == value) return S_OK;

  switch (value) {
    case MF_STREAM_STATE_RUNNING:
      streamState_ = MF_STREAM_STATE_RUNNING;
      if (eventQueue_) eventQueue_->QueueEventParamVar(MEStreamStarted, GUID_NULL, S_OK, nullptr);
      break;
    case MF_STREAM_STATE_PAUSED:
      streamState_ = MF_STREAM_STATE_PAUSED;
      if (eventQueue_) eventQueue_->QueueEventParamVar(MEStreamPaused, GUID_NULL, S_OK, nullptr);
      break;
    case MF_STREAM_STATE_STOPPED:
      streamState_ = MF_STREAM_STATE_STOPPED;
      if (eventQueue_) eventQueue_->QueueEventParamVar(MEStreamStopped, GUID_NULL, S_OK, nullptr);
      break;
    default:
      return MF_E_INVALID_STATE_TRANSITION;
  }
  return S_OK;
}

STDMETHODIMP PhoneCamMediaStream::GetStreamState(MF_STREAM_STATE* value) {
  if (!value) return E_POINTER;
  std::lock_guard<std::mutex> lock(mutex_);
  *value = streamState_;
  return S_OK;
}

HRESULT PhoneCamMediaStream::Shutdown() {
  std::lock_guard<std::mutex> lock(mutex_);
  streamState_ = MF_STREAM_STATE_STOPPED;
  if (frameReader_.IsOpen()) {
    frameReader_.Close();
    shmOpened_ = false;
  }
  if (eventQueue_) eventQueue_->Shutdown();
  return S_OK;
}

void PhoneCamMediaStream::SetActiveFormat(uint32_t width, uint32_t height, uint32_t fps) {
  std::lock_guard<std::mutex> lock(mutex_);
  width_ = width;
  height_ = height;
  fps_ = (fps > 0 ? fps : 30);
  sampleDuration100ns_ = 10000000ull / fps_;
  nextSampleDue100ns_ = 0;
}

HRESULT PhoneCamMediaStream::Start() {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  streamState_ = MF_STREAM_STATE_RUNNING;
  nextSampleDue100ns_ = 0;
  return eventQueue_->QueueEventParamVar(MEStreamStarted, GUID_NULL, S_OK, nullptr);
}

HRESULT PhoneCamMediaStream::Stop() {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  streamState_ = MF_STREAM_STATE_STOPPED;
  return eventQueue_->QueueEventParamVar(MEStreamStopped, GUID_NULL, S_OK, nullptr);
}

HRESULT PhoneCamMediaStream::Pause() {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  streamState_ = MF_STREAM_STATE_PAUSED;
  return eventQueue_->QueueEventParamVar(MEStreamPaused, GUID_NULL, S_OK, nullptr);
}

HRESULT PhoneCamMediaStream::DeliverFrame(IUnknown* pToken) {
  const size_t expectedBytes = (static_cast<size_t>(width_) * height_ * 3) / 2;
  const uint8_t* nv12Ptr = nullptr;
  std::vector<uint8_t> shmBuffer;

  if (!forceTestPattern_) {
    if (!shmOpened_) {
      shmOpened_ = frameReader_.Open(phonecam::shm::SectionName());
    }

    // A resolution change makes the core recreate this file at a different size.
    // A reader still attached to the old, smaller mapping cannot see the new
    // frames, and copying out of it would read past the end of the view. Drop
    // the stale mapping and reattach; the writer only ever grows the file, so
    // reopening always lands on a view that covers the current layout.
    if (shmOpened_ && frameReader_.IsStale()) {
      VcamLog("frame buffer changed size; reattaching to the new layout");
      frameReader_.Close();
      shmOpened_ = false;
    }
    if (!shmOpened_) {
      shmOpened_ = frameReader_.Open(phonecam::shm::SectionName());
      if (shmOpened_) {
        char detail[128];
        std::snprintf(detail, sizeof(detail), "frame buffer opened (%u bytes of frames)",
                      static_cast<unsigned>(frameReader_.MappedCapacity()));
        VcamLog(detail);
      }
    }

    if (shmOpened_) {
      phonecam::shm::FrameReader::Snapshot snap;
      const bool readOk = frameReader_.Read(&shmBuffer, &snap);
      lastReadOk_ = readOk;
      lastSnapshot_ = snap;
      if (readOk && snap.state == phonecam::shm::FrameState::kLive &&
          snap.width > 0 && snap.height > 0) {
        // The frame's own presentation time, so the sample can be stamped with
        // it rather than with the moment the consumer happened to ask.
        deliveredPtsUs_ = snap.ptsUs;
        if (snap.width == width_ && snap.height == height_ && shmBuffer.size() == expectedBytes) {
          nv12Ptr = shmBuffer.data();
        } else {
          const size_t sourceBytes = (static_cast<size_t>(snap.width) * snap.height * 3) / 2;
          if (shmBuffer.size() >= sourceBytes) {
            scaledNv12_.resize(expectedBytes);
            phonecam::nv12::Scale(shmBuffer.data(), snap.width, snap.height, scaledNv12_.data(),
                                  width_, height_);
            nv12Ptr = scaledNv12_.data();
          }
        }
        if (nv12Ptr != nullptr && (snap.mirror || snap.rotation != 0)) {
          transformedNv12_.resize(expectedBytes);
          phonecam::nv12::Transform(nv12Ptr, width_, height_, snap.mirror, snap.rotation,
                                    transformedNv12_.data());
          nv12Ptr = transformedNv12_.data();
        }
      }
    }
  }

  if (nv12Ptr == nullptr) {
    // A placeholder has no frame time of its own; stamping it from the wall clock
    // keeps a consumer's timeline moving while the picture is explicitly absent.
    deliveredPtsUs_ = 0;
    // Logged on the transition, never per frame. This runs once per delivered
    // sample, so a line here wrote a 1.3 GB log in a single session while the
    // only new information was on the first line. Which of the two failures it
    // is matters: a buffer that cannot be opened at all is a different problem
    // from one that opens but holds no live frame, and the old message hid that
    // distinction behind an identical line for both.
    if (!loggedPlaceholder_) {
      loggedPlaceholder_ = true;
      // The values, not just the verdict. "no live frame" covered four different
      // faults — the buffer never opened, the read failed, the header was not
      // initialised, or the core simply is not publishing — and they need
      // different fixes. One line with the numbers separates them.
      char detail[256];
      std::snprintf(detail, sizeof(detail),
                    "no live frame (buffer %s, read %s, %ux%u, state %u, capacity %u)",
                    shmOpened_ ? "open" : "unavailable",
                    lastReadOk_ ? "ok" : "failed",
                    static_cast<unsigned>(lastSnapshot_.width),
                    static_cast<unsigned>(lastSnapshot_.height),
                    static_cast<unsigned>(lastSnapshot_.state),
                    static_cast<unsigned>(frameReader_.MappedCapacity()));
      VcamLog(detail);
    }
    if (cachedNv12_.size() != expectedBytes) {
      if (forceTestPattern_) {
        GenerateColorBarsNV12(width_, height_, &cachedNv12_);
      } else {
        GenerateDisconnectedNV12(width_, height_, &cachedNv12_);
      }
    }
    nv12Ptr = cachedNv12_.data();
  } else if (loggedPlaceholder_) {
    loggedPlaceholder_ = false;
    VcamLog("live frame acquired; placeholder no longer shown");
  }

  // Create MF buffer
  IMFMediaBuffer* pBuffer = nullptr;
  HRESULT hr = MFCreateMemoryBuffer(static_cast<DWORD>(expectedBytes), &pBuffer);
  if (FAILED(hr)) return hr;

  BYTE* pDest = nullptr;
  DWORD maxLen = 0, curLen = 0;
  hr = pBuffer->Lock(&pDest, &maxLen, &curLen);
  if (SUCCEEDED(hr)) {
    std::memcpy(pDest, nv12Ptr, expectedBytes);
    pBuffer->Unlock();
    pBuffer->SetCurrentLength(static_cast<DWORD>(expectedBytes));
  } else {
    pBuffer->Release();
    return hr;
  }

  // Create sample
  IMFSample* pSample = nullptr;
  hr = MFCreateSample(&pSample);
  if (FAILED(hr)) {
    pBuffer->Release();
    return hr;
  }

  pSample->AddBuffer(pBuffer);
  pBuffer->Release();

  // Timestamping
  //
  // The sample is stamped from the stream's own timeline, anchored to the wall
  // clock at the first delivered live frame — not with the raw phone PTS, and
  // not with the wall clock at every delivery.
  //
  // Raw PTS alone cannot be handed to Media Foundation: the phone's PTS base is
  // arbitrary (encoder restarts, muxer offsets), and a base far behind the wall
  // clock made the Frame Server's pacer hold every sample — it computed a
  // presentation time already in the past and waited for a future that had
  // gone — so a consumer's synchronous ReadSample blocked forever and the
  // camera worked only while a phone was live.
  //
  // A wall-clock stamp at every delivery has the opposite failure: a consumer
  // that pulls faster than the phone produces sees each repeated frame as a new
  // moment in time, and a recording built from those timestamps stutters while
  // the live preview looks smooth.
  //
  // Anchoring once and then stepping by the stream's own deltas keeps the
  // ordering and pacing of the stream on a timeline the consumer accepts.
  LONGLONG now = MFGetSystemTime();
  if (deliveredPtsUs_ != 0) {
    const LONGLONG frameDelta100ns =
        static_cast<LONGLONG>(deliveredPtsUs_ - lastDeliveredPtsUs_) * 10;
    if (ptsAnchored_) {
      const LONGLONG candidate = lastSampleTime100ns_ + frameDelta100ns;
      // A repeated frame (delta 0) or a backwards jump steps one full duration;
      // overlapping timestamps are what stall the Frame Server's pacer.
      now = frameDelta100ns > 0
                ? candidate
                : lastSampleTime100ns_ + sampleDuration100ns_;
      // A huge forward jump (the encoder restarted at a larger base) would run
      // ahead of the wall clock; clamp to now rather than letting the timeline
      // diverge from real time by minutes.
      if (now > MFGetSystemTime() + sampleDuration100ns_) {
        now = MFGetSystemTime();
      }
    } else {
      // First live frame: anchor the stream's timeline to now.
      ptsAnchored_ = true;
    }
  } else {
    // A placeholder has no frame time of its own; stamping it from the wall
    // clock keeps a consumer's timeline moving while the picture is absent.
    ptsAnchored_ = false;
  }
  lastDeliveredPtsUs_ = deliveredPtsUs_;
  if (now <= lastSampleTime100ns_) {
    now = lastSampleTime100ns_ + sampleDuration100ns_;
  }
  lastSampleTime100ns_ = now;

  pSample->SetSampleTime(now);
  pSample->SetSampleDuration(sampleDuration100ns_);

  if (pToken) {
    pSample->SetUnknown(MFSampleExtension_Token, pToken);
  }

  hr = eventQueue_->QueueEventParamUnk(MEMediaSample, GUID_NULL, S_OK, pSample);
  pSample->Release();
  return hr;
}

// ============================================================================
// PhoneCamMediaSource
// ============================================================================

PhoneCamMediaSource::PhoneCamMediaSource() {
  AddRefModule();
}

PhoneCamMediaSource::~PhoneCamMediaSource() {
  Shutdown();
  ReleaseModule();
}

HRESULT PhoneCamMediaSource::Initialize() {
  HRESULT hr = MFCreateEventQueue(&eventQueue_);
  if (FAILED(hr)) return hr;

  hr = MFCreateAttributes(&attributes_, 16);
  if (FAILED(hr)) return hr;

  // The Frame Server reads the source's attributes to find the sensor profile
  // collection; without it the camera cannot be activated at all.
  IMFSensorProfileCollection* profiles = nullptr;
  hr = MFCreateSensorProfileCollection(&profiles);
  if (FAILED(hr)) return hr;

  IMFSensorProfile* profile = nullptr;
  if (SUCCEEDED(MFCreateSensorProfile(KSCAMERAPROFILE_Legacy, 0, nullptr, &profile))) {
    profile->AddProfileFilter(0, L"((RES==;FRT<=30,1;SUT==))");
    profiles->AddProfile(profile);
    profile->Release();
  }

  if (SUCCEEDED(MFCreateSensorProfile(KSCAMERAPROFILE_HighFrameRate, 0, nullptr, &profile))) {
    profile->AddProfileFilter(0, L"((RES==;FRT>=60,1;SUT==))");
    profiles->AddProfile(profile);
    profile->Release();
  }

  hr = attributes_->SetUnknown(MF_DEVICEMFT_SENSORPROFILE_COLLECTION, profiles);
  profiles->Release();
  if (FAILED(hr)) return hr;

  attributes_->SetString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, kVirtualCameraFriendlyName);

  return BuildDescriptors();
}

HRESULT PhoneCamMediaSource::BuildDescriptors() {
  // Construct 5 standard formats:
  // 1. 1920x1080 @ 30fps (Default)
  // 2. 1920x1080 @ 60fps
  // 3. 1280x720  @ 30fps
  // 4. 1280x720  @ 60fps
  // 5. 640x480   @ 30fps
  struct Mode { uint32_t w; uint32_t h; uint32_t fps; };
  const Mode modes[5] = {
      {1920, 1080, 30},
      {1920, 1080, 60},
      {1280,  720, 30},
      {1280,  720, 60},
      { 640,  480, 30}
  };

  IMFMediaType* mediaTypes[5] = {nullptr};
  for (int i = 0; i < 5; ++i) {
    HRESULT hr = CreateNV12MediaType(modes[i].w, modes[i].h, modes[i].fps, &mediaTypes[i]);
    if (FAILED(hr)) {
      for (int j = 0; j < i; ++j) if (mediaTypes[j]) mediaTypes[j]->Release();
      return hr;
    }
  }

  HRESULT hr = MFCreateStreamDescriptor(0, 5, mediaTypes, &streamDescriptor_);
  if (FAILED(hr)) {
    for (int i = 0; i < 5; ++i) {
      if (mediaTypes[i]) mediaTypes[i]->Release();
    }
    return hr;
  }

  // Set default media type on stream handler
  IMFMediaTypeHandler* pHandler = nullptr;
  hr = streamDescriptor_->GetMediaTypeHandler(&pHandler);
  if (SUCCEEDED(hr)) {
    pHandler->SetCurrentMediaType(mediaTypes[0]);
    pHandler->Release();
  }
  for (int i = 0; i < 5; ++i) {
    if (mediaTypes[i]) mediaTypes[i]->Release();
  }

  // The Frame Server identifies and publishes the stream from these four
  // attributes. It reads them from the source's stream attribute store, and the
  // descriptor carries the same values so both entry points agree.
  hr = MFCreateAttributes(&streamAttributes_, 8);
  if (FAILED(hr)) return hr;
  hr = ApplyDeviceStreamAttributes(streamAttributes_);
  if (FAILED(hr)) return hr;
  hr = ApplyDeviceStreamAttributes(streamDescriptor_);
  if (FAILED(hr)) return hr;

  // Create stream object
  stream_ = new (std::nothrow) PhoneCamMediaStream(this, streamDescriptor_);
  if (!stream_) return E_OUTOFMEMORY;

  // Create presentation descriptor with single stream
  hr = MFCreatePresentationDescriptor(1, &streamDescriptor_, &presentationDescriptor_);
  if (FAILED(hr)) return hr;

  return presentationDescriptor_->SelectStream(0);
}

STDMETHODIMP PhoneCamMediaSource::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;

  if (riid == IID_IUnknown || riid == IID_IMFMediaEventGenerator || riid == IID_IMFMediaSource) {
    *ppv = static_cast<IMFMediaSource*>(this);
  } else if (riid == IID_IMFMediaSourceEx) {
    *ppv = static_cast<IMFMediaSourceEx*>(this);
  } else if (riid == IID_IMFGetService) {
    *ppv = static_cast<IMFGetService*>(this);
  } else if (riid == __uuidof(IKsControl)) {
    *ppv = static_cast<IKsControl*>(this);
  } else if (riid == IID_IMFAttributes) {
    *ppv = static_cast<IMFAttributes*>(this);
  } else {
    return E_NOINTERFACE;
  }

  AddRef();
  return S_OK;
}

STDMETHODIMP_(ULONG) PhoneCamMediaSource::AddRef() {
  return refCount_.fetch_add(1) + 1;
}

STDMETHODIMP_(ULONG) PhoneCamMediaSource::Release() {
  ULONG count = refCount_.fetch_sub(1) - 1;
  if (count == 0) {
    delete this;
  }
  return count;
}

STDMETHODIMP PhoneCamMediaSource::BeginGetEvent(IMFAsyncCallback* pCallback, IUnknown* punkState) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  return eventQueue_->BeginGetEvent(pCallback, punkState);
}

STDMETHODIMP PhoneCamMediaSource::EndGetEvent(IMFAsyncResult* pResult, IMFMediaEvent** ppEvent) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  return eventQueue_->EndGetEvent(pResult, ppEvent);
}

STDMETHODIMP PhoneCamMediaSource::GetEvent(DWORD dwFlags, IMFMediaEvent** ppEvent) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  return eventQueue_->GetEvent(dwFlags, ppEvent);
}

STDMETHODIMP PhoneCamMediaSource::QueueEvent(MediaEventType met, REFGUID guidExtendedType, HRESULT hrStatus, const PROPVARIANT* pvValue) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!eventQueue_) return MF_E_SHUTDOWN;
  return eventQueue_->QueueEventParamVar(met, guidExtendedType, hrStatus, pvValue);
}

STDMETHODIMP PhoneCamMediaSource::GetCharacteristics(DWORD* pdwCharacteristics) {
  if (!pdwCharacteristics) return E_POINTER;
  *pdwCharacteristics = MFMEDIASOURCE_IS_LIVE;
  return S_OK;
}

STDMETHODIMP PhoneCamMediaSource::CreatePresentationDescriptor(IMFPresentationDescriptor** ppPresentationDescriptor) {
  if (!ppPresentationDescriptor) return E_POINTER;
  *ppPresentationDescriptor = nullptr;
  std::lock_guard<std::mutex> lock(mutex_);
  if (isShutdown_ || !presentationDescriptor_) return MF_E_SHUTDOWN;
  return presentationDescriptor_->Clone(ppPresentationDescriptor);
}

STDMETHODIMP PhoneCamMediaSource::Start(IMFPresentationDescriptor* pPresentationDescriptor, const GUID* pguidTimeFormat, const PROPVARIANT* pvarStartPosition) {
  (void)pguidTimeFormat;
  std::lock_guard<std::mutex> lock(mutex_);
  if (isShutdown_) return MF_E_SHUTDOWN;

  // Determine selected media type if changed
  if (pPresentationDescriptor && stream_) {
    BOOL selected = FALSE;
    IMFStreamDescriptor* pSD = nullptr;
    if (SUCCEEDED(pPresentationDescriptor->GetStreamDescriptorByIndex(0, &selected, &pSD)) && selected && pSD) {
      IMFMediaTypeHandler* pHandler = nullptr;
      if (SUCCEEDED(pSD->GetMediaTypeHandler(&pHandler))) {
        IMFMediaType* pType = nullptr;
        if (SUCCEEDED(pHandler->GetCurrentMediaType(&pType)) && pType) {
          UINT32 w = 0, h = 0, num = 0, den = 0;
          if (SUCCEEDED(MFGetAttributeSize(pType, MF_MT_FRAME_SIZE, &w, &h))) {
            MFGetAttributeRatio(pType, MF_MT_FRAME_RATE, &num, &den);
            stream_->SetActiveFormat(w, h, (den > 0 ? num / den : 30));
          }
          pType->Release();
        }
        pHandler->Release();
      }
      pSD->Release();
    }
  }

  if (stream_) {
    stream_->Start();
  }

  if (eventQueue_) {
    // Deliver MENewStream with the active stream
    eventQueue_->QueueEventParamUnk(MENewStream, GUID_NULL, S_OK, stream_);
    // Deliver MESourceStarted
    eventQueue_->QueueEventParamVar(MESourceStarted, GUID_NULL, S_OK, pvarStartPosition);
  }
  return S_OK;
}

STDMETHODIMP PhoneCamMediaSource::Stop() {
  std::lock_guard<std::mutex> lock(mutex_);
  if (isShutdown_) return MF_E_SHUTDOWN;

  if (stream_) stream_->Stop();
  if (presentationDescriptor_) presentationDescriptor_->DeselectStream(0);

  PROPVARIANT time;
  PropVariantInit(&time);
  InitPropVariantFromInt64(MFGetSystemTime(), &time);
  if (eventQueue_) eventQueue_->QueueEventParamVar(MESourceStopped, GUID_NULL, S_OK, &time);
  PropVariantClear(&time);
  return S_OK;
}

STDMETHODIMP PhoneCamMediaSource::Pause() {
  (void)0;
  return MF_E_INVALID_STATE_TRANSITION;
}

STDMETHODIMP PhoneCamMediaSource::Shutdown() {
  std::lock_guard<std::mutex> lock(mutex_);
  if (isShutdown_) return S_OK;
  isShutdown_ = true;

  if (stream_) {
    stream_->Shutdown();
    stream_->Release();
    stream_ = nullptr;
  }
  if (presentationDescriptor_) {
    presentationDescriptor_->Release();
    presentationDescriptor_ = nullptr;
  }
  if (streamDescriptor_) {
    streamDescriptor_->Release();
    streamDescriptor_ = nullptr;
  }
  if (attributes_) {
    attributes_->Release();
    attributes_ = nullptr;
  }
  if (streamAttributes_) {
    streamAttributes_->Release();
    streamAttributes_ = nullptr;
  }
  if (eventQueue_) {
    eventQueue_->Shutdown();
    eventQueue_->Release();
    eventQueue_ = nullptr;
  }
  return S_OK;
}

STDMETHODIMP PhoneCamMediaSource::GetSourceAttributes(IMFAttributes** ppAttributes) {
  if (!ppAttributes) return E_POINTER;
  *ppAttributes = nullptr;
  std::lock_guard<std::mutex> lock(mutex_);
  if (isShutdown_) return MF_E_SHUTDOWN;

  // The attribute store *is* the source: the Frame Server reads the sensor
  // profile collection off this interface.
  return QueryInterface(IID_PPV_ARGS(ppAttributes));
}

STDMETHODIMP PhoneCamMediaSource::GetStreamAttributes(DWORD dwStreamIdentifier, IMFAttributes** ppAttributes) {
  if (!ppAttributes) return E_POINTER;
  *ppAttributes = nullptr;
  if (dwStreamIdentifier != 0) return MF_E_INVALIDSTREAMNUMBER;
  std::lock_guard<std::mutex> lock(mutex_);
  if (isShutdown_ || !streamAttributes_) return MF_E_SHUTDOWN;
  *ppAttributes = streamAttributes_;
  (*ppAttributes)->AddRef();
  return S_OK;
}

STDMETHODIMP PhoneCamMediaSource::SetD3DManager(IUnknown* pManager) {
  (void)pManager;
  return S_OK;
}

// IMFAttributes: delegate to the store the Frame Server reads.

#define PHONECAM_SOURCE_ATTR_FWD(name, args, call)      \
  STDMETHODIMP PhoneCamMediaSource::name args {          \
    if (!attributes_) return MF_E_SHUTDOWN;              \
    return attributes_->call;                            \
  }

PHONECAM_SOURCE_ATTR_FWD(GetItem, (REFGUID guidKey, PROPVARIANT* pValue), GetItem(guidKey, pValue))
PHONECAM_SOURCE_ATTR_FWD(GetItemType, (REFGUID guidKey, MF_ATTRIBUTE_TYPE* pType), GetItemType(guidKey, pType))
PHONECAM_SOURCE_ATTR_FWD(CompareItem, (REFGUID guidKey, REFPROPVARIANT Value, BOOL* pbResult), CompareItem(guidKey, Value, pbResult))
PHONECAM_SOURCE_ATTR_FWD(Compare, (IMFAttributes* pTheirs, MF_ATTRIBUTES_MATCH_TYPE MatchType, BOOL* pbResult), Compare(pTheirs, MatchType, pbResult))
PHONECAM_SOURCE_ATTR_FWD(GetUINT32, (REFGUID guidKey, UINT32* punValue), GetUINT32(guidKey, punValue))
PHONECAM_SOURCE_ATTR_FWD(GetUINT64, (REFGUID guidKey, UINT64* punValue), GetUINT64(guidKey, punValue))
PHONECAM_SOURCE_ATTR_FWD(GetDouble, (REFGUID guidKey, double* pfValue), GetDouble(guidKey, pfValue))
PHONECAM_SOURCE_ATTR_FWD(GetGUID, (REFGUID guidKey, GUID* pguidValue), GetGUID(guidKey, pguidValue))
PHONECAM_SOURCE_ATTR_FWD(GetStringLength, (REFGUID guidKey, UINT32* pcchLength), GetStringLength(guidKey, pcchLength))
PHONECAM_SOURCE_ATTR_FWD(GetString, (REFGUID guidKey, LPWSTR pwszValue, UINT32 cchBufSize, UINT32* pcchLength), GetString(guidKey, pwszValue, cchBufSize, pcchLength))
PHONECAM_SOURCE_ATTR_FWD(GetAllocatedString, (REFGUID guidKey, LPWSTR* ppwszValue, UINT32* pcchLength), GetAllocatedString(guidKey, ppwszValue, pcchLength))
PHONECAM_SOURCE_ATTR_FWD(GetBlobSize, (REFGUID guidKey, UINT32* pcbBlobSize), GetBlobSize(guidKey, pcbBlobSize))
PHONECAM_SOURCE_ATTR_FWD(GetBlob, (REFGUID guidKey, UINT8* pBuf, UINT32 cbBufSize, UINT32* pcbBlobSize), GetBlob(guidKey, pBuf, cbBufSize, pcbBlobSize))
PHONECAM_SOURCE_ATTR_FWD(GetAllocatedBlob, (REFGUID guidKey, UINT8** ppBuf, UINT32* pcbSize), GetAllocatedBlob(guidKey, ppBuf, pcbSize))
PHONECAM_SOURCE_ATTR_FWD(GetUnknown, (REFGUID guidKey, REFIID riid, LPVOID* ppv), GetUnknown(guidKey, riid, ppv))
PHONECAM_SOURCE_ATTR_FWD(SetItem, (REFGUID guidKey, REFPROPVARIANT Value), SetItem(guidKey, Value))
PHONECAM_SOURCE_ATTR_FWD(DeleteItem, (REFGUID guidKey), DeleteItem(guidKey))
PHONECAM_SOURCE_ATTR_FWD(SetUINT32, (REFGUID guidKey, UINT32 unValue), SetUINT32(guidKey, unValue))
PHONECAM_SOURCE_ATTR_FWD(SetUINT64, (REFGUID guidKey, UINT64 unValue), SetUINT64(guidKey, unValue))
PHONECAM_SOURCE_ATTR_FWD(SetDouble, (REFGUID guidKey, double fValue), SetDouble(guidKey, fValue))
PHONECAM_SOURCE_ATTR_FWD(SetGUID, (REFGUID guidKey, REFGUID guidValue), SetGUID(guidKey, guidValue))
PHONECAM_SOURCE_ATTR_FWD(SetString, (REFGUID guidKey, LPCWSTR wszValue), SetString(guidKey, wszValue))
PHONECAM_SOURCE_ATTR_FWD(SetBlob, (REFGUID guidKey, const UINT8* pBuf, UINT32 cbBufSize), SetBlob(guidKey, pBuf, cbBufSize))
PHONECAM_SOURCE_ATTR_FWD(SetUnknown, (REFGUID guidKey, IUnknown* pUnknown), SetUnknown(guidKey, pUnknown))
PHONECAM_SOURCE_ATTR_FWD(LockStore, (), LockStore())
PHONECAM_SOURCE_ATTR_FWD(UnlockStore, (), UnlockStore())
PHONECAM_SOURCE_ATTR_FWD(GetCount, (UINT32* pcItems), GetCount(pcItems))
PHONECAM_SOURCE_ATTR_FWD(GetItemByIndex, (UINT32 unIndex, GUID* pguidKey, PROPVARIANT* pValue), GetItemByIndex(unIndex, pguidKey, pValue))
PHONECAM_SOURCE_ATTR_FWD(CopyAllItems, (IMFAttributes* pDest), CopyAllItems(pDest))

STDMETHODIMP PhoneCamMediaSource::DeleteAllItems() {
  if (!attributes_) return MF_E_SHUTDOWN;
  return attributes_->DeleteAllItems();
}

#undef PHONECAM_SOURCE_ATTR_FWD

STDMETHODIMP PhoneCamMediaSource::GetService(REFGUID guidService, REFIID riid, LPVOID* ppvObject) {
  (void)guidService;
  (void)riid;
  if (ppvObject) *ppvObject = nullptr;
  return MF_E_UNSUPPORTED_SERVICE;
}

STDMETHODIMP PhoneCamMediaSource::KsProperty(PKSPROPERTY Property, ULONG PropertyLength, LPVOID PropertyData, ULONG DataLength, ULONG* BytesReturned) {
  (void)Property; (void)PropertyLength; (void)PropertyData; (void)DataLength;
  if (BytesReturned) *BytesReturned = 0;
  return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
}

STDMETHODIMP PhoneCamMediaSource::KsMethod(PKSMETHOD Method, ULONG MethodLength, LPVOID MethodData, ULONG DataLength, ULONG* BytesReturned) {
  (void)Method; (void)MethodLength; (void)MethodData; (void)DataLength;
  if (BytesReturned) *BytesReturned = 0;
  return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
}

STDMETHODIMP PhoneCamMediaSource::KsEvent(PKSEVENT Event, ULONG EventLength, LPVOID EventData, ULONG DataLength, ULONG* BytesReturned) {
  (void)Event; (void)EventLength; (void)EventData; (void)DataLength;
  if (BytesReturned) *BytesReturned = 0;
  return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
}

// ============================================================================
// PhoneCamActivate
// ============================================================================

PhoneCamActivate::PhoneCamActivate() {
  AddRefModule();
}

PhoneCamActivate::~PhoneCamActivate() {
  // Only drop our reference. The Frame Server owns the source's lifetime and
  // calls IMFMediaSource::Shutdown itself; shutting it down here would kill a
  // source it still holds, and every later call would return MF_E_SHUTDOWN.
  std::lock_guard<std::mutex> lock(mutex_);
  if (activeSource_) {
    activeSource_->Release();
    activeSource_ = nullptr;
  }
  if (attributes_) {
    attributes_->Release();
    attributes_ = nullptr;
  }
  ReleaseModule();
}

HRESULT PhoneCamActivate::Initialize() {
  HRESULT hr = MFCreateAttributes(&attributes_, 16);
  if (FAILED(hr)) return hr;

  // PhoneCam is a synthetic camera: it wraps no physical device, so there are
  // no associated camera sources to provide. Asking the Frame Server for them
  // makes activation fail with ERROR_PATH_NOT_FOUND.
  attributes_->SetUINT32(MF_VIRTUALCAMERA_PROVIDE_ASSOCIATED_CAMERA_SOURCES, 0);
  attributes_->SetGUID(MFT_TRANSFORM_CLSID_Attribute, CLSID_PhoneCamVirtualCamera);
  attributes_->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
  attributes_->SetString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, kVirtualCameraFriendlyName);
  return S_OK;
}

STDMETHODIMP PhoneCamActivate::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;

  if (riid == IID_IUnknown || riid == IID_IMFActivate) {
    *ppv = static_cast<IMFActivate*>(this);
  } else if (riid == IID_IMFAttributes) {
    *ppv = static_cast<IMFAttributes*>(this);
  } else {
    return E_NOINTERFACE;
  }

  AddRef();
  return S_OK;
}

STDMETHODIMP_(ULONG) PhoneCamActivate::AddRef() {
  return refCount_.fetch_add(1) + 1;
}

STDMETHODIMP_(ULONG) PhoneCamActivate::Release() {
  ULONG count = refCount_.fetch_sub(1) - 1;
  if (count == 0) {
    delete this;
  }
  return count;
}

STDMETHODIMP PhoneCamActivate::GetItem(REFGUID guidKey, PROPVARIANT* pValue) {
  return attributes_ ? attributes_->GetItem(guidKey, pValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetItemType(REFGUID guidKey, MF_ATTRIBUTE_TYPE* pType) {
  return attributes_ ? attributes_->GetItemType(guidKey, pType) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::CompareItem(REFGUID guidKey, REFPROPVARIANT Value, BOOL* pbResult) {
  return attributes_ ? attributes_->CompareItem(guidKey, Value, pbResult) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::Compare(IMFAttributes* pTheirs, MF_ATTRIBUTES_MATCH_TYPE MatchType, BOOL* pbResult) {
  return attributes_ ? attributes_->Compare(pTheirs, MatchType, pbResult) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetUINT32(REFGUID guidKey, UINT32* punValue) {
  return attributes_ ? attributes_->GetUINT32(guidKey, punValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetUINT64(REFGUID guidKey, UINT64* punValue) {
  return attributes_ ? attributes_->GetUINT64(guidKey, punValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetDouble(REFGUID guidKey, double* pfValue) {
  return attributes_ ? attributes_->GetDouble(guidKey, pfValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetGUID(REFGUID guidKey, GUID* pguidValue) {
  return attributes_ ? attributes_->GetGUID(guidKey, pguidValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetStringLength(REFGUID guidKey, UINT32* pcchLength) {
  return attributes_ ? attributes_->GetStringLength(guidKey, pcchLength) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetString(REFGUID guidKey, LPWSTR pwszValue, UINT32 cchBufSize, UINT32* pcchLength) {
  return attributes_ ? attributes_->GetString(guidKey, pwszValue, cchBufSize, pcchLength) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetAllocatedString(REFGUID guidKey, LPWSTR* ppwszValue, UINT32* pcchLength) {
  return attributes_ ? attributes_->GetAllocatedString(guidKey, ppwszValue, pcchLength) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetBlobSize(REFGUID guidKey, UINT32* pcbBlobSize) {
  return attributes_ ? attributes_->GetBlobSize(guidKey, pcbBlobSize) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetBlob(REFGUID guidKey, UINT8* pBuf, UINT32 cbBufSize, UINT32* pcbBlobSize) {
  return attributes_ ? attributes_->GetBlob(guidKey, pBuf, cbBufSize, pcbBlobSize) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetAllocatedBlob(REFGUID guidKey, UINT8** ppBuf, UINT32* pcbSize) {
  return attributes_ ? attributes_->GetAllocatedBlob(guidKey, ppBuf, pcbSize) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetUnknown(REFGUID guidKey, REFIID riid, LPVOID* ppv) {
  return attributes_ ? attributes_->GetUnknown(guidKey, riid, ppv) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::ActivateObject(REFIID riid, void** ppv) {
  VcamLog("ActivateObject");
  if (!ppv) return E_POINTER;
  *ppv = nullptr;
  std::lock_guard<std::mutex> lock(mutex_);

  if (!activeSource_) {
    activeSource_ = new (std::nothrow) PhoneCamMediaSource();
    if (!activeSource_) return E_OUTOFMEMORY;
    HRESULT hr = activeSource_->Initialize();
    VcamLogHr("PhoneCamMediaSource::Initialize", hr);
    if (FAILED(hr)) {
      activeSource_->Release();
      activeSource_ = nullptr;
      return hr;
    }
  }

  HRESULT hr = activeSource_->QueryInterface(riid, ppv);
  VcamLogHr("ActivateObject: QueryInterface", hr);
  return hr;
}

STDMETHODIMP PhoneCamActivate::ShutdownObject() {
  // The Frame Server calls this to ask us to shut down the source it activated.
  std::lock_guard<std::mutex> lock(mutex_);
  if (activeSource_) {
    activeSource_->Shutdown();
    activeSource_->Release();
    activeSource_ = nullptr;
  }
  return S_OK;
}

STDMETHODIMP PhoneCamActivate::DetachObject() {
  std::lock_guard<std::mutex> lock(mutex_);
  if (activeSource_) {
    activeSource_->Release();
    activeSource_ = nullptr;
  }
  return S_OK;
}
STDMETHODIMP PhoneCamActivate::SetItem(REFGUID guidKey, REFPROPVARIANT Value) {
  return attributes_ ? attributes_->SetItem(guidKey, Value) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::DeleteItem(REFGUID guidKey) {
  return attributes_ ? attributes_->DeleteItem(guidKey) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::DeleteAllItems() {
  return attributes_ ? attributes_->DeleteAllItems() : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::SetUINT32(REFGUID guidKey, UINT32 unValue) {
  return attributes_ ? attributes_->SetUINT32(guidKey, unValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::SetUINT64(REFGUID guidKey, UINT64 unValue) {
  return attributes_ ? attributes_->SetUINT64(guidKey, unValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::SetDouble(REFGUID guidKey, double fValue) {
  return attributes_ ? attributes_->SetDouble(guidKey, fValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::SetGUID(REFGUID guidKey, REFGUID guidValue) {
  return attributes_ ? attributes_->SetGUID(guidKey, guidValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::SetString(REFGUID guidKey, LPCWSTR wszValue) {
  return attributes_ ? attributes_->SetString(guidKey, wszValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::SetBlob(REFGUID guidKey, const UINT8* pBuf, UINT32 cbBufSize) {
  return attributes_ ? attributes_->SetBlob(guidKey, pBuf, cbBufSize) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::SetUnknown(REFGUID guidKey, IUnknown* pUnknown) {
  return attributes_ ? attributes_->SetUnknown(guidKey, pUnknown) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::LockStore() {
  return attributes_ ? attributes_->LockStore() : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::UnlockStore() {
  return attributes_ ? attributes_->UnlockStore() : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetCount(UINT32* pcItems) {
  return attributes_ ? attributes_->GetCount(pcItems) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::GetItemByIndex(UINT32 unIndex, GUID* pguidKey, PROPVARIANT* pValue) {
  return attributes_ ? attributes_->GetItemByIndex(unIndex, pguidKey, pValue) : E_UNEXPECTED;
}
STDMETHODIMP PhoneCamActivate::CopyAllItems(IMFAttributes* pDest) {
  return attributes_ ? attributes_->CopyAllItems(pDest) : E_UNEXPECTED;
}

// ============================================================================
// PhoneCamClassFactory
// ============================================================================

STDMETHODIMP PhoneCamClassFactory::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;

  if (riid == IID_IUnknown || riid == IID_IClassFactory) {
    *ppv = static_cast<IClassFactory*>(this);
  } else {
    return E_NOINTERFACE;
  }

  AddRef();
  return S_OK;
}

STDMETHODIMP_(ULONG) PhoneCamClassFactory::AddRef() {
  return refCount_.fetch_add(1) + 1;
}

STDMETHODIMP_(ULONG) PhoneCamClassFactory::Release() {
  ULONG count = refCount_.fetch_sub(1) - 1;
  if (count == 0) {
    delete this;
  }
  return count;
}

STDMETHODIMP PhoneCamClassFactory::CreateInstance(IUnknown* pUnkOuter, REFIID riid, void** ppv) {
  if (pUnkOuter != nullptr) return CLASS_E_NOAGGREGATION;
  if (!ppv) return E_POINTER;
  *ppv = nullptr;

  PhoneCamActivate* activate = new (std::nothrow) PhoneCamActivate();
  if (!activate) return E_OUTOFMEMORY;

  HRESULT hr = activate->Initialize();
  if (FAILED(hr)) {
    activate->Release();
    return hr;
  }

  hr = activate->QueryInterface(riid, ppv);
  VcamLogHr("ClassFactory::CreateInstance", hr);
  activate->Release();
  return hr;
}

STDMETHODIMP PhoneCamClassFactory::LockServer(BOOL fLock) {
  if (fLock) {
    AddRefModule();
  } else {
    ReleaseModule();
  }
  return S_OK;
}

}  // namespace phonecam::vcam
