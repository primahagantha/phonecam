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

    // --- identity fast path: no resampling required
    if sw == tw && sh == th {
        let bytes = sw * sh + (sh / 2) * sw;
        dst[..bytes].copy_from_slice(&src[..bytes]);
        return;
    }

    const MAX_LUT_WIDTH: usize = 4096;
    if tw <= MAX_LUT_WIDTH && sw <= u16::MAX as usize {
        scale_lut(src, sw, sh, dst, tw, th);
        return;
    }

    scale_scalar(src, sw, sh, dst, tw, th);
}

fn scale_lut(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], tw: usize, th: usize) {
    const MAX_LUT_WIDTH: usize = 4096;

    // Precompute source X offsets once per frame instead of tw * th integer divisions.
    let mut x_lut = [0u16; MAX_LUT_WIDTH];
    for (x, slot) in x_lut[..tw].iter_mut().enumerate() {
        *slot = ((x * sw) / tw) as u16;
    }

    let mut prev_sy = usize::MAX;
    let mut prev_dst_row = 0usize;
    let src_ptr = src.as_ptr();
    let dst_ptr = dst.as_mut_ptr();

    // --- luma
    for y in 0..th {
        let sy = (y * sh) / th;
        let dst_row = y * tw;
        if sy == prev_sy {
            dst.copy_within(prev_dst_row..prev_dst_row + tw, dst_row);
            continue;
        }
        prev_sy = sy;
        prev_dst_row = dst_row;

        let src_row_off = sy * sw;
        // SAFETY:
        // - `src.len() >= sw * sh * 3 / 2` and `sy < sh`, so `src_row_off + sw <= src.len()`.
        // - Every `x_lut[x]` equals `(x * sw) / tw < sw` since `x < tw`.
        // - `dst.len() >= tw * th * 3 / 2` and `dst_row + tw <= tw * th <= dst.len()`.
        unsafe {
            let s = src_ptr.add(src_row_off);
            let mut d = dst_ptr.add(dst_row);
            let mut x = 0usize;
            while x + 8 <= tw {
                *d = *s.add(*x_lut.get_unchecked(x) as usize);
                *d.add(1) = *s.add(*x_lut.get_unchecked(x + 1) as usize);
                *d.add(2) = *s.add(*x_lut.get_unchecked(x + 2) as usize);
                *d.add(3) = *s.add(*x_lut.get_unchecked(x + 3) as usize);
                *d.add(4) = *s.add(*x_lut.get_unchecked(x + 4) as usize);
                *d.add(5) = *s.add(*x_lut.get_unchecked(x + 5) as usize);
                *d.add(6) = *s.add(*x_lut.get_unchecked(x + 6) as usize);
                *d.add(7) = *s.add(*x_lut.get_unchecked(x + 7) as usize);
                d = d.add(8);
                x += 8;
            }
            while x < tw {
                *d = *s.add(*x_lut.get_unchecked(x) as usize);
                d = d.add(1);
                x += 1;
            }
        }
    }

    // --- chroma: interleaved U/V, sh/2 rows of sw bytes
    let src_chroma = sw * sh;
    let dst_chroma = tw * th;
    let src_chroma_rows = sh / 2;
    let dst_chroma_rows = th / 2;
    if dst_chroma_rows == 0 || src_chroma_rows == 0 {
        return;
    }

    if (sw & 1) == 0 && (tw & 1) == 0 {
        let pairs = tw / 2;
        let mut uv_lut = [0u16; MAX_LUT_WIDTH / 2];
        for (p, slot) in uv_lut[..pairs].iter_mut().enumerate() {
            *slot = ((((p * 2) * sw) / tw) & !1usize) as u16;
        }

        let mut prev_sy = usize::MAX;
        let mut prev_dst_row = 0usize;
        for y in 0..dst_chroma_rows {
            let sy = (y * src_chroma_rows) / dst_chroma_rows;
            let dst_row = dst_chroma + y * tw;
            if sy == prev_sy {
                dst.copy_within(prev_dst_row..prev_dst_row + tw, dst_row);
                continue;
            }
            prev_sy = sy;
            prev_dst_row = dst_row;

            let src_row_off = src_chroma + sy * sw;
            // SAFETY:
            // - `sw` is even and `uv_lut[p]` is even and `< sw`, so `uv_lut[p] + 1 < sw`.
            // - `sy < src_chroma_rows`, so `src_row_off + sw <= src.len()`.
            // - `tw = 2 * pairs`, so `dst_row + tw <= dst.len()`.
            unsafe {
                let s = src_ptr.add(src_row_off);
                let mut d = dst_ptr.add(dst_row);
                let mut p = 0usize;
                while p + 4 <= pairs {
                    let v0 = std::ptr::read_unaligned(s.add(*uv_lut.get_unchecked(p) as usize) as *const u16);
                    let v1 = std::ptr::read_unaligned(s.add(*uv_lut.get_unchecked(p + 1) as usize) as *const u16);
                    let v2 = std::ptr::read_unaligned(s.add(*uv_lut.get_unchecked(p + 2) as usize) as *const u16);
                    let v3 = std::ptr::read_unaligned(s.add(*uv_lut.get_unchecked(p + 3) as usize) as *const u16);
                    std::ptr::write_unaligned(d as *mut u16, v0);
                    std::ptr::write_unaligned(d.add(2) as *mut u16, v1);
                    std::ptr::write_unaligned(d.add(4) as *mut u16, v2);
                    std::ptr::write_unaligned(d.add(6) as *mut u16, v3);
                    d = d.add(8);
                    p += 4;
                }
                while p < pairs {
                    let v = std::ptr::read_unaligned(s.add(*uv_lut.get_unchecked(p) as usize) as *const u16);
                    std::ptr::write_unaligned(d as *mut u16, v);
                    d = d.add(2);
                    p += 1;
                }
            }
        }
    } else {
        for y in 0..dst_chroma_rows {
            let sy = (y * src_chroma_rows) / dst_chroma_rows;
            let src_row = src_chroma + sy * sw;
            let dst_row = dst_chroma + y * tw;
            let mut x = 0usize;
            while x < tw {
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
}

fn scale_scalar(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], tw: usize, th: usize) {
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
    if dst_chroma_rows == 0 || src_chroma_rows == 0 {
        return;
    }
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

    #[test]
    fn optimized_scale_matches_scalar_reference_bit_for_bit() {
        let cases = [
            (640u32, 360u32, 1280u32, 720u32),
            (1280, 720, 1920, 1080),
            (1920, 1080, 1280, 720),
            (1920, 1080, 1920, 1080),
            (720, 480, 1000, 600),
            (13, 6, 21, 10),
            (14, 8, 17, 10),
        ];
        for (sw, sh, tw, th) in cases {
            let src = pattern(sw as usize, sh as usize);
            let expected_len = (tw * th * 3 / 2) as usize;
            let mut got = vec![0u8; expected_len];
            let mut want = vec![0u8; expected_len];
            scale(&src, sw, sh, &mut got, tw, th);
            scale_scalar(&src, sw as usize, sh as usize, &mut want, tw as usize, th as usize);
            assert_eq!(got, want, "mismatch for {sw}x{sh} -> {tw}x{th}");
        }
    }

    #[test]
    fn perf_nv12_scale_720p_to_1080p_and_1080p_identity() {
        let iters = 200u32;
        let src_720 = pattern(1280, 720);
        let mut dst_1080 = vec![0u8; 1920 * 1080 * 3 / 2];
        let start = std::time::Instant::now();
        for _ in 0..iters {
            scale(&src_720, 1280, 720, &mut dst_1080, 1920, 1080);
            std::hint::black_box(&dst_1080);
        }
        let elapsed = start.elapsed();
        let ms_per_frame = elapsed.as_secs_f64() * 1000.0 / f64::from(iters);
        let fps = 1000.0 / ms_per_frame.max(1e-6);
        eprintln!(
            "nv12::scale 1280x720->1920x1080: {ms_per_frame:.3} ms/frame ({fps:.1} FPS)"
        );

        let src_1080 = pattern(1920, 1080);
        let start_id = std::time::Instant::now();
        for _ in 0..iters {
            scale(&src_1080, 1920, 1080, &mut dst_1080, 1920, 1080);
            std::hint::black_box(&dst_1080);
        }
        let elapsed_id = start_id.elapsed();
        let ms_id = elapsed_id.as_secs_f64() * 1000.0 / f64::from(iters);
        let fps_id = 1000.0 / ms_id.max(1e-6);
        eprintln!(
            "nv12::scale 1920x1080 identity: {ms_id:.3} ms/frame ({fps_id:.1} FPS)"
        );
    }
}
