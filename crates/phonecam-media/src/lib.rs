//! H.264 decoding.
//!
//! The C++ core muxed each group of pictures into a fragmented MP4 on disk and
//! read it back through a Media Foundation Source Reader, because the bare
//! H.264 decoder MFT produced zero frames on some machines. That path costs a
//! temp-file write plus a read per group (≈24 ms/batch) and is the single
//! largest latency term in the old pipeline.
//!
//! This decodes in-process with openh264 instead: no temp file, no container, no
//! second copy, and it works on any machine regardless of which MFTs are
//! installed. The output is I420, which is NV12's planar sibling — the frame bus
//! wants NV12, so the interleave happens once, at publish time.

#![allow(
    clippy::manual_checked_ops,
    clippy::cast_abs_to_unsigned,
    clippy::manual_range_contains,
    clippy::manual_div_ceil,
    clippy::if_same_then_else,
    clippy::manual_is_multiple_of,
    clippy::manual_repeat_n,
    clippy::chunks_exact_to_as_chunks,
    clippy::needless_range_loop
)]

pub mod flicker;
pub mod pacer;
pub mod sps;

pub use flicker::{FlickerDetector, FlickerEvent, LUMA_JUMP_THRESHOLD};
pub use pacer::{FramePacer, PaceAction, TARGET_FPS, TARGET_FRAME_DURATION_US};
pub use sps::{parse_avc_dimensions, Dimensions};

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("could not create the H.264 decoder: {0}")]
    Init(String),
    #[error("the decoder rejected the access unit")]
    Rejected,
    #[error("the decoder produced no picture")]
    NoPicture,
}

/// A borrowed I420 plane view (Y, U, V with explicit row strides).
///
/// Returned by [`H264Decoder::decode_and_process`] without copying or
/// allocating an intermediate I420 buffer.
#[derive(Debug, Clone, Copy)]
pub struct YuvPlanes<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub y_stride: usize,
    pub u_stride: usize,
    pub v_stride: usize,
}

impl<'a> YuvPlanes<'a> {
    /// Computes the mean luma by sampling every 16th pixel across the active
    /// `width * height` picture, matching [`DecodedFrame::mean_luma`] even when
    /// `y_stride > width`.
    pub fn mean_luma(&self) -> u8 {
        let w = self.width as usize;
        let h = self.height as usize;
        if w == 0 || h == 0 {
            return 0;
        }
        if self.y_stride == w {
            return flicker::FlickerDetector::compute_mean_luma(self.y, w, h);
        }
        let mut sum: u64 = 0;
        let mut count: u64 = 0;
        let mut row = 0usize;
        let mut col = 0usize;
        while row < h {
            let row_slice = &self.y[row * self.y_stride..];
            if col < row_slice.len() {
                sum += row_slice[col] as u64;
                count += 1;
            }
            col += 16;
            while col >= w {
                col -= w;
                row += 1;
            }
        }
        sum.checked_div(count).map(|v| v as u8).unwrap_or(0)
    }
}

/// A decoded picture in I420: full luma, then quarter-size U and V planes.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub yuv: Vec<u8>,
    pub pts_us: u64,
}

impl DecodedFrame {
    pub fn new(width: u32, height: u32, yuv: Vec<u8>, pts_us: u64) -> Self {
        Self {
            width,
            height,
            yuv,
            pts_us,
        }
    }

    /// Borrows packed I420 planes if `self.yuv` is large enough for `width x height`.
    pub fn as_planes(&self) -> Option<YuvPlanes<'_>> {
        let (width, height) = (self.width as usize, self.height as usize);
        let luma_len = width.checked_mul(height)?;
        let chroma_len = luma_len / 2;
        if self.yuv.len() < luma_len + chroma_len {
            return None;
        }
        let (u_plane, v_plane) = self.yuv[luma_len..luma_len + chroma_len].split_at(chroma_len / 2);
        Some(YuvPlanes {
            width: self.width,
            height: self.height,
            y: &self.yuv[..luma_len],
            u: u_plane,
            v: v_plane,
            y_stride: width,
            u_stride: width / 2,
            v_stride: width / 2,
        })
    }

    /// Computes the mean luma of this decoded frame.
    pub fn mean_luma(&self) -> u8 {
        flicker::FlickerDetector::compute_mean_luma(
            &self.yuv,
            self.width as usize,
            self.height as usize,
        )
    }

    /// Interleaves the chroma planes into NV12: luma, then UV pairs.
    ///
    /// Written into a caller-owned buffer so a streaming decode does not
    /// allocate once a second. `dst` must be exactly `nv12_size(w, h)` bytes.
    pub fn write_nv12(&self, dst: &mut [u8]) -> bool {
        let Some(planes) = self.as_planes() else {
            return false;
        };
        write_planes_nv12(&planes, dst)
    }

    /// Converts to BGRA for display.
    ///
    /// gpui's `RenderImage` expects **BGRA, premultiplied**, so the channel order
    /// here is B, G, R, A. Emitting RGBA produces a picture with red and blue
    /// swapped, which looks like a broken camera rather than a broken
    /// conversion.
    pub fn write_bgra(&self, dst: &mut [u8], stride_pixels: usize) -> bool {
        let Some(planes) = self.as_planes() else {
            return false;
        };
        write_planes_bgra(&planes, dst, stride_pixels)
    }
}

fn planes_have_valid_bounds(planes: &YuvPlanes<'_>) -> bool {
    let width = planes.width as usize;
    let height = planes.height as usize;
    if width == 0 || height == 0 {
        return true;
    }
    let chroma_width = width / 2;
    let chroma_rows = height / 2;
    if planes.y_stride < width || planes.u_stride < chroma_width || planes.v_stride < chroma_width {
        return false;
    }
    let y_needed = (height - 1) * planes.y_stride + width;
    if planes.y.len() < y_needed {
        return false;
    }
    if chroma_rows > 0 && chroma_width > 0 {
        let u_needed = (chroma_rows - 1) * planes.u_stride + chroma_width;
        let v_needed = (chroma_rows - 1) * planes.v_stride + chroma_width;
        if planes.u.len() < u_needed || planes.v.len() < v_needed {
            return false;
        }
    }
    true
}

