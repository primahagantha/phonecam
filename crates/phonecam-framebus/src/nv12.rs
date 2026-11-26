//! NV12 scaling — area/box downscale and bilinear upscale over both planes.
//!
//! Ported from `native/shared/src/nv12.cpp` and upgraded from nearest-neighbor to
//! box downscaling + bilinear interpolation for crisp, alias-free resolution
//! adaptation across both luma (Y) and interleaved chroma (UV) planes.
//!
//! Chroma is subsampled 2x vertically and stored as interleaved U/V pairs, so U
//! and V channels are always interpolated or averaged independently to prevent
//! chroma channel swapping.

const MAX_LUT_WIDTH: usize = 4096;

/// Scales an NV12 frame from `sw x sh` to `tw x th` using box downscaling when
/// shrinking (`tw < sw && th < sh`) and bilinear interpolation otherwise.
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

    if tw <= MAX_LUT_WIDTH && sw <= u16::MAX as usize {
        scale_lut(src, sw, sh, dst, tw, th);
        return;
    }

    scale_scalar(src, sw, sh, dst, tw, th);
}

#[inline]
fn bilinear_coord(d: usize, s_len: usize, d_len: usize) -> (usize, usize, u32) {
    let max_fp = ((s_len - 1) as i64) * 256;
    let fp = (((2 * d as i64 + 1) * (s_len as i64) * 256) / (2 * d_len as i64)) - 128;
    let clamped = fp.clamp(0, max_fp);
    let c0 = (clamped >> 8) as usize;
    let w = (clamped & 0xFF) as u32;
    let c1 = (c0 + 1).min(s_len - 1);
    (c0, c1, w)
}

#[inline]
fn box_span(d: usize, s_len: usize, d_len: usize) -> (usize, usize) {
    let s0 = (d * s_len) / d_len;
    let s1 = (((d + 1) * s_len) / d_len).max(s0 + 1).min(s_len);
    (s0, s1)
}

fn scale_lut(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], tw: usize, th: usize) {
    if tw < sw && th < sh {
        scale_box_lut(src, sw, sh, dst, tw, th);
    } else {
        scale_bilinear_lut(src, sw, sh, dst, tw, th);
    }
}

fn scale_box_lut(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], tw: usize, th: usize) {
    let mut sx0_lut = [0u16; MAX_LUT_WIDTH];
    let mut sx1_lut = [0u16; MAX_LUT_WIDTH];
    for x in 0..tw {
        let (s0, s1) = box_span(x, sw, tw);
        sx0_lut[x] = s0 as u16;
        sx1_lut[x] = s1 as u16;
    }

    let mut row_sums = [0u32; MAX_LUT_WIDTH];

    // --- luma box downscale
    for dy in 0..th {
        let (sy0, sy1) = box_span(dy, sh, th);
        let y_count = (sy1 - sy0) as u32;
        row_sums[..tw].fill(0);

        for sy in sy0..sy1 {
            let src_row = &src[sy * sw..(sy + 1) * sw];
            for dx in 0..tw {
                let s0 = sx0_lut[dx] as usize;
                let s1 = sx1_lut[dx] as usize;
                let mut hsum = 0u32;
                for &px in &src_row[s0..s1] {
                    hsum += px as u32;
                }
                row_sums[dx] += hsum;
            }
        }

        let dst_row = &mut dst[dy * tw..(dy + 1) * tw];
        for dx in 0..tw {
            let count = y_count * (sx1_lut[dx] - sx0_lut[dx]) as u32;
            dst_row[dx] = ((row_sums[dx] + count / 2) / count) as u8;
        }
    }

    // --- chroma box downscale: interleaved U/V pairs
    let src_chroma = sw * sh;
    let dst_chroma = tw * th;
    let src_cw = sw / 2;
    let src_ch = sh / 2;
    let dst_cw = tw / 2;
    let dst_ch = th / 2;
    if src_cw == 0 || src_ch == 0 || dst_cw == 0 || dst_ch == 0 {
        return;
    }

    let mut csx0_lut = [0u16; MAX_LUT_WIDTH / 2];
    let mut csx1_lut = [0u16; MAX_LUT_WIDTH / 2];
    for cx in 0..dst_cw {
        let (s0, s1) = box_span(cx, src_cw, dst_cw);
        csx0_lut[cx] = s0 as u16;
        csx1_lut[cx] = s1 as u16;
    }
    let mut u_sums = [0u32; MAX_LUT_WIDTH / 2];
    let mut v_sums = [0u32; MAX_LUT_WIDTH / 2];

    for cdy in 0..dst_ch {
        let (csy0, csy1) = box_span(cdy, src_ch, dst_ch);
        let y_count = (csy1 - csy0) as u32;
        u_sums[..dst_cw].fill(0);
        v_sums[..dst_cw].fill(0);

        for csy in csy0..csy1 {
            let src_row = &src[src_chroma + csy * sw..src_chroma + (csy + 1) * sw];
            for cdx in 0..dst_cw {
                let s0 = csx0_lut[cdx] as usize;
                let s1 = csx1_lut[cdx] as usize;
                let mut hu = 0u32;
                let mut hv = 0u32;
                for csx in s0..s1 {
                    hu += src_row[csx * 2] as u32;
                    hv += src_row[csx * 2 + 1] as u32;
                }
                u_sums[cdx] += hu;
                v_sums[cdx] += hv;
            }
        }

        let dst_row = &mut dst[dst_chroma + cdy * tw..dst_chroma + (cdy + 1) * tw];
        for cdx in 0..dst_cw {
            let count = y_count * (csx1_lut[cdx] - csx0_lut[cdx]) as u32;
            let half = count / 2;
            dst_row[cdx * 2] = ((u_sums[cdx] + half) / count) as u8;
            dst_row[cdx * 2 + 1] = ((v_sums[cdx] + half) / count) as u8;
        }
    }
}

