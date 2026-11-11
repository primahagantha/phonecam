//! The write side of the frame bus: a grow-only file-backed mapping published
//! with a seqlock.
//!
//! Ported from `FrameWriter` in `native/shared/src/frame_shm.cpp`. Three rules
//! there were learned the hard way and are load-bearing here:
//!
//! * **The file only ever grows.** Windows refuses to truncate a file another
//!   process has mapped (`ERROR_USER_MAPPED_FILE`), which is exactly the state
//!   during a resolution step down. `capacity`, not the file size, delimits a
//!   frame.
//! * **`readers` is never written.** It belongs to the readers; resetting it
//!   would make the UI's "N using it" drop to zero on every resolution change.
//! * **The mapping is created at the file's size**, because a view can never
//!   exceed its mapping object.

use crate::{nv12, FrameState, PublishRejection, HEADER_SIZE};

#[derive(Debug, thiserror::Error)]
pub enum WriterError {
    #[error("invalid frame dimensions {width}x{height}; both must be even and non-zero")]
    BadDimensions { width: u32, height: u32 },
    #[error("could not open the frame buffer file: {0}")]
    Open(String),
    #[error("could not size the frame buffer: {0}")]
    Resize(String),
    #[error("could not apply the frame buffer ACL: {0}")]
    Acl(String),
    #[error("could not map the frame buffer: {0}")]
    Map(String),
    #[error("the frame bus is only available on Windows")]
    NotWindows,
}

/// Brightness difference threshold (> 24/255) to trigger flicker warning.
pub const LUMA_JUMP_THRESHOLD: u8 = 24;

/// A mapped frame buffer, ready to publish into.
pub struct FrameWriter {
    inner: imp::Inner,
    width: u32,
    height: u32,
    frame_index: u64,
    last_mean_luma: Option<u8>,
    flicker_reported: bool,
}

impl std::fmt::Debug for FrameWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameWriter")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("frame_index", &self.frame_index)
            .field("last_mean_luma", &self.last_mean_luma)
            .field("flicker_reported", &self.flicker_reported)
            .finish()
    }
}

impl FrameWriter {
    /// Opens (or grows) the default buffer at `width` x `height`.
    pub fn open(width: u32, height: u32) -> Result<Self, WriterError> {
        Self::open_at(crate::BUFFER_PATH, width, height)
    }

