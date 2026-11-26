/**
 * PhoneCam Windows Media Foundation Virtual Camera.
 *
 * Implements an in-process COM server providing an IMFMediaSourceEx that delivers
 * NV12 camera frames from the PhoneCam shared-memory frame buffer (frame_shm.hpp).
 *
 * Registered as a system or user software camera source via Windows 11 MFCreateVirtualCamera.
 */

#pragma once

#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfobjects.h>
#include <mferror.h>
#include <mftransform.h>
#include <ks.h>
#include <ksproxy.h>
#include <initguid.h>

#include <atomic>
#include <mutex>
#include <vector>
#include <string>

#include "phonecam/frame_shm.hpp"
#include "placeholder_generator.hpp"

namespace phonecam::vcam {

// {B7593649-C791-4966-9321-7B8B1CE54D38}
DEFINE_GUID(CLSID_PhoneCamVirtualCamera,
    0xb7593649, 0xc791, 0x4966, 0x93, 0x21, 0x7b, 0x8b, 0x1c, 0xe5, 0x4d, 0x38);
DEFINE_GUID(MF_VIRTUALCAMERA_PROVIDE_ASSOCIATED_CAMERA_SOURCES,
    0xF0273718, 0x4A4D, 0x4AC5, 0xA1, 0x5D, 0x30, 0x5E, 0xB5, 0xE9, 0x06, 0x67);

inline constexpr const wchar_t* kClsidVirtualCameraString = L"{B7593649-C791-4966-9321-7B8B1CE54D38}";
inline constexpr const wchar_t* kVirtualCameraFriendlyName = L"PhoneCam Camera";

void AddRefModule();
void ReleaseModule();
bool CanUnloadNow();

class PhoneCamMediaSource;

/**
 * Single-stream video source delivering NV12 IMFSamples.
 */
class PhoneCamMediaStream : public IMFMediaStream2 {
 public:
  PhoneCamMediaStream(PhoneCamMediaSource* source, IMFStreamDescriptor* pDescriptor);
  virtual ~PhoneCamMediaStream();

  // IUnknown
  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  // IMFMediaEventGenerator
  STDMETHODIMP BeginGetEvent(IMFAsyncCallback* pCallback, IUnknown* punkState) override;
  STDMETHODIMP EndGetEvent(IMFAsyncResult* pResult, IMFMediaEvent** ppEvent) override;
  STDMETHODIMP GetEvent(DWORD dwFlags, IMFMediaEvent** ppEvent) override;
  STDMETHODIMP QueueEvent(MediaEventType met, REFGUID guidExtendedType, HRESULT hrStatus, const PROPVARIANT* pvValue) override;

  // IMFMediaStream
  STDMETHODIMP GetMediaSource(IMFMediaSource** ppMediaSource) override;
  STDMETHODIMP GetStreamDescriptor(IMFStreamDescriptor** ppStreamDescriptor) override;
  STDMETHODIMP RequestSample(IUnknown* pToken) override;

  // IMFMediaStream2
  STDMETHODIMP SetStreamState(MF_STREAM_STATE value) override;
  STDMETHODIMP GetStreamState(MF_STREAM_STATE* value) override;

  // Internal lifecycle
  HRESULT Start();
  HRESULT Stop();
  HRESULT Pause();
  HRESULT Shutdown();
  void SetActiveFormat(uint32_t width, uint32_t height, uint32_t fps);

 private:
  HRESULT DeliverFrame(IUnknown* pToken);
  bool WaitPacing100ns(LONGLONG wait100ns);

  std::atomic<ULONG> refCount_{1};
  std::mutex mutex_;
  std::mutex paceMutex_;
  HANDLE paceTimer_{nullptr};
  HANDLE stopEvent_{nullptr};
  PhoneCamMediaSource* source_{nullptr};
  IMFStreamDescriptor* streamDescriptor_{nullptr};
  IMFMediaEventQueue* eventQueue_{nullptr};
  MF_STREAM_STATE streamState_{MF_STREAM_STATE_STOPPED};

  uint32_t width_{1920};
  uint32_t height_{1080};
  uint32_t fps_{30};
  uint64_t sampleDuration100ns_{333333};
  LONGLONG lastSampleTime100ns_{0};

