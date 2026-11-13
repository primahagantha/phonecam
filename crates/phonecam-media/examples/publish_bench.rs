//! How long the per-frame conversion actually takes.
//!
//! The end-to-end harness cannot answer this: on a contended machine the same
//! binary measured 15.7, 30.0 and 29.8 fps across three consecutive runs, so
//! machine scheduling swamps the difference between one implementation and
//! another. This measures the conversion alone, on a fixed frame, so the number
//! means something.
//!
//! Run with `cargo bench -p phonecam-media`.

use std::time::Instant;

use phonecam_media::DecodedFrame;

/// A plausible 720p frame: mid-grey luma with a moving gradient, neutral chroma.
fn frame(width: u32, height: u32) -> DecodedFrame {
    let (w, h) = (width as usize, height as usize);
    let mut yuv = vec![0u8; w * h * 3 / 2];
    for y in 0..h {
        for x in 0..w {
            yuv[y * w + x] = ((x + y) % 220 + 16) as u8;
        }
    }
    let chroma = w * h;
    for i in 0..(w * h / 2) {
        yuv[chroma + i] = if i % 2 == 0 { 90 } else { 160 };
    }
    DecodedFrame {
        width,
        height,
        yuv,
        pts_us: 0,
    }
}

fn bench(name: &str, iterations: usize, mut body: impl FnMut()) {
    // Warm the caches and the allocator first.
    for _ in 0..50 {
        body();
    }
    let started = Instant::now();
    for _ in 0..iterations {
        body();
    }
    let per = started.elapsed().as_secs_f64() * 1000.0 / iterations as f64;
    println!("{name:<34} {per:>7.2} ms/frame");
}

fn main() {
    let source = frame(1280, 720);
    let out = (1920u32, 1080u32);
    let out_len = (out.0 as usize) * (out.1 as usize) * 3 / 2;
    let bgra_len = 1280 * 720 * 4;

    println!("--- 1280x720 in, 1920x1080 out, 200 iterations ---\n");

    // The old shape: allocate everything, every frame.
    bench("allocate-every-frame", 200, || {
        let mut nv12 = vec![0u8; out_len];
        let mut source_buf = vec![0u8; 1280 * 720 * 3 / 2];
        source.write_nv12(&mut source_buf);
        phonecam_framebus::nv12::scale(&source_buf, 1280, 720, &mut nv12, out.0, out.1);
        let mut bgra = vec![0u8; bgra_len];
        source.write_bgra(&mut bgra, 1280);
        std::hint::black_box(&nv12);
        std::hint::black_box(&bgra);
    });

    // The current shape: reuse the buffers.
    let mut nv12 = vec![0u8; out_len];
    let mut source_buf = vec![0u8; 1280 * 720 * 3 / 2];
    let mut bgra = vec![0u8; bgra_len];
    bench("reuse-buffers", 200, || {
        source.write_nv12(&mut source_buf);
        phonecam_framebus::nv12::scale(&source_buf, 1280, 720, &mut nv12, out.0, out.1);
        source.write_bgra(&mut bgra, 1280);
        std::hint::black_box(&nv12);
        std::hint::black_box(&bgra);
    });

    // Each stage on its own, so the split is visible.
    bench("write_nv12 only", 200, || {
        source.write_nv12(&mut source_buf);
        std::hint::black_box(&source_buf);
    });
    bench("scale only", 200, || {
        phonecam_framebus::nv12::scale(&source_buf, 1280, 720, &mut nv12, out.0, out.1);
        std::hint::black_box(&nv12);
    });
    bench("write_bgra only", 200, || {
        source.write_bgra(&mut bgra, 1280);
        std::hint::black_box(&bgra);
    });

    let src_1080 = frame(1920, 1080);
    let planes_1080 = src_1080.as_planes().expect("1080p planes");
    let mut nv12_1080 = vec![0u8; 1920 * 1080 * 3 / 2];
    let mut bgra_1080 = vec![0u8; 1920 * 1080 * 4];
    println!("\n--- 1920x1080 in, 1920x1080 out (1080p60), 200 iterations ---\n");
    bench("1080p write_planes_nv12", 200, || {
        phonecam_media::write_planes_nv12(&planes_1080, &mut nv12_1080);
        std::hint::black_box(&nv12_1080);
    });
    bench("1080p write_planes_bgra", 200, || {
        phonecam_media::write_planes_bgra(&planes_1080, &mut bgra_1080, 1920);
        std::hint::black_box(&bgra_1080);
    });
    bench("1080p nv12+bgra (preview on)", 200, || {
        phonecam_media::write_planes_nv12(&planes_1080, &mut nv12_1080);
        phonecam_media::write_planes_bgra(&planes_1080, &mut bgra_1080, 1920);
        std::hint::black_box(&nv12_1080);
        std::hint::black_box(&bgra_1080);
    });
}
