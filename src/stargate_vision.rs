//! Local Stargate detector: one capture, one full-screen search, no network.
//!
//! The Stargate instance of the shared [`crate::building_vision`] profile
//! detector. Complementary lower/upper hull templates handle clipping by the
//! screen and HUD. A smaller bottom-left fragment recovers top-edge gates when
//! even the lower-hull centre is clipped. Clicks remain inside the guarded
//! playfield and detections are merged using calibrated building anchors.
//!
//! The calibration screenshots are not evidence of generalisation. Candidate
//! clicks must still pass selection-panel verification before receiving `A`.

use std::time::Instant;

use crate::building_vision::{BuildingProfile, CLIENT_WIDTH};
use crate::frame::{Frame, Point, Rect};

pub use crate::building_vision::{BuildingDetection, BuildingScan, SelectionVerification};

/// Stargate detector/verifier profile instance.
pub static PROFILE: BuildingProfile = BuildingProfile {
    label: "Stargate",
    template: STARGATE_TEMPLATE,
    template_w: TEMPLATE_W,
    template_h: TEMPLATE_H,
    click_offset: TEMPLATE_CLICK_OFFSET,
    safe_viewport: SAFE_VIEWPORT,
    portrait_roi: PORTRAIT_ROI,
    portrait_mask: STARGATE_PORTRAIT_MASK,
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

/// Alternate world profile for the upper hull. This remains visible when a
/// bottom-row Stargate's lower hull is covered by the in-game HUD.
static UPPER_PROFILE: BuildingProfile = BuildingProfile {
    label: "Stargate",
    template: STARGATE_UPPER_TEMPLATE,
    template_w: UPPER_TEMPLATE_W,
    template_h: UPPER_TEMPLATE_H,
    click_offset: UPPER_TEMPLATE_CLICK_OFFSET,
    safe_viewport: SAFE_VIEWPORT,
    portrait_roi: PORTRAIT_ROI,
    portrait_mask: STARGATE_PORTRAIT_MASK,
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

/// Bottom-left fragment of the lower hull, still visible when its centre is
/// clipped at the top. Keep the whole fragment left of resource-bar artwork.
static TOP_EDGE_PROFILE: BuildingProfile = BuildingProfile {
    template: include_bytes!("../tests/fixtures/stargate-screen-1080/stargate-top-edge-40x32.gray"),
    template_w: 40,
    template_h: 32,
    click_offset: Point::new(20, 16),
    safe_viewport: Rect::new(24, 24, CLIENT_WIDTH - 48, 80),
    min_ncc: 0.75,
    min_edge_agreement: 0.50,
    ..PROFILE
};
/// Fragment click minus the original lower-hull centre.
const TOP_EDGE_OFFSET: Point = Point::new(-12, 16);

/// Right-hand lower-hull fin survives when the left edge clips the normal core.
/// Search only near that edge; never move the cursor into the scroll zone.
static LEFT_EDGE_PROFILE: BuildingProfile = BuildingProfile {
    template: include_bytes!(
        "../tests/fixtures/stargate-screen-1080/stargate-left-edge-48x64.gray"
    ),
    template_w: 48,
    template_h: 64,
    click_offset: Point::new(24, 32),
    safe_viewport: Rect::new(24, 24, 128, 746),
    min_edge_agreement: 0.50,
    ..PROFILE
};
const LEFT_EDGE_OFFSET: Point = Point::new(64, -8);

/// One confirmed Stargate.
pub type StargateDetection = BuildingDetection;
/// Result of one full-screen Stargate scan.
pub type StargateScan = BuildingScan;

/// Search region: the guarded top edge through the world area above the HUD.
/// The resource display itself is excluded by the normal detector/profile
/// gates and a detected click still requires Stargate portrait verification.
pub const SAFE_VIEWPORT: Rect = Rect::new(
    crate::play_area::EDGE_GUARD,
    crate::play_area::EDGE_GUARD,
    CLIENT_WIDTH - 2 * crate::play_area::EDGE_GUARD,
    770 - crate::play_area::EDGE_GUARD,
);
/// Hull template size in pixels.
pub const TEMPLATE_W: i32 = 64;
/// Hull template size in pixels.
pub const TEMPLATE_H: i32 = 64;
/// Where inside the template the click lands: the visible lower-hull centre.
pub const TEMPLATE_CLICK_OFFSET: Point = Point::new(32, 32);
/// Upper-hull fragment used when the lower hull is hidden by the HUD.
const UPPER_TEMPLATE_W: i32 = 64;
const UPPER_TEMPLATE_H: i32 = 64;
// Click the upper portion of this matched hull, not its centre: the latter
// can be below the conservative command-card skyline even when the hull is visible.
const UPPER_TEMPLATE_CLICK_OFFSET: Point = Point::new(32, 16);

/// Screen rectangle of the single-unit information-panel portrait (both hulls),
/// used to verify that a click really selected a Stargate. It stops left of the
/// unit-name text and above the HP line.
pub const PORTRAIT_ROI: Rect = Rect::new(608, 874, 160, 140);

/// Grayscale lower-hull core, corresponding to `screen.png` at `(736, 388)`.
const STARGATE_TEMPLATE: &[u8] =
    include_bytes!("../tests/fixtures/stargate-screen-1080/stargate-template-64x64.gray");
/// Grayscale upper-hull core from `screen.png` at `(672, 288)`.
const STARGATE_UPPER_TEMPLATE: &[u8] =
    include_bytes!("../tests/fixtures/stargate-screen-1080/stargate-upper-template-64x64.gray",);
/// Portrait silhouette mask (`255` = sprite pixel), cropped from `screen.png`
/// at `(608, 874)` and thresholded with `max(r, g, b) > 32`.
const STARGATE_PORTRAIT_MASK: &[u8] =
    include_bytes!("../tests/fixtures/stargate-screen-1080/stargate-portrait-160x140.mask");

/// Score above which a full-resolution candidate is reported.
// Real hull cores score >= 0.90 in the captured scenes; Gateway lookalikes
// score around 0.48. Do not compensate for clipping by accepting weak matches.
const MIN_NCC: f32 = 0.75;
/// Minimum frame window contrast (0..255 luma stddev) to accept a match.
const MIN_FRAME_STDDEV: f32 = 9.0;
/// Minimum fraction of template edge pixels that coincide with screen edges.
const MIN_EDGE_AGREEMENT: f32 = 0.35;
/// Coarse stage accept threshold.
const COARSE_MIN_NCC: f32 = 0.18;
const MID_MIN_NCC: f32 = 0.28;
/// Upper bound on reported Stargates in one scene.
const MAX_DETECTIONS: usize = 32;
/// Two detections closer than this are the same building.
const NMS_RADIUS: i32 = 84;
/// Expected lower-centre minus upper-centre offset for one Stargate.
const HULL_OFFSET: Point = Point::new(64, 116);
const HULL_OFFSET_TOLERANCE: i32 = 28;
/// A portrait pixel exists when any channel is above this; the panel is black.
const PORTRAIT_CHANNEL_MIN: u8 = 32;
/// Selection verification thresholds (shape overlap with the real portrait).
///
/// The real panel and its ±2 px shifts score >= 0.93 coverage / >= 0.87 IoU,
/// while the Spire/Colony/Drone panels of the existing fixtures stay below 0.61
/// IoU, so 0.85/0.80 accepts the Stargate and rejects the other portraits.
const VERIFY_COVERAGE_MIN: f32 = 0.85;
const VERIFY_IOU_MIN: f32 = 0.80;

/// True when `frame` is the supported 1920×1080 client at `(0, 0)`.
pub fn supported_profile(frame: &Frame) -> bool {
    crate::building_vision::supported_client(frame)
}

/// Template rectangle for a detection whose lower-hull centre is `center`.
pub fn template_rect(center: Point) -> Rect {
    crate::building_vision::template_rect(center, &PROFILE)
}

/// The embedded lower-hull grayscale pixels (`TEMPLATE_W * TEMPLATE_H`),
/// exposed so tests can paint a synthetic Stargate without the file system.
pub fn stargate_template_pixels() -> &'static [u8] {
    STARGATE_TEMPLATE
}

/// The embedded portrait mask pixels (`PORTRAIT_ROI.w * PORTRAIT_ROI.h`).
pub fn portrait_mask_pixels() -> &'static [u8] {
    STARGATE_PORTRAIT_MASK
}

