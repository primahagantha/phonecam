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
        Self { width, height, yuv, pts_us }
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
        let (width, height) = (self.width as usize, self.height as usize);
        let luma_len = width * height;
        let chroma_len = luma_len / 2;
        if self.yuv.len() < luma_len + chroma_len {
            return false;
        }
        if dst.len() != luma_len + chroma_len {
            return false;
        }

        let (dst_luma, dst_chroma) = dst.split_at_mut(luma_len);
        dst_luma.copy_from_slice(&self.yuv[..luma_len]);

        let (u_plane, v_plane) = self.yuv[luma_len..luma_len + chroma_len].split_at(chroma_len / 2);
        let u_chunks = u_plane.chunks_exact(2);
        let v_chunks = v_plane.chunks_exact(2);
        let mut dst_chunks = dst_chroma.chunks_exact_mut(4);
        let u_rem = u_chunks.remainder();
        let v_rem = v_chunks.remainder();

        for ((u, v), out) in u_chunks.zip(v_chunks).zip(&mut dst_chunks) {
            let packed = (u[0] as u32)
                | ((v[0] as u32) << 8)
                | ((u[1] as u32) << 16)
                | ((v[1] as u32) << 24);
            out.copy_from_slice(&packed.to_le_bytes());
        }

        let dst_rem = dst_chunks.into_remainder();
        for i in 0..u_rem.len() {
            dst_rem[i * 2] = u_rem[i];
            dst_rem[i * 2 + 1] = v_rem[i];
        }
        true
    }

    /// Converts to BGRA for display.
    ///
    /// gpui's `RenderImage` expects **BGRA, premultiplied**, so the channel order
    /// here is B, G, R, A. Emitting RGBA produces a picture with red and blue
    /// swapped, which looks like a broken camera rather than a broken
    /// conversion.
    pub fn write_bgra(&self, dst: &mut [u8], stride_pixels: usize) -> bool {
        let (width, height) = (self.width as usize, self.height as usize);
        let luma_len = width * height;
        let chroma_len = luma_len / 2;
        if self.yuv.len() < luma_len + chroma_len {
            return false;
        }
        if stride_pixels < width || dst.len() < stride_pixels * height * 4 {
            return false;
        }

        let (u_plane, v_plane) = self.yuv[luma_len..luma_len + chroma_len].split_at(chroma_len / 2);
        let chroma_width = width / 2;

        // BT.601 limited range in fixed point, the transform the phone's encoder
        // targets. The float version cost about 8 ms per 720p frame on the
        // publish path; the integer form is the same arithmetic with the scale
        // folded in, and the clamp is a saturating cast.
        const SCALE: i32 = 10;
        const HALF: i32 = 1 << (SCALE - 1);
        // 1.164, 1.596, 0.813, 0.391, 2.018 scaled by 2^10.
        const Y_GAIN: i32 = 1192;
        const RV: i32 = 1634;
        const GV: i32 = 833;
        const GU: i32 = 400;
        const BU: i32 = 2066;

        #[inline(always)]
        fn clamp(value: i32) -> u8 {
            let non_neg = value & !(value >> 31);
            (non_neg | ((255 - non_neg) >> 31)) as u8
        }

        const Y_OFFSET: i32 = Y_GAIN * 16;

        for y in 0..height {
            let luma_start = y * width;
            let luma_row = &self.yuv[luma_start..luma_start + width];
            let chroma_start = (y / 2) * chroma_width;
            let u_row = &u_plane[chroma_start..chroma_start + chroma_width];
            let v_row = &v_plane[chroma_start..chroma_start + chroma_width];
            let row_dst = &mut dst[y * stride_pixels * 4..y * stride_pixels * 4 + width * 4];

            let luma_chunks = luma_row.chunks_exact(2);
            let dst_chunks = row_dst.chunks_exact_mut(8);

            for (((&u_byte, &v_byte), y_pair), dst_pair) in
                u_row.iter().zip(v_row.iter()).zip(luma_chunks).zip(dst_chunks)
            {
                let &[y0, y1] = y_pair else { continue };
                let Ok(dst_8): Result<&mut [u8; 8], _> = dst_pair.try_into() else { continue };

                let u = u_byte as i32 - 128;
                let v = v_byte as i32 - 128;

                let bu = BU * u;
                let g_chroma = GV * v + GU * u;
                let rv = RV * v;

                let b_term = bu + HALF - Y_OFFSET;
                let g_term = HALF - g_chroma - Y_OFFSET;
                let r_term = rv + HALF - Y_OFFSET;

                let y0_gain = Y_GAIN * y0 as i32;
                let b0 = clamp((y0_gain + b_term) >> SCALE);
                let g0 = clamp((y0_gain + g_term) >> SCALE);
                let r0 = clamp((y0_gain + r_term) >> SCALE);

                let y1_gain = Y_GAIN * y1 as i32;
                let b1 = clamp((y1_gain + b_term) >> SCALE);
                let g1 = clamp((y1_gain + g_term) >> SCALE);
                let r1 = clamp((y1_gain + r_term) >> SCALE);

                *dst_8 = [b0, g0, r0, 255, b1, g1, r1, 255];
            }
        }
        true
    }
}

/// A streaming H.264 decoder.
pub struct H264Decoder {
    inner: Decoder,
    /// SPS/PPS held from the last codec-config packet or keyframe.
    parameter_sets: Vec<u8>,
    /// The size the decoder is currently producing, if anything has decoded.
    dimensions: Option<Dimensions>,
}

impl std::fmt::Debug for H264Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("H264Decoder").field("dimensions", &self.dimensions).finish()
    }
}

