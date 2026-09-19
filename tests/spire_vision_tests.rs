//! Vision-specific integration tests for Spire detection and HUD verification.

use std::path::PathBuf;

use oh_my_macro::frame::{Frame, Point, Rect};
use oh_my_macro::spire_vision::{
    CLIENT_HEIGHT, CLIENT_WIDTH, PORTRAIT_ROI, SAFE_VIEWPORT, TEMPLATE_CLICK_OFFSET, TEMPLATE_H,
    TEMPLATE_W, detect_spires, supported_profile, verify_spire_selection,
};

fn fixture(name: &str) -> Frame {
    let path = PathBuf::from("tests/fixtures/spire-screen-1080").join(name);
    Frame::from_png(&path).unwrap_or_else(|err| panic!("failed to load {path:?}: {err}"))
}

fn remastered(name: &str) -> Frame {
    let path = PathBuf::from("tests/fixtures/remastered-1080").join(name);
    Frame::from_png(&path).unwrap_or_else(|err| panic!("failed to load {path:?}: {err}"))
}

fn crop(frame: &Frame, x: i32, y: i32, w: i32, h: i32) -> Frame {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for py in y..y + h {
        for px in x..x + w {
            let pixel = frame
                .pixel(px, py)
                .unwrap_or(oh_my_macro::frame::Rgb::new(0, 0, 0));
            rgba.extend_from_slice(&[pixel.r, pixel.g, pixel.b, 255]);
        }
    }
    Frame::new(w as u32, h as u32, Point::new(x, y), rgba).expect("crop frame")
}

fn blit(crop: &Frame, offset: Point) -> Frame {
    let w = CLIENT_WIDTH as usize;
    let h = CLIENT_HEIGHT as usize;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..crop.height() as i32 {
        for x in 0..crop.width() as i32 {
            let dx = offset.x + x;
            let dy = offset.y + y;
            if (0..CLIENT_WIDTH).contains(&dx)
                && (0..CLIENT_HEIGHT).contains(&dy)
                && let Some(px) = crop.pixel(x, y)
            {
                let idx = (dy as usize * w + dx as usize) * 4;
                rgba[idx] = px.r;
                rgba[idx + 1] = px.g;
                rgba[idx + 2] = px.b;
                rgba[idx + 3] = 255;
            }
        }
    }
    Frame::new(
        CLIENT_WIDTH as u32,
        CLIENT_HEIGHT as u32,
        Point::new(0, 0),
        rgba,
    )
    .expect("blit frame")
}

#[test]
fn test_exact_crop_and_template_metadata() {
    assert_eq!(TEMPLATE_W, 96);
    assert_eq!(TEMPLATE_H, 96);
    assert_eq!(TEMPLATE_CLICK_OFFSET, Point::new(48, 48));
    assert_eq!(PORTRAIT_ROI, Rect::new(636, 874, 104, 140));
    assert_eq!(SAFE_VIEWPORT, Rect::new(24, 24, 1872, 746));
}

#[test]
fn test_separate_evaluation_of_trained_building_vs_candidates() {
    let screen = fixture("screen.png");
    assert!(supported_profile(&screen));

    let scan = detect_spires(&screen);
    assert!(scan.supported_profile);
    assert_eq!(scan.detections.len(), 4, "must detect exactly 4 spires");

    // Building #1: the training building at (790, 158)
    let b1 = scan
        .detections
        .iter()
        .find(|d| (d.center.x - 790).abs() <= 4 && (d.center.y - 158).abs() <= 4)
        .expect("building 1 near (790, 158) must be found");
    assert!(
        (b1.score - 1.0).abs() < 0.001,
        "trained building should match near 1.0; got {}",
        b1.score
    );
    assert!(
        b1.edge_agreement > 0.95,
        "trained building edge agreement should be near 1.0; got {}",
        b1.edge_agreement
    );

    // Building #2: untrained crown at (790, 374), crossed by building #1's health bar at y=354..359
    let b2 = scan
        .detections
        .iter()
        .find(|d| (d.center.x - 790).abs() <= 4 && (d.center.y - 374).abs() <= 4)
        .expect("building 2 near (790, 374) must be found");
    assert!(
        b2.score >= 0.75 && b2.score <= 0.85,
        "building 2 score expected ~0.79 due to crossing health bar; got {}",
        b2.score
    );
    assert!(
        b2.edge_agreement >= 0.70,
        "building 2 edge agreement expected >= 0.70; got {}",
        b2.edge_agreement
    );

    // Building #3: untrained crown on right at (1366, 373)
    let b3 = scan
        .detections
        .iter()
        .find(|d| (d.center.x - 1366).abs() <= 4 && (d.center.y - 373).abs() <= 4)
        .expect("building 3 near (1366, 373) must be found");
    assert!(
        b3.score >= 0.80 && b3.score <= 0.90,
        "building 3 score expected ~0.85; got {}",
        b3.score
    );
    assert!(
        b3.edge_agreement >= 0.75,
        "building 3 edge agreement expected >= 0.75; got {}",
        b3.edge_agreement
    );

    // Building #4: untrained lower crown at (790, 662), base near HUD
    let b4 = scan
        .detections
        .iter()
        .find(|d| (d.center.x - 790).abs() <= 4 && (d.center.y - 662).abs() <= 4)
        .expect("building 4 near (790, 662) must be found");
    assert!(
        b4.score >= 0.95,
        "building 4 score expected ~0.99; got {}",
        b4.score
    );
    assert!(
        b4.edge_agreement >= 0.95,
        "building 4 edge agreement expected >= 0.95; got {}",
        b4.edge_agreement
    );
}