  /**
   * When the next sample is due, in 100 ns units; zero means "now".
   *
   * A camera is a fixed-rate source, and the pipeline pulls samples as fast as
   * the source hands them over. Answering every pull immediately runs the
   * stream's timeline ahead of the wall clock, so the timestamp clamp below
   * ends up stamping each repeated frame with the moment it was asked for — a
   * 30 fps camera then looks like a 300 fps one, which is what makes OBS and
   * NVIDIA Broadcast drop and repeat frames on a stream the in-app preview
   * shows smoothly.
   */
  LONGLONG nextSampleDue100ns_{0};

  /** Wall-clock start time (100 ns) of the previous successful DeliverFrame(). */
  LONGLONG lastDeliverStart100ns_{0};

  /** Shared-memory frameIndex of the last live frame delivered, for early wake. */
  uint64_t lastDeliveredFrameIndex_{0};

  /** True if the previous delivered sample was a live frame rather than a placeholder. */
  bool lastDeliveredLive_{false};

  /**
   * Presentation time of the frame about to be delivered, in microseconds.
   *
   * Zero means "no frame time available" (a placeholder, or a buffer without a
   * PTS), in which case the sample is stamped from the wall clock as before. A
   * consumer that pulls faster than the phone produces would otherwise see each
   * repeated frame as a new moment in time, and a recording built from those
   * timestamps stutters even though the live picture is smooth.
   */
  uint64_t deliveredPtsUs_{0};

  /** PTS of the previous delivered live frame, for delta-based stamping. */
  uint64_t lastDeliveredPtsUs_{0};

  /** True once the stream's PTS timeline has been anchored to the wall clock. */
  bool ptsAnchored_{false};

  phonecam::shm::FrameReader frameReader_;
  bool shmOpened_{false};
  bool forceTestPattern_{false};

  /** Set while the placeholder is being shown, so the log records the
   *  transition into and out of it rather than one line per delivered frame. */
  bool loggedPlaceholder_{false};

  /** Last read result and snapshot, reported when the placeholder appears. */
  bool lastReadOk_{false};
  phonecam::shm::FrameReader::Snapshot lastSnapshot_{};
  std::vector<uint8_t> cachedNv12_;

  /** Reusable scratch buffer for shared-memory NV12 reads (zero per-frame heap alloc). */
  std::vector<uint8_t> shmBuffer_;

  /** Scratch buffer for output mirror/rotation transforms. */
  std::vector<uint8_t> transformedNv12_;

