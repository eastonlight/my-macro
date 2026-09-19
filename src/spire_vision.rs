//! Local Spire detector: one capture, one full-screen search, no network.
//!
//! This is the Spire instance of the shared [`crate::building_vision`] profile
//! detector. The calibrated 96×96 crown template is the primary pass; two
//! complementary fragment passes cover crowns that the client edge clips:
//! a top-band body fragment for the Spires whose crowns are above `y = 0` and
//! a right-crown fragment for a crown clipped by the left edge. Their clicks
//! stay inside the guarded playfield and are merged by crown-centre anchors.
//!
//! The committed screenshots are **single calibration scenes**, not evidence of
//! generalisation. Candidate clicks must still pass selection-panel verification
//! before receiving `A`, so a body fragment that matches the wrong place fails
//! closed instead of upgrading something by accident.

use std::time::Instant;

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

/// Complementary fragment pass for a Spire crown clipped by the left client
/// edge: the crown's right 80 columns are still on screen, so the click stays at
/// the crown centre and only the search region is restricted to that edge.
///
/// The real left-edge Spire scores 0.85 here while terrain in the same strip
/// stays below 0.24; the 0.70/0.60 pair keeps that separation.
static LEFT_EDGE_PROFILE: BuildingProfile = BuildingProfile {
    template: LEFT_EDGE_TEMPLATE,
    template_w: LEFT_EDGE_TEMPLATE_W,
    template_h: TEMPLATE_H,
    // The crown centre expressed in the fragment's own coordinates, so the
    // click lands on the crown, not 16 px to its right.
    click_offset: Point::new(
        TEMPLATE_CLICK_OFFSET.x - LEFT_EDGE_COLUMN,
        TEMPLATE_CLICK_OFFSET.y,
    ),
    safe_viewport: Rect::new(
        crate::play_area::EDGE_GUARD,
        crate::play_area::EDGE_GUARD,
        128,
        746,
    ),
    min_ncc: 0.70,
    min_edge_agreement: 0.60,
    ..PROFILE
};

/// Complementary fragment pass for the Spires whose crowns are above the top
/// edge: only the crown's lowest rows and the upper body are on screen, so the
/// fragment is cut from a visible Spire body and its click lands on that body
/// (`y = 40`), never on the off-screen crown centre.
///
/// The five top-edge Spires score 0.83…0.98 here while terrain, resource-bar
/// artwork and the row-A crowns stay at or below 0.24; the click also stays
/// inside the top band (`y < 88`), which is what keeps the pass from re-finding
/// fully visible crowns.
static TOP_BAND_PROFILE: BuildingProfile = BuildingProfile {
    template: TOP_BAND_TEMPLATE,
    template_w: TOP_BAND_TEMPLATE_W,
    template_h: TOP_BAND_TEMPLATE_H,
    click_offset: Point::new(TOP_BAND_TEMPLATE_W / 2, TOP_BAND_CLICK_Y),
    safe_viewport: Rect::new(
        crate::play_area::EDGE_GUARD,
        crate::play_area::EDGE_GUARD,
        CLIENT_WIDTH - 2 * crate::play_area::EDGE_GUARD,
        64,
    ),
    min_ncc: 0.70,
    min_edge_agreement: 0.60,
    ..PROFILE
};

/// Fragment click minus the crown centre (the primary pass's anchor) for each
/// complementary pass. The left-edge fragment clicks the crown centre itself;
/// the top-band fragment clicks the body 62 px below the off-screen centre.
const TOP_BAND_OFFSET: Point = Point::new(0, 62);
const LEFT_EDGE_OFFSET: Point = Point::new(0, 0);

/// Two anchors closer than this are the same building when the complementary
/// passes are merged into the primary detections.
const ANCHOR_TOLERANCE: i32 = 24;

/// Left-edge fragment width: the crown columns that survive a left-edge clip.
const LEFT_EDGE_TEMPLATE_W: i32 = 80;
/// First crown column of the left-edge fragment (see the fixture README).
const LEFT_EDGE_COLUMN: i32 = 16;
/// The visible-body crop of this fixture's own Spire, free of the neighbouring
/// crown that overlaps below it (see the fixture README).
const TOP_BAND_CROP: Rect = Rect::new(794, 144, 64, 64);
const TOP_BAND_TEMPLATE_W: i32 = TOP_BAND_CROP.w;
const TOP_BAND_TEMPLATE_H: i32 = TOP_BAND_CROP.h;
/// Where inside the top-band fragment the click lands: the visible body.
const TOP_BAND_CLICK_Y: i32 = 40;