#[inline]
fn interleave_uv_row(u: &[u8], v: &[u8], dst_uv: &mut [u8]) {
    let n = u.len().min(v.len()).min(dst_uv.len() / 2);
    let mut i = 0usize;

    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // SAFETY: SSE2 is guaranteed on x86_64; bounds checked via `i + 16 <= n`.
        unsafe {
            let u_ptr = u.as_ptr();
            let v_ptr = v.as_ptr();
            let d_ptr = dst_uv.as_mut_ptr();
            while i + 16 <= n {
                let uu = _mm_loadu_si128(u_ptr.add(i) as *const __m128i);
                let vv = _mm_loadu_si128(v_ptr.add(i) as *const __m128i);
                let lo = _mm_unpacklo_epi8(uu, vv);
                let hi = _mm_unpackhi_epi8(uu, vv);
                _mm_storeu_si128(d_ptr.add(i * 2) as *mut __m128i, lo);
                _mm_storeu_si128(d_ptr.add(i * 2 + 16) as *mut __m128i, hi);
                i += 16;
            }
        }
    }

    while i + 2 <= n {
        let packed = (u[i] as u32)
            | ((v[i] as u32) << 8)
            | ((u[i + 1] as u32) << 16)
            | ((v[i + 1] as u32) << 24);
        dst_uv[i * 2..i * 2 + 4].copy_from_slice(&packed.to_le_bytes());
        i += 2;
    }
    while i < n {
        dst_uv[i * 2] = u[i];
        dst_uv[i * 2 + 1] = v[i];
        i += 1;
    }
}

/// Interleaves borrowed [`YuvPlanes`] into an NV12 destination buffer.
///
/// `dst` must be exactly `width * height * 3 / 2` bytes.
pub fn write_planes_nv12(planes: &YuvPlanes<'_>, dst: &mut [u8]) -> bool {
    let (width, height) = (planes.width as usize, planes.height as usize);
    let luma_len = width * height;
    let chroma_len = luma_len / 2;
    if dst.len() != luma_len + chroma_len || !planes_have_valid_bounds(planes) {
        return false;
    }
    if width == 0 || height == 0 {
        return true;
    }

    let (dst_luma, dst_chroma) = dst.split_at_mut(luma_len);
    if planes.y_stride == width {
        dst_luma.copy_from_slice(&planes.y[..luma_len]);
    } else {
        for r in 0..height {
            let src_row = &planes.y[r * planes.y_stride..r * planes.y_stride + width];
            dst_luma[r * width..(r + 1) * width].copy_from_slice(src_row);
        }
    }

    let chroma_width = width / 2;
    let chroma_rows = height / 2;
    if chroma_width == 0 || chroma_rows == 0 {
        return true;
    }

    if planes.u_stride == chroma_width && planes.v_stride == chroma_width {
        let half = chroma_width * chroma_rows;
        interleave_uv_row(&planes.u[..half], &planes.v[..half], dst_chroma);
    } else {
        let row_uv_bytes = chroma_width * 2;
        for r in 0..chroma_rows {
            let u_row = &planes.u[r * planes.u_stride..r * planes.u_stride + chroma_width];
            let v_row = &planes.v[r * planes.v_stride..r * planes.v_stride + chroma_width];
            let d_row = &mut dst_chroma[r * row_uv_bytes..(r + 1) * row_uv_bytes];
            interleave_uv_row(u_row, v_row, d_row);
        }
    }
    true
}

// BT.601 limited range fixed-point constants (scaled by 2^10).
const BT601_SCALE: i32 = 10;
const BT601_HALF: i32 = 1 << (BT601_SCALE - 1);
const BT601_Y_GAIN: i32 = 1192;
const BT601_RV: i32 = 1634;
const BT601_GV: i32 = 833;
const BT601_GU: i32 = 400;
const BT601_BU: i32 = 2066;
const BT601_Y_OFFSET: i32 = BT601_Y_GAIN * 16;
const BT601_BASE_BIAS: i32 = BT601_HALF - BT601_Y_OFFSET;

#[inline(always)]
fn clamp_u8(value: i32) -> u8 {
    let non_neg = value & !(value >> 31);
    (non_neg | ((255 - non_neg) >> 31)) as u8
}

