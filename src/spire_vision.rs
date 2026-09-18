//! Local Spire detector: one capture, one full-screen search, no network.
//!
//! This is the Spire instance of the shared [`crate::building_vision`] profile
//! detector: the template, click offset, portrait ROI/mask and thresholds below
//! are the only Spire-specific data. Matching is *shape* matching, not a colour
//! blob; see `building_vision` for the algorithm and the safety gates.
//!
//! The committed screenshot is a **single calibration scene**, not evidence of
//! generalisation. Thresholds were chosen so all four real crowns pass and the
//! recorded distractors do not.

pub use crate::building_vision::{BuildingProfile, CLIENT_HEIGHT, CLIENT_WIDTH};
use crate::frame::{Frame, Point, Rect};

pub use crate::building_vision::{
    BuildingDetection, BuildingScan, SelectionVerification, detect_buildings, supported_client,
    verify_building_selection,
};

/// Spire detector/verifier profile instance.
pub static PROFILE: BuildingProfile = BuildingProfile {
    label: "Spire",
    template: CROWN_TEMPLATE,
    template_w: TEMPLATE_W,
    template_h: TEMPLATE_H,
    click_offset: TEMPLATE_CLICK_OFFSET,
    safe_viewport: SAFE_VIEWPORT,
    portrait_roi: PORTRAIT_ROI,
    portrait_mask: SPIRE_PORTRAIT_MASK,
    min_ncc: MIN_NCC,
    min_frame_stddev: MIN_FRAME_STDDEV,
    min_edge_agreement: MIN_EDGE_AGREEMENT,
    coarse_min_ncc: COARSE_MIN_NCC,
    mid_min_ncc: MID_MIN_NCC,
    nms_radius: NMS_RADIUS,
    max_detections: MAX_DETECTIONS,
    portrait_channel_min: PORTRAIT_CHANNEL_MIN,
    verify_coverage_min: VERIFY_COVERAGE_MIN,
    verify_iou_min: VERIFY_IOU_MIN,
};

/// One confirmed Spire crown.
pub type SpireDetection = BuildingDetection;
/// Result of one full-screen Spire scan.
pub type SpireScan = BuildingScan;

/// Conservative search region: keeps the top resource bar and the bottom
/// HUD/minimap out of the search so the detector can never "find" a HUD icon.
/// The lowest visible crown centre (`y ≈ 655`) is still inside it.
pub const SAFE_VIEWPORT: Rect = Rect::new(
    crate::play_area::EDGE_GUARD,
    crate::play_area::EDGE_GUARD,
    CLIENT_WIDTH - 2 * crate::play_area::EDGE_GUARD,
    746,
);
/// Crown template size in pixels.
pub const TEMPLATE_W: i32 = 96;
/// Crown template size in pixels.
pub const TEMPLATE_H: i32 = 96;
/// Where inside the template the click lands: the crown centre.
pub const TEMPLATE_CLICK_OFFSET: Point = Point::new(48, 48);

/// Screen rectangle of the single-unit information-panel portrait, used to
/// verify that a click really selected a Spire.
pub const PORTRAIT_ROI: Rect = Rect::new(636, 874, 104, 140);

/// Grayscale crown template, cropped from `screen.png` at `(742, 110)`.
const CROWN_TEMPLATE: &[u8] =
    include_bytes!("../tests/fixtures/spire-screen-1080/crown-template-96x96.gray");
/// Portrait silhouette mask (`255` = sprite pixel), cropped from `screen.png`
/// at `(636, 874)` and thresholded with `max(r, g, b) > 32`.
const SPIRE_PORTRAIT_MASK: &[u8] =
    include_bytes!("../tests/fixtures/spire-screen-1080/spire-portrait-104x140.mask");