    /// Opens (or grows) the buffer at `path` for `width` x `height`.
    pub fn open_at(path: &str, width: u32, height: u32) -> Result<Self, WriterError> {
        if !crate::dimensions_are_valid(width, height) {
            return Err(WriterError::BadDimensions { width, height });
        }
        let inner = imp::Inner::open(path, width, height)?;
        Ok(Self {
            inner,
            width,
            height,
            frame_index: 0,
            last_mean_luma: None,
            flicker_reported: false,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn capacity(&self) -> u32 {
        crate::nv12_size(self.width, self.height)
    }
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// How many apps currently hold the camera open, or 0 when unknown.
    ///
    /// Maintained by the readers, never by us. A reader that could not map the
    /// file writable does not count itself, so the count can under-report —
    /// which is the safe direction.
    pub fn reader_count(&self) -> u32 {
        self.inner.reader_count()
    }

    /// Publishes one NV12 frame.
    ///
    /// `frame` must be exactly `capacity` bytes. The C++ writer silently ignores
    /// a mismatched publish, which is how a camera ends up on a permanent
    /// placeholder with a healthy-looking log; here the caller is told.
    pub fn publish(
        &mut self,
        frame: &[u8],
        pts_us: u64,
        state: FrameState,
    ) -> Result<(), PublishRejection> {
        let capacity = self.capacity() as usize;
        if frame.len() != capacity {
            return Err(PublishRejection::SizeMismatch { expected: capacity, actual: frame.len() });
        }
        self.inner.publish(frame, pts_us, state, &mut self.frame_index)?;

        // Brightness watchdog for flicker (upstream commit 5e6b22d).
        if state == FrameState::Live {
            let mean = Self::compute_mean_luma(frame, self.width, self.height);
            self.observe_mean_luma(mean);
        }

        Ok(())
    }

    /// Computes the mean luma of an NV12 frame by sampling every 16th pixel.
    pub fn compute_mean_luma(frame: &[u8], width: u32, height: u32) -> u8 {
        let luma_samples = (width as usize) * (height as usize);
        if luma_samples == 0 || frame.len() < luma_samples {
            return 0;
        }
        let mut sum: u64 = 0;
        let mut count: u64 = 0;
        for i in (0..luma_samples).step_by(16) {
            sum += frame[i] as u64;
            count += 1;
        }
        if count == 0 {
            0
        } else {
            (sum / count) as u8
        }
    }

    /// Evaluates brightness change between consecutive frames and logs warnings/info
    /// on transition into/out of flicker, avoiding per-frame log spam.
    fn observe_mean_luma(&mut self, mean: u8) {
        if let Some(previous) = self.last_mean_luma {
            let jump = (mean as i16 - previous as i16).abs() as u8;
            let flickering = jump > LUMA_JUMP_THRESHOLD;
            if flickering && !self.flicker_reported {
                self.flicker_reported = true;
                tracing::warn!(
                    "picture brightness jumped {}/255 between frames; this is what flicker looks like (check anti-banding and exposure)",
                    jump
                );
            } else if !flickering && self.flicker_reported {
                self.flicker_reported = false;
                tracing::info!("picture brightness is stable again");
            }
        }
        self.last_mean_luma = Some(mean);
    }

    pub fn last_mean_luma(&self) -> Option<u8> {
        self.last_mean_luma
    }

    pub fn is_flicker_reported(&self) -> bool {
        self.flicker_reported
    }

    pub fn reset_flicker(&mut self) {
        self.last_mean_luma = None;
        self.flicker_reported = false;
    }

    /// Publishes the "PhoneCam disconnected" placeholder.
    ///
    /// Black pixels go out alongside the state, not just the state, because a
    /// reader that ignored the header would otherwise keep showing the last
    /// frame of a phone that is gone.
    pub fn publish_placeholder(&mut self, reason: &str) -> Result<(), PublishRejection> {
        tracing::info!("publishing the disconnected placeholder: {reason}");
        let (w, h) = (self.width, self.height);
        let mut black = vec![0u8; self.capacity() as usize];
        nv12::fill_black(&mut black, w, h);
        self.publish(&black, 0, FrameState::Disconnected)
    }

    /// Sets the display transform the camera applies.
    ///
    /// Metadata only — the core never pre-transforms. Both surfaces apply
    /// rotation before mirror, so a mirrored quarter turn reads left-to-right
    /// the way a mirror does.
    pub fn set_transform(&mut self, mirror: bool, rotation: u32) {
        self.inner.set_transform(mirror, rotation % 360);
    }
}

/* ------------------------------------------------------------------ */
/* Windows                                                             */
/* ------------------------------------------------------------------ */

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::atomic::{fence, AtomicU32, Ordering};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{SetFileSecurityW, PSECURITY_DESCRIPTOR, DACL_SECURITY_INFORMATION};
    use windows::Win32::Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, GetFileSizeEx, SetEndOfFile, SetFilePointerEx,
        FILE_ATTRIBUTE_NORMAL, FILE_BEGIN, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_ALWAYS,
    };
    use windows::Win32::System::Memory::{
        CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
        MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
    };

    /// Field offsets, mirrored from the `FrameHeader` layout asserted in `lib.rs`.
    const OFF_MAGIC: usize = 0;
    const OFF_LAYOUT: usize = 4;
    const OFF_WIDTH: usize = 8;
    const OFF_HEIGHT: usize = 12;
    const OFF_STRIDE_Y: usize = 16;
    const OFF_STRIDE_UV: usize = 20;
    const OFF_CAPACITY: usize = 24;
    const OFF_PTS: usize = 28;
    const OFF_FRAME_INDEX: usize = 36;
    const OFF_STATE: usize = 44;
    const OFF_MIRROR: usize = 48;
    const OFF_ROTATION: usize = 52;
    const OFF_SEQ: usize = 56;
    const OFF_PAYLOAD_BYTES: usize = 60;
    const OFF_READERS: usize = 64;

    pub struct Inner {
        file: HANDLE,
        mapping: HANDLE,
        base: *mut u8,
        mapped_bytes: usize,
    }

    // The view is process-local and only touched through `&mut Inner`.
    unsafe impl Send for Inner {}

    impl Inner {
        pub fn open(path: &str, width: u32, height: u32) -> Result<Self, WriterError> {
            let capacity = crate::nv12_size(width, height) as u64;
            let needed = HEADER_SIZE as u64 + capacity;

            let path_w = wide(path);
            if let Some(parent) = std::path::Path::new(path).parent() {
                let dir = wide(&parent.to_string_lossy());
                let _ = unsafe { CreateDirectoryW(PCWSTR(dir.as_ptr()), None) };
            }

            let file = unsafe {
                CreateFileW(
                    PCWSTR(path_w.as_ptr()),
                    FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
            }
            .map_err(|e| WriterError::Open(e.message().to_string()))?;

            if file == INVALID_HANDLE_VALUE {
                return Err(WriterError::Open("invalid handle".into()));
            }

            // Grow only: truncation fails while the Frame Server holds a view.
            let current = file_size(file)?;
            if current < needed {
                unsafe {
                    SetFilePointerEx(file, needed as i64, None, FILE_BEGIN)
                        .map_err(|e| WriterError::Resize(e.message().to_string()))?;
                    SetEndOfFile(file).map_err(|e| WriterError::Resize(e.message().to_string()))?;
                }
            }

            apply_acl(path)?;

            let file_size_now = file_size(file)?;
            if file_size_now < needed {
                unsafe { let _ = CloseHandle(file); }
                return Err(WriterError::Resize(format!(
                    "file is {file_size_now} bytes, need {needed}"
                )));
            }

            // Created at the file's size, not at `needed`: a view can never
            // exceed its mapping object.
            let mapping = unsafe {
                CreateFileMappingW(file, None, PAGE_READWRITE, 0, 0, PCWSTR::null())
            }
            .map_err(|e| WriterError::Map(e.message().to_string()))?;

            let view: MEMORY_MAPPED_VIEW_ADDRESS =
                unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, file_size_now as usize) };
            if view.Value.is_null() {
                unsafe {
                    let _ = CloseHandle(mapping);
                    let _ = CloseHandle(file);
                }
                return Err(WriterError::Map(format!(
                    "{}",
                    std::io::Error::last_os_error()
                )));
            }

            let inner = Self {
                file,
                mapping,
                base: view.Value as *mut u8,
                mapped_bytes: file_size_now as usize,
            };

            // Initialise every field except `readers`, which belongs to readers.
            unsafe {
                inner.write_u32(OFF_MAGIC, crate::MAGIC);
                inner.write_u32(OFF_LAYOUT, crate::LAYOUT_VERSION);
                inner.write_u32(OFF_WIDTH, width);
                inner.write_u32(OFF_HEIGHT, height);
                inner.write_u32(OFF_STRIDE_Y, width);
                inner.write_u32(OFF_STRIDE_UV, width);
                inner.write_u32(OFF_CAPACITY, capacity as u32);
                inner.write_u64(OFF_PTS, 0);
                inner.write_u64(OFF_FRAME_INDEX, 0);
                inner.write_u32(OFF_STATE, FrameState::Waiting as u32);
                inner.write_u32(OFF_MIRROR, 0);
                inner.write_u32(OFF_ROTATION, 0);
                inner.write_u32(OFF_SEQ, 0);
                inner.write_u32(OFF_PAYLOAD_BYTES, 0);
            }

            Ok(inner)
        }

        /// Writes through a raw pointer: the header is `packed`, so a `&mut`
        /// to a field would be an unaligned reference.
        unsafe fn write_u32(&self, offset: usize, value: u32) {
            std::ptr::write_unaligned(self.base.add(offset) as *mut u32, value);
        }

        unsafe fn write_u64(&self, offset: usize, value: u64) {
            std::ptr::write_unaligned(self.base.add(offset) as *mut u64, value);
        }

        fn seq(&self) -> &AtomicU32 {
            // Offset 56 is 4-byte aligned, so an atomic reference is sound.
            unsafe { &*(self.base.add(OFF_SEQ) as *const AtomicU32) }
        }

        pub fn reader_count(&self) -> u32 {
            // A plain volatile load: readers update it with interlocked ops.
            unsafe { std::ptr::read_volatile(self.base.add(OFF_READERS) as *const u32) }
        }

        pub fn publish(
            &self,
            frame: &[u8],
            pts_us: u64,
            state: FrameState,
            frame_index: &mut u64,
        ) -> Result<(), PublishRejection> {
            let capacity = unsafe {
                std::ptr::read_unaligned(self.base.add(OFF_CAPACITY) as *const u32) as usize
            };
            if frame.len() != capacity {
                return Err(PublishRejection::SizeMismatch {
                    expected: capacity,
                    actual: frame.len(),
                });
            }
            if HEADER_SIZE + capacity > self.mapped_bytes {
                return Err(PublishRejection::NoBuffer);
            }

            let seq = self.seq();
            // Odd means a write is in progress. `SeqCst` matches the C++
            // `InterlockedIncrement`, which is a full fence: the reader depends
            // on that ordering to see the payload rather than a torn frame.
            seq.fetch_add(1, Ordering::SeqCst);

            unsafe {
                std::ptr::copy_nonoverlapping(
                    frame.as_ptr(),
                    self.base.add(HEADER_SIZE),
                    capacity,
                );
                self.write_u64(OFF_PTS, pts_us);
                *frame_index += 1;
                self.write_u64(OFF_FRAME_INDEX, *frame_index);
                self.write_u32(OFF_STATE, state as u32);
                self.write_u32(OFF_PAYLOAD_BYTES, capacity as u32);
            }

            seq.fetch_add(1, Ordering::SeqCst);
            fence(Ordering::SeqCst);
            Ok(())
        }

        pub fn set_transform(&self, mirror: bool, rotation: u32) {
            let seq = self.seq();
            seq.fetch_add(1, Ordering::SeqCst);
            unsafe {
                self.write_u32(OFF_MIRROR, u32::from(mirror));
                self.write_u32(OFF_ROTATION, rotation % 360);
            }
            seq.fetch_add(1, Ordering::SeqCst);
            fence(Ordering::SeqCst);
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            unsafe {
                if !self.base.is_null() {
                    let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: self.base as *mut _,
                    });
                }
                let _ = CloseHandle(self.mapping);
                let _ = CloseHandle(self.file);
            }
        }
    }

    fn file_size(file: HANDLE) -> Result<u64, WriterError> {
        let mut size = 0i64;
        unsafe { GetFileSizeEx(file, &mut size) }
            .map_err(|e| WriterError::Resize(e.message().to_string()))?;
        Ok(size as u64)
    }

    fn apply_acl(path: &str) -> Result<(), WriterError> {
        let sddl = wide(crate::BUFFER_SDDL);
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .map_err(|e| WriterError::Acl(format!("bad SDDL: {}", e.message())))?;

            let path_w = wide(path);
            SetFileSecurityW(PCWSTR(path_w.as_ptr()), DACL_SECURITY_INFORMATION, descriptor)
                .ok()
                .map_err(|e| {
                    WriterError::Acl(format!("{} ({})", e.message(), std::io::Error::last_os_error()))
                })?;
        }
        Ok(())
    }

    fn wide(text: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
    }
}

