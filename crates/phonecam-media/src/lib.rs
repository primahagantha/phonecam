//! H.264 decoding.
//!
//! The C++ core muxed each group of pictures into a fragmented MP4 on disk and
//! read it back through a Media Foundation Source Reader, because the bare
//! H.264 decoder MFT produced zero frames on some machines. That path costs a
//! temp-file write plus a read per group (â‰ˆ24 ms/batch) and is the single
//! largest latency term in the old pipeline.
//!
//! This decodes in-process with openh264 instead: no temp file, no container, no
//! second copy, and it works on any machine regardless of which MFTs are
//! installed. The output is I420, which is NV12's planar sibling â€” the frame bus
//! wants NV12, so the interleave happens once, at publish time.

pub mod sps;

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
}

impl DecodedFrame {
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

        dst[..luma_len].copy_from_slice(&self.yuv[..luma_len]);

        let (u_plane, v_plane) = self.yuv[luma_len..luma_len + chroma_len].split_at(chroma_len / 2);
        for i in 0..chroma_len / 2 {
            dst[luma_len + i * 2] = u_plane[i];
            dst[luma_len + i * 2 + 1] = v_plane[i];
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
        if dst.len() < stride_pixels * height * 4 {
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

        let clamp = |value: i32| -> u8 {
            if value < 0 {
                0
            } else if value > 255 {
                255
            } else {
                value as u8
            }
        };

        for y in 0..height {
            let luma_row = y * width;
            let chroma_row = (y / 2) * chroma_width;
            let mut offset = y * stride_pixels * 4;

            for x in 0..width {
                let luma = self.yuv[luma_row + x] as i32 - 16;
                let chroma_index = chroma_row + (x / 2);
                let u = u_plane[chroma_index] as i32 - 128;
                let v = v_plane[chroma_index] as i32 - 128;

                let c = Y_GAIN * luma;
                // + HALF rounds to nearest rather than truncating, which the
                // float path did implicitly.
                dst[offset] = clamp((c + BU * u + HALF) >> SCALE);
                dst[offset + 1] = clamp((c - GV * v - GU * u + HALF) >> SCALE);
                dst[offset + 2] = clamp((c + RV * v + HALF) >> SCALE);
                dst[offset + 3] = 255;
                offset += 4;
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
    /// the C++ core warns about and discards â€” a codec-config packet that cannot
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

    /// Decodes one Annex-B access unit.
    ///
    /// On failure the caller should treat the stream as needing a new keyframe;
    /// `reset` returns the decoder to that state.
    pub fn decode(&mut self, access_unit: &[u8]) -> Result<DecodedFrame, DecodeError> {
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

        Ok(DecodedFrame { width: w, height: h, yuv: buffer })
    }

    /// Drops the current picture state so the next keyframe re-syncs.
    pub fn reset(&mut self) {
        if let Ok(fresh) = Decoder::new() {
            self.inner = fresh;
        }
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
        DecodedFrame { width: w as u32, height: h as u32, yuv }
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
        let frame = DecodedFrame { width: 2, height: 2, yuv: vec![81, 81, 81, 81, 90, 240] };
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
}