/// Left-edge crown fragment, derived byte-for-byte from the calibrated crown
/// template's columns [`LEFT_EDGE_COLUMN`..96).
const LEFT_EDGE_TEMPLATE: &[u8] =
    include_bytes!("../tests/fixtures/spire-clipped-1080/spire-left-crown-80x96.gray");
/// Top-band body fragment, a luma crop of this fixture's own visible Spire.
const TOP_BAND_TEMPLATE: &[u8] =
    include_bytes!("../tests/fixtures/spire-clipped-1080/spire-top-band-64x64.gray");

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

/// The embedded left-edge crown fragment pixels, exposed so tests can verify
/// that it really is a sub-crop of the calibrated crown template.
pub fn left_edge_template_pixels() -> &'static [u8] {
    LEFT_EDGE_TEMPLATE
}

/// The embedded top-band body fragment pixels (`TOP_BAND_TEMPLATE_W *
/// TOP_BAND_TEMPLATE_H`), exposed for the same provenance test.
pub fn top_band_template_pixels() -> &'static [u8] {
    TOP_BAND_TEMPLATE
}

/// Scans one frame for Spire crowns. The primary crown pass runs once, then the
/// two complementary fragment passes cover the crowns the client edge clips;
/// their detections are merged by crown-centre anchor so one building is never
/// reported twice.
pub fn detect_spires(frame: &Frame) -> SpireScan {
    let started = Instant::now();
    let primary = detect_buildings(frame, &PROFILE);
    if !primary.supported_profile {
        return primary;
    }
    let top_band = detect_buildings(frame, &TOP_BAND_PROFILE);
    let left_edge = detect_buildings(frame, &LEFT_EDGE_PROFILE);
    let evaluated = primary
        .evaluated
        .saturating_add(top_band.evaluated)
        .saturating_add(left_edge.evaluated);

    let mut detections = primary.detections;
    // Compare physical crown centres, not click points on different fragments.
    let mut anchors: Vec<Point> = detections.iter().map(|d| d.center).collect();
    for (scan, offset) in [(top_band, TOP_BAND_OFFSET), (left_edge, LEFT_EDGE_OFFSET)] {
        for candidate in scan.detections {
            let anchor = Point::new(candidate.center.x - offset.x, candidate.center.y - offset.y);
            if anchors.iter().any(|kept| {
                (kept.x - anchor.x).abs() <= ANCHOR_TOLERANCE
                    && (kept.y - anchor.y).abs() <= ANCHOR_TOLERANCE
            }) {
                continue;
            }
            anchors.push(anchor);
            detections.push(candidate);
        }
    }

    detections.sort_by(|a, b| {
        a.center
            .y
            .cmp(&b.center.y)
            .then(a.center.x.cmp(&b.center.x))
    });
    detections.truncate(MAX_DETECTIONS);

    SpireScan {
        detections,
        detect_ms: started.elapsed().as_millis(),
        evaluated,
        supported_profile: true,
    }
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

    fn clipped(name: &str) -> Frame {
        let path = Path::new("tests/fixtures/spire-clipped-1080").join(name);
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

    /// The two Spires in the dense fixture whose crowns are clipped by the top
    /// edge. Their crown centres are off-screen, so the top-band fragment clicks
    /// an opaque part of the visible body instead.
    const DENSE_TOP_CLIPPED: [Point; 2] = [Point::new(574, 74), Point::new(1006, 74)];

    #[test]
    fn the_dense_fixture_yields_every_visible_spire() {
        let path = PathBuf::from("tests/fixtures/spire-dense-1080/screen.png");
        let screen = Frame::from_png(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        let scan = detect_spires(&screen);
        assert!(scan.supported_profile);
        // The fifteen fully visible crowns plus the two whose crowns are above
        // the top edge, which the top-band fragment reaches on their bodies.
        assert_eq!(scan.count(), 17, "detections: {:?}", scan.detections);
        for expected in DENSE_EXPECTED {
            assert!(
                scan.detections
                    .iter()
                    .any(|d| distance(d.center, expected) <= 8),
                "expected spire near {expected:?} not found in detections: {:?}",
                scan.detections
            );
        }
        for expected in DENSE_TOP_CLIPPED {
            assert!(
                scan.detections
                    .iter()
                    .any(|d| distance(d.center, expected) <= 4),
                "expected top-clipped body click near {expected:?}; got {:?}",
                scan.detections
            );
        }
    }

    /// Every Spire the clipped-edge fixture shows, in the detector's `(y, x)`
    /// order. The first five are the Spires whose crowns are above `y = 0`
    /// (clicked on their visible bodies), `(34, 265)` is the Spire clipped by
    /// the left edge (clicked at its crown centre), and the rest are complete
    /// crowns the primary pass finds.
    #[test]
    fn the_clipped_fixture_yields_every_visible_spire() {
        let scan = detect_spires(&clipped("screen.png"));
        assert!(scan.supported_profile);
        let centers: Vec<Point> = scan.detections.iter().map(|d| d.center).collect();
        assert_eq!(
            centers,
            vec![
                Point::new(826, 40),
                Point::new(970, 40),
                Point::new(1114, 40),
                Point::new(1258, 40),
                Point::new(1402, 40),
                Point::new(826, 122),
                Point::new(1114, 122),
                Point::new(1258, 122),
                Point::new(1402, 122),
                Point::new(34, 265),
                Point::new(826, 265),
                Point::new(970, 266),
                Point::new(1258, 266),
                Point::new(1402, 266),
                Point::new(970, 409),
                Point::new(682, 410),
                Point::new(826, 410),
                Point::new(1114, 410),
                Point::new(1258, 410),
                Point::new(1114, 625),
                Point::new(538, 626),
                Point::new(1258, 626),
            ]
        );
        // Every click, including the five body clicks of the top-edge Spires,
        // has to be a playable cursor point.
        for detection in &scan.detections {
            assert!(point_in_viewport(detection.center), "{detection:?}");
        }
        assert!(
            scan.detections[..5]
                .iter()
                .all(|detection| detection.score > 0.8),
            "{:?}",
            &scan.detections[..5]
        );
    }

    #[test]
    fn the_top_band_fragment_never_matches_inside_the_resource_bar() {
        let paste = |x: i32| {
            let mut frame = Frame::blank(CLIENT_WIDTH as u32, CLIENT_HEIGHT as u32);
            let pixels = top_band_template_pixels();
            for j in 0..TOP_BAND_TEMPLATE_H {
                for i in 0..TOP_BAND_TEMPLATE_W {
                    let value = pixels[(j * TOP_BAND_TEMPLATE_W + i) as usize];
                    frame.set_pixel(x + i, j, crate::frame::Rgb::new(value, value, value));
                }
            }
            frame
        };
        // Left of the resource bar the same pixels are a valid body fragment...
        assert_eq!(detect_spires(&paste(1350)).count(), 1);
        // ...but inside the resource bar no click may be produced at all.
        assert_eq!(detect_spires(&paste(1460)).count(), 0);
    }

    #[test]
    fn the_clipped_fixture_assets_are_derived_from_the_calibrated_screenshots() {
        // The left-edge fragment is the crown template's right 80 columns.
        let crown = crown_template_pixels();
        let left = left_edge_template_pixels();
        assert_eq!(left.len(), (LEFT_EDGE_TEMPLATE_W * TEMPLATE_H) as usize);
        for j in 0..TEMPLATE_H {
            for i in 0..LEFT_EDGE_TEMPLATE_W {
                assert_eq!(
                    left[(j * LEFT_EDGE_TEMPLATE_W + i) as usize],
                    crown[(j * TEMPLATE_W + LEFT_EDGE_COLUMN + i) as usize],
                    "left-crown fragment pixel ({i}, {j})"
                );
            }
        }
        // The top-band fragment is a luma crop of this fixture's own Spire body.
        let source = clipped("screen.png");
        let top = top_band_template_pixels();
        assert_eq!(
            top.len(),
            (TOP_BAND_TEMPLATE_W * TOP_BAND_TEMPLATE_H) as usize
        );
        for j in 0..TOP_BAND_TEMPLATE_H {
            for i in 0..TOP_BAND_TEMPLATE_W {
                let px = source
                    .pixel(TOP_BAND_CROP.x + i, TOP_BAND_CROP.y + j)
                    .expect("pixel");
                assert_eq!(
                    top[(j * TOP_BAND_TEMPLATE_W + i) as usize],
                    luma(px.r, px.g, px.b),
                    "top-band fragment pixel ({i}, {j})"
                );
            }
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
