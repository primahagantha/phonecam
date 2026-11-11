//! The shared-memory frame bus between the core and the virtual camera DLL
//! (`native/shared/include/phonecam/frame_shm.hpp`).
//!
//! Windows instantiates software camera sources **itself**, in the Camera Frame
//! Server process, so `PhoneCamVirtualCamera.dll` cannot share this process's
//! heap. A file-backed mapping with a seqlock is the cheapest correct channel:
//! the reader never blocks the writer, and a torn read is detected and retried
//! rather than rendered.
//!
//! This is a **second wire contract**, independent of the phone protocol. The
//! DLL is C++ and stays C++ (it is loaded into the Frame Server), so this
//! module must be byte-identical to `frame_shm.hpp`:
//!
//! * 68-byte header, `#pragma pack(1)` → `#[repr(C, packed)]`
//! * `magic = 0x5043_414D`, `layout_version = 2`
//! * payload at file offset 68
//! * grow-only file; `capacity`, not the file size, bounds a read
//! * `readers` is written **only** by readers

pub mod nv12;
pub mod reader;
pub mod writer;

pub use reader::{FrameReader, FrameSnapshot, ReaderError, VcamTimestampTracker};
pub use writer::{FrameWriter, WriterError, LUMA_JUMP_THRESHOLD};

pub const MAGIC: u32 = 0x5043_414D;
pub const LAYOUT_VERSION: u32 = 2;
pub const HEADER_SIZE: usize = 68;

/// The state a published frame carries.
///
/// `Stale` is declared in the C++ enum but never written; it is reproduced here
/// only so the discriminant numbers match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FrameState {
    Waiting = 0,
    Live = 1,
    Stale = 2,
    /// Published after a grace period with no frames, so a consumer sees an
    /// explicit state rather than a freeze.
    Disconnected = 3,
}

/// The header, at file offset 0.
///
/// The field order and offsets are fixed by the C++ `static_assert`; adding a
/// field here without a matching DLL rebuild produces a camera that reads a
/// buffer nobody writes — which looks exactly like a disconnected phone.
#[repr(C, packed)]
pub struct FrameHeader {
    pub magic: u32,
    pub layout_version: u32,
    pub width: u32,
    pub height: u32,
    /// Always written as `width`; never read by the reader.
    pub stride_y: u32,
    pub stride_uv: u32,
    /// NV12 bytes. This — not the file length — bounds every read.
    pub capacity: u32,
    pub pts_us: u64,
    /// Monotonic; 0 means nothing has been published.
    pub frame_index: u64,
    pub state: u32,
    pub mirror: u32,
    pub rotation: u32,
    /// Seqlock: odd means a write is in progress.
    pub seq: u32,
    pub payload_bytes: u32,
    /// Live reader count. Written only by readers; the writer must not touch it.
    pub readers: u32,
}

const _: () = assert!(std::mem::size_of::<FrameHeader>() == HEADER_SIZE);

/// NV12 bytes for `width` x `height`.
///
/// `w*h + w*((h+1)/2)`, which equals `w*h*3/2` for even heights — and heights
/// are always even, because odd output sizes are rejected. The `+1` matters for
/// the odd case the C++ side tolerates.
pub const fn nv12_size(width: u32, height: u32) -> u32 {
    width * height + width * ((height + 1) / 2)
}

/// The buffer path. This literal is compiled into both binaries and is checked
/// at packaging time, so it must stay a plain ASCII string.
pub const BUFFER_PATH: &str = r"C:\ProgramData\PhoneCam\framebuffer-v2.bin";

/// The ACL the buffer is created with.
///
/// `D:P` — protected DACL, no inheritance. `LS` (LocalService) needs **write**
/// as well as read: it interlocked-updates the four-byte `readers` counter
/// through its own mapping, and a read-only view made that an access violation
/// that killed the Frame Server on the first live frame. Frame pixels are still
/// written only by the core.
pub const BUFFER_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)(A;;GRGW;;;LS)";

/// Why a frame publish was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishRejection {
    NoBuffer,
    /// The payload must equal `capacity` exactly; anything else is a silent
    /// no-op on the C++ side too, and here it is said out loud.
    SizeMismatch { expected: usize, actual: usize },
}

/// Both dimensions must be even: NV12 stores chroma in interleaved pairs, so an
/// odd size has no valid layout.
pub const fn dimensions_are_valid(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && width % 2 == 0 && height % 2 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_exactly_68_bytes() {
        assert_eq!(std::mem::size_of::<FrameHeader>(), 68);
    }

    /// Offsets are pinned field by field, because a single inserted field would
    /// silently move every one after it and the DLL would read garbage while
    /// still passing the magic check.
    #[test]
    fn header_field_offsets_match_the_cpp_layout() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(FrameHeader, magic), 0);
        assert_eq!(offset_of!(FrameHeader, layout_version), 4);
        assert_eq!(offset_of!(FrameHeader, width), 8);
        assert_eq!(offset_of!(FrameHeader, height), 12);
        assert_eq!(offset_of!(FrameHeader, stride_y), 16);
        assert_eq!(offset_of!(FrameHeader, stride_uv), 20);
        assert_eq!(offset_of!(FrameHeader, capacity), 24);
        assert_eq!(offset_of!(FrameHeader, pts_us), 28);
        assert_eq!(offset_of!(FrameHeader, frame_index), 36);
        assert_eq!(offset_of!(FrameHeader, state), 44);
        assert_eq!(offset_of!(FrameHeader, mirror), 48);
        assert_eq!(offset_of!(FrameHeader, rotation), 52);
        assert_eq!(offset_of!(FrameHeader, seq), 56);
        assert_eq!(offset_of!(FrameHeader, payload_bytes), 60);
        assert_eq!(offset_of!(FrameHeader, readers), 64);
    }

    #[test]
    fn frame_state_discriminants_match_the_cpp_enum() {
        assert_eq!(FrameState::Waiting as u32, 0);
        assert_eq!(FrameState::Live as u32, 1);
        assert_eq!(FrameState::Stale as u32, 2);
        assert_eq!(FrameState::Disconnected as u32, 3);
    }

    #[test]
    fn nv12_size_matches_the_cpp_formula() {
        assert_eq!(nv12_size(1920, 1080), 1920 * 1080 * 3 / 2);
        assert_eq!(nv12_size(1280, 720), 1280 * 720 * 3 / 2);
        assert_eq!(nv12_size(640, 480), 640 * 480 * 3 / 2);
        // Odd height: the round-up branch the C++ has and w*h*3/2 does not.
        assert_eq!(nv12_size(4, 3), 4 * 3 + 4 * 2);
    }

    #[test]
    fn buffer_constants_match_the_shipped_dll() {
        assert_eq!(MAGIC, 0x5043_414D);
        assert_eq!(LAYOUT_VERSION, 2);
        assert_eq!(HEADER_SIZE, 68);
        assert_eq!(BUFFER_PATH, r"C:\ProgramData\PhoneCam\framebuffer-v2.bin");
        assert_eq!(BUFFER_SDDL, "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)(A;;GRGW;;;LS)");
    }

    #[test]
    fn odd_dimensions_are_rejected() {
        assert!(dimensions_are_valid(1920, 1080));
        assert!(!dimensions_are_valid(1921, 1080));
        assert!(!dimensions_are_valid(1920, 1081));
        assert!(!dimensions_are_valid(0, 0));
    }
}