#[inline(always)]
unsafe fn write_bgra_pair_ptr(
    y_ptr: *const u8,
    dst_ptr: *mut u8,
    b_term: i32,
    g_term: i32,
    r_term: i32,
) {
    let y0_gain = BT601_Y_GAIN * (*y_ptr as i32);
    let b0 = clamp_u8((y0_gain + b_term) >> BT601_SCALE);
    let g0 = clamp_u8((y0_gain + g_term) >> BT601_SCALE);
    let r0 = clamp_u8((y0_gain + r_term) >> BT601_SCALE);

    let y1_gain = BT601_Y_GAIN * (*y_ptr.add(1) as i32);
    let b1 = clamp_u8((y1_gain + b_term) >> BT601_SCALE);
    let g1 = clamp_u8((y1_gain + g_term) >> BT601_SCALE);
    let r1 = clamp_u8((y1_gain + r_term) >> BT601_SCALE);

    let out = [b0, g0, r0, 255, b1, g1, r1, 255];
    std::ptr::copy_nonoverlapping(out.as_ptr(), dst_ptr, 8);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn write_planes_bgra_avx2(planes: &YuvPlanes<'_>, dst: &mut [u8], stride_pixels: usize) {
    use std::arch::x86_64::*;

    let width = planes.width as usize;
    let height = planes.height as usize;
    let chroma_pairs = width / 2;
    let dst_stride_bytes = stride_pixels * 4;

    let v_128_128 = _mm_set1_epi32(128);
    let v_bu_128 = _mm_set1_epi32(BT601_BU);
    let v_gu_128 = _mm_set1_epi32(BT601_GU);
    let v_gv_128 = _mm_set1_epi32(BT601_GV);
    let v_rv_128 = _mm_set1_epi32(BT601_RV);
    let v_bias_128 = _mm_set1_epi32(BT601_BASE_BIAS);
    let perm_dup = _mm256_setr_epi32(0, 0, 1, 1, 2, 2, 3, 3);

    let v_y_gain = _mm256_set1_epi32(BT601_Y_GAIN);
    let v_zero = _mm256_setzero_si256();
    let v_255 = _mm256_set1_epi32(255);
    let v_alpha = _mm256_set1_epi32(0xFF00_0000u32 as i32);

    let y_ptr = planes.y.as_ptr();
    let u_ptr = planes.u.as_ptr();
    let v_ptr = planes.v.as_ptr();
    let dst_ptr = dst.as_mut_ptr();

    let simd_pairs = chroma_pairs & !3usize;
    let pair_rows = height / 2;

    for pr in 0..pair_rows {
        let y0 = pr * 2;
        let y1 = y0 + 1;
        let y0_row = y_ptr.add(y0 * planes.y_stride);
        let y1_row = y_ptr.add(y1 * planes.y_stride);
        let u_row = u_ptr.add(pr * planes.u_stride);
        let v_row = v_ptr.add(pr * planes.v_stride);
        let d0_row = dst_ptr.add(y0 * dst_stride_bytes);
        let d1_row = dst_ptr.add(y1 * dst_stride_bytes);

        let mut cp = 0usize;
        while cp < simd_pairs {
            let px = cp * 2;
            let u4_raw = std::ptr::read_unaligned(u_row.add(cp) as *const i32);
            let v4_raw = std::ptr::read_unaligned(v_row.add(cp) as *const i32);
            let u4 = _mm_sub_epi32(_mm_cvtepu8_epi32(_mm_cvtsi32_si128(u4_raw)), v_128_128);
            let v4 = _mm_sub_epi32(_mm_cvtepu8_epi32(_mm_cvtsi32_si128(v4_raw)), v_128_128);

            let b_term4 = _mm_add_epi32(_mm_mullo_epi32(u4, v_bu_128), v_bias_128);
            let g_chroma4 =
                _mm_add_epi32(_mm_mullo_epi32(v4, v_gv_128), _mm_mullo_epi32(u4, v_gu_128));
            let g_term4 = _mm_sub_epi32(v_bias_128, g_chroma4);
            let r_term4 = _mm_add_epi32(_mm_mullo_epi32(v4, v_rv_128), v_bias_128);

            let b_term8 = _mm256_permutevar8x32_epi32(_mm256_castsi128_si256(b_term4), perm_dup);
            let g_term8 = _mm256_permutevar8x32_epi32(_mm256_castsi128_si256(g_term4), perm_dup);
            let r_term8 = _mm256_permutevar8x32_epi32(_mm256_castsi128_si256(r_term4), perm_dup);

            // Row 0: 8 pixels
            let y0_raw = _mm_loadl_epi64(y0_row.add(px) as *const __m128i);
            let y0_gain = _mm256_mullo_epi32(_mm256_cvtepu8_epi32(y0_raw), v_y_gain);
            let b0 = _mm256_min_epi32(
                v_255,
                _mm256_max_epi32(
                    v_zero,
                    _mm256_srai_epi32::<BT601_SCALE>(_mm256_add_epi32(y0_gain, b_term8)),
                ),
            );
            let g0 = _mm256_min_epi32(
                v_255,
                _mm256_max_epi32(
                    v_zero,
                    _mm256_srai_epi32::<BT601_SCALE>(_mm256_add_epi32(y0_gain, g_term8)),
                ),
            );
            let r0 = _mm256_min_epi32(
                v_255,
                _mm256_max_epi32(
                    v_zero,
                    _mm256_srai_epi32::<BT601_SCALE>(_mm256_add_epi32(y0_gain, r_term8)),
                ),
            );
            let bgra0 = _mm256_or_si256(
                _mm256_or_si256(b0, _mm256_slli_epi32::<8>(g0)),
                _mm256_or_si256(_mm256_slli_epi32::<16>(r0), v_alpha),
            );
            _mm256_storeu_si256(d0_row.add(px * 4) as *mut __m256i, bgra0);

            // Row 1: 8 pixels, reusing b_term8 / g_term8 / r_term8
            let y1_raw = _mm_loadl_epi64(y1_row.add(px) as *const __m128i);
            let y1_gain = _mm256_mullo_epi32(_mm256_cvtepu8_epi32(y1_raw), v_y_gain);
            let b1 = _mm256_min_epi32(
                v_255,
                _mm256_max_epi32(
                    v_zero,
                    _mm256_srai_epi32::<BT601_SCALE>(_mm256_add_epi32(y1_gain, b_term8)),
                ),
            );
            let g1 = _mm256_min_epi32(
                v_255,
                _mm256_max_epi32(
                    v_zero,
                    _mm256_srai_epi32::<BT601_SCALE>(_mm256_add_epi32(y1_gain, g_term8)),
                ),
            );
            let r1 = _mm256_min_epi32(
                v_255,
                _mm256_max_epi32(
                    v_zero,
                    _mm256_srai_epi32::<BT601_SCALE>(_mm256_add_epi32(y1_gain, r_term8)),
                ),
            );
            let bgra1 = _mm256_or_si256(
                _mm256_or_si256(b1, _mm256_slli_epi32::<8>(g1)),
                _mm256_or_si256(_mm256_slli_epi32::<16>(r1), v_alpha),
            );
            _mm256_storeu_si256(d1_row.add(px * 4) as *mut __m256i, bgra1);

            cp += 4;
        }

        while cp < chroma_pairs {
            let px = cp * 2;
            let u = *u_row.add(cp) as i32 - 128;
            let v = *v_row.add(cp) as i32 - 128;
            let b_term = BT601_BU * u + BT601_BASE_BIAS;
            let g_term = BT601_BASE_BIAS - (BT601_GV * v + BT601_GU * u);
            let r_term = BT601_RV * v + BT601_BASE_BIAS;

            write_bgra_pair_ptr(y0_row.add(px), d0_row.add(px * 4), b_term, g_term, r_term);
            write_bgra_pair_ptr(y1_row.add(px), d1_row.add(px * 4), b_term, g_term, r_term);
            cp += 1;
        }
    }
}

fn write_planes_bgra_scalar(planes: &YuvPlanes<'_>, dst: &mut [u8], stride_pixels: usize) {
    let width = planes.width as usize;
    let height = planes.height as usize;
    let chroma_pairs = width / 2;
    let dst_stride_bytes = stride_pixels * 4;
    let pair_rows = height / 2;

    let y_ptr = planes.y.as_ptr();
    let u_ptr = planes.u.as_ptr();
    let v_ptr = planes.v.as_ptr();
    let dst_ptr = dst.as_mut_ptr();

    for pr in 0..pair_rows {
        let y0 = pr * 2;
        let y1 = y0 + 1;
        // SAFETY: `planes_have_valid_bounds` verified Y/U/V bounds and caller verified `dst` bounds.
        unsafe {
            let y0_row = y_ptr.add(y0 * planes.y_stride);
            let y1_row = y_ptr.add(y1 * planes.y_stride);
            let u_row = u_ptr.add(pr * planes.u_stride);
            let v_row = v_ptr.add(pr * planes.v_stride);
            let d0_row = dst_ptr.add(y0 * dst_stride_bytes);
            let d1_row = dst_ptr.add(y1 * dst_stride_bytes);

            for cp in 0..chroma_pairs {
                let px = cp * 2;
                let u = *u_row.add(cp) as i32 - 128;
                let v = *v_row.add(cp) as i32 - 128;
                let b_term = BT601_BU * u + BT601_BASE_BIAS;
                let g_term = BT601_BASE_BIAS - (BT601_GV * v + BT601_GU * u);
                let r_term = BT601_RV * v + BT601_BASE_BIAS;

                write_bgra_pair_ptr(y0_row.add(px), d0_row.add(px * 4), b_term, g_term, r_term);
                write_bgra_pair_ptr(y1_row.add(px), d1_row.add(px * 4), b_term, g_term, r_term);
            }
        }
    }
}

/// Converts borrowed [`YuvPlanes`] directly to BGRA without intermediate I420 copying.
pub fn write_planes_bgra(planes: &YuvPlanes<'_>, dst: &mut [u8], stride_pixels: usize) -> bool {
    let (width, height) = (planes.width as usize, planes.height as usize);
    if !planes_have_valid_bounds(planes) {
        return false;
    }
    if stride_pixels < width || dst.len() < stride_pixels * height * 4 {
        return false;
    }
    if width < 2 || height < 2 {
        return true;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            // SAFETY: AVX2 support verified at runtime and bounds checked above.
            unsafe {
                write_planes_bgra_avx2(planes, dst, stride_pixels);
            }
            return true;
        }
    }

    write_planes_bgra_scalar(planes, dst, stride_pixels);
    true
}

