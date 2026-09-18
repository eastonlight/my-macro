//! Local Stargate detector: one capture, one full-screen search, no network.
//!
//! The Stargate instance of the shared [`crate::building_vision`] profile
//! detector. The template is a very compact Stargate lower-hull core. A gate
//! can remain a candidate even when more than half of its outer sprite is clipped
//! at a screen edge, as long as this distinctive central fragment is still visible.
//! The thresholds intentionally favor recall; selection-panel verification still
//! prevents `A` from being sent when an aggressive candidate click does not select
//! a Stargate.
//!
//! The committed screenshot is a **single calibration scene**, not evidence of
//! generalisation. The more permissive profile may click additional candidates,
//! but only a verified Stargate selection is allowed to receive `A`.

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

/// Screen rectangle of the single-unit information-panel portrait (both hulls),
/// used to verify that a click really selected a Stargate. It stops left of the
/// unit-name text and above the HP line.
pub const PORTRAIT_ROI: Rect = Rect::new(608, 874, 160, 140);

/// Grayscale lower-hull core, corresponding to `screen.png` at `(736, 388)`.
const STARGATE_TEMPLATE: &[u8] =
    include_bytes!("../tests/fixtures/stargate-screen-1080/stargate-template-64x64.gray");
/// Portrait silhouette mask (`255` = sprite pixel), cropped from `screen.png`
/// at `(608, 874)` and thresholded with `max(r, g, b) > 32`.
const STARGATE_PORTRAIT_MASK: &[u8] =
    include_bytes!("../tests/fixtures/stargate-screen-1080/stargate-portrait-160x140.mask");

/// Score above which a full-resolution candidate is reported.
const MIN_NCC: f32 = 0.46;
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

/// Scans one frame for Stargates. This is the single expensive pass.
pub fn detect_stargates(frame: &Frame) -> StargateScan {
    crate::building_vision::detect_buildings(frame, &PROFILE)
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
}