/// Score above which a full-resolution candidate is reported.
const MIN_NCC: f32 = 0.55;
/// Minimum frame window contrast (0..255 luma stddev) to accept a match.
const MIN_FRAME_STDDEV: f32 = 12.0;
/// Minimum fraction of template edge pixels that coincide with screen edges.
const MIN_EDGE_AGREEMENT: f32 = 0.5;
/// Coarse stage accept threshold and candidate caps.
const COARSE_MIN_NCC: f32 = 0.25;
const MID_MIN_NCC: f32 = 0.4;
/// Upper bound on reported Spires in one scene.
const MAX_DETECTIONS: usize = 32;
/// Two detections closer than this are the same building.
const NMS_RADIUS: i32 = 48;
/// A portrait pixel exists when any channel is above this; the panel is black.
const PORTRAIT_CHANNEL_MIN: u8 = 32;
/// Selection verification thresholds (shape overlap with the real portrait).
///
/// Calibrated to prevent false positive acceptances on other Zerg single-unit
/// information panels. For example, a morphing Creep Colony (`not-drone-colony.png`)
/// exhibits ~51% IoU / ~76% coverage against the Spire silhouette. At 0.80 coverage
/// and 0.65 IoU, the Spire is accepted with up to ±3 px shift while colony
/// (max 0.55 IoU) and drone (max 0.45 IoU) are strictly rejected.
const VERIFY_COVERAGE_MIN: f32 = 0.8;
const VERIFY_IOU_MIN: f32 = 0.65;

/// True when `frame` is the supported 1920×1080 client at `(0, 0)`.
pub fn supported_profile(frame: &Frame) -> bool {
    supported_client(frame)
}

/// Template rectangle for a detection whose crown centre is `center`.
pub fn template_rect(center: Point) -> Rect {
    crate::building_vision::template_rect(center, &PROFILE)
}

/// The embedded crown grayscale pixels (`TEMPLATE_W * TEMPLATE_H`), exposed so
/// tests can paint a synthetic crown without touching the file system.
pub fn crown_template_pixels() -> &'static [u8] {
    CROWN_TEMPLATE
}

/// The embedded portrait mask pixels (`PORTRAIT_ROI.w * PORTRAIT_ROI.h`).
pub fn portrait_mask_pixels() -> &'static [u8] {
    SPIRE_PORTRAIT_MASK
}

/// Scans one frame for Spire crowns. This is the single expensive pass.
pub fn detect_spires(frame: &Frame) -> SpireScan {
    detect_buildings(frame, &PROFILE)
}

/// Scores a captured selection-panel ROI against the reference Spire portrait.
pub fn verify_spire_selection(roi: &Frame) -> SelectionVerification {
    verify_building_selection(roi, &PROFILE)
}