/// A streaming H.264 decoder.
pub struct H264Decoder {
    inner: Decoder,
    /// SPS/PPS held from the last codec-config packet or keyframe.
    parameter_sets: Vec<u8>,
    /// The size the decoder is currently producing, if anything has decoded.
    dimensions: Option<Dimensions>,
    /// Reusable buffer for prepending parameter sets to access units.
    combined_buf: Vec<u8>,
}

impl std::fmt::Debug for H264Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("H264Decoder")
            .field("dimensions", &self.dimensions)
            .finish()
    }
}

impl H264Decoder {
    pub fn new() -> Result<Self, DecodeError> {
        let inner = Decoder::new().map_err(|e| DecodeError::Init(format!("{e}")))?;
        Ok(Self {
            inner,
            parameter_sets: Vec::new(),
            dimensions: None,
            combined_buf: Vec::new(),
        })
    }

    pub fn dimensions(&self) -> Option<Dimensions> {
        self.dimensions
    }

    /// Replaces the held parameter sets from a `codecConfig` packet.
    ///
    /// Returns false when the buffer holds no readable SPS, which is the case
    /// the C++ core warns about and discards — a codec-config packet that cannot
    /// size a decoder is not a codec-config packet.
    pub fn set_parameter_sets(&mut self, data: &[u8]) -> bool {
        match parse_avc_dimensions(data) {
            Some(dims) => {
                if self.parameter_sets.is_empty() || self.dimensions != Some(dims) {
                    tracing::info!("stream parameter sets: {}x{}", dims.width, dims.height);
                }
                self.parameter_sets.clear();
                self.parameter_sets.extend_from_slice(data);
                self.dimensions = Some(dims);
                let _ = self.inner.decode(data);
                true
            }
            None => false,
        }
    }

    pub fn parameter_sets(&self) -> &[u8] {
        &self.parameter_sets
    }

