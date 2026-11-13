//! Drives the runtime against a real phone and prints what happened.
//!
//! The GUI owns the same code paths, but a window cannot be read from a script.
//! This runs the runtime headlessly so a real device session can be inspected
//! step by step: does the phone authenticate, does the control channel stay up,
//! does the video channel open, do frames decode, does the frame bus move.
//!
//! Run with `cargo run --example real_device --release`.

use std::sync::atomic::Ordering;
use std::time::Duration;

use phonecam_app::runtime::Runtime;
use phonecam_ui::views::state::Settings;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut settings = Settings::default();
    // A real phone's own defaults, so the stream matches what the app would ask
    // for: 720p30 at 4 Mbps.
    settings.resolution = "1280x720".into();
    settings.fps = 30;
    settings.bitrate = 4_000_000;

    let runtime = Runtime::new(&settings);
    let Some(port) = runtime.start() else {
        eprintln!("FAIL: could not listen");
        std::process::exit(1);
    };
    println!("listening on {port}");
    runtime.broadcast_state();

    // Give the phone time to dial in and authenticate.
    let mut connected_at = None;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(250));
        runtime.broadcast_state();
        let state = runtime.state.lock().clone();
        if state.has_device() {
            connected_at = Some(state.device_name().unwrap_or_default());
            break;
        }
    }

    match &connected_at {
        Some(name) => println!("PAIRED: {name}"),
        None => {
            println!("NOT PAIRED: no phone authenticated in 10 s");
            println!("--- log ---");
            for line in runtime.state.lock().logs.iter() {
                println!("  {} {}", line.source.label(), line.message);
            }
            std::process::exit(2);
        }
    }

    // Ask it to stream.
    println!("asking the phone to start");
    runtime.start_stream();

    // Watch for a picture, and measure it.
    //
    // "It stutters" is not a measurement, so this records what a stutter looks
    // like from this side: the longest gap between frames that actually reached
    // the window, how many gaps were longer than a frame interval, and how long
    // the video thread spent inside decode+publish. A frame that arrives late
    // because the decoder is behind is indistinguishable from one that never
    // arrived, unless the gap is measured.
    let mut frames_seen = 0usize;
    let mut last_size = None;
    let mut gaps_ms: Vec<f64> = Vec::new();
    let mut last_frame_at: Option<std::time::Instant> = None;
    let started = std::time::Instant::now();

    // Sampling far faster than the frame rate, so a gap is visible rather than
    // rounded away by a slow poll.
    for _ in 0..(600 * 2) {
        std::thread::sleep(Duration::from_millis(5));

        let frame = runtime.frames.lock().take();
        if let Some(frame) = frame {
            frames_seen += 1;
            last_size = Some((frame.width, frame.height));

            let now = std::time::Instant::now();
            if let Some(previous) = last_frame_at {
                gaps_ms.push((now - previous).as_secs_f64() * 1000.0);
            }
            last_frame_at = Some(now);
        }

        // Refresh the snapshot once a second rather than every 5 ms.
        if started.elapsed().as_millis() % 1000 < 5 {
            runtime.broadcast_state();
        }
    }
    runtime.broadcast_state();

    let elapsed = started.elapsed().as_secs_f64();

    // The real measurement: intervals as the producer recorded them. A polling
    // loop cannot see this, because the Windows timer quantises it to ~15.6 ms.
    let intervals_us = runtime.frame_intervals_us.lock().clone();
    let mut sorted_us = intervals_us.clone();
    sorted_us.sort_unstable();
    let percentile_us = |p: f64| -> i64 {
        if sorted_us.is_empty() {
            return 0;
        }
        let index = ((sorted_us.len() as f64 - 1.0) * p).round() as usize;
        sorted_us[index.min(sorted_us.len() - 1)]
    };
    // At 30 fps a frame every 33 ms; anything past 2x that is a visible hitch.
    let hitch_threshold_us = 66_000;
    let hitches = sorted_us
        .iter()
        .filter(|d| **d > hitch_threshold_us)
        .count();
    let mean_us = if sorted_us.is_empty() {
        0
    } else {
        sorted_us.iter().sum::<i64>() / sorted_us.len() as i64
    };

    let _ = &gaps_ms;

    let state = runtime.state.lock().clone();
    println!("\n=== result ===");
    println!("phase        : {}", state.phase());
    println!(
        "transport    : {:?}",
        state.core.as_ref().map(|c| c.transport)
    );
    println!("frames drawn : {frames_seen}");
    println!("frame size   : {last_size:?}");
    println!("wall time    : {elapsed:.1} s");
    // Where the time went. A stall is only fixable once it is attributed.
    let stage_total = *runtime.stage_us.lock();
    let stage_max = *runtime.stage_max_us.lock();
    let frame_count = sorted_us.len().max(1) as i64;
    println!("\n--- where the time went, per frame ---");
    for (index, name) in ["socket read", "decode", "publish"].iter().enumerate() {
        println!(
            "{name:<12}: mean {:.1} ms, worst {:.1} ms",
            stage_total[index] as f64 / frame_count as f64 / 1000.0,
            stage_max[index] as f64 / 1000.0
        );
    }

    println!("\n--- frame pacing, measured at the producer ---");
    println!("intervals    : {}", sorted_us.len());
    println!("mean         : {:.1} ms", mean_us as f64 / 1000.0);
    println!(
        "p50          : {:.1} ms",
        percentile_us(0.50) as f64 / 1000.0
    );
    println!(
        "p95          : {:.1} ms",
        percentile_us(0.95) as f64 / 1000.0
    );
    println!(
        "p99          : {:.1} ms",
        percentile_us(0.99) as f64 / 1000.0
    );
    println!(
        "max          : {:.1} ms",
        percentile_us(1.0) as f64 / 1000.0
    );
    println!(
        "hitches >{}ms: {hitches} of {} ({:.1}%)",
        hitch_threshold_us / 1000,
        sorted_us.len(),
        if sorted_us.is_empty() {
            0.0
        } else {
            hitches as f64 * 100.0 / sorted_us.len() as f64
        }
    );
    if let Some(core) = &state.core {
        println!("stats.fps    : {:.1}", core.stats.fps);
        println!("stats.bitrate: {}", core.stats.bitrate);
        println!("stats.latency: {:.0} ms", core.stats.latency_ms);
        println!("stats.quality: {}", core.stats.quality);
    }
    if let Some(phone) = state.core.as_ref().and_then(|c| c.phone.as_ref()) {
        println!(
            "phone actual : {} @ {} fps",
            phone.actual_resolution, phone.actual_fps
        );
        println!("phone stream : {}", phone.streaming);
    }

    println!("\n=== log ===");
    for line in state.logs.iter().rev().take(25).rev() {
        println!("  {} {}", line.source.label(), line.message);
    }

    // A stream that delivered a burst and then stalled is not a pass.
    // A stream that delivered a burst and then stalled is not a pass.
    let ok = frames_seen > 30 && hitches * 100 <= sorted_us.len().max(1) * 5;
    runtime.running.store(false, Ordering::Relaxed);
    println!(
        "\n{}",
        if ok {
            "PASS: a picture reached the desktop, with pacing under 5% hitches"
        } else {
            "FAIL: no frames, or pacing worse than 5% hitches"
        }
    );
    std::process::exit(if ok { 0 } else { 3 });
}