  /** Scratch for a frame scaled to the format the consumer asked for. */
  std::vector<uint8_t> scaledNv12_;
};

/**
 * IMFMediaSourceEx implementation for the PhoneCam Virtual Camera.
 */
class PhoneCamMediaSource : public IMFMediaSourceEx,
                            public IMFGetService,
                            public IKsControl,
                            public IMFAttributes {
 public:
  PhoneCamMediaSource();
  virtual ~PhoneCamMediaSource();

  HRESULT Initialize();

  // IUnknown
  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  // IMFMediaEventGenerator
  STDMETHODIMP BeginGetEvent(IMFAsyncCallback* pCallback, IUnknown* punkState) override;
  STDMETHODIMP EndGetEvent(IMFAsyncResult* pResult, IMFMediaEvent** ppEvent) override;
  STDMETHODIMP GetEvent(DWORD dwFlags, IMFMediaEvent** ppEvent) override;
  STDMETHODIMP QueueEvent(MediaEventType met, REFGUID guidExtendedType, HRESULT hrStatus, const PROPVARIANT* pvValue) override;

  // IMFMediaSource
  STDMETHODIMP GetCharacteristics(DWORD* pdwCharacteristics) override;
  STDMETHODIMP CreatePresentationDescriptor(IMFPresentationDescriptor** ppPresentationDescriptor) override;
  STDMETHODIMP Start(IMFPresentationDescriptor* pPresentationDescriptor, const GUID* pguidTimeFormat, const PROPVARIANT* pvarStartPosition) override;
  STDMETHODIMP Stop() override;
  STDMETHODIMP Pause() override;
  STDMETHODIMP Shutdown() override;

  // IMFMediaSourceEx
  STDMETHODIMP GetSourceAttributes(IMFAttributes** ppAttributes) override;
  STDMETHODIMP GetStreamAttributes(DWORD dwStreamIdentifier, IMFAttributes** ppAttributes) override;
  STDMETHODIMP SetD3DManager(IUnknown* pManager) override;

  // IMFGetService
  STDMETHODIMP GetService(REFGUID guidService, REFIID riid, LPVOID* ppvObject) override;

  // IKsControl
  STDMETHODIMP KsProperty(PKSPROPERTY Property, ULONG PropertyLength, LPVOID PropertyData, ULONG DataLength, ULONG* BytesReturned) override;
  STDMETHODIMP KsMethod(PKSMETHOD Method, ULONG MethodLength, LPVOID MethodData, ULONG DataLength, ULONG* BytesReturned) override;
  STDMETHODIMP KsEvent(PKSEVENT Event, ULONG EventLength, LPVOID EventData, ULONG DataLength, ULONG* BytesReturned) override;

  // IMFAttributes: the source is itself the attribute store the Frame Server reads,
  // which is how the Media Foundation sample source publishes its sensor profiles.
  STDMETHODIMP GetItem(REFGUID guidKey, PROPVARIANT* pValue) override;
  STDMETHODIMP GetItemType(REFGUID guidKey, MF_ATTRIBUTE_TYPE* pType) override;
  STDMETHODIMP CompareItem(REFGUID guidKey, REFPROPVARIANT Value, BOOL* pbResult) override;
  STDMETHODIMP Compare(IMFAttributes* pTheirs, MF_ATTRIBUTES_MATCH_TYPE MatchType, BOOL* pbResult) override;
  STDMETHODIMP GetUINT32(REFGUID guidKey, UINT32* punValue) override;
  STDMETHODIMP GetUINT64(REFGUID guidKey, UINT64* punValue) override;
  STDMETHODIMP GetDouble(REFGUID guidKey, double* pfValue) override;
  STDMETHODIMP GetGUID(REFGUID guidKey, GUID* pguidValue) override;
  STDMETHODIMP GetStringLength(REFGUID guidKey, UINT32* pcchLength) override;
  STDMETHODIMP GetString(REFGUID guidKey, LPWSTR pwszValue, UINT32 cchBufSize, UINT32* pcchLength) override;
  STDMETHODIMP GetAllocatedString(REFGUID guidKey, LPWSTR* ppwszValue, UINT32* pcchLength) override;
  STDMETHODIMP GetBlobSize(REFGUID guidKey, UINT32* pcbBlobSize) override;
  STDMETHODIMP GetBlob(REFGUID guidKey, UINT8* pBuf, UINT32 cbBufSize, UINT32* pcbBlobSize) override;
  STDMETHODIMP GetAllocatedBlob(REFGUID guidKey, UINT8** ppBuf, UINT32* pcbSize) override;
  STDMETHODIMP GetUnknown(REFGUID guidKey, REFIID riid, LPVOID* ppv) override;
  STDMETHODIMP SetItem(REFGUID guidKey, REFPROPVARIANT Value) override;
  STDMETHODIMP DeleteItem(REFGUID guidKey) override;
  STDMETHODIMP DeleteAllItems() override;
  STDMETHODIMP SetUINT32(REFGUID guidKey, UINT32 unValue) override;
  STDMETHODIMP SetUINT64(REFGUID guidKey, UINT64 unValue) override;
  STDMETHODIMP SetDouble(REFGUID guidKey, double fValue) override;
  STDMETHODIMP SetGUID(REFGUID guidKey, REFGUID guidValue) override;
  STDMETHODIMP SetString(REFGUID guidKey, LPCWSTR wszValue) override;
  STDMETHODIMP SetBlob(REFGUID guidKey, const UINT8* pBuf, UINT32 cbBufSize) override;
  STDMETHODIMP SetUnknown(REFGUID guidKey, IUnknown* pUnknown) override;
  STDMETHODIMP LockStore() override;
  STDMETHODIMP UnlockStore() override;
  STDMETHODIMP GetCount(UINT32* pcItems) override;
  STDMETHODIMP GetItemByIndex(UINT32 unIndex, GUID* pguidKey, PROPVARIANT* pValue) override;
  STDMETHODIMP CopyAllItems(IMFAttributes* pDest) override;

 private:
  HRESULT BuildDescriptors();

  std::atomic<ULONG> refCount_{1};
  std::mutex mutex_;
  bool isShutdown_{false};

  IMFMediaEventQueue* eventQueue_{nullptr};
  IMFAttributes* attributes_{nullptr};
  IMFAttributes* streamAttributes_{nullptr};
  IMFPresentationDescriptor* presentationDescriptor_{nullptr};
  IMFStreamDescriptor* streamDescriptor_{nullptr};
  PhoneCamMediaStream* stream_{nullptr};
};

/**
 * IMFActivate implementation: instantiated by COM ClassFactory and passed to MF.
 */
class PhoneCamActivate : public IMFActivate {
 public:
  PhoneCamActivate();
  virtual ~PhoneCamActivate();

