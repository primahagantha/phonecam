//! The read side of the shared-memory frame bus.
//!
//! Ported from `FrameReader` in `native/shared/src/frame_shm.cpp` and
//! `PhoneCamMediaStream::DeliverFrame` in `native/virtual-camera/phonecam_virtual_camera.cpp`.
//!
//! Windows Camera Frame Server loads `PhoneCamVirtualCamera.dll` which reads from
//! the file-backed mapping. This module implements the client/reader side with:
//! * Seqlock verification and retry loop (up to 64 attempts).
//! * Stale mapping detection (so resolution changes by the writer can be handled
//!   cleanly without reading out of bounds).
//! * Presentation timestamp (PTS) extraction so downstream consumers receive the
//!   frame's real capture/presentation time rather than wall clock.
//! * Reader count tracking in the shared header when mapped writable.

use crate::{FrameState, HEADER_SIZE};

#[derive(Debug, thiserror::Error)]
pub enum ReaderError {
    #[error("could not open the frame buffer file: {0}")]
    Open(String),
    #[error("could not map the frame buffer: {0}")]
    Map(String),
    #[error("invalid frame buffer header (bad magic or layout version)")]
    BadHeader,
    #[error("mapping is too small for header")]
    TooSmall,
    #[error("the frame bus is only available on Windows")]
    NotWindows,
}

/// A snapshot of metadata for a frame read from the shared buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSnapshot {
    pub width: u32,
    pub height: u32,
    pub pts_us: u64,
    pub frame_index: u64,
    pub state: FrameState,
    pub mirror: bool,
    pub rotation: u32,
}

/// A reader attached to the shared-memory frame buffer.
pub struct FrameReader {
    inner: imp::Inner,
}

impl std::fmt::Debug for FrameReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameReader")
            .field("is_stale", &self.is_stale())
            .field("mapped_capacity", &self.mapped_capacity())
            .finish()
    }
}

impl FrameReader {
    /// Opens the default frame buffer file (`C:\ProgramData\PhoneCam\framebuffer-v2.bin`).
    pub fn open_default() -> Result<Self, ReaderError> {
        Self::open(crate::BUFFER_PATH)
    }

    /// Opens the frame buffer at `path`.
    pub fn open(path: &str) -> Result<Self, ReaderError> {
        let inner = imp::Inner::open(path)?;
        Ok(Self { inner })
    }

    /// True when the mapping no longer describes the buffer the writer is using
    /// (e.g. resolution change).
    pub fn is_stale(&self) -> bool {
        self.inner.is_stale()
    }

    /// Capacity the mapping was created with, in NV12 bytes.
    pub fn mapped_capacity(&self) -> u32 {
        self.inner.mapped_capacity()
    }

    /// Checks if a newer frame exists compared to `last_frame_index`.
    pub fn has_new_frame(&self, last_frame_index: u64) -> bool {
        self.inner.has_new_frame(last_frame_index)
    }

    /// Peeks at the current frame metadata without copying frame pixels.
    pub fn peek(&self) -> Option<FrameSnapshot> {
        self.inner.peek()
    }

    /// Reads one NV12 frame and its snapshot metadata.
    ///
    /// Copies `capacity` bytes into `out`. Retries with seqlock up to 64 times.
    /// Returns `Some(snapshot)` on success, or `None` if buffer is invalid.
    pub fn read(&mut self, out: &mut Vec<u8>) -> Option<FrameSnapshot> {
        self.inner.read(out)
    }
}

/// Presentation timestamp calculation and timeline anchoring for virtual camera sample delivery.
///
/// Ported from `PhoneCamMediaStream::DeliverFrame` in `native/virtual-camera/phonecam_virtual_camera.cpp`
/// (upstream commits 5e6b22d and ec19949).
///
/// Ensures samples carry the frame's own presentation timestamp (PTS) rather than the wall clock
/// at delivery time:
/// - Repeated frames (same PTS) retain the exact same timestamp, preventing stutter/stepping
///   in consumers like OBS or browsers polling at 60 fps on a 30 fps stream.
/// - When PTS advances, sample timestamp advances by the PTS delta.
/// - Placeholder frames (PTS = 0) fall back to wall clock.
/// - Clamps to wall clock + sample_duration on huge forward jumps.
/// - Guaranteed monotonic: timestamps never step backwards.
#[derive(Debug, Clone)]
pub struct VcamTimestampTracker {
    sample_duration_100ns: i64,
    last_sample_time_100ns: i64,
    last_delivered_pts_us: u64,
    pts_anchored: bool,
}