pub fn point_in_viewport(point: Point) -> bool {
    crate::building_vision::point_in_viewport(point)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::building_vision::luma;
    use std::path::{Path, PathBuf};

    fn fixture(name: &str) -> Frame {
        let path = PathBuf::from("tests/fixtures/spire-screen-1080").join(name);
        Frame::from_png(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
    }

    fn remastered(name: &str) -> Frame {
        let path = Path::new("tests/fixtures/remastered-1080").join(name);
        Frame::from_png(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
    }

    /// Places a crop at `offset` inside a blank supported-profile client frame.
    fn blit(crop: &Frame, offset: Point) -> Frame {
        let mut frame = Frame::blank(CLIENT_WIDTH as u32, CLIENT_HEIGHT as u32);
        for y in 0..crop.height() as i32 {
            for x in 0..crop.width() as i32 {
                if let Some(px) = crop.pixel(x, y) {
                    frame.set_pixel(offset.x + x, offset.y + y, px);
                }
            }
        }
        frame
    }

    fn crop(frame: &Frame, x: i32, y: i32, w: i32, h: i32) -> Frame {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for py in y..y + h {
            for px in x..x + w {
                let pixel = frame
                    .pixel(px, py)
                    .unwrap_or(crate::frame::Rgb::new(0, 0, 0));
                rgba.extend_from_slice(&[pixel.r, pixel.g, pixel.b, 255]);
            }
        }
        Frame::new(w as u32, h as u32, Point::new(x, y), rgba).expect("crop frame")
    }

    /// Paints the embedded crown template with its centre at `center`.
    fn paint_crown(frame: &mut Frame, center: Point) {
        let rect = template_rect(center);
        let pixels = crown_template_pixels();
        for j in 0..TEMPLATE_H {
            for i in 0..TEMPLATE_W {
                let value = pixels[(j * TEMPLATE_W + i) as usize];
                frame.set_pixel(
                    rect.x + i,
                    rect.y + j,
                    crate::frame::Rgb::new(value, value, value),
                );
            }
        }
    }

    fn distance(a: Point, b: Point) -> i32 {
        (a.x - b.x).abs().max((a.y - b.y).abs())
    }

    /// The four crowns actually visible in the calibration screenshot. The
    /// detector reports the crown centre, which sits a few pixels below the
    /// approximate coordinates quoted in the task, so the tolerance is 24 px.
    const EXPECTED: [Point; 4] = [
        Point::new(790, 150),
        Point::new(790, 365),
        Point::new(1365, 365),
        Point::new(790, 655),
    ];

    /// The fifteen fully visible crowns in the dense fixture `spire-dense-1080/screen.png`.
    const DENSE_EXPECTED: [Point; 15] = [
        Point::new(718, 84),
        Point::new(862, 84),
        Point::new(574, 156),
        Point::new(718, 227),
        Point::new(862, 228),
        Point::new(1006, 227),
        Point::new(574, 299),
        Point::new(718, 371),
        Point::new(862, 372),
        Point::new(1006, 372),
        Point::new(574, 443),
        Point::new(862, 515),
        Point::new(1006, 515),
        Point::new(718, 516),
        Point::new(574, 587),
    ];

    #[test]
    fn the_dense_fixture_yields_all_fifteen_visible_spires() {
        let path = PathBuf::from("tests/fixtures/spire-dense-1080/screen.png");
        let screen = Frame::from_png(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        let scan = detect_spires(&screen);
        assert!(scan.supported_profile);
        assert_eq!(scan.count(), 15, "detections: {:?}", scan.detections);
        for expected in DENSE_EXPECTED {
            assert!(
                scan.detections
                    .iter()
                    .any(|d| distance(d.center, expected) <= 8),
                "expected spire near {expected:?} not found in detections: {:?}",
                scan.detections
            );
        }
    }

    #[test]
    fn the_real_fixture_yields_the_four_visible_spires() {
        let scan = detect_spires(&fixture("screen.png"));
        assert!(scan.supported_profile);
        let centers: Vec<Point> = scan.detections.iter().map(|d| d.center).collect();
        assert_eq!(centers.len(), 4, "detections: {centers:?}");
        for expected in EXPECTED {
            assert!(
                centers
                    .iter()
                    .any(|center| distance(*center, expected) <= 24),
                "no detection near {expected:?}; got {centers:?}"
            );
        }
    }

    #[test]
    fn the_lower_crown_partly_behind_the_hud_is_detected_inside_the_viewport() {
        let scan = detect_spires(&fixture("screen.png"));
        let lower = scan
            .detections
            .iter()
            .find(|d| d.center.x < 900 && d.center.y > 600)
            .expect("the lower Spire must be found");
        assert!(point_in_viewport(lower.center));
    }

    #[test]
    fn detections_are_deduplicated() {
        let scan = detect_spires(&fixture("screen.png"));
        for (i, a) in scan.detections.iter().enumerate() {
            for b in scan.detections.iter().skip(i + 1) {
                assert!(
                    distance(a.center, b.center) > NMS_RADIUS,
                    "{} and {} are the same building",
                    format_point(a.center),
                    format_point(b.center)
                );
            }
        }
    }

    #[test]
    fn every_detection_passes_the_confidence_gates() {
        let scan = detect_spires(&fixture("screen.png"));
        for detection in &scan.detections {
            assert!(detection.score >= MIN_NCC, "{detection:?}");
            assert!(
                detection.edge_agreement >= MIN_EDGE_AGREEMENT,
                "{detection:?}"
            );
            assert!(detection.frame_stddev >= MIN_FRAME_STDDEV, "{detection:?}");
        }
    }

    #[test]
    fn the_trained_crown_is_not_the_only_shape_that_matches() {
        // The template was taken from the top-right crown; the other three are
        // matched independently. Their scores are reported separately so a
        // regression on any single one is visible.
        let scan = detect_spires(&fixture("screen.png"));
        let right = scan
            .detections
            .iter()
            .find(|d| d.center.x > 1200)
            .expect("right crown");
        let second = scan
            .detections
            .iter()
            .find(|d| (d.center.y - 365).abs() < 40 && d.center.x < 900)
            .expect("second crown");
        assert!(right.score > 0.6, "{right:?}");
        assert!(second.score > 0.6, "{second:?}");
    }

    #[test]
    fn drone_and_colony_screenshots_are_negatives() {
        for name in [
            "drones-2.png",
            "drones-3.png",
            "drones-4.png",
            "drones-5.png",
            "drones-5-tooltip.png",
            "drone-single.png",
            "not-drone-colony.png",
            "preview-first.png",
            "preview-second.png",
            "no-preview.png",
        ] {
            let placed = blit(&remastered(name), Point::new(600, 300));
            let scan = detect_spires(&placed);
            assert_eq!(scan.count(), 0, "{name} produced {:?}", scan.detections);
        }
    }

    #[test]
    fn a_blank_capture_is_not_a_spire() {
        let scan = detect_spires(&Frame::blank(CLIENT_WIDTH as u32, CLIENT_HEIGHT as u32));
        assert!(scan.supported_profile);
        assert_eq!(scan.count(), 0);
    }

    #[test]
    fn a_wrong_profile_is_refused_instead_of_scaled() {
        let frame = Frame::blank(1280, 720);
        assert!(!supported_profile(&frame));
        let scan = detect_spires(&frame);
        assert!(!scan.supported_profile);
        assert_eq!(scan.count(), 0);
    }

    #[test]
    fn a_crown_outside_the_safe_viewport_is_ignored() {
        // Top resource bar and bottom HUD are excluded on purpose: a click
        // there would scroll the camera or press a HUD button.
        let mut top = Frame::blank(CLIENT_WIDTH as u32, CLIENT_HEIGHT as u32);
        paint_crown(&mut top, Point::new(400, 40));
        assert_eq!(detect_spires(&top).count(), 0, "top bar");

        let mut hud = Frame::blank(CLIENT_WIDTH as u32, CLIENT_HEIGHT as u32);
        paint_crown(&mut hud, Point::new(400, 760));
        assert_eq!(detect_spires(&hud).count(), 0, "HUD");
    }

    #[test]
    fn a_high_contrast_border_is_not_a_crown() {
        let mut frame = Frame::blank(CLIENT_WIDTH as u32, CLIENT_HEIGHT as u32);
        for x in 200..1000 {
            frame.set_pixel(x, 200, crate::frame::Rgb::new(255, 255, 255));
            frame.set_pixel(x, 320, crate::frame::Rgb::new(255, 255, 255));
        }
        for y in 200..320 {
            frame.set_pixel(200, y, crate::frame::Rgb::new(255, 255, 255));
            frame.set_pixel(1000, y, crate::frame::Rgb::new(255, 255, 255));
        }
        assert_eq!(detect_spires(&frame).count(), 0);
    }

    #[test]
    fn the_selection_portrait_inside_the_play_field_is_not_a_crown() {
        // The HUD portrait is the Spire wireframe icon, not the live crown; it
        // is also below the safe viewport. Painting it *inside* the field must
        // still not be mistaken for a crown shape.
        let source = fixture("screen.png");
        let portrait = crop(
            &source,
            PORTRAIT_ROI.x,
            PORTRAIT_ROI.y,
            PORTRAIT_ROI.w,
            PORTRAIT_ROI.h,
        );
        let placed = blit(&portrait, Point::new(600, 200));
        assert_eq!(detect_spires(&placed).count(), 0);
    }

    #[test]
    fn the_embedded_assets_are_derived_from_the_committed_screenshot() {
        // The template and the portrait mask are committed as bytes; this
        // recomputes both from `screen.png` so they cannot silently drift.
        let source = fixture("screen.png");
        let origin = Point::new(742, 110); // the crown crop documented in the fixture README
        assert_eq!(
            template_rect(Point::new(790, 158)),
            Rect::new(origin.x, origin.y, 96, 96)
        );
        let template = crown_template_pixels();
        for j in 0..TEMPLATE_H {
            for i in 0..TEMPLATE_W {
                let px = source.pixel(origin.x + i, origin.y + j).expect("pixel");
                assert_eq!(
                    template[(j * TEMPLATE_W + i) as usize],
                    luma(px.r, px.g, px.b),
                    "crown template pixel ({i}, {j})"
                );
            }
        }
        let mask = portrait_mask_pixels();
        for y in 0..PORTRAIT_ROI.h {
            for x in 0..PORTRAIT_ROI.w {
                let px = source
                    .pixel(PORTRAIT_ROI.x + x, PORTRAIT_ROI.y + y)
                    .expect("pixel");
                let expected = if px.r.max(px.g).max(px.b) > PORTRAIT_CHANNEL_MIN {
                    255
                } else {
                    0
                };
                assert_eq!(
                    mask[(y * PORTRAIT_ROI.w + x) as usize],
                    expected,
                    "portrait mask pixel ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn selection_verification_accepts_the_real_portrait() {
        let source = fixture("screen.png");
        let roi = crop(
            &source,
            PORTRAIT_ROI.x,
            PORTRAIT_ROI.y,
            PORTRAIT_ROI.w,
            PORTRAIT_ROI.h,
        );
        let verification = verify_spire_selection(&roi);
        assert!(verification.accepted, "{verification:?}");
        assert!(verification.coverage > 0.95, "{verification:?}");
    }

    #[test]
    fn selection_verification_rejects_a_blank_panel() {
        let blank = Frame::blank(PORTRAIT_ROI.w as u32, PORTRAIT_ROI.h as u32);
        let verification = verify_spire_selection(&blank);
        assert!(!verification.accepted, "{verification:?}");
        assert_eq!(verification.coverage, 0.0);
    }

    #[test]
    fn selection_verification_rejects_a_drone_portrait() {
        let drones = remastered("drones-2.png");
        let slot = crop(&drones, 53, 40, PORTRAIT_ROI.w, PORTRAIT_ROI.h);
        let verification = verify_spire_selection(&slot);
        assert!(!verification.accepted, "{verification:?}");
    }

    #[test]
    fn selection_verification_rejects_a_morphing_colony_panel() {
        let colony = remastered("not-drone-colony.png");
        // Fixture was cropped at (600, 880). Screen PORTRAIT_ROI is (636, 874).
        // Relative to fixture crop, offset is (636 - 600, 874 - 880) = (36, -6).
        let slot = crop(&colony, 36, -6, PORTRAIT_ROI.w, PORTRAIT_ROI.h);
        let verification = verify_spire_selection(&slot);
        assert!(
            !verification.accepted,
            "morphing colony must not be accepted: {verification:?}"
        );
        assert!(
            verification.iou < VERIFY_IOU_MIN,
            "colony IoU ({}) must be below minimum {VERIFY_IOU_MIN}",
            verification.iou
        );
    }

    #[test]
    fn selection_verification_rejects_a_single_drone_panel() {
        let drone = remastered("drone-single.png");
        let slot = crop(&drone, 36, -6, PORTRAIT_ROI.w, PORTRAIT_ROI.h);
        let verification = verify_spire_selection(&slot);
        assert!(
            !verification.accepted,
            "single drone must not be accepted: {verification:?}"
        );
    }

    #[test]
    fn selection_verification_tolerates_small_misalignments() {
        let source = fixture("screen.png");
        for dy in -2..=2 {
            for dx in -2..=2 {
                let roi = crop(
                    &source,
                    PORTRAIT_ROI.x + dx,
                    PORTRAIT_ROI.y + dy,
                    PORTRAIT_ROI.w,
                    PORTRAIT_ROI.h,
                );
                let verification = verify_spire_selection(&roi);
                assert!(
                    verification.accepted,
                    "offset ({dx}, {dy}) failed: {verification:?}"
                );
            }
        }
    }

    #[test]
    fn selection_verification_rejects_a_different_roi_size() {
        let roi = Frame::blank(10, 10);
        let verification = verify_spire_selection(&roi);
        assert!(!verification.accepted);
        assert_eq!(verification.coverage, 0.0);
    }

    fn format_point(point: Point) -> String {
        format!("({}, {})", point.x, point.y)
    }
}
