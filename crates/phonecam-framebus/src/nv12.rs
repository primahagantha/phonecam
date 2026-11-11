//! NV12 scaling — nearest neighbour over both planes.
//!
//! Ported from `native/shared/src/nv12.cpp`. The core needs this because the
//! shared buffer is sized by the **output** setting while the phone encodes at
//! whatever its camera produced; the core scales each decoded frame into the
//! buffer, which is the only place the output size can have an effect.
//!
//! `Transform` (rotation + mirror) is deliberately **not** ported: it lives
//! inside the DLL, which reads `mirror`/`rotation` from the header. The core
//! writes the metadata and the preview applies its own transform on the GPU, so
//! there is one implementation of each.

/// Scales an NV12 frame from `sw x sh` to `tw x th`, nearest neighbour.
///
/// Chroma is subsampled 2x vertically and stored as interleaved U/V pairs, so
/// the horizontal source index is rounded **down to an even byte** — both bytes
/// of a pair must come from the same source pair, or the colours separate.
pub fn scale(src: &[u8], sw: u32, sh: u32, dst: &mut [u8], tw: u32, th: u32) {
    if sw == 0 || sh == 0 || tw == 0 || th == 0 {
        return;
    }
    let expected = tw as usize * th as usize * 3 / 2;
    if dst.len() < expected {
        return;
    }
    let sw = sw as usize;
    let sh = sh as usize;
    let tw = tw as usize;
    let th = th as usize;

    if src.len() < sw * sh * 3 / 2 {
        return;
    }

    // --- luma
    for y in 0..th {
        let sy = (y * sh) / th;
        let src_row = sy * sw;
        let dst_row = y * tw;
        for x in 0..tw {
            let sx = (x * sw) / tw;
            dst[dst_row + x] = src[src_row + sx];
        }
    }

    // --- chroma: interleaved U/V, sh/2 rows of sw bytes
    let src_chroma = sw * sh;
    let dst_chroma = tw * th;
    let src_chroma_rows = sh / 2;
    let dst_chroma_rows = th / 2;

    for y in 0..dst_chroma_rows {
        let sy = (y * src_chroma_rows) / dst_chroma_rows;
        let src_row = src_chroma + sy * sw;
        let dst_row = dst_chroma + y * tw;
        let mut x = 0usize;
        while x < tw {
            // Round down to an even byte so the pair is coherent.
            let sx = ((x * sw) / tw) & !1usize;
            dst[dst_row + x] = src[src_row + sx];
            if x + 1 < tw {
                let second = if sx + 1 < sw { sx + 1 } else { sx };
                dst[dst_row + x + 1] = src[src_row + second];
            }
            x += 2;
        }
    }
}