impl VcamTimestampTracker {
    pub fn new(fps: u32) -> Self {
        let fps = if fps > 0 { fps } else { 30 };
        Self {
            sample_duration_100ns: 10_000_000 / (fps as i64),
            last_sample_time_100ns: 0,
            last_delivered_pts_us: 0,
            pts_anchored: false,
        }
    }

    /// Calculates sample timestamp in 100ns units for Media Foundation delivery.
    pub fn sample_timestamp(&mut self, delivered_pts_us: u64, wall_clock_100ns: i64) -> i64 {
        let mut now = wall_clock_100ns;
        if delivered_pts_us != 0 {
            if self.pts_anchored {
                if delivered_pts_us == self.last_delivered_pts_us {
                    // A repeated frame carries the exact same timestamp,
                    // so the consumer sees an honest still image rather than invented motion.
                    return self.last_sample_time_100ns;
                }
                let frame_delta_100ns =
                    (delivered_pts_us as i64 - self.last_delivered_pts_us as i64) * 10;
                let candidate = self.last_sample_time_100ns + frame_delta_100ns;
                now = if frame_delta_100ns > 0 {
                    candidate
                } else {
                    self.last_sample_time_100ns + self.sample_duration_100ns
                };
                if now > wall_clock_100ns + self.sample_duration_100ns {
                    now = wall_clock_100ns;
                }
            } else {
                // First live frame: anchor stream timeline to wall clock.
                self.pts_anchored = true;
                now = wall_clock_100ns;
            }
        } else {
            // A placeholder has no frame time of its own; stamping it from the wall clock
            // keeps a consumer's timeline moving while the picture is absent.
            self.pts_anchored = false;
        }

        self.last_delivered_pts_us = delivered_pts_us;
        if now <= self.last_sample_time_100ns {
            now = self.last_sample_time_100ns + self.sample_duration_100ns;
        }
        self.last_sample_time_100ns = now;
        now
    }

    pub fn last_sample_time_100ns(&self) -> i64 {
        self.last_sample_time_100ns
    }

    pub fn is_anchored(&self) -> bool {
        self.pts_anchored
    }

    pub fn sample_duration_100ns(&self) -> i64 {
        self.sample_duration_100ns
    }

    pub fn reset(&mut self) {
        self.last_sample_time_100ns = 0;
        self.last_delivered_pts_us = 0;
        self.pts_anchored = false;
    }
}

