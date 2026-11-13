//! SPS parsing: the stream's real dimensions, read from its own parameter sets.
//!
//! This is the only authoritative statement of the picture size before any
//! `state` message arrives, which is what keeps the decoder from being sized to
//! a guess. It is also re-read from **every** keyframe, because the adaptive
//! ladder restarts the phone's encoder at a new size mid-session and a decoder
//! that keeps the first size it saw will then reject every frame.
//!
//! Ported from `ParseAvcDimensions` in `native/core/src/decoder.cpp`, including
//! the emulation-prevention strip: without it, a 640x480 SPS parses as 640x1080
//! because the `00 00 03` escape sequences are read as payload bits.

use phonecam_protocol::{split_annex_b, NAL_SPS};

/// A picture size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

/// Reads the dimensions out of the first SPS found in an Annex-B buffer.
///
/// Returns `None` when there is no SPS, the bit reader runs out, or the result
/// is not a positive size — all of which mean "this buffer cannot size a
/// decoder", never "zero".
pub fn parse_avc_dimensions(data: &[u8]) -> Option<Dimensions> {
    for nal in split_annex_b(data) {
        if phonecam_protocol::nal_unit_type(nal) != Some(NAL_SPS) {
            continue;
        }
        // Skip the start code and the 1-byte NAL header.
        let start = if nal.len() >= 4 && nal[0] == 0 && nal[1] == 0 && nal[2] == 0 && nal[3] == 1 {
            4
        } else if nal.len() >= 3 && nal[0] == 0 && nal[1] == 0 && nal[2] == 1 {
            3
        } else {
            continue;
        };
        let payload = strip_emulation_prevention(&nal[start + 1..]);
        if let Some(dims) = parse_sps_payload(&payload) {
            return Some(dims);
        }
    }
    None
}

/// Removes the `00 00 03` escape sequences the encoder inserts.
///
/// Only the `03` is dropped; the two zeros before it are payload.
fn strip_emulation_prevention(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0usize;
    for &byte in data {
        if zeros >= 2 && byte == 0x03 {
            zeros = 0;
            continue;
        }
        out.push(byte);
        if byte == 0 {
            zeros += 1;
        } else {
            zeros = 0;
        }
    }
    out
}

/// A bit reader over the RBSP.
struct BitReader<'a> {
    data: &'a [u8],
    /// Bit position from the start.
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.pos / 8)?;
        let shift = 7 - (self.pos % 8);
        self.pos += 1;
        Some(((byte >> shift) & 1) as u32)
    }

    fn bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | self.bit()?;
        }
        Some(value)
    }

    /// Exp-Golomb unsigned, as used throughout the SPS.
    fn ue(&mut self) -> Option<u32> {
        let mut leading_zeros = 0u32;
        while self.bit()? == 0 {
            leading_zeros += 1;
            if leading_zeros > 31 {
                return None;
            }
        }
        if leading_zeros == 0 {
            return Some(0);
        }
        let rest = self.bits(leading_zeros)?;
        Some((1u32 << leading_zeros) - 1 + rest)
    }

    /// Exp-Golomb signed.
    fn se(&mut self) -> Option<i32> {
        let value = self.ue()?;
        let magnitude = value.div_ceil(2) as i32;
        Some(if value % 2 == 1 {
            magnitude
        } else {
            -magnitude
        })
    }
}