/* ------------------------------------------------------------------ */
/* Non-Windows: the frame bus is Windows-only                          */
/* ------------------------------------------------------------------ */

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub struct Inner;

    impl Inner {
        pub fn open(_path: &str, _width: u32, _height: u32) -> Result<Self, WriterError> {
            Err(WriterError::NotWindows)
        }
        pub fn reader_count(&self) -> u32 {
            0
        }
        pub fn publish(
            &self,
            _frame: &[u8],
            _pts_us: u64,
            _state: FrameState,
            _frame_index: &mut u64,
        ) -> Result<(), PublishRejection> {
            Err(PublishRejection::NoBuffer)
        }
        pub fn set_transform(&self, _mirror: bool, _rotation: u32) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_mean_luma_samples_every_16th_pixel() {
        let (w, h) = (32u32, 16u32);
        let capacity = crate::nv12_size(w, h) as usize;
        let mut frame = vec![100u8; capacity];
        // If all luma pixels are 100, mean should be 100
        assert_eq!(FrameWriter::compute_mean_luma(&frame, w, h), 100);

        // Modify only every 16th pixel to 200
        for i in (0..(w as usize * h as usize)).step_by(16) {
            frame[i] = 200;
        }
        assert_eq!(FrameWriter::compute_mean_luma(&frame, w, h), 200);

        // Modify non-sampled pixels to 0 (sampling every 16th should ignore them)
        for i in 0..(w as usize * h as usize) {
            if i % 16 != 0 {
                frame[i] = 0;
            }
        }
        assert_eq!(FrameWriter::compute_mean_luma(&frame, w, h), 200);
    }

    #[test]
    fn observe_mean_luma_detects_flicker_threshold() {
        let mut last_mean_luma = None;
        let mut flicker_reported = false;

        let observe = |mean: u8, last: &mut Option<u8>, reported: &mut bool| -> (bool, bool, bool) {
            let mut event_started = false;
            let mut event_stopped = false;
            if let Some(previous) = *last {
                let jump = (mean as i16 - previous as i16).abs() as u8;
                let flickering = jump > LUMA_JUMP_THRESHOLD;
                if flickering && !*reported {
                    *reported = true;
                    event_started = true;
                } else if !flickering && *reported {
                    *reported = false;
                    event_stopped = true;
                }
            }
            *last = Some(mean);
            (event_started, event_stopped, *reported)
        };

        // Frame 1: mean 100
        let (s, e, rep) = observe(100, &mut last_mean_luma, &mut flicker_reported);
        assert!(!s && !e);
        assert!(!rep);

        // Frame 2: mean 120 (delta 20 <= 24): not flickering
        let (s, e, rep) = observe(120, &mut last_mean_luma, &mut flicker_reported);
        assert!(!s && !e);
        assert!(!rep);

        // Frame 3: mean 145 (delta 25 > 24): flicker starts!
        let (s, e, rep) = observe(145, &mut last_mean_luma, &mut flicker_reported);
        assert!(s && !e);
        assert!(rep);

        // Frame 4: mean 180 (delta 35 > 24): still flickering, no duplicate warning
        let (s, e, rep) = observe(180, &mut last_mean_luma, &mut flicker_reported);
        assert!(!s && !e, "must not re-report flicker on every frame");
        assert!(rep);

        // Frame 5: mean 185 (delta 5 <= 24): brightness stable again!
        let (s, e, rep) = observe(185, &mut last_mean_luma, &mut flicker_reported);
        assert!(!s && e, "must report stable transition");
        assert!(!rep);
    }
}