/* ------------------------------------------------------------------ */
/* Windows                                                             */
/* ------------------------------------------------------------------ */

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Memory::{
        CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, VirtualQuery, FILE_MAP_ALL_ACCESS,
        FILE_MAP_READ, MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READONLY,
        PAGE_READWRITE,
    };

    const OFF_MAGIC: usize = 0;
    const OFF_LAYOUT: usize = 4;
    const OFF_WIDTH: usize = 8;
    const OFF_HEIGHT: usize = 12;
    const OFF_CAPACITY: usize = 24;
    const OFF_PTS: usize = 28;
    const OFF_FRAME_INDEX: usize = 36;
    const OFF_STATE: usize = 44;
    const OFF_MIRROR: usize = 48;
    const OFF_ROTATION: usize = 52;
    const OFF_SEQ: usize = 56;
    const OFF_READERS: usize = 64;

    pub struct Inner {
        file: HANDLE,
        mapping: HANDLE,
        base: *mut u8,
        mapped_bytes: usize,
        writable: bool,
        last_good: Option<FrameSnapshot>,
        last_buffer: Vec<u8>,
    }

    unsafe impl Send for Inner {}

    impl Inner {
        pub fn open(path: &str) -> Result<Self, ReaderError> {
            let path_w = wide(path);

            // First attempt to open with write access to maintain reader count.
            let mut writable = true;
            let mut file = unsafe {
                CreateFileW(
                    PCWSTR(path_w.as_ptr()),
                    FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
            }
            .unwrap_or(INVALID_HANDLE_VALUE);

            if file == INVALID_HANDLE_VALUE {
                writable = false;
                file = unsafe {
                    CreateFileW(
                        PCWSTR(path_w.as_ptr()),
                        FILE_GENERIC_READ.0,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        None,
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL,
                        None,
                    )
                }
                .map_err(|e| ReaderError::Open(e.message().to_string()))?;

                if file == INVALID_HANDLE_VALUE {
                    return Err(ReaderError::Open("invalid handle".into()));
                }
            }

            let mut mapping = unsafe {
                CreateFileMappingW(
                    file,
                    None,
                    if writable {
                        PAGE_READWRITE
                    } else {
                        PAGE_READONLY
                    },
                    0,
                    0,
                    PCWSTR::null(),
                )
            }
            .unwrap_or(HANDLE(std::ptr::null_mut()));

            if mapping.is_invalid() && writable {
                writable = false;
                unsafe {
                    let _ = CloseHandle(file);
                }
                file = unsafe {
                    CreateFileW(
                        PCWSTR(path_w.as_ptr()),
                        FILE_GENERIC_READ.0,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        None,
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL,
                        None,
                    )
                }
                .map_err(|e| ReaderError::Open(e.message().to_string()))?;

                mapping =
                    unsafe { CreateFileMappingW(file, None, PAGE_READONLY, 0, 0, PCWSTR::null()) }
                        .map_err(|e| ReaderError::Map(e.message().to_string()))?;
            }

            if mapping.is_invalid() {
                unsafe {
                    let _ = CloseHandle(file);
                }
                return Err(ReaderError::Map("invalid mapping handle".into()));
            }

            let view: MEMORY_MAPPED_VIEW_ADDRESS = unsafe {
                MapViewOfFile(
                    mapping,
                    if writable {
                        FILE_MAP_ALL_ACCESS
                    } else {
                        FILE_MAP_READ
                    },
                    0,
                    0,
                    0,
                )
            };

            if view.Value.is_null() {
                unsafe {
                    let _ = CloseHandle(mapping);
                    let _ = CloseHandle(file);
                }
                return Err(ReaderError::Map(format!(
                    "{}",
                    std::io::Error::last_os_error()
                )));
            }

            let mut info = MEMORY_BASIC_INFORMATION::default();
            let query_res = unsafe {
                VirtualQuery(
                    Some(view.Value as *const _),
                    &mut info,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };

            if query_res == 0 || info.RegionSize < HEADER_SIZE {
                unsafe {
                    let _ = UnmapViewOfFile(view);
                    let _ = CloseHandle(mapping);
                    let _ = CloseHandle(file);
                }
                return Err(ReaderError::TooSmall);
            }

            let base = view.Value as *mut u8;
            let magic = unsafe { std::ptr::read_unaligned(base.add(OFF_MAGIC) as *const u32) };
            let layout = unsafe { std::ptr::read_unaligned(base.add(OFF_LAYOUT) as *const u32) };

            if magic != crate::MAGIC || layout != crate::LAYOUT_VERSION {
                unsafe {
                    let _ = UnmapViewOfFile(view);
                    let _ = CloseHandle(mapping);
                    let _ = CloseHandle(file);
                }
                return Err(ReaderError::BadHeader);
            }

            if writable {
                let readers = unsafe { &*(base.add(OFF_READERS) as *const AtomicU32) };
                readers.fetch_add(1, Ordering::SeqCst);
            }

            Ok(Self {
                file,
                mapping,
                base,
                mapped_bytes: info.RegionSize,
                writable,
                last_good: None,
                last_buffer: Vec::new(),
            })
        }

        pub fn is_stale(&self) -> bool {
            let capacity =
                unsafe { std::ptr::read_unaligned(self.base.add(OFF_CAPACITY) as *const u32) }
                    as usize;
            HEADER_SIZE + capacity > self.mapped_bytes
        }

        pub fn mapped_capacity(&self) -> u32 {
            if self.mapped_bytes <= HEADER_SIZE {
                0
            } else {
                (self.mapped_bytes - HEADER_SIZE) as u32
            }
        }

        pub fn has_new_frame(&self, last_frame_index: u64) -> bool {
            let current_index =
                unsafe { std::ptr::read_unaligned(self.base.add(OFF_FRAME_INDEX) as *const u64) };
            current_index != last_frame_index
        }

        fn read_snapshot_fields(&self) -> FrameSnapshot {
            unsafe {
                let width = std::ptr::read_unaligned(self.base.add(OFF_WIDTH) as *const u32);
                let height = std::ptr::read_unaligned(self.base.add(OFF_HEIGHT) as *const u32);
                let pts_us = std::ptr::read_unaligned(self.base.add(OFF_PTS) as *const u64);
                let frame_index =
                    std::ptr::read_unaligned(self.base.add(OFF_FRAME_INDEX) as *const u64);
                let state_raw = std::ptr::read_unaligned(self.base.add(OFF_STATE) as *const u32);
                let mirror = std::ptr::read_unaligned(self.base.add(OFF_MIRROR) as *const u32) != 0;
                let rotation = std::ptr::read_unaligned(self.base.add(OFF_ROTATION) as *const u32);
                let state = match state_raw {
                    0 => FrameState::Waiting,
                    1 => FrameState::Live,
                    2 => FrameState::Stale,
                    3 => FrameState::Disconnected,
                    _ => FrameState::Waiting,
                };
                FrameSnapshot {
                    width,
                    height,
                    pts_us,
                    frame_index,
                    state,
                    mirror,
                    rotation,
                }
            }
        }

        pub fn peek(&self) -> Option<FrameSnapshot> {
            let seq = unsafe { &*(self.base.add(OFF_SEQ) as *const AtomicU32) };
            for _ in 0..4 {
                let before = seq.load(Ordering::Acquire);
                if before & 1 != 0 {
                    continue;
                }
                let snapshot = self.read_snapshot_fields();
                let after = seq.load(Ordering::Acquire);
                if before == after {
                    return Some(snapshot);
                }
            }
            self.last_good
        }

        pub fn read(&mut self, out: &mut Vec<u8>) -> Option<FrameSnapshot> {
            let capacity =
                unsafe { std::ptr::read_unaligned(self.base.add(OFF_CAPACITY) as *const u32) }
                    as usize;
            let width = unsafe { std::ptr::read_unaligned(self.base.add(OFF_WIDTH) as *const u32) };
            let height =
                unsafe { std::ptr::read_unaligned(self.base.add(OFF_HEIGHT) as *const u32) };

            if width == 0 || height == 0 || capacity == 0 {
                return None;
            }
            if HEADER_SIZE + capacity > self.mapped_bytes {
                return None;
            }

            out.resize(capacity, 0);
            let seq = unsafe { &*(self.base.add(OFF_SEQ) as *const AtomicU32) };

            for _ in 0..64 {
                let before = seq.load(Ordering::Acquire);
                if before & 1 != 0 {
                    std::hint::spin_loop();
                    continue;
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        self.base.add(HEADER_SIZE),
                        out.as_mut_ptr(),
                        capacity,
                    );
                }
                let snapshot = self.read_snapshot_fields();
                let after = seq.load(Ordering::Acquire);
                if before == after {
                    self.last_good = Some(snapshot);
                    self.last_buffer.clone_from(out);
                    return Some(snapshot);
                }
            }

            if let Some(good) = self.last_good {
                if self.last_buffer.len() == capacity {
                    out.clone_from(&self.last_buffer);
                    return Some(good);
                }
            }

            None
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            unsafe {
                if !self.base.is_null() {
                    if self.writable {
                        let readers = &*(self.base.add(OFF_READERS) as *const AtomicU32);
                        readers.fetch_sub(1, Ordering::SeqCst);
                    }
                    let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: self.base as *mut _,
                    });
                }
                let _ = CloseHandle(self.mapping);
                let _ = CloseHandle(self.file);
            }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
}