/// Parses the SPS RBSP, after the NAL header byte has been removed.
fn parse_sps_payload(rbsp: &[u8]) -> Option<Dimensions> {
    let mut r = BitReader::new(rbsp);

    let profile_idc = r.bits(8)?;
    let _constraint_flags = r.bits(8)?;
    let _level_idc = r.bits(8)?;
    let _sps_id = r.ue()?;

    let mut chroma_format_idc = 1u32; // 4:2:0 when absent
    let mut separate_colour_plane = false;

    // The high profiles carry chroma and scaling-list fields the baseline ones
    // do not. Reading them unconditionally desynchronises the bit reader and
    // yields a plausible but wrong size.
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134
    ) {
        chroma_format_idc = r.ue()?;
        if chroma_format_idc == 3 {
            separate_colour_plane = r.bit()? == 1;
        }
        let _bit_depth_luma = r.ue()?;
        let _bit_depth_chroma = r.ue()?;
        let _transform_bypass = r.bit()?;
        let scaling_matrix_present = r.bit()? == 1;
        if scaling_matrix_present {
            let count = if chroma_format_idc != 3 { 8 } else { 12 };
            for i in 0..count {
                if r.bit()? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    skip_scaling_list(&mut r, size)?;
                }
            }
        }
    }

    let _log2_max_frame_num_minus4 = r.ue()?;
    let pic_order_cnt_type = r.ue()?;

    if pic_order_cnt_type == 0 {
        let _log2_max_pic_order_cnt_lsb_minus4 = r.ue()?;
    } else if pic_order_cnt_type == 1 {
        let _delta_pic_order_always_zero = r.bit()?;
        let _offset_for_non_ref_pic = r.se()?;
        let _offset_for_top_to_bottom = r.se()?;
        let num_ref_frames_in_pic_order_cnt_cycle = r.ue()?;
        for _ in 0..num_ref_frames_in_pic_order_cnt_cycle {
            let _offset_for_ref_frame = r.se()?;
        }
    }

    let _max_num_ref_frames = r.ue()?;
    let _gaps_in_frame_num_allowed = r.bit()?;

    let width_in_mbs = r.ue()?.checked_add(1)?;
    let height_in_map_units = r.ue()?.checked_add(1)?;

    let frame_mbs_only = r.bit()? == 1;
    if !frame_mbs_only {
        let _mb_adaptive_frame_field = r.bit()?;
    }
    let _direct_8x8_inference = r.bit()?;

    let mut crop_left = 0u32;
    let mut crop_right = 0u32;
    let mut crop_top = 0u32;
    let mut crop_bottom = 0u32;
    if r.bit()? == 1 {
        crop_left = r.ue()?;
        crop_right = r.ue()?;
        crop_top = r.ue()?;
        crop_bottom = r.ue()?;
    }

    let chroma_array_type = if separate_colour_plane {
        0
    } else {
        chroma_format_idc
    };

    let crop_unit_x = if chroma_array_type == 0 || chroma_array_type == 3 {
        1
    } else {
        2
    };

    let frame_factor: u32 = if frame_mbs_only { 1 } else { 2 };
    let crop_unit_y = if chroma_array_type == 0 || chroma_array_type == 3 {
        frame_factor
    } else {
        2 * frame_factor
    };

    // 1080 is carried in a 1088-line coded frame, so the crop is what makes the
    // height correct.
    let width = width_in_mbs
        .checked_mul(16)?
        .checked_sub((crop_left + crop_right).checked_mul(crop_unit_x)?)?;
    let height = (frame_factor)
        .checked_mul(height_in_map_units)?
        .checked_mul(16)?
        .checked_sub((crop_top + crop_bottom).checked_mul(crop_unit_y)?)?;

    if width == 0 || height == 0 {
        return None;
    }

    Some(Dimensions { width, height })
}