/// Scans one captured frame with complementary hull fragments, merging by
/// calibrated building anchors. The small top-edge fragment runs only near
/// that edge; the upper hull recovers gates hidden by the bottom HUD.
pub fn detect_stargates(frame: &Frame) -> StargateScan {
    let started = Instant::now();
    let lower = crate::building_vision::detect_buildings(frame, &PROFILE);
    if !lower.supported_profile {
        return lower;
    }
    let upper = crate::building_vision::detect_buildings(frame, &UPPER_PROFILE);
    let top = crate::building_vision::detect_buildings(frame, &TOP_EDGE_PROFILE);
    let left = crate::building_vision::detect_buildings(frame, &LEFT_EDGE_PROFILE);
    let evaluated = lower
        .evaluated
        .saturating_add(upper.evaluated)
        .saturating_add(top.evaluated)
        .saturating_add(left.evaluated);
    let mut detections = lower.detections;
    // Compare physical building anchors, not click positions on different hulls.
    // Prefer the existing lower-hull click, then upper, then the top-edge fragment.
    let mut anchors: Vec<Point> = detections.iter().map(|d| d.center).collect();
    for (scan, offset) in [
        (upper, Point::new(-HULL_OFFSET.x, -HULL_OFFSET.y)),
        (top, TOP_EDGE_OFFSET),
        (left, LEFT_EDGE_OFFSET),
    ] {
        for candidate in scan.detections {
            let anchor = Point::new(candidate.center.x - offset.x, candidate.center.y - offset.y);
            if anchors.iter().any(|kept| {
                (kept.x - anchor.x).abs() <= HULL_OFFSET_TOLERANCE
                    && (kept.y - anchor.y).abs() <= HULL_OFFSET_TOLERANCE
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
    StargateScan {
        detections,
        detect_ms: started.elapsed().as_millis(),
        evaluated,
        supported_profile: true,
    }
}

/// Scores a captured selection-panel ROI against the reference Stargate portrait.
pub fn verify_stargate_selection(roi: &Frame) -> SelectionVerification {
    crate::building_vision::verify_building_selection(roi, &PROFILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn blank_and_unrelated_unit_portraits_are_not_stargates() {
        assert_eq!(detect_stargates(&Frame::blank(1920, 1080)).count(), 0);
        for name in ["drone-single.png", "drones-5.png", "not-drone-colony.png"] {
            let full = crate::vision::synthetic::selection_fixture(name);
            let crop = full.crop(crate::vision::SELECTION_ROI).unwrap();
            let world = crate::vision::synthetic::blit(&crop, 400, 240);
            assert_eq!(detect_stargates(&world).count(), 0, "{name}");
        }
    }

    #[test]
    fn resource_bar_artwork_cannot_supply_a_template_match() {
        for top in [0, 40] {
            let mut frame = Frame::blank(1920, 1080);
            for y in 0..TEMPLATE_H {
                for x in 0..TEMPLATE_W {
                    let v = STARGATE_TEMPLATE[(y * TEMPLATE_W + x) as usize];
                    frame.set_pixel(1500 + x, top + y, crate::frame::Rgb::new(v, v, v));
                }
            }
            for found in detect_stargates(&frame).detections {
                let bounds = template_rect(found.center);
                assert!(bounds.y >= 48 || bounds.right() <= 1440, "{found:?}");
            }
        }
    }

    #[test]
    fn core_at_the_guarded_left_edge_remains_clickable() {
        let mut frame = Frame::blank(1920, 1080);
        let left = SAFE_VIEWPORT.x;
        let top = 240;
        for y in 0..TEMPLATE_H {
            for x in 0..TEMPLATE_W {
                let v = STARGATE_TEMPLATE[(y * TEMPLATE_W + x) as usize];
                frame.set_pixel(left + x, top + y, crate::frame::Rgb::new(v, v, v));
            }
        }

        let scan = detect_stargates(&frame);
        assert!(
            scan.detections
                .iter()
                .any(|found| found.center == Point::new(left + 32, top + 32)),
            "{:?}",
            scan.detections
        );
    }

    #[test]
    fn aggressive_core_finds_all_fixture_stargates_without_duplicates() {
        let frame = Frame::from_png(Path::new("tests/fixtures/stargate-screen-1080/screen.png"))
            .expect("fixture");
        let scan = detect_stargates(&frame);
        let mut centers: Vec<Point> = scan
            .detections
            .iter()
            .map(|detection| detection.center)
            .collect();
        centers.sort_by_key(|center| (center.y, center.x));
        assert_eq!(
            centers,
            vec![
                Point::new(1128, 132),
                Point::new(768, 420),
                Point::new(1056, 420),
                Point::new(1488, 420),
                Point::new(912, 636),
                Point::new(1200, 636),
                Point::new(1488, 636),
            ]
        );
    }

    #[test]
    fn upper_hull_recovers_the_three_gates_hidden_by_the_bottom_hud() {
        let frame = Frame::from_png(Path::new(
            "tests/fixtures/stargate-bottom-hud-1080/screen.png",
        ))
        .expect("fixture");
        let scan = detect_stargates(&frame);
        let centers: Vec<Point> = scan
            .detections
            .iter()
            .map(|detection| detection.center)
            .collect();
        assert_eq!(
            centers,
            vec![
                Point::new(1128, 278),
                Point::new(624, 566),
                Point::new(912, 566),
                Point::new(1344, 566),
                Point::new(704, 666),
                Point::new(992, 666),
                Point::new(1280, 666),
            ]
        );
    }

    #[test]
    fn eleven_gate_scene_includes_both_top_clipped_gates() {
        let frame = Frame::from_png(Path::new("tests/fixtures/stargate-eleven-1080/screen.png"))
            .expect("fixture");
        let scan = detect_stargates(&frame);
        let centers: Vec<_> = scan.detections.iter().map(|d| d.center).collect();
        assert_eq!(
            centers,
            vec![
                Point::new(1121, 40),
                Point::new(1409, 40),
                Point::new(484, 312),
                Point::new(1493, 312),
                Point::new(52, 528),
                Point::new(989, 600),
                Point::new(1277, 600),
                Point::new(1709, 600),
                Point::new(1069, 700),
                Point::new(1357, 700),
                Point::new(1645, 700),
            ]
        );
        assert!(
            centers
                .iter()
                .all(|&p| crate::play_area::is_playable_point(p))
        );
        assert!(scan.detections[..2].iter().all(|d| d.score > 0.95));
    }

    #[test]
    fn top_fragment_does_not_duplicate_a_complete_lower_hull() {
        let mut frame = Frame::blank(1920, 1080);
        for y in 0..TEMPLATE_H {
            for x in 0..TEMPLATE_W {
                let v = STARGATE_TEMPLATE[(y * TEMPLATE_W + x) as usize];
                frame.set_pixel(868 + x, 32 + y, crate::frame::Rgb::new(v, v, v));
            }
        }
        // Prove that both passes see the same gate before checking the merge.
        assert_eq!(
            crate::building_vision::detect_buildings(&frame, &TOP_EDGE_PROFILE).count(),
            1
        );
        let scan = detect_stargates(&frame);
        assert_eq!(scan.count(), 1, "{:?}", scan.detections);
        assert_eq!(scan.detections[0].center, Point::new(900, 64));
    }

    #[test]
    fn top_fragment_never_matches_inside_resource_bar() {
        let mut frame = Frame::blank(1920, 1080);
        for y in 0..TOP_EDGE_PROFILE.template_h {
            for x in 0..TOP_EDGE_PROFILE.template_w {
                let v = TOP_EDGE_PROFILE.template[(y * 40 + x) as usize];
                frame.set_pixel(1472 + x, 16 + y, crate::frame::Rgb::new(v, v, v));
            }
        }
        assert_eq!(detect_stargates(&frame).count(), 0);
    }

    #[test]
    fn left_clipped_stargate_is_found_without_selecting_any_gateway() {
        let frame = Frame::from_png(Path::new("tests/fixtures/stargate-gateway-1080/screen.png"))
            .expect("fixture");
        let scan = detect_stargates(&frame);
        let centers: Vec<_> = scan.detections.iter().map(|d| d.center).collect();
        assert_eq!(
            centers,
            vec![
                Point::new(1085, 40),
                Point::new(1373, 40),
                Point::new(448, 312),
                Point::new(1457, 312),
                Point::new(80, 520),
                Point::new(953, 600),
                Point::new(1241, 600),
                Point::new(1673, 600),
                Point::new(1033, 700),
                Point::new(1321, 700),
                Point::new(1609, 700),
            ]
        );
        assert!(
            centers
                .iter()
                .all(|&p| crate::play_area::is_playable_point(p))
        );
        assert!(scan.detections.iter().all(|d| d.score >= MIN_NCC));
        for gateway in [
            Rect::new(288, 360, 304, 264),
            Rect::new(576, 144, 304, 264),
            Rect::new(1008, 72, 304, 264),
        ] {
            assert!(!centers.iter().any(|p| p.x >= gateway.x
                && p.x < gateway.right()
                && p.y >= gateway.y
                && p.y < gateway.bottom()));
        }
    }

    #[test]
    fn gateway_crops_remain_negative_at_different_search_locations() {
        let frame = Frame::from_png(Path::new("tests/fixtures/stargate-gateway-1080/screen.png"))
            .expect("fixture");
        for (bounds, dest) in [
            (Rect::new(292, 372, 284, 240), Point::new(24, 180)),
            (Rect::new(580, 156, 284, 240), Point::new(768, 240)),
            (Rect::new(1012, 84, 284, 240), Point::new(1100, 24)),
        ] {
            let crop = frame.crop(bounds).expect("Gateway crop");
            let world = crate::vision::synthetic::blit(&crop, dest.x, dest.y);
            let scan = detect_stargates(&world);
            assert_eq!(
                scan.count(),
                0,
                "{bounds:?} at {dest:?}: {:?}",
                scan.detections
            );
        }
    }

    #[test]
    fn left_fragment_merges_with_a_visible_lower_hull() {
        let source = Frame::from_png(Path::new("tests/fixtures/stargate-screen-1080/screen.png"))
            .expect("fixture");
        let crop = source.crop(Rect::new(720, 356, 176, 144)).unwrap();
        // Lower-hull centre (80, 400); fin click (144, 392): both safely visible.
        let frame = crate::vision::synthetic::blit(&crop, 32, 336);
        assert_eq!(
            crate::building_vision::detect_buildings(&frame, &LEFT_EDGE_PROFILE).count(),
            1
        );
        let scan = detect_stargates(&frame);
        assert_eq!(scan.count(), 1, "{:?}", scan.detections);
        assert_eq!(scan.detections[0].center, Point::new(80, 400));
    }
}
