//! Read-only Spire detector diagnostic and benchmark.
//!
//! Prints the crown positions, the count, and the capture/detection timings for
//! a saved 1920×1080 screenshot. It never touches the game, never opens a
//! window and never injects input.
//!
//! ```text
//! cargo run --example spire-scan -- tests/fixtures/spire-screen-1080/screen.png
//! cargo run --release --example spire-scan -- capture.png --repeat 5
//! ```
//!
//! `--repeat N` runs the detection `N` more times and prints the best (lowest)
//! detection time; build with `--release` for a realistic scan benchmark.

use std::path::Path;
use std::time::Instant;

use oh_my_macro::frame::Frame;
use oh_my_macro::spire_vision::{SAFE_VIEWPORT, detect_spires, supported_profile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: spire-scan <png> [--repeat N]");
        std::process::exit(2);
    };
    let mut repeat = 0usize;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--repeat" => {
                repeat = args
                    .get(index + 1)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                index += 2;
            }
            other => {
                eprintln!("unknown argument '{other}'");
                std::process::exit(2);
            }
        }
    }

    let capture_started = Instant::now();
    let frame = Frame::from_png(Path::new(path))?;
    let capture_ms = capture_started.elapsed().as_millis();
    println!(
        "image: {}x{} ({} profile) decoded in {capture_ms} ms",
        frame.width(),
        frame.height(),
        if supported_profile(&frame) {
            "supported 1920x1080"
        } else {
            "UNSUPPORTED"
        }
    );
    println!(
        "safe viewport: x {}..{} y {}..{}",
        SAFE_VIEWPORT.x,
        SAFE_VIEWPORT.x + SAFE_VIEWPORT.w,
        SAFE_VIEWPORT.y,
        SAFE_VIEWPORT.y + SAFE_VIEWPORT.h
    );

    let scan = detect_spires(&frame);
    let mut timings = vec![scan.detect_ms];
    for _ in 0..repeat {
        timings.push(detect_spires(&frame).detect_ms);
    }
    timings.sort_unstable();
    let best_detect_ms = timings.first().copied().unwrap_or(scan.detect_ms);
    println!(
        "spires: {} (detect {} ms{}, evaluated {} positions)",
        scan.count(),
        best_detect_ms,
        if repeat > 0 {
            format!(" best of {}", repeat + 1)
        } else {
            String::new()
        },
        scan.evaluated
    );
    if repeat > 0 {
        let max_detect_ms = timings.last().copied().unwrap_or(best_detect_ms);
        let median_detect_ms = timings[timings.len() / 2];
        let mean_detect_ms = timings.iter().sum::<u128>() as f64 / timings.len() as f64;
        println!(
            "detection distribution ({} iterations): min = {} ms, median = {} ms, mean = {:.1} ms, max = {} ms",
            timings.len(),
            best_detect_ms,
            median_detect_ms,
            mean_detect_ms,
            max_detect_ms
        );
        println!("  runs (ms): {timings:?}");
    }
    for detection in &scan.detections {
        println!(
            "  click ({}, {}) score={:.3} edge={:.3} stddev={:.1}",
            detection.center.x,
            detection.center.y,
            detection.score,
            detection.edge_agreement,
            detection.frame_stddev
        );
    }
    if !scan.supported_profile {
        eprintln!("this build only detects the 1920x1080 Remastered profile");
    }
    Ok(())
}