fn skip_scaling_list(r: &mut BitReader<'_>, size: usize) -> Option<()> {
    let mut last_scale = 8i32;
    let mut next_scale = 8i32;
    for _ in 0..size {
        if next_scale != 0 {
            let delta = r.se()?;
            next_scale = (last_scale + delta + 256) % 256;
        }
        if next_scale != 0 {
            last_scale = next_scale;
        }
    }
    Some(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a minimal baseline-profile SPS Annex-B buffer for a given size.
    ///
    /// Written by hand so the parser is tested against bytes it did not produce.
    pub(crate) fn make_sps(
        width_mbs: u32,
        height_map_units: u32,
        crop_bottom_units: u32,
    ) -> Vec<u8> {
        let mut bits: Vec<u8> = Vec::new();

        fn push_bits(bits: &mut Vec<u8>, value: u32, count: u32) {
            for i in (0..count).rev() {
                bits.push(((value >> i) & 1) as u8);
            }
        }
        fn push_ue(bits: &mut Vec<u8>, value: u32) {
            let v = value + 1;
            let len = 32 - v.leading_zeros();
            for _ in 0..len - 1 {
                bits.push(0);
            }
            push_bits(bits, v, len);
        }

        push_bits(&mut bits, 66, 8); // profile_idc = baseline
        push_bits(&mut bits, 0xC0, 8); // constraint flags
        push_bits(&mut bits, 30, 8); // level_idc
        push_ue(&mut bits, 0); // sps_id
        push_ue(&mut bits, 0); // log2_max_frame_num_minus4
        push_ue(&mut bits, 0); // pic_order_cnt_type
        push_ue(&mut bits, 4); // log2_max_pic_order_cnt_lsb_minus4
        push_ue(&mut bits, 1); // max_num_ref_frames
        push_bits(&mut bits, 0, 1); // gaps_in_frame_num_allowed
        push_ue(&mut bits, width_mbs - 1);
        push_ue(&mut bits, height_map_units - 1);
        push_bits(&mut bits, 1, 1); // frame_mbs_only_flag
        push_bits(&mut bits, 1, 1); // direct_8x8_inference
        if crop_bottom_units > 0 {
            push_bits(&mut bits, 1, 1); // frame_cropping_flag
            push_ue(&mut bits, 0); // left
            push_ue(&mut bits, 0); // right
            push_ue(&mut bits, 0); // top
            push_ue(&mut bits, crop_bottom_units);
        } else {
            push_bits(&mut bits, 0, 1);
        }
        push_bits(&mut bits, 1, 1); // vui_parameters_present
        push_bits(&mut bits, 0, 1); // rbsp_stop

        // Pack bits into bytes, then insert emulation prevention.
        while !bits.len().is_multiple_of(8) {
            bits.push(0);
        }
        let mut raw = Vec::new();
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for &b in chunk {
                byte = (byte << 1) | b;
            }
            raw.push(byte);
        }

        let mut escaped = Vec::new();
        let mut zeros = 0;
        for byte in raw {
            if zeros >= 2 && byte <= 0x03 {
                escaped.push(0x03);
                zeros = 0;
            }
            escaped.push(byte);
            if byte == 0 {
                zeros += 1;
            } else {
                zeros = 0;
            }
        }

        let mut out = vec![0u8, 0, 0, 1, 0x67]; // start code + NAL header (SPS)
        out.extend_from_slice(&escaped);
        out
    }

    #[test]
    fn parses_1080p_from_a_cropped_1088_frame() {
        // 1080p is coded as 120x68 macroblocks and cropped by 4 chroma units.
        let sps = make_sps(120, 68, 4);
        let dims = parse_avc_dimensions(&sps).expect("1080p SPS must parse");
        assert_eq!(
            dims,
            Dimensions {
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn parses_720p() {
        let sps = make_sps(80, 45, 0);
        let dims = parse_avc_dimensions(&sps).expect("720p SPS must parse");
        assert_eq!(
            dims,
            Dimensions {
                width: 1280,
                height: 720
            }
        );
    }

    #[test]
    fn parses_480p() {
        let sps = make_sps(40, 30, 0);
        let dims = parse_avc_dimensions(&sps).expect("480p SPS must parse");
        assert_eq!(
            dims,
            Dimensions {
                width: 640,
                height: 480
            }
        );
    }

    /// Without the emulation-prevention strip the escape bytes are read as
    /// payload and the size comes out wrong — the documented 640x1080 bug.
    #[test]
    fn emulation_prevention_bytes_are_stripped() {
        let with_escapes = vec![0x00, 0x00, 0x03, 0x01, 0x00, 0x00, 0x03, 0x02];
        assert_eq!(
            strip_emulation_prevention(&with_escapes),
            vec![0x00, 0x00, 0x01, 0x00, 0x00, 0x02]
        );
    }

    #[test]
    fn strip_leaves_ordinary_zeros_alone() {
        let plain = vec![0x10, 0x00, 0x00, 0x05, 0x00];
        assert_eq!(strip_emulation_prevention(&plain), plain);
    }

    #[test]
    fn no_sps_is_none_not_zero() {
        let mut idr = vec![0u8, 0, 0, 1, 0x65, 0x88];
        idr.extend_from_slice(&[0u8; 8]);
        assert_eq!(parse_avc_dimensions(&idr), None);
        assert_eq!(parse_avc_dimensions(&[]), None);
    }

    #[test]
    fn truncated_sps_is_none_not_a_panic() {
        let sps = make_sps(80, 45, 0);
        for cut in 1..sps.len() {
            let _ = parse_avc_dimensions(&sps[..cut]);
        }
    }

    #[test]
    fn exp_golomb_round_trips_the_documented_values() {
        // ue: 0 -> "1", 1 -> "010", 2 -> "011", 3 -> "00100"
        let cases: [(Vec<u8>, u32); 4] = [
            (vec![0b1000_0000], 0),
            (vec![0b0100_0000], 1),
            (vec![0b0110_0000], 2),
            (vec![0b0010_0000], 3),
        ];
        for (bytes, expected) in cases {
            let mut r = BitReader::new(&bytes);
            assert_eq!(r.ue(), Some(expected), "ue of {expected}");
        }
    }
}