    /// Decodes one Annex-B access unit and invokes `f` on borrowed planar YUV
    /// views without allocating or copying an intermediate I420 frame buffer.
    pub fn decode_and_process<R>(
        &mut self,
        access_unit: &[u8],
        pts_us: u64,
        f: impl FnOnce(YuvPlanes<'_>, u64) -> R,
    ) -> Result<R, DecodeError> {
        if access_unit.is_empty() {
            return Err(DecodeError::Rejected);
        }

        // Re-read the size from every keyframe: the adaptive ladder restarts the
        // phone's encoder at a new size, and a decoder sized to the first one
        // rejects every frame after the change.
        let parsed_dims = parse_avc_dimensions(access_unit);
        if let Some(dims) = parsed_dims {
            if self.dimensions != Some(dims) {
                tracing::info!("stream resized to {}x{}", dims.width, dims.height);
                self.dimensions = Some(dims);
            }
        }

        let input = if parsed_dims.is_none() && !self.parameter_sets.is_empty() {
            self.combined_buf.clear();
            self.combined_buf
                .reserve(self.parameter_sets.len() + access_unit.len());
            self.combined_buf.extend_from_slice(&self.parameter_sets);
            self.combined_buf.extend_from_slice(access_unit);
            &self.combined_buf[..]
        } else {
            access_unit
        };

        let yuv = self
            .inner
            .decode(input)
            .map_err(|e| {
                tracing::debug!("decode error: {e}");
                DecodeError::Rejected
            })?
            .ok_or(DecodeError::NoPicture)?;

        let (width, height) = yuv.dimensions();
        let (w, h) = (width as u32, height as u32);
        self.dimensions = Some(Dimensions {
            width: w,
            height: h,
        });

        let (y_stride, u_stride, v_stride) = yuv.strides();
        let planes = YuvPlanes {
            width: w,
            height: h,
            y: yuv.y(),
            u: yuv.u(),
            v: yuv.v(),
            y_stride,
            u_stride,
            v_stride,
        };
        Ok(f(planes, pts_us))
    }

    /// Decodes one Annex-B access unit into `out` reusing `out.yuv` capacity.
    pub fn decode_with_pts_into(
        &mut self,
        access_unit: &[u8],
        pts_us: u64,
        out: &mut DecodedFrame,
    ) -> Result<(), DecodeError> {
        self.decode_and_process(access_unit, pts_us, |planes, pts| {
            let w = planes.width as usize;
            let h = planes.height as usize;
            let luma_len = w * h;
            let chroma_w = w / 2;
            let chroma_h = h / 2;
            let chroma_half = chroma_w * chroma_h;
            let needed = luma_len + chroma_half * 2;

            out.width = planes.width;
            out.height = planes.height;
            out.pts_us = pts;
            out.yuv.clear();
            if out.yuv.capacity() < needed {
                out.yuv.reserve(needed - out.yuv.len());
            }

            if planes.y_stride == w && planes.u_stride == chroma_w && planes.v_stride == chroma_w {
                out.yuv.extend_from_slice(&planes.y[..luma_len]);
                out.yuv.extend_from_slice(&planes.u[..chroma_half]);
                out.yuv.extend_from_slice(&planes.v[..chroma_half]);
            } else {
                for r in 0..h {
                    out.yuv
                        .extend_from_slice(&planes.y[r * planes.y_stride..r * planes.y_stride + w]);
                }
                for r in 0..chroma_h {
                    out.yuv.extend_from_slice(
                        &planes.u[r * planes.u_stride..r * planes.u_stride + chroma_w],
                    );
                }
                for r in 0..chroma_h {
                    out.yuv.extend_from_slice(
                        &planes.v[r * planes.v_stride..r * planes.v_stride + chroma_w],
                    );
                }
            }
        })
    }

    /// Decodes one Annex-B access unit with a presentation timestamp (PTS).
    ///
    /// On failure the caller should treat the stream as needing a new keyframe;
    /// `reset` returns the decoder to that state.
    pub fn decode_with_pts(
        &mut self,
        access_unit: &[u8],
        pts_us: u64,
    ) -> Result<DecodedFrame, DecodeError> {
        let mut frame = DecodedFrame {
            width: 0,
            height: 0,
            yuv: Vec::new(),
            pts_us,
        };
        self.decode_with_pts_into(access_unit, pts_us, &mut frame)?;
        Ok(frame)
    }

    /// Decodes one Annex-B access unit.
    pub fn decode(&mut self, access_unit: &[u8]) -> Result<DecodedFrame, DecodeError> {
        self.decode_with_pts(access_unit, 0)
    }

    /// Drops the current picture state so the next keyframe re-syncs.
    pub fn reset(&mut self) {
        if let Ok(mut fresh) = Decoder::new() {
            if !self.parameter_sets.is_empty() {
                let _ = fresh.decode(&self.parameter_sets);
            }
            self.inner = fresh;
        }
    }
}

/// Applies clockwise `rotation` (`0`, `90`, `180`, `270`) and then horizontal
/// `mirror` to a packed BGRA `u32` pixel buffer without allocating when `dst`
/// already has capacity. Returns the output `(width, height)`.
pub fn transform_bgra(
    src: &[u32],
    dst: &mut Vec<u32>,
    width: u32,
    height: u32,
    mirror: bool,
    rotation: u32,
) -> (u32, u32) {
    let w = width as usize;
    let h = height as usize;
    let count = w.saturating_mul(h);
    if count == 0 || src.len() < count {
        dst.clear();
        return (width, height);
    }
    let rot = normalize_rotation(rotation);
    let (out_w, out_h) = if matches!(rot, 90 | 270) {
        (height, width)
    } else {
        (width, height)
    };
    dst.resize(count, 0);
    transform_pixels(&src[..count], &mut dst[..count], w, h, mirror, rot);
    (out_w, out_h)
}

/// Byte-slice variant of [`transform_bgra`] for 4-byte BGRA frame buffers.
///
/// Uses aligned `u32` pixel moves when both slices are 4-byte aligned and falls
/// back to 4-byte pixel copies otherwise, reusing `dst` capacity with zero
/// heap allocation once warmed.
pub fn transform_bgra_bytes(
    src: &[u8],
    dst: &mut Vec<u8>,
    width: u32,
    height: u32,
    mirror: bool,
    rotation: u32,
) -> (u32, u32) {
    let w = width as usize;
    let h = height as usize;
    let count = w.saturating_mul(h);
    let byte_len = count.saturating_mul(4);
    if count == 0 || src.len() < byte_len {
        dst.clear();
        return (width, height);
    }
    let rot = normalize_rotation(rotation);
    let (out_w, out_h) = if matches!(rot, 90 | 270) {
        (height, width)
    } else {
        (width, height)
    };
    dst.resize(byte_len, 0);

    // SAFETY: `align_to` / `align_to_mut` checks alignment dynamically; when
    // both `src` and `dst` are 4-byte aligned (the standard allocator case),
    // we operate directly on `&[u32]` / `&mut [u32]` pixel words.
    let (s_pre, s_u32, _) = unsafe { src[..byte_len].align_to::<u32>() };
    let (d_pre, d_u32, _) = unsafe { dst[..byte_len].align_to_mut::<u32>() };
    if s_pre.is_empty() && d_pre.is_empty() && s_u32.len() == count && d_u32.len() == count {
        transform_u32_fast(s_u32, d_u32, w, h, mirror, rot);
    } else {
        let (s_chunks, _) = src[..byte_len].as_chunks::<4>();
        let (d_chunks, _) = dst[..byte_len].as_chunks_mut::<4>();
        transform_pixels(s_chunks, d_chunks, w, h, mirror, rot);
    }
    (out_w, out_h)
}

fn transform_u32_fast(src: &[u32], dst: &mut [u32], w: usize, h: usize, mirror: bool, rot: u32) {
    #[cfg(target_arch = "x86_64")]
    if matches!((rot, mirror), (0, true) | (180, false))
        && std::arch::is_x86_feature_detected!("avx2")
    {
        // SAFETY: AVX2 checked and `src`/`dst` both have `w * h` u32 elements.
        unsafe {
            reverse_rows_u32_avx2(src, dst, w, h, rot == 180);
        }
        return;
    }
    transform_pixels(src, dst, w, h, mirror, rot);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn reverse_rows_u32_avx2(src: &[u32], dst: &mut [u32], w: usize, h: usize, flip_y: bool) {
    use std::arch::x86_64::*;
    let perm_rev = _mm256_setr_epi32(7, 6, 5, 4, 3, 2, 1, 0);
    let src_ptr = src.as_ptr();
    let dst_ptr = dst.as_mut_ptr();

    for dy in 0..h {
        let sy = if flip_y { h - 1 - dy } else { dy };
        let s_row = src_ptr.add(sy * w);
        let d_row = dst_ptr.add(dy * w);
        let mut x = 0usize;
        while x + 8 <= w {
            let v = _mm256_loadu_si256(s_row.add(w - x - 8) as *const __m256i);
            let r = _mm256_permutevar8x32_epi32(v, perm_rev);
            _mm256_storeu_si256(d_row.add(x) as *mut __m256i, r);
            x += 8;
        }
        while x < w {
            *d_row.add(x) = *s_row.add(w - 1 - x);
            x += 1;
        }
    }
}

#[inline]
fn normalize_rotation(rotation: u32) -> u32 {
    match (rotation % 360) / 90 * 90 {
        90 => 90,
        180 => 180,
        270 => 270,
        _ => 0,
    }
}

#[allow(clippy::needless_range_loop)]
fn transform_pixels<T: Copy>(src: &[T], dst: &mut [T], w: usize, h: usize, mirror: bool, rot: u32) {
    match (rot, mirror) {
        (0, false) => {
            dst.copy_from_slice(src);
        }
        (0, true) => {
            for y in 0..h {
                let row_start = y * w;
                let src_row = &src[row_start..row_start + w];
                let dst_row = &mut dst[row_start..row_start + w];
                for (d, s) in dst_row.iter_mut().zip(src_row.iter().rev()) {
                    *d = *s;
                }
            }
        }
        (180, false) => {
            for dy in 0..h {
                let sy = h - 1 - dy;
                let src_row = &src[sy * w..sy * w + w];
                let dst_row = &mut dst[dy * w..dy * w + w];
                for (d, s) in dst_row.iter_mut().zip(src_row.iter().rev()) {
                    *d = *s;
                }
            }
        }
        (180, true) => {
            for dy in 0..h {
                let sy = h - 1 - dy;
                let src_row = &src[sy * w..sy * w + w];
                let dst_row = &mut dst[dy * w..dy * w + w];
                dst_row.copy_from_slice(src_row);
            }
        }
        (90, false) => {
            let (out_w, out_h) = (h, w);
            for dy_block in (0..out_h).step_by(16) {
                let dy_end = (dy_block + 16).min(out_h);
                for dx_block in (0..out_w).step_by(16) {
                    let dx_end = (dx_block + 16).min(out_w);
                    for dy in dy_block..dy_end {
                        let dst_row = &mut dst[dy * out_w..dy * out_w + out_w];
                        for dx in dx_block..dx_end {
                            let sy = h - 1 - dx;
                            dst_row[dx] = src[sy * w + dy];
                        }
                    }
                }
            }
        }
        (90, true) => {
            let (out_w, out_h) = (h, w);
            for dy_block in (0..out_h).step_by(16) {
                let dy_end = (dy_block + 16).min(out_h);
                for dx_block in (0..out_w).step_by(16) {
                    let dx_end = (dx_block + 16).min(out_w);
                    for dy in dy_block..dy_end {
                        let dst_row = &mut dst[dy * out_w..dy * out_w + out_w];
                        for dx in dx_block..dx_end {
                            dst_row[dx] = src[dx * w + dy];
                        }
                    }
                }
            }
        }
        (270, false) => {
            let (out_w, out_h) = (h, w);
            for dy_block in (0..out_h).step_by(16) {
                let dy_end = (dy_block + 16).min(out_h);
                for dx_block in (0..out_w).step_by(16) {
                    let dx_end = (dx_block + 16).min(out_w);
                    for dy in dy_block..dy_end {
                        let dst_row = &mut dst[dy * out_w..dy * out_w + out_w];
                        let sx = w - 1 - dy;
                        for dx in dx_block..dx_end {
                            dst_row[dx] = src[dx * w + sx];
                        }
                    }
                }
            }
        }
        (270, true) => {
            let (out_w, out_h) = (h, w);
            for dy_block in (0..out_h).step_by(16) {
                let dy_end = (dy_block + 16).min(out_h);
                for dx_block in (0..out_w).step_by(16) {
                    let dx_end = (dx_block + 16).min(out_w);
                    for dy in dy_block..dy_end {
                        let dst_row = &mut dst[dy * out_w..dy * out_w + out_w];
                        let sx = w - 1 - dy;
                        for dx in dx_block..dx_end {
                            let sy = h - 1 - dx;
                            dst_row[dx] = src[sy * w + sx];
                        }
                    }
                }
            }
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_constructs() {
        assert!(H264Decoder::new().is_ok());
    }

    #[test]
    fn garbage_is_rejected_rather_than_panicking() {
        let mut decoder = H264Decoder::new().unwrap();
        assert!(decoder
            .decode(&[0x00, 0x00, 0x01, 0x65, 0xff, 0xff])
            .is_err());
        assert!(decoder.decode(&[]).is_err());
    }

    #[test]
    fn a_codec_config_without_an_sps_is_refused() {
        let mut decoder = H264Decoder::new().unwrap();
        let mut idr = vec![0u8, 0, 0, 1, 0x65];
        idr.extend_from_slice(&[0x88; 16]);
        assert!(!decoder.set_parameter_sets(&idr));
        assert!(decoder.parameter_sets().is_empty());
    }

    #[test]
    fn parameter_sets_are_preloaded_preserved_on_reset_and_prepended() {
        let mut decoder = H264Decoder::new().unwrap();
        let sps = sps::tests::make_sps(80, 45, 0); // 1280x720
        assert!(decoder.set_parameter_sets(&sps));
        assert_eq!(decoder.parameter_sets(), &sps[..]);
        assert_eq!(
            decoder.dimensions(),
            Some(Dimensions {
                width: 1280,
                height: 720
            })
        );

        // reset preserves parameter sets and re-decodes them
        decoder.reset();
        assert_eq!(decoder.parameter_sets(), &sps[..]);

        // A standalone IDR without SPS: parse_avc_dimensions is None, so parameter_sets are prepended
        let standalone_idr = vec![0u8, 0, 0, 1, 0x65, 0x88, 0x88];
        assert!(parse_avc_dimensions(&standalone_idr).is_none());
        // Decoding malformed NAL payload fails, but combined buffer was formed and rejected cleanly
        assert!(decoder.decode(&standalone_idr).is_err());
    }

    fn grey_frame() -> DecodedFrame {
        let (w, h) = (4usize, 2usize);
        let mut yuv = vec![126u8; w * h];
        yuv.extend(std::iter::repeat_n(128u8, w * h / 2));
        yuv.extend(std::iter::repeat_n(128u8, w * h / 2));
        DecodedFrame {
            width: w as u32,
            height: h as u32,
            yuv,
            pts_us: 0,
        }
    }

    #[test]
    fn decoded_frame_mean_luma_matches_y_plane() {
        let frame = grey_frame();
        assert_eq!(frame.mean_luma(), 126);
    }

    #[test]
    fn nv12_interleaves_chroma_pairs() {
        let frame = grey_frame();
        let mut nv12 = vec![0u8; 4 * 2 * 3 / 2];
        assert!(frame.write_nv12(&mut nv12));
        assert!(nv12[..8].iter().all(|&b| b == 126));
        assert_eq!(&nv12[8..], &[128, 128, 128, 128]);
    }

    #[test]
    fn nv12_refuses_a_mismatched_buffer() {
        let frame = grey_frame();
        let mut too_small = vec![0u8; 4];
        assert!(!frame.write_nv12(&mut too_small));
    }

    /// BGRA, not RGBA. A swap here is invisible in a grey test and obvious in a
    /// real picture, so it is pinned with a coloured frame.
    #[test]
    fn bgra_emits_blue_then_green_then_red() {
        // Pure red in limited-range YUV601: Y=81, U=90, V=240.
        let frame = DecodedFrame {
            width: 2,
            height: 2,
            yuv: vec![81, 81, 81, 81, 90, 240],
            pts_us: 0,
        };
        let mut bgra = vec![0u8; 2 * 2 * 4];
        assert!(frame.write_bgra(&mut bgra, 2));
        let (b, g, r, a) = (bgra[0], bgra[1], bgra[2], bgra[3]);
        assert!(r > 200, "red channel should be high, got {r}");
        assert!(g < 90, "green channel should be low, got {g}");
        assert!(b < 90, "blue channel should be low, got {b}");
        assert_eq!(a, 255);
    }

    #[test]
    fn bgra_honours_the_stride() {
        let frame = grey_frame();
        let stride = 8;
        let mut bgra = vec![0u8; stride * 2 * 4];
        assert!(frame.write_bgra(&mut bgra, stride));
        assert_eq!(bgra[3], 255);
        assert_eq!(bgra[4 * 3 + 3], 255);
        assert_eq!(bgra[4 * 4], 0);
    }

    #[test]
    fn bgra_refuses_a_buffer_that_is_too_small() {
        let frame = grey_frame();
        let mut tiny = vec![0u8; 8];
        assert!(!frame.write_bgra(&mut tiny, 4));
    }

    #[test]
    fn bgra_matches_reference_bt601_math() {
        // Test frame with a variety of luma and chroma values
        let (w, h) = (6usize, 4usize);
        let mut yuv = Vec::new();
        for y in 0..h {
            for x in 0..w {
                yuv.push(((x * 40 + y * 50) % 256) as u8);
            }
        }
        // U plane (w*h/4 = 6)
        yuv.extend_from_slice(&[16, 64, 128, 180, 220, 240]);
        // V plane (w*h/4 = 6)
        yuv.extend_from_slice(&[240, 200, 128, 90, 50, 16]);

        let frame = DecodedFrame {
            width: w as u32,
            height: h as u32,
            yuv: yuv.clone(),
            pts_us: 0,
        };
        let mut bgra = vec![0u8; w * h * 4];
        assert!(frame.write_bgra(&mut bgra, w));

        // Reference scalar calculation
        let (u_plane, v_plane) = yuv[w * h..].split_at(w * h / 4);
        let chroma_w = w / 2;
        const SCALE: i32 = 10;
        const HALF: i32 = 1 << (SCALE - 1);
        const Y_GAIN: i32 = 1192;
        const RV: i32 = 1634;
        const GV: i32 = 833;
        const GU: i32 = 400;
        const BU: i32 = 2066;

        let clamp_ref = |v: i32| -> u8 {
            if v < 0 {
                0
            } else if v > 255 {
                255
            } else {
                v as u8
            }
        };

        for y in 0..h {
            for x in 0..w {
                let luma = yuv[y * w + x] as i32 - 16;
                let c_idx = (y / 2) * chroma_w + (x / 2);
                let u = u_plane[c_idx] as i32 - 128;
                let v = v_plane[c_idx] as i32 - 128;
                let c = Y_GAIN * luma;

                let b_ref = clamp_ref((c + BU * u + HALF) >> SCALE);
                let g_ref = clamp_ref((c - GV * v - GU * u + HALF) >> SCALE);
                let r_ref = clamp_ref((c + RV * v + HALF) >> SCALE);

                let offset = (y * w + x) * 4;
                assert_eq!(bgra[offset], b_ref, "B mismatch at ({x},{y})");
                assert_eq!(bgra[offset + 1], g_ref, "G mismatch at ({x},{y})");
                assert_eq!(bgra[offset + 2], r_ref, "R mismatch at ({x},{y})");
                assert_eq!(bgra[offset + 3], 255, "A mismatch at ({x},{y})");
            }
        }
    }

    #[test]
    fn nv12_matches_reference_interleave() {
        let (w, h) = (6usize, 4usize);
        let mut yuv = vec![42u8; w * h];
        let u_data: Vec<u8> = (0..(w * h / 4)).map(|i| (i * 30 + 10) as u8).collect();
        let v_data: Vec<u8> = (0..(w * h / 4)).map(|i| (i * 30 + 50) as u8).collect();
        yuv.extend(&u_data);
        yuv.extend(&v_data);

        let frame = DecodedFrame {
            width: w as u32,
            height: h as u32,
            yuv,
            pts_us: 0,
        };
        let mut nv12 = vec![0u8; w * h * 3 / 2];
        assert!(frame.write_nv12(&mut nv12));

        for i in 0..(w * h / 4) {
            assert_eq!(nv12[w * h + i * 2], u_data[i], "U mismatch at {i}");
            assert_eq!(nv12[w * h + i * 2 + 1], v_data[i], "V mismatch at {i}");
        }
    }

    #[test]
    fn transform_bgra_mirror_and_rotations() {
        // 3x2 image:
        // [1, 2, 3]
        // [4, 5, 6]
        let src = [1u32, 2, 3, 4, 5, 6];
        let mut dst = Vec::new();

        // 0 deg + mirror
        assert_eq!(transform_bgra(&src, &mut dst, 3, 2, true, 0), (3, 2));
        assert_eq!(dst, vec![3, 2, 1, 6, 5, 4]);

        // 90 deg CW -> 2x3:
        // [4, 1]
        // [5, 2]
        // [6, 3]
        assert_eq!(transform_bgra(&src, &mut dst, 3, 2, false, 90), (2, 3));
        assert_eq!(dst, vec![4, 1, 5, 2, 6, 3]);

        // 90 deg CW + mirror -> 2x3:
        // [1, 4]
        // [2, 5]
        // [3, 6]
        assert_eq!(transform_bgra(&src, &mut dst, 3, 2, true, 90), (2, 3));
        assert_eq!(dst, vec![1, 4, 2, 5, 3, 6]);

        // 180 deg -> 3x2:
        // [6, 5, 4]
        // [3, 2, 1]
        assert_eq!(transform_bgra(&src, &mut dst, 3, 2, false, 180), (3, 2));
        assert_eq!(dst, vec![6, 5, 4, 3, 2, 1]);

        // 180 deg + mirror -> 3x2 (vertical flip):
        // [4, 5, 6]
        // [1, 2, 3]
        assert_eq!(transform_bgra(&src, &mut dst, 3, 2, true, 180), (3, 2));
        assert_eq!(dst, vec![4, 5, 6, 1, 2, 3]);

        // 270 deg CW -> 2x3:
        // [3, 6]
        // [2, 5]
        // [1, 4]
        assert_eq!(transform_bgra(&src, &mut dst, 3, 2, false, 270), (2, 3));
        assert_eq!(dst, vec![3, 6, 2, 5, 1, 4]);

        // 270 deg CW + mirror -> 2x3:
        // [6, 3]
        // [5, 2]
        // [4, 1]
        assert_eq!(transform_bgra(&src, &mut dst, 3, 2, true, 270), (2, 3));
        assert_eq!(dst, vec![6, 3, 5, 2, 4, 1]);
    }

    #[test]
    fn transform_bgra_bytes_matches_u32_and_reuses_buffer() {
        let src_u32 = [
            0x11223344u32,
            0x55667788,
            0x99AABBCC,
            0xDDEEFF00,
            0x01020304,
            0x05060708,
        ];
        let mut src_bytes = Vec::with_capacity(src_u32.len() * 4);
        for px in src_u32 {
            src_bytes.extend_from_slice(&px.to_ne_bytes());
        }
        let mut dst_bytes = Vec::with_capacity(64);
        let initial_ptr = dst_bytes.as_ptr();

        let dims = transform_bgra_bytes(&src_bytes, &mut dst_bytes, 3, 2, false, 90);
        assert_eq!(dims, (2, 3));
        assert_eq!(
            dst_bytes.as_ptr(),
            initial_ptr,
            "must reuse preallocated capacity"
        );

        let mut dst_u32 = Vec::new();
        transform_bgra(&src_u32, &mut dst_u32, 3, 2, false, 90);
        let expected_bytes: Vec<u8> = dst_u32.iter().flat_map(|p| p.to_ne_bytes()).collect();
        assert_eq!(dst_bytes, expected_bytes);
    }

    #[test]
    fn write_planes_bgra_and_nv12_match_scalar_reference_including_strided() {
        for &(w, h, pad_y, pad_uv) in &[
            (6u32, 4u32, 0usize, 0usize),
            (18, 4, 8, 4),
            (32, 8, 64, 32),
            (128, 16, 16, 8),
        ] {
            let packed_luma = (w * h) as usize;
            let cw = (w / 2) as usize;
            let ch = (h / 2) as usize;
            let mut packed = vec![0u8; packed_luma * 3 / 2];
            for (i, b) in packed.iter_mut().enumerate() {
                *b = ((i * 31 + 17) & 0xFF) as u8;
            }
            let frame = DecodedFrame::new(w, h, packed.clone(), 123);

            let mut expected_bgra = vec![0u8; (w * h * 4) as usize];
            write_planes_bgra_scalar(&frame.as_planes().unwrap(), &mut expected_bgra, w as usize);

            let mut got_bgra = vec![0u8; (w * h * 4) as usize];
            assert!(frame.write_bgra(&mut got_bgra, w as usize));
            assert_eq!(got_bgra, expected_bgra, "write_bgra mismatch at {w}x{h}");

            // Build strided planes and verify both write_planes_bgra & write_planes_nv12 & mean_luma.
            let y_stride = w as usize + pad_y;
            let u_stride = cw + pad_uv;
            let v_stride = cw + pad_uv;
            let mut y_strided = vec![0xA5u8; h as usize * y_stride];
            let mut u_strided = vec![0x5Au8; ch * u_stride];
            let mut v_strided = vec![0x3Cu8; ch * v_stride];
            for r in 0..h as usize {
                y_strided[r * y_stride..r * y_stride + w as usize]
                    .copy_from_slice(&packed[r * w as usize..(r + 1) * w as usize]);
            }
            let u_src = &packed[packed_luma..packed_luma + cw * ch];
            let v_src = &packed[packed_luma + cw * ch..];
            for r in 0..ch {
                u_strided[r * u_stride..r * u_stride + cw]
                    .copy_from_slice(&u_src[r * cw..(r + 1) * cw]);
                v_strided[r * v_stride..r * v_stride + cw]
                    .copy_from_slice(&v_src[r * cw..(r + 1) * cw]);
            }
            let strided = YuvPlanes {
                width: w,
                height: h,
                y: &y_strided,
                u: &u_strided,
                v: &v_strided,
                y_stride,
                u_stride,
                v_stride,
            };

            let mut strided_bgra = vec![0u8; (w * h * 4) as usize];
            assert!(write_planes_bgra(&strided, &mut strided_bgra, w as usize));
            assert_eq!(
                strided_bgra, expected_bgra,
                "strided bgra mismatch at {w}x{h}"
            );

            let mut expected_nv12 = vec![0u8; packed_luma * 3 / 2];
            assert!(frame.write_nv12(&mut expected_nv12));
            let mut strided_nv12 = vec![0u8; packed_luma * 3 / 2];
            assert!(write_planes_nv12(&strided, &mut strided_nv12));
            assert_eq!(
                strided_nv12, expected_nv12,
                "strided nv12 mismatch at {w}x{h}"
            );
            assert_eq!(strided.mean_luma(), frame.mean_luma());
        }
    }

    #[test]
    #[allow(clippy::chunks_exact_to_as_chunks)]
    fn reverse_rows_u32_matches_scalar_mirror_and_180() {
        let (w, h) = (35u32, 6u32);
        let src: Vec<u8> = (0..(w * h * 4) as usize)
            .map(|i| (i & 0xFF) as u8)
            .collect();
        let src_u32: Vec<u32> = src
            .chunks_exact(4)
            .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        for &(mirror, rot) in &[(true, 0u32), (false, 180u32)] {
            let mut got_bytes = Vec::new();
            let dims = transform_bgra_bytes(&src, &mut got_bytes, w, h, mirror, rot);
            assert_eq!(dims, (w, h));
            let mut want_u32 = Vec::new();
            transform_bgra(&src_u32, &mut want_u32, w, h, mirror, rot);
            let want_bytes: Vec<u8> = want_u32.iter().flat_map(|p| p.to_ne_bytes()).collect();
            assert_eq!(got_bytes, want_bytes);
        }
    }

    #[test]
    fn perf_1080p_csc_nv12_and_transform_throughput() {
        let (w, h) = (1920u32, 1080u32);
        let luma = (w * h) as usize;
        let mut yuv = vec![128u8; luma * 3 / 2];
        for (i, b) in yuv.iter_mut().enumerate() {
            *b = ((i * 7) & 0xFF) as u8;
        }
        let frame = DecodedFrame::new(w, h, yuv, 0);
        let planes = frame.as_planes().unwrap();

        let mut nv12 = vec![0u8; luma * 3 / 2];
        let mut bgra = vec![0u8; luma * 4];
        let mut transformed = Vec::with_capacity(luma * 4);
        let iters = 200u32;

        let t0 = std::time::Instant::now();
        for _ in 0..iters {
            assert!(write_planes_nv12(&planes, &mut nv12));
            std::hint::black_box(&nv12);
        }
        let nv12_ms = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(iters);
        eprintln!(
            "1080p write_planes_nv12: {nv12_ms:.3} ms/frame ({:.1} FPS)",
            1000.0 / nv12_ms.max(1e-6)
        );

        let t1 = std::time::Instant::now();
        for _ in 0..iters {
            assert!(write_planes_bgra(&planes, &mut bgra, w as usize));
            std::hint::black_box(&bgra);
        }
        let bgra_ms = t1.elapsed().as_secs_f64() * 1000.0 / f64::from(iters);
        eprintln!(
            "1080p write_planes_bgra: {bgra_ms:.3} ms/frame ({:.1} FPS)",
            1000.0 / bgra_ms.max(1e-6)
        );

        let t2 = std::time::Instant::now();
        for _ in 0..iters {
            transform_bgra_bytes(&bgra, &mut transformed, w, h, true, 0);
            std::hint::black_box(&transformed);
        }
        let mir_ms = t2.elapsed().as_secs_f64() * 1000.0 / f64::from(iters);
        eprintln!(
            "1080p transform_bgra_bytes(mirror): {mir_ms:.3} ms/frame ({:.1} FPS)",
            1000.0 / mir_ms.max(1e-6)
        );
    }
}