/* ------------------------------------------------------------------ */
/* Non-Windows                                                         */
/* ------------------------------------------------------------------ */

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub struct Inner;

    impl Inner {
        pub fn open(_path: &str) -> Result<Self, ReaderError> {
            Err(ReaderError::NotWindows)
        }
        pub fn is_stale(&self) -> bool {
            false
        }
        pub fn mapped_capacity(&self) -> u32 {
            0
        }
        pub fn has_new_frame(&self, _last_frame_index: u64) -> bool {
            false
        }
        pub fn peek(&self) -> Option<FrameSnapshot> {
            None
        }
        pub fn read(&mut self, _out: &mut Vec<u8>) -> Option<FrameSnapshot> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcam_timestamp_anchors_first_frame_to_wall_clock() {
        let mut tracker = VcamTimestampTracker::new(30);
        assert!(!tracker.is_anchored());

        let wall_clock = 100_000_000;
        let ts = tracker.sample_timestamp(1_000, wall_clock);
        assert_eq!(ts, wall_clock);
        assert!(tracker.is_anchored());
    }

    #[test]
    fn vcam_timestamp_repeated_frame_retains_same_timestamp() {
        let mut tracker = VcamTimestampTracker::new(30);
        let wall_1 = 100_000_000;
        let ts1 = tracker.sample_timestamp(10_000, wall_1);
        assert_eq!(ts1, wall_1);

        // Downstream consumer asks again 16ms later (e.g. 60 fps consumer on 30 fps stream)
        // with the same frame PTS.
        let wall_2 = 100_166_666;
        let ts2 = tracker.sample_timestamp(10_000, wall_2);
        assert_eq!(
            ts2, ts1,
            "repeated frame must retain the same timestamp to avoid recording stutter"
        );
    }

    #[test]
    fn vcam_timestamp_advances_by_pts_delta() {
        let mut tracker = VcamTimestampTracker::new(30);
        let wall_1 = 100_000_000;
        let ts1 = tracker.sample_timestamp(10_000, wall_1);

        // Frame 2 arrives ~33.3ms later: PTS delta = 33_333 us (333_330 in 100ns)
        let wall_2 = 100_333_333;
        let ts2 = tracker.sample_timestamp(10_000 + 33_333, wall_2);
        assert_eq!(ts2, ts1 + 333_330);

        // Repeated Frame 2 retains ts2
        let wall_3 = 100_500_000;
        let ts3 = tracker.sample_timestamp(10_000 + 33_333, wall_3);
        assert_eq!(ts3, ts2);
    }

    #[test]
    fn vcam_timestamp_placeholder_unanchors_and_falls_back_to_wall_clock() {
        let mut tracker = VcamTimestampTracker::new(30);
        let wall_1 = 100_000_000;
        tracker.sample_timestamp(10_000, wall_1);
        assert!(tracker.is_anchored());

        // Placeholder frame has pts_us = 0
        let wall_2 = 105_000_000;
        let ts2 = tracker.sample_timestamp(0, wall_2);
        assert_eq!(ts2, wall_2);
        assert!(!tracker.is_anchored(), "placeholder should unanchor PTS");
    }

    #[test]
    fn vcam_timestamp_large_jump_clamps_to_wall_clock() {
        let mut tracker = VcamTimestampTracker::new(30);
        let wall_1 = 100_000_000;
        tracker.sample_timestamp(10_000, wall_1);

        // Huge PTS jump (e.g. encoder restart with large base)
        let wall_2 = 100_033_333;
        let huge_pts = 10_000 + 10_000_000; // +10s
        let ts2 = tracker.sample_timestamp(huge_pts, wall_2);
        assert!(ts2 <= wall_2 + tracker.sample_duration_100ns());
    }

    #[test]
    fn vcam_timestamp_monotonicity_is_strictly_enforced() {
        let mut tracker = VcamTimestampTracker::new(30);
        let wall_1 = 100_000_000;
        let ts1 = tracker.sample_timestamp(10_000, wall_1);

        // Backwards PTS jump: should step one sample duration forward, never backwards
        let wall_2 = 100_033_333;
        let ts2 = tracker.sample_timestamp(5_000, wall_2);
        assert!(ts2 > ts1, "timestamps must never go backwards");
    }

    #[cfg(windows)]
    #[test]
    fn reader_reads_published_frames_and_pts_coherently() {
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join(format!("phonecam-test-{}.bin", std::process::id()));
        let test_path_str = test_path.to_string_lossy().to_string();

        let (w, h) = (64u32, 48u32);
        let mut writer = crate::FrameWriter::open_at(&test_path_str, w, h).unwrap();
        assert_eq!(writer.reader_count(), 0);

        let mut reader = FrameReader::open(&test_path_str).unwrap();
        assert_eq!(writer.reader_count(), 1);

        let capacity = crate::nv12_size(w, h) as usize;
        let frame_data = vec![42u8; capacity];
        let pts = 1_234_567u64;

        writer.publish(&frame_data, pts, FrameState::Live).unwrap();

        assert!(reader.has_new_frame(0));

        let mut read_buf = Vec::new();
        let snap = reader.read(&mut read_buf).expect("must read live frame");

        assert_eq!(snap.width, w);
        assert_eq!(snap.height, h);
        assert_eq!(snap.pts_us, pts);
        assert_eq!(snap.frame_index, 1);
        assert_eq!(snap.state, FrameState::Live);
        assert_eq!(read_buf, frame_data);

        // Reader drop decrements readers
        drop(reader);
        assert_eq!(writer.reader_count(), 0);

        drop(writer);
        let _ = std::fs::remove_file(test_path);
    }
}