#[test]
fn test_hud_selection_verification_rejects_colony_and_drone_negatives() {
    // 1. Morphing Creep Colony fixture (not-drone-colony.png, size 550x200, captured at (600, 880))
    // Screen PORTRAIT_ROI is (636, 874). In crop coordinates: (36, -6).
    let colony = remastered("not-drone-colony.png");
    let colony_roi = crop(&colony, 36, -6, PORTRAIT_ROI.w, PORTRAIT_ROI.h);
    let colony_v = verify_spire_selection(&colony_roi);
    assert!(
        !colony_v.accepted,
        "morphing creep colony must be REJECTED by selection verification: {colony_v:?}"
    );
    assert!(
        colony_v.iou < 0.65,
        "colony IoU ({:.3}) must be strictly below 0.65 threshold",
        colony_v.iou
    );

    // Also test without vertical offset (36, 0)
    let colony_roi_0 = crop(&colony, 36, 0, PORTRAIT_ROI.w, PORTRAIT_ROI.h);
    let colony_v_0 = verify_spire_selection(&colony_roi_0);
    assert!(
        !colony_v_0.accepted,
        "morphing creep colony (at 0 offset) must be REJECTED: {colony_v_0:?}"
    );

    // 2. Single Drone panel (drone-single.png, size 550x200, captured at (600, 880))
    let drone = remastered("drone-single.png");
    let drone_roi = crop(&drone, 36, -6, PORTRAIT_ROI.w, PORTRAIT_ROI.h);
    let drone_v = verify_spire_selection(&drone_roi);
    assert!(
        !drone_v.accepted,
        "single drone panel must be REJECTED: {drone_v:?}"
    );

    // 3. Multi-drone selections (drones-2..5)
    for name in [
        "drones-2.png",
        "drones-3.png",
        "drones-4.png",
        "drones-5.png",
        "drones-5-tooltip.png",
    ] {
        let frame = remastered(name);
        let roi = crop(&frame, 36, 0, PORTRAIT_ROI.w, PORTRAIT_ROI.h);
        let v = verify_spire_selection(&roi);
        assert!(!v.accepted, "{name} must be rejected: {v:?}");
    }
}

#[test]
fn test_hud_selection_verification_resilience_to_small_misalignments() {
    let screen = fixture("screen.png");
    // Verify that true Spire portrait passes with ±1 and ±2 pixel displacements
    for dy in -2..=2 {
        for dx in -2..=2 {
            let roi = crop(
                &screen,
                PORTRAIT_ROI.x + dx,
                PORTRAIT_ROI.y + dy,
                PORTRAIT_ROI.w,
                PORTRAIT_ROI.h,
            );
            let v = verify_spire_selection(&roi);
            assert!(
                v.accepted,
                "displacement ({dx}, {dy}) must pass: cov={:.3}, iou={:.3}",
                v.coverage, v.iou
            );
            assert!(v.coverage >= 0.85);
            assert!(v.iou >= 0.75);
        }
    }
}

#[test]
fn test_fullscreen_detect_spires_rejects_distractor_crops_at_multiple_offsets() {
    let colony = remastered("not-drone-colony.png");
    let drone = remastered("drone-single.png");

    for offset in [
        Point::new(100, 150),
        Point::new(500, 200),
        Point::new(700, 300),
        Point::new(1000, 400),
    ] {
        let f_colony = blit(&colony, offset);
        let scan_c = detect_spires(&f_colony);
        assert_eq!(
            scan_c.count(),
            0,
            "colony crop at {offset:?} must not trigger spire detection: {:?}",
            scan_c.detections
        );

        let f_drone = blit(&drone, offset);
        let scan_d = detect_spires(&f_drone);
        assert_eq!(
            scan_d.count(),
            0,
            "drone crop at {offset:?} must not trigger spire detection: {:?}",
            scan_d.detections
        );
    }
}