/// Fills an NV12 buffer with black: limited-range luma 16, neutral chroma 128.
///
/// Publishing black pixels alongside the disconnected state is what keeps the
/// pixels and the state consistent — otherwise a consumer that ignores the
/// header keeps showing the last frame of a phone that is gone.
pub fn fill_black(dst: &mut [u8], width: u32, height: u32) {
    let (luma, chroma) = dst.split_at_mut((width * height) as usize);
    luma.fill(16);
    chroma.fill(128);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny synthesised NV12 frame with a known pattern: luma rises across the
    /// row, chroma is a constant non-neutral pair so a U/V swap is visible.
    fn pattern(width: usize, height: usize) -> Vec<u8> {
        let mut v = vec![0u8; width * height * 3 / 2];
        for y in 0..height {
            for x in 0..width {
                v[y * width + x] = ((x + y) % 251) as u8;
            }
        }
        let chroma = width * height;
        for i in 0..(width * height / 2) {
            v[chroma + i] = if i % 2 == 0 { 40 } else { 210 };
        }
        v
    }

    #[test]
    fn identity_scale_is_a_byte_for_byte_copy() {
        let (w, h) = (16usize, 8usize);
        let src = pattern(w, h);
        let mut dst = vec![0u8; w * h * 3 / 2];
        scale(&src, w as u32, h as u32, &mut dst, w as u32, h as u32);
        assert_eq!(dst, src, "identity must not resample");
    }

    #[test]
    fn doubling_picks_the_nearest_source_pixel() {
        let (w, h) = (4usize, 4usize);
        let src = pattern(w, h);
        let mut dst = vec![0u8; (w * 2) * (h * 2) * 3 / 2];
        scale(&src, w as u32, h as u32, &mut dst, (w * 2) as u32, (h * 2) as u32);

        // Nearest-neighbour: destination (x,y) samples source (x/2, y/2).
        for y in 0..h * 2 {
            for x in 0..w * 2 {
                assert_eq!(dst[y * w * 2 + x], src[(y / 2) * w + (x / 2)]);
            }
        }
    }

    #[test]
    fn scale_halves_without_reading_out_of_bounds() {
        let (w, h) = (64usize, 32usize);
        let src = pattern(w, h);
        let mut dst = vec![0u8; (w / 2) * (h / 2) * 3 / 2];
        scale(&src, w as u32, h as u32, &mut dst, (w / 2) as u32, (h / 2) as u32);
        // Every sampled index must come from inside the source luma plane.
        for y in 0..h / 2 {
            for x in 0..w / 2 {
                assert_eq!(dst[y * (w / 2) + x], src[((y * h) / (h / 2)) * w + (x * w) / (w / 2)]);
            }
        }
    }

    /// The chroma plane must keep U/V pairs coherent. A nearest-neighbour that
    /// rounds to an odd byte would swap U and V across the whole picture, which
    /// reads as a violent colour cast rather than as a scaling bug.
    ///
    /// Both dimensions are even here because NV12 has no valid layout for an odd
    /// width — `dimensions_are_valid` rejects one before it can reach a frame.
    #[test]
    fn chroma_pairs_stay_in_the_same_order_after_scaling() {
        for (sw, tw) in [(16u32, 8u32), (8, 16), (32, 12), (12, 32), (10, 8), (8, 10)] {
            let sh = 8u32;
            let th = 8u32;
            let src = pattern(sw as usize, sh as usize);
            let mut dst = vec![0u8; (tw as usize) * (th as usize) * 3 / 2];
            scale(&src, sw, sh, &mut dst, tw, th);

            let chroma = (tw as usize) * (th as usize);
            for i in (0..(tw as usize)).step_by(2) {
                let u = dst[chroma + i];
                let v = dst[chroma + i + 1];
                // The source alternates 40, 210 starting at an even offset, so a
                // coherent pair is always (40, 210) — never (210, 40).
                assert_eq!((u, v), (40, 210), "swapped at {sw}->{tw} index {i}");
            }
        }
    }

    #[test]
    fn scale_is_refused_when_the_destination_is_too_small() {
        let src = pattern(16, 8);
        let mut dst = vec![0u8; 16];
        scale(&src, 16, 8, &mut dst, 16, 8);
        assert_eq!(dst, vec![0u8; 16], "must not write past the buffer");
    }

    #[test]
    fn scale_is_refused_when_the_source_is_truncated() {
        let src = vec![0u8; 10];
        let mut dst = vec![0u8; 16 * 8 * 3 / 2];
        scale(&src, 16, 8, &mut dst, 16, 8);
        assert!(dst.iter().all(|&b| b == 0));
    }

    #[test]
    fn zero_dimensions_are_a_no_op() {
        let mut dst = vec![1u8; 64];
        scale(&[], 0, 0, &mut dst, 0, 0);
        assert!(dst.iter().all(|&b| b == 1));
    }

    #[test]
    fn fill_black_uses_limited_range_luma_and_neutral_chroma() {
        let (w, h) = (8u32, 4u32);
        let mut buf = vec![0u8; (w * h * 3 / 2) as usize];
        fill_black(&mut buf, w, h);
        assert!(buf[..(w * h) as usize].iter().all(|&b| b == 16));
        assert!(buf[(w * h) as usize..].iter().all(|&b| b == 128));
    }
}