  HRESULT Initialize();

  // IUnknown
  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  // IMFActivate
  STDMETHODIMP ActivateObject(REFIID riid, void** ppv) override;
  STDMETHODIMP ShutdownObject() override;
  STDMETHODIMP DetachObject() override;

  // IMFAttributes delegation
  STDMETHODIMP GetItem(REFGUID guidKey, PROPVARIANT* pValue) override;
  STDMETHODIMP GetItemType(REFGUID guidKey, MF_ATTRIBUTE_TYPE* pType) override;
  STDMETHODIMP CompareItem(REFGUID guidKey, REFPROPVARIANT Value, BOOL* pbResult) override;
  STDMETHODIMP Compare(IMFAttributes* pTheirs, MF_ATTRIBUTES_MATCH_TYPE MatchType, BOOL* pbResult) override;
  STDMETHODIMP GetUINT32(REFGUID guidKey, UINT32* punValue) override;
  STDMETHODIMP GetUINT64(REFGUID guidKey, UINT64* punValue) override;
  STDMETHODIMP GetDouble(REFGUID guidKey, double* pfValue) override;
  STDMETHODIMP GetGUID(REFGUID guidKey, GUID* pguidValue) override;
  STDMETHODIMP GetStringLength(REFGUID guidKey, UINT32* pcchLength) override;
  STDMETHODIMP GetString(REFGUID guidKey, LPWSTR pwszValue, UINT32 cchBufSize, UINT32* pcchLength) override;
  STDMETHODIMP GetAllocatedString(REFGUID guidKey, LPWSTR* ppwszValue, UINT32* pcchLength) override;
  STDMETHODIMP GetBlobSize(REFGUID guidKey, UINT32* pcbBlobSize) override;
  STDMETHODIMP GetBlob(REFGUID guidKey, UINT8* pBuf, UINT32 cbBufSize, UINT32* pcbBlobSize) override;
  STDMETHODIMP GetAllocatedBlob(REFGUID guidKey, UINT8** ppBuf, UINT32* pcbSize) override;
  STDMETHODIMP GetUnknown(REFGUID guidKey, REFIID riid, LPVOID* ppv) override;
  STDMETHODIMP SetItem(REFGUID guidKey, REFPROPVARIANT Value) override;
  STDMETHODIMP DeleteItem(REFGUID guidKey) override;
  STDMETHODIMP DeleteAllItems() override;
  STDMETHODIMP SetUINT32(REFGUID guidKey, UINT32 unValue) override;
  STDMETHODIMP SetUINT64(REFGUID guidKey, UINT64 unValue) override;
  STDMETHODIMP SetDouble(REFGUID guidKey, double fValue) override;
  STDMETHODIMP SetGUID(REFGUID guidKey, REFGUID guidValue) override;
  STDMETHODIMP SetString(REFGUID guidKey, LPCWSTR wszValue) override;
  STDMETHODIMP SetBlob(REFGUID guidKey, const UINT8* pBuf, UINT32 cbBufSize) override;
  STDMETHODIMP SetUnknown(REFGUID guidKey, IUnknown* pUnknown) override;
  STDMETHODIMP LockStore() override;
  STDMETHODIMP UnlockStore() override;
  STDMETHODIMP GetCount(UINT32* pcItems) override;
  STDMETHODIMP GetItemByIndex(UINT32 unIndex, GUID* pguidKey, PROPVARIANT* pValue) override;
  STDMETHODIMP CopyAllItems(IMFAttributes* pDest) override;

 private:
  std::atomic<ULONG> refCount_{1};
  std::mutex mutex_;
  IMFAttributes* attributes_{nullptr};
  PhoneCamMediaSource* activeSource_{nullptr};
};

/**
 * Standard IClassFactory for creating PhoneCamActivate instances.
 */
class PhoneCamClassFactory : public IClassFactory {
 public:
  PhoneCamClassFactory() = default;
  virtual ~PhoneCamClassFactory() = default;

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  STDMETHODIMP CreateInstance(IUnknown* pUnkOuter, REFIID riid, void** ppv) override;
  STDMETHODIMP LockServer(BOOL fLock) override;

 private:
  std::atomic<ULONG> refCount_{1};
};

}  // namespace phonecam::vcam