fn scale_bilinear_lut(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], tw: usize, th: usize) {
    let mut x0_lut = [0u16; MAX_LUT_WIDTH];
    let mut x1_lut = [0u16; MAX_LUT_WIDTH];
    let mut wx_lut = [0u16; MAX_LUT_WIDTH];
    for x in 0..tw {
        let (x0, x1, wx) = bilinear_coord(x, sw, tw);
        x0_lut[x] = x0 as u16;
        x1_lut[x] = x1 as u16;
        wx_lut[x] = wx as u16;
    }

    // --- luma bilinear
    for dy in 0..th {
        let (y0, y1, wy) = bilinear_coord(dy, sh, th);
        let inv_wy = 256 - wy;
        let row0 = &src[y0 * sw..(y0 + 1) * sw];
        let row1 = &src[y1 * sw..(y1 + 1) * sw];
        let dst_row = &mut dst[dy * tw..(dy + 1) * tw];

        for dx in 0..tw {
            let x0 = x0_lut[dx] as usize;
            let x1 = x1_lut[dx] as usize;
            let wx = wx_lut[dx] as u32;
            let inv_wx = 256 - wx;

            let p00 = row0[x0] as u32;
            let p10 = row0[x1] as u32;
            let p01 = row1[x0] as u32;
            let p11 = row1[x1] as u32;

            let val = p00 * inv_wx * inv_wy + p10 * wx * inv_wy + p01 * inv_wx * wy + p11 * wx * wy;
            dst_row[dx] = ((val + 32_768) >> 16) as u8;
        }
    }

    // --- chroma bilinear: interleaved U/V pairs
    let src_chroma = sw * sh;
    let dst_chroma = tw * th;
    let src_cw = sw / 2;
    let src_ch = sh / 2;
    let dst_cw = tw / 2;
    let dst_ch = th / 2;
    if src_cw == 0 || src_ch == 0 || dst_cw == 0 || dst_ch == 0 {
        return;
    }

    let mut cx0_lut = [0u16; MAX_LUT_WIDTH / 2];
    let mut cx1_lut = [0u16; MAX_LUT_WIDTH / 2];
    let mut cwx_lut = [0u16; MAX_LUT_WIDTH / 2];
    for cx in 0..dst_cw {
        let (x0, x1, wx) = bilinear_coord(cx, src_cw, dst_cw);
        cx0_lut[cx] = (x0 * 2) as u16;
        cx1_lut[cx] = (x1 * 2) as u16;
        cwx_lut[cx] = wx as u16;
    }

    for cdy in 0..dst_ch {
        let (cy0, cy1, wy) = bilinear_coord(cdy, src_ch, dst_ch);
        let inv_wy = 256 - wy;
        let row0 = &src[src_chroma + cy0 * sw..src_chroma + (cy0 + 1) * sw];
        let row1 = &src[src_chroma + cy1 * sw..src_chroma + (cy1 + 1) * sw];
        let dst_row = &mut dst[dst_chroma + cdy * tw..dst_chroma + (cdy + 1) * tw];

        for cdx in 0..dst_cw {
            let x0 = cx0_lut[cdx] as usize;
            let x1 = cx1_lut[cdx] as usize;
            let wx = cwx_lut[cdx] as u32;
            let inv_wx = 256 - wx;

            let w00 = inv_wx * inv_wy;
            let w10 = wx * inv_wy;
            let w01 = inv_wx * wy;
            let w11 = wx * wy;

            let u = row0[x0] as u32 * w00
                + row0[x1] as u32 * w10
                + row1[x0] as u32 * w01
                + row1[x1] as u32 * w11;
            let v = row0[x0 + 1] as u32 * w00
                + row0[x1 + 1] as u32 * w10
                + row1[x0 + 1] as u32 * w01
                + row1[x1 + 1] as u32 * w11;

            dst_row[cdx * 2] = ((u + 32_768) >> 16) as u8;
            dst_row[cdx * 2 + 1] = ((v + 32_768) >> 16) as u8;
        }
    }
}

