//! Local Stargate detector: one capture, one full-screen search, no network.
//!
//! The Stargate instance of the shared [`crate::building_vision`] profile
//! detector. The template is one Stargate's upper hull; the click offset lands
//! on that hull. A Stargate sprite is two separated hulls, so the search
//! viewport deliberately starts below the top resource bar
//! ([`SAFE_VIEWPORT`]): the calibration scene has a partially clipped Stargate
//! at the top edge (`y ≈ -42..214`) whose lower hull must never be clicked, and
//! every fully visible gate centre sits at `y >= 328`.
//!
//! The committed screenshot is a **single calibration scene**, not evidence of
//! generalisation. Thresholds were chosen so the six real Stargates pass and
//! the recorded distractors (clipped gate, Pylon, minerals, Probe, HUD) do not.

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

/// Search region: below the top bar so a top-clipped Stargate cannot be
/// clicked, above the bottom HUD like the Spire profile.
pub const SAFE_VIEWPORT: Rect = Rect::new(
    crate::play_area::EDGE_GUARD,
    230,
    CLIENT_WIDTH - 2 * crate::play_area::EDGE_GUARD,
    770 - 230,
);
/// Hull template size in pixels.
pub const TEMPLATE_W: i32 = 128;
/// Hull template size in pixels.
pub const TEMPLATE_H: i32 = 128;
/// Where inside the template the click lands: the upper-hull centre.
pub const TEMPLATE_CLICK_OFFSET: Point = Point::new(64, 64);

/// Screen rectangle of the single-unit information-panel portrait (both hulls),
/// used to verify that a click really selected a Stargate. It stops left of the
/// unit-name text and above the HP line.
pub const PORTRAIT_ROI: Rect = Rect::new(608, 874, 160, 140);

/// Grayscale upper-hull template, cropped from `screen.png` at `(648, 264)`.
const STARGATE_TEMPLATE: &[u8] =
    include_bytes!("../tests/fixtures/stargate-screen-1080/stargate-template-128x128.gray");
/// Portrait silhouette mask (`255` = sprite pixel), cropped from `screen.png`
/// at `(608, 874)` and thresholded with `max(r, g, b) > 32`.
const STARGATE_PORTRAIT_MASK: &[u8] =
    include_bytes!("../tests/fixtures/stargate-screen-1080/stargate-portrait-160x140.mask");

/// Score above which a full-resolution candidate is reported.
const MIN_NCC: f32 = 0.55;
/// Minimum frame window contrast (0..255 luma stddev) to accept a match.
const MIN_FRAME_STDDEV: f32 = 12.0;
/// Minimum fraction of template edge pixels that coincide with screen edges.
const MIN_EDGE_AGREEMENT: f32 = 0.5;
/// Coarse stage accept threshold.
const COARSE_MIN_NCC: f32 = 0.25;
const MID_MIN_NCC: f32 = 0.4;
/// Upper bound on reported Stargates in one scene.
const MAX_DETECTIONS: usize = 32;
/// Two detections closer than this are the same building.
const NMS_RADIUS: i32 = 48;
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

/// Template rectangle for a detection whose upper-hull centre is `center`.
pub fn template_rect(center: Point) -> Rect {
    crate::building_vision::template_rect(center, &PROFILE)
}

/// The embedded hull grayscale pixels (`TEMPLATE_W * TEMPLATE_H`), exposed so
/// tests can paint a synthetic Stargate without touching the file system.
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