impl H264Decoder {
    pub fn new() -> Result<Self, DecodeError> {
        let inner = Decoder::new().map_err(|e| DecodeError::Init(format!("{e}")))?;
        Ok(Self { inner, parameter_sets: Vec::new(), dimensions: None })
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
                true
            }
            None => false,
        }
    }

    pub fn parameter_sets(&self) -> &[u8] {
        &self.parameter_sets
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
        if access_unit.is_empty() {
            return Err(DecodeError::Rejected);
        }

        // Re-read the size from every keyframe: the adaptive ladder restarts the
        // phone's encoder at a new size, and a decoder sized to the first one
        // rejects every frame after the change.
        if let Some(dims) = parse_avc_dimensions(access_unit) {
            if self.dimensions != Some(dims) {
                tracing::info!("stream resized to {}x{}", dims.width, dims.height);
                self.dimensions = Some(dims);
            }
        }

        let yuv = self
            .inner
            .decode(access_unit)
            .map_err(|e| {
                tracing::debug!("decode error: {e}");
                DecodeError::Rejected
            })?
            .ok_or(DecodeError::NoPicture)?;

        let (width, height) = yuv.dimensions();
        let (w, h) = (width as u32, height as u32);
        self.dimensions = Some(Dimensions { width: w, height: h });

        // openh264 hands back three planes; flatten them into I420 so the
        // converter has one contiguous buffer to work from.
        let (y_plane, u_plane, v_plane) = (yuv.y(), yuv.u(), yuv.v());
        let mut buffer = Vec::with_capacity(y_plane.len() + u_plane.len() + v_plane.len());
        buffer.extend_from_slice(y_plane);
        buffer.extend_from_slice(u_plane);
        buffer.extend_from_slice(v_plane);

        Ok(DecodedFrame { width: w, height: h, yuv: buffer, pts_us })
    }

    /// Decodes one Annex-B access unit.
    pub fn decode(&mut self, access_unit: &[u8]) -> Result<DecodedFrame, DecodeError> {
        self.decode_with_pts(access_unit, 0)
    }

    /// Drops the current picture state so the next keyframe re-syncs.
    pub fn reset(&mut self) {
        if let Ok(fresh) = Decoder::new() {
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
    let (out_w, out_h) = if matches!(rot, 90 | 270) { (height, width) } else { (width, height) };
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
    let (out_w, out_h) = if matches!(rot, 90 | 270) { (height, width) } else { (width, height) };
    dst.resize(byte_len, 0);

    // SAFETY: `align_to` / `align_to_mut` checks alignment dynamically; when
    // both `src` and `dst` are 4-byte aligned (the standard allocator case),
    // we operate directly on `&[u32]` / `&mut [u32]` pixel words.
    let (s_pre, s_u32, _) = unsafe { src[..byte_len].align_to::<u32>() };
    let (d_pre, d_u32, _) = unsafe { dst[..byte_len].align_to_mut::<u32>() };
    if s_pre.is_empty() && d_pre.is_empty() && s_u32.len() == count && d_u32.len() == count {
        transform_pixels(s_u32, d_u32, w, h, mirror, rot);
    } else {
        let (s_chunks, _) = src[..byte_len].as_chunks::<4>();
        let (d_chunks, _) = dst[..byte_len].as_chunks_mut::<4>();
        transform_pixels(s_chunks, d_chunks, w, h, mirror, rot);
    }
    (out_w, out_h)
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

fn transform_pixels<T: Copy>(
    src: &[T],
    dst: &mut [T],
    w: usize,
    h: usize,
    mirror: bool,
    rot: u32,
) {
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
        assert!(decoder.decode(&[0x00, 0x00, 0x01, 0x65, 0xff, 0xff]).is_err());
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

    fn grey_frame() -> DecodedFrame {
        let (w, h) = (4usize, 2usize);
        let mut yuv = vec![126u8; w * h];
        yuv.extend(std::iter::repeat(128u8).take(w * h / 2));
        yuv.extend(std::iter::repeat(128u8).take(w * h / 2));
        DecodedFrame { width: w as u32, height: h as u32, yuv, pts_us: 0 }
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
        let frame = DecodedFrame { width: 2, height: 2, yuv: vec![81, 81, 81, 81, 90, 240], pts_us: 0 };
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

        let frame = DecodedFrame { width: w as u32, height: h as u32, yuv: yuv.clone(), pts_us: 0 };
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
            if v < 0 { 0 } else if v > 255 { 255 } else { v as u8 }
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

        let frame = DecodedFrame { width: w as u32, height: h as u32, yuv, pts_us: 0 };
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
        let src_u32 = [0x11223344u32, 0x55667788, 0x99AABBCC, 0xDDEEFF00, 0x01020304, 0x05060708];
        let mut src_bytes = Vec::with_capacity(src_u32.len() * 4);
        for px in src_u32 {
            src_bytes.extend_from_slice(&px.to_ne_bytes());
        }
        let mut dst_bytes = Vec::with_capacity(64);
        let initial_ptr = dst_bytes.as_ptr();

        let dims = transform_bgra_bytes(&src_bytes, &mut dst_bytes, 3, 2, false, 90);
        assert_eq!(dims, (2, 3));
        assert_eq!(dst_bytes.as_ptr(), initial_ptr, "must reuse preallocated capacity");

        let mut dst_u32 = Vec::new();
        transform_bgra(&src_u32, &mut dst_u32, 3, 2, false, 90);
        let expected_bytes: Vec<u8> = dst_u32.iter().flat_map(|p| p.to_ne_bytes()).collect();
        assert_eq!(dst_bytes, expected_bytes);
    }
}