#[test]
fn test_dense_spire_scene_detects_all_safe_spires_with_exact_ground_truth()
-> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new("tests/fixtures/spire-dense-1080/screen.png");
    let screen = Frame::from_png(path)?;
    assert!(supported_profile(&screen));

    let scan = detect_spires(&screen);
    assert!(scan.supported_profile);
    // The fifteen fully visible crowns plus the two Spires whose crowns sit
    // above the top edge. The top-band fragment reaches those two on their
    // visible bodies, so this scene reports 17 clicks instead of 15.
    assert_eq!(
        scan.detections.len(),
        17,
        "dense scene must detect the 15 safe fully visible crowns plus the 2 \
         top-clipped body clicks; got {:?}",
        scan.detections
    );
    for top_clipped_body_click in [Point::new(574, 74), Point::new(1006, 74)] {
        assert!(
            scan.detections.iter().any(|d| {
                (d.center.x - top_clipped_body_click.x).abs() <= 4
                    && (d.center.y - top_clipped_body_click.y).abs() <= 4
            }),
            "expected the top-clipped Spire body click near {top_clipped_body_click:?}; \
             got {:?}",
            scan.detections
        );
    }

    // Ground truth definition for each visible crown:
    // (exact center, score range, min edge agreement)
    let ground_truth: [(Point, (f32, f32), f32); 15] = [
        (Point::new(718, 84), (0.85, 0.93), 0.85),
        (Point::new(862, 84), (0.95, 1.00), 0.95),
        (Point::new(574, 156), (0.95, 1.00), 0.94),
        (Point::new(718, 227), (0.83, 0.90), 0.78),
        (Point::new(1006, 227), (0.75, 0.83), 0.70),
        (Point::new(862, 228), (0.85, 0.93), 0.85),
        (Point::new(574, 299), (0.84, 0.91), 0.78),
        (Point::new(718, 371), (0.83, 0.90), 0.78),
        (Point::new(862, 372), (0.85, 0.93), 0.85),
        (Point::new(1006, 372), (0.94, 1.00), 0.94),
        (Point::new(574, 443), (0.84, 0.91), 0.78),
        (Point::new(862, 515), (0.74, 0.82), 0.70),
        (Point::new(1006, 515), (0.83, 0.91), 0.78),
        (Point::new(718, 516), (0.86, 0.93), 0.85),
        (Point::new(574, 587), (0.74, 0.82), 0.68),
    ];

    for (gt_point, (min_score, max_score), min_edge) in ground_truth {
        let detection = scan
            .detections
            .iter()
            .find(|d| (d.center.x - gt_point.x).abs() <= 4 && (d.center.y - gt_point.y).abs() <= 4)
            .unwrap_or_else(|| {
                panic!(
                    "crown at {gt_point:?} was not detected; all detections: {:?}",
                    scan.detections
                )
            });

        assert!(
            detection.score >= min_score && detection.score <= max_score,
            "crown at {gt_point:?}: score expected in [{min_score}, {max_score}]; got {}",
            detection.score
        );
        assert!(
            detection.edge_agreement >= min_edge,
            "crown at {gt_point:?}: edge agreement expected >= {min_edge}; got {}",
            detection.edge_agreement
        );
        assert!(
            detection.frame_stddev >= 28.0,
            "crown at {gt_point:?}: frame stddev expected >= 28.0; got {}",
            detection.frame_stddev
        );
    }

    // Explicit verification of edge safety and excluded / negative objects:
    // 1. Top-clipped crown anchors are above the screen, but their body clicks
    // stay comfortably below the top camera-scroll guard.
    assert!(
        !scan.detections.iter().any(|d| d.center.y < 50),
        "no detection may click inside the top scroll region"
    );
    // 2. Lower partially HUD-obscured crown at x=862 y≈803:
    assert!(
        !scan.detections.iter().any(|d| d.center.y > 650),
        "lower HUD-obscured crown must be excluded from safe spire detections"
    );
    // 3. Greater Spire at x≈1150:
    assert!(
        !scan.detections.iter().any(|d| d.center.x > 1050),
        "greater Spire and terrain on right must not trigger false positive detection"
    );
    Ok(())
}