fn scale_scalar(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], tw: usize, th: usize) {
    let src_chroma = sw * sh;
    let dst_chroma = tw * th;
    let src_cw = sw / 2;
    let src_ch = sh / 2;
    let dst_cw = tw / 2;
    let dst_ch = th / 2;

    if tw < sw && th < sh {
        // --- luma box downscale
        for dy in 0..th {
            let (sy0, sy1) = box_span(dy, sh, th);
            for dx in 0..tw {
                let (sx0, sx1) = box_span(dx, sw, tw);
                let mut sum = 0u32;
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        sum += src[sy * sw + sx] as u32;
                    }
                }
                let count = ((sy1 - sy0) * (sx1 - sx0)) as u32;
                dst[dy * tw + dx] = ((sum + count / 2) / count) as u8;
            }
        }

        if src_cw == 0 || src_ch == 0 || dst_cw == 0 || dst_ch == 0 {
            return;
        }

        // --- chroma box downscale
        for cdy in 0..dst_ch {
            let (csy0, csy1) = box_span(cdy, src_ch, dst_ch);
            for cdx in 0..dst_cw {
                let (csx0, csx1) = box_span(cdx, src_cw, dst_cw);
                let mut u_sum = 0u32;
                let mut v_sum = 0u32;
                for csy in csy0..csy1 {
                    let row = src_chroma + csy * sw;
                    for csx in csx0..csx1 {
                        u_sum += src[row + csx * 2] as u32;
                        v_sum += src[row + csx * 2 + 1] as u32;
                    }
                }
                let count = ((csy1 - csy0) * (csx1 - csx0)) as u32;
                let half = count / 2;
                dst[dst_chroma + cdy * tw + cdx * 2] = ((u_sum + half) / count) as u8;
                dst[dst_chroma + cdy * tw + cdx * 2 + 1] = ((v_sum + half) / count) as u8;
            }
        }
    } else {
        // --- luma bilinear
        for dy in 0..th {
            let (y0, y1, wy) = bilinear_coord(dy, sh, th);
            let inv_wy = 256 - wy;
            for dx in 0..tw {
                let (x0, x1, wx) = bilinear_coord(dx, sw, tw);
                let inv_wx = 256 - wx;
                let val = src[y0 * sw + x0] as u32 * inv_wx * inv_wy
                    + src[y0 * sw + x1] as u32 * wx * inv_wy
                    + src[y1 * sw + x0] as u32 * inv_wx * wy
                    + src[y1 * sw + x1] as u32 * wx * wy;
                dst[dy * tw + dx] = ((val + 32_768) >> 16) as u8;
            }
        }

        if src_cw == 0 || src_ch == 0 || dst_cw == 0 || dst_ch == 0 {
            return;
        }

        // --- chroma bilinear
        for cdy in 0..dst_ch {
            let (cy0, cy1, wy) = bilinear_coord(cdy, src_ch, dst_ch);
            let inv_wy = 256 - wy;
            for cdx in 0..dst_cw {
                let (cx0, cx1, wx) = bilinear_coord(cdx, src_cw, dst_cw);
                let inv_wx = 256 - wx;
                let w00 = inv_wx * inv_wy;
                let w10 = wx * inv_wy;
                let w01 = inv_wx * wy;
                let w11 = wx * wy;

                let r0 = src_chroma + cy0 * sw;
                let r1 = src_chroma + cy1 * sw;
                let u = src[r0 + cx0 * 2] as u32 * w00
                    + src[r0 + cx1 * 2] as u32 * w10
                    + src[r1 + cx0 * 2] as u32 * w01
                    + src[r1 + cx1 * 2] as u32 * w11;
                let v = src[r0 + cx0 * 2 + 1] as u32 * w00
                    + src[r0 + cx1 * 2 + 1] as u32 * w10
                    + src[r1 + cx0 * 2 + 1] as u32 * w01
                    + src[r1 + cx1 * 2 + 1] as u32 * w11;

                dst[dst_chroma + cdy * tw + cdx * 2] = ((u + 32_768) >> 16) as u8;
                dst[dst_chroma + cdy * tw + cdx * 2 + 1] = ((v + 32_768) >> 16) as u8;
            }
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
    fn doubling_bilinear_interpolates_smoothly() {
        let (w, h) = (4usize, 4usize);
        let mut src = vec![0u8; w * h * 3 / 2];
        // Horizontal ramp: [0, 80, 160, 240]
        for y in 0..h {
            for x in 0..w {
                src[y * w + x] = (x * 80) as u8;
            }
        }
        src[w * h..].fill(128);

        let mut dst = vec![0u8; (w * 2) * (h * 2) * 3 / 2];
        scale(
            &src,
            w as u32,
            h as u32,
            &mut dst,
            (w * 2) as u32,
            (h * 2) as u32,
        );

        // Center-aligned 2x bilinear ramp between 0 and 80 yields 0, 20, 60, 100, 140, 180, 220, 240
        assert_eq!(&dst[..8], &[0, 20, 60, 100, 140, 180, 220, 240]);
    }

    #[test]
    fn scale_halves_box_downscales_accurately() {
        let (w, h) = (64usize, 32usize);
        let src = pattern(w, h);
        let mut dst = vec![0u8; (w / 2) * (h / 2) * 3 / 2];
        scale(
            &src,
            w as u32,
            h as u32,
            &mut dst,
            (w / 2) as u32,
            (h / 2) as u32,
        );
        // Every destination luma pixel must equal the rounded 2x2 box average of source pixels.
        for y in 0..h / 2 {
            for x in 0..w / 2 {
                let sum = src[(y * 2) * w + (x * 2)] as u32
                    + src[(y * 2) * w + (x * 2 + 1)] as u32
                    + src[(y * 2 + 1) * w + (x * 2)] as u32
                    + src[(y * 2 + 1) * w + (x * 2 + 1)] as u32;
                let expected = ((sum + 2) / 4) as u8;
                assert_eq!(dst[y * (w / 2) + x], expected);
            }
        }
    }

    /// The chroma plane must keep U/V pairs coherent. Both dimensions are even
    /// because NV12 has no valid layout for an odd width.
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
            (14, 8, 22, 12),
            (22, 12, 14, 8),
        ];
        for (sw, sh, tw, th) in cases {
            let src = pattern(sw as usize, sh as usize);
            let expected_len = (tw * th * 3 / 2) as usize;
            let mut got = vec![0u8; expected_len];
            let mut want = vec![0u8; expected_len];
            scale(&src, sw, sh, &mut got, tw, th);
            scale_scalar(
                &src,
                sw as usize,
                sh as usize,
                &mut want,
                tw as usize,
                th as usize,
            );
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
        eprintln!("nv12::scale 1280x720->1920x1080: {ms_per_frame:.3} ms/frame ({fps:.1} FPS)");

        let src_1080 = pattern(1920, 1080);
        let start_id = std::time::Instant::now();
        for _ in 0..iters {
            scale(&src_1080, 1920, 1080, &mut dst_1080, 1920, 1080);
            std::hint::black_box(&dst_1080);
        }
        let elapsed_id = start_id.elapsed();
        let ms_id = elapsed_id.as_secs_f64() * 1000.0 / f64::from(iters);
        let fps_id = 1000.0 / ms_id.max(1e-6);
        eprintln!("nv12::scale 1920x1080 identity: {ms_id:.3} ms/frame ({fps_id:.1} FPS)");
    }
}