#[test]
fn test_dense_spire_scene_generates_annotated_png() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new("tests/fixtures/spire-dense-1080/screen.png");
    let screen = Frame::from_png(path)?;
    let scan = detect_spires(&screen);

    let (w, h) = (screen.width() as i32, screen.height() as i32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for py in 0..h {
        for px in 0..w {
            let px_val = screen
                .pixel(px, py)
                .unwrap_or(oh_my_macro::frame::Rgb::new(0, 0, 0));
            rgba.extend_from_slice(&[px_val.r, px_val.g, px_val.b, 255]);
        }
    }

    {
        let mut canvas = Canvas {
            rgba: &mut rgba,
            w,
            h,
        };
        // Draw detected crowns in bright green, with a centre crosshair.
        for d in &scan.detections {
            let (cx, cy) = (d.center.x, d.center.y);
            canvas.outline_rect(cx - 48, cy - 48, 96, 96, 2, GREEN);
            canvas.crosshair(cx, cy, 8, GREEN);
        }

        // Draw the off-screen crown anchors in yellow for reference. Their safe
        // body click points are already rendered in green above.
        for cx in [574, 1006] {
            canvas.outline_rect(cx - 48, 0, 96, 48, 2, YELLOW);
            canvas.crosshair(cx, 12, 6, YELLOW);
        }

        // Draw the lower HUD-obscured excluded crown in orange at (862, 803).
        canvas.outline_rect(862 - 48, 803 - 48, 96, 96, 2, ORANGE);
        canvas.crosshair(862, 803, 8, ORANGE);

        // Draw the Greater Spire excluded negative in magenta at (1150, 250).
        canvas.outline_rect(1150 - 48, 250 - 48, 120, 120, 2, MAGENTA);
        canvas.crosshair(1150, 250, 8, MAGENTA);
    }

    // The annotated overlay is a diagnostic artifact, not a test result, so a
    // normal `cargo test` does not rewrite (or dirty) the repository. Run with
    // `OH_MY_MACRO_WRITE_ANNOTATIONS=1` to regenerate it after a detector
    // change and inspect which crowns were accepted, excluded, or rejected.
    if std::env::var_os("OH_MY_MACRO_WRITE_ANNOTATIONS").is_some() {
        let out = std::path::Path::new("tests/fixtures/spire-dense-1080/annotated-detections.png");
        save_png(out, w as u32, h as u32, &rgba)?;
    }
    Ok(())
}

/// One annotation colour, so the drawing helpers stay under the argument
/// budget.
#[derive(Clone, Copy)]
struct Ink(u8, u8, u8);

const GREEN: Ink = Ink(0, 255, 0);
const YELLOW: Ink = Ink(255, 255, 0);
const ORANGE: Ink = Ink(255, 140, 0);
const MAGENTA: Ink = Ink(255, 0, 255);

/// The annotation target of the diagnostic overlay: one RGBA canvas.
///
/// Keeping the buffer, its width and its height in one value keeps every
/// drawing call well under the argument budget.
struct Canvas<'a> {
    rgba: &'a mut [u8],
    w: i32,
    h: i32,
}

impl Canvas<'_> {
    /// Fills a clipped rectangle with one colour.
    fn fill_rect(&mut self, x0: i32, y0: i32, rw: i32, rh: i32, ink: Ink) {
        for y in y0.max(0)..(y0 + rh).min(self.h) {
            for x in x0.max(0)..(x0 + rw).min(self.w) {
                let idx = ((y * self.w + x) * 4) as usize;
                self.rgba[idx] = ink.0;
                self.rgba[idx + 1] = ink.1;
                self.rgba[idx + 2] = ink.2;
                self.rgba[idx + 3] = 255;
            }
        }
    }

    /// Draws the four sides of a rectangle.
    fn outline_rect(&mut self, x0: i32, y0: i32, rw: i32, rh: i32, thickness: i32, ink: Ink) {
        self.fill_rect(x0, y0, rw, thickness, ink);
        self.fill_rect(x0, y0 + rh - thickness, rw, thickness, ink);
        self.fill_rect(x0, y0, thickness, rh, ink);
        self.fill_rect(x0 + rw - thickness, y0, thickness, rh, ink);
    }

    /// Draws a small crosshair at a centre point.
    fn crosshair(&mut self, cx: i32, cy: i32, radius: i32, ink: Ink) {
        self.fill_rect(cx - radius, cy - 1, radius * 2 + 1, 3, ink);
        self.fill_rect(cx - 1, cy - radius, 3, radius * 2 + 1, ink);
    }
}

fn save_png(
    path: &std::path::Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(&mut writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut header = encoder.write_header()?;
    header.write_image_data(rgba)?;
    Ok(())
}
