//! Local Spire detector: one capture, one full-screen search, no network.
//!
//! The single supported profile is the 1920×1080 Remastered client at screen
//! origin `(0, 0)` — the same contract as [`crate::vision`]. Everything here is
//! pure and host testable: it reads a [`Frame`] and returns positions.
//!
//! Matching is *shape* matching, not a colour blob:
//!
//! * a small grayscale crown template is extracted from the calibration
//!   screenshot `tests/fixtures/spire-screen-1080/screen.png` and embedded with
//!   `include_bytes!` (see that fixture's README for provenance);
//! * a coarse-to-fine normalised cross correlation finds candidate positions;
//! * a candidate survives only when the window has real contrast
//!   (frame stddev), the correlation is high, and the template's edges coincide
//!   with real screen edges (edge agreement) — flat terrain, HUD borders and
//!   colour blobs cannot pass all three;
//! * detections are deduplicated by non-maximum suppression.
//!
//! The committed screenshot is a **single calibration scene**, not evidence of
//! generalisation. Thresholds were chosen so all four real crowns pass and the
//! recorded distractors do not.

use std::time::Instant;

use crate::frame::{Frame, Point, Rect};

/// Client size of the only supported profile.
pub const CLIENT_WIDTH: i32 = 1920;
/// Client height of the only supported profile.
pub const CLIENT_HEIGHT: i32 = 1080;

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
const MAX_COARSE_CANDIDATES: usize = 128;
const MAX_MID_CANDIDATES: usize = 96;
/// Upper bound on reported Spires in one scene.
const MAX_DETECTIONS: usize = 32;
/// Two detections closer than this are the same building.
const NMS_RADIUS: i32 = 48;
/// Sobel magnitude (0..255 scale) that counts as an edge.
const EDGE_MIN_TEMPLATE: i32 = 48;
const EDGE_MIN_FRAME: i32 = 36;
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

/// One confirmed Spire crown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpireDetection {
    /// Crown centre; the point the action clicks.
    pub center: Point,
    /// Zero-mean normalised cross correlation of the crown template (`~1.0` is
    /// a perfect match).
    pub score: f32,
    /// Fraction of template edge pixels matched on screen.
    pub edge_agreement: f32,
    /// Contrast of the matched window; flat areas are rejected on this.
    pub frame_stddev: f32,
}

/// Result of one full-screen scan.
#[derive(Clone, Debug, PartialEq)]
pub struct SpireScan {
    pub detections: Vec<SpireDetection>,
    /// Wall time of the full detection pass.
    pub detect_ms: u128,
    /// Full-resolution positions the last stage scored.
    pub evaluated: usize,
    /// False when the frame is not the supported 1920×1080 profile; no
    /// detection is attempted then.
    pub supported_profile: bool,
}

impl SpireScan {
    pub fn count(&self) -> usize {
        self.detections.len()
    }
}

/// Shape overlap of a captured portrait ROI with the reference Spire portrait.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionVerification {
    /// Fraction of reference portrait pixels also present in the capture.
    pub coverage: f32,
    /// Intersection over union of the two silhouettes.
    pub iou: f32,
    /// True when the capture really shows the Spire portrait.
    pub accepted: bool,
}

/// True when `frame` is the supported 1920×1080 client.
pub fn supported_profile(frame: &Frame) -> bool {
    frame.width() as i32 == CLIENT_WIDTH && frame.height() as i32 == CLIENT_HEIGHT
}

/// Template rectangle for a detection whose crown centre is `center`.
pub fn template_rect(center: Point) -> Rect {
    Rect::new(
        center.x - TEMPLATE_CLICK_OFFSET.x,
        center.y - TEMPLATE_CLICK_OFFSET.y,
        TEMPLATE_W,
        TEMPLATE_H,
    )
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
    let started = Instant::now();
    if !supported_profile(frame) {
        return SpireScan {
            detections: Vec::new(),
            detect_ms: started.elapsed().as_millis(),
            evaluated: 0,
            supported_profile: false,
        };
    }
    let (w, h) = (frame.width() as i32, frame.height() as i32);
    let gray = gray_image(frame);

    let (g2, w2, h2) = downsample(&gray, w, h, 2);
    let (g4, w4, h4) = downsample(&g2, w2, h2, 2);
    let (t2, tw2, th2) = downsample(CROWN_TEMPLATE, TEMPLATE_W, TEMPLATE_H, 2);
    let (t4, tw4, th4) = downsample(&t2, tw2, th2, 2);

    let t_stats = TemplateStats::new(CROWN_TEMPLATE, TEMPLATE_W, TEMPLATE_H);
    let t2_stats = TemplateStats::new(&t2, tw2, th2);
    let t4_stats = TemplateStats::new(&t4, tw4, th4);

    // Coarse stage: every 4th pixel, cheap 24×24 correlation inside safe viewport.
    let min_cx = ((SAFE_VIEWPORT.x - TEMPLATE_CLICK_OFFSET.x).max(0) + 3) / 4;
    let max_cx =
        ((SAFE_VIEWPORT.x + SAFE_VIEWPORT.w - TEMPLATE_CLICK_OFFSET.x).min(w - TEMPLATE_W) - 1) / 4;
    let min_cy = ((SAFE_VIEWPORT.y - TEMPLATE_CLICK_OFFSET.y).max(0) + 3) / 4;
    let max_cy =
        ((SAFE_VIEWPORT.y + SAFE_VIEWPORT.h - TEMPLATE_CLICK_OFFSET.y).min(h - TEMPLATE_H) - 1) / 4;

    let mut coarse_raw: Vec<(i32, i32, f32)> = Vec::new();
    for cy in min_cy..=max_cy {
        for cx in min_cx..=max_cx {
            let tx = cx * 4;
            let ty = cy * 4;
            let center = Point::new(tx + TEMPLATE_CLICK_OFFSET.x, ty + TEMPLATE_CLICK_OFFSET.y);
            if !point_in_viewport(center) {
                continue;
            }
            if let Some((ncc, _)) = window_ncc(&g4, w4, h4, cx, cy, &t4, tw4, th4, &t4_stats, 4.0)
                && ncc >= COARSE_MIN_NCC
            {
                coarse_raw.push((tx, ty, ncc));
            }
        }
    }
    coarse_raw.sort_by(|a, b| b.2.total_cmp(&a.2));

    // Diverse coarse candidate retention: suppress immediate neighbors (within 16px)
    // so redundant points from one crown do not displace other crowns.
    let mut coarse: Vec<(i32, i32, f32)> = Vec::new();
    for (tx, ty, score) in coarse_raw {
        if coarse
            .iter()
            .any(|(cx, cy, _)| (cx - tx).abs() <= 16 && (cy - ty).abs() <= 16)
        {
            continue;
        }
        coarse.push((tx, ty, score));
        if coarse.len() >= MAX_COARSE_CANDIDATES {
            break;
        }
    }

    // Mid stage: half resolution, ±4 px around each coarse candidate.
    let mut mid_raw: Vec<(i32, i32, f32)> = Vec::new();
    for (tx, ty, _) in &coarse {
        for dy in [-4, -2, 0, 2, 4] {
            for dx in [-4, -2, 0, 2, 4] {
                let (mx, my) = (tx + dx, ty + dy);
                if mx < 0 || my < 0 || mx + TEMPLATE_W > w || my + TEMPLATE_H > h {
                    continue;
                }
                if let Some((ncc, _)) =
                    window_ncc(&g2, w2, h2, mx >> 1, my >> 1, &t2, tw2, th2, &t2_stats, 8.0)
                    && ncc >= MID_MIN_NCC
                {
                    mid_raw.push((mx, my, ncc));
                }
            }
        }
    }
    mid_raw.sort_by(|a, b| b.2.total_cmp(&a.2));

    // Diverse mid candidate retention: keep local maxima with a radius of 24px.
    // This prevents clustered sub-pixel variants from choking out distinct crowns.
    let mut mid: Vec<(i32, i32, f32)> = Vec::new();
    for (mx, my, score) in mid_raw {
        if mid
            .iter()
            .any(|(kx, ky, _)| (kx - mx).abs() <= 24 && (ky - my).abs() <= 24)
        {
            continue;
        }
        mid.push((mx, my, score));
        if mid.len() >= MAX_MID_CANDIDATES {
            break;
        }
    }

    // Full stage: exact pixels, with the edge and contrast gates.
    let grad = gradient_magnitude(&gray, w, h);
    let edge_mask = template_edge_mask();
    let mut full: Vec<(i32, i32, f32, f32, f32)> = Vec::new();
    let mut evaluated = 0;
    for (tx, ty, _) in &mid {
        for dy in -3..=3 {
            for dx in -3..=3 {
                let (fx, fy) = (tx + dx, ty + dy);
                if fx < 0 || fy < 0 || fx + TEMPLATE_W > w || fy + TEMPLATE_H > h {
                    continue;
                }
                let center = Point::new(fx + TEMPLATE_CLICK_OFFSET.x, fy + TEMPLATE_CLICK_OFFSET.y);
                if !point_in_viewport(center) {
                    continue;
                }
                evaluated += 1;
                let Some((ncc, stddev)) = window_ncc(
                    &gray,
                    w,
                    h,
                    fx,
                    fy,
                    CROWN_TEMPLATE,
                    TEMPLATE_W,
                    TEMPLATE_H,
                    &t_stats,
                    MIN_FRAME_STDDEV,
                ) else {
                    continue;
                };
                if ncc < MIN_NCC {
                    continue;
                }
                let agreement = edge_agreement(&grad, w, fx, fy, &edge_mask);
                if agreement < MIN_EDGE_AGREEMENT {
                    continue;
                }
                full.push((fx, fy, ncc.max(0.0), agreement, stddev));
            }
        }
    }
    full.sort_by(|a, b| b.2.total_cmp(&a.2));

    // Non-maximum suppression: keep the best score per cluster.
    let mut detections: Vec<SpireDetection> = Vec::new();
    for (tx, ty, score, agreement, stddev) in full {
        let center = Point::new(tx + TEMPLATE_CLICK_OFFSET.x, ty + TEMPLATE_CLICK_OFFSET.y);
        if detections.iter().any(|kept| {
            (kept.center.x - center.x).abs() <= NMS_RADIUS
                && (kept.center.y - center.y).abs() <= NMS_RADIUS
        }) {
            continue;
        }
        detections.push(SpireDetection {
            center,
            score,
            edge_agreement: agreement,
            frame_stddev: stddev,
        });
        if detections.len() >= MAX_DETECTIONS {
            break;
        }
    }
    detections.sort_by(|a, b| {
        a.center
            .y
            .cmp(&b.center.y)
            .then(a.center.x.cmp(&b.center.x))
    });

    SpireScan {
        detections,
        detect_ms: started.elapsed().as_millis(),
        evaluated,
        supported_profile: true,
    }
}

/// Scores a captured selection-panel ROI against the reference Spire portrait.
///
/// The comparison is on the silhouette (any channel above the black panel), so
/// it is independent of the player's team colour and of the HP text.
pub fn verify_spire_selection(roi: &Frame) -> SelectionVerification {
    let (rw, rh) = (PORTRAIT_ROI.w, PORTRAIT_ROI.h);
    if roi.width() as i32 != rw || roi.height() as i32 != rh {
        return SelectionVerification {
            coverage: 0.0,
            iou: 0.0,
            accepted: false,
        };
    }
    let mut reference = 0usize;
    let mut capture = 0usize;
    let mut intersection = 0usize;
    for y in 0..rh {
        for x in 0..rw {
            let index = (y * rw + x) as usize;
            let wants = SPIRE_PORTRAIT_MASK[index] != 0;
            let has = match roi.pixel(x, y) {
                Some(px) => px.r.max(px.g).max(px.b) > PORTRAIT_CHANNEL_MIN,
                None => false,
            };
            if wants {
                reference += 1;
            }
            if has {
                capture += 1;
            }
            if wants && has {
                intersection += 1;
            }
        }
    }
    if reference == 0 {
        return SelectionVerification {
            coverage: 0.0,
            iou: 0.0,
            accepted: false,
        };
    }
    let coverage = intersection as f32 / reference as f32;
    let union = reference + capture - intersection;
    let iou = if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    };
    SelectionVerification {
        coverage,
        iou,
        accepted: coverage >= VERIFY_COVERAGE_MIN && iou >= VERIFY_IOU_MIN,
    }
}

pub fn point_in_viewport(point: Point) -> bool {
    crate::play_area::is_playable_point(point)
}

fn gray_image(frame: &Frame) -> Vec<u8> {
    let (w, h) = (frame.width() as i32, frame.height() as i32);
    let mut out = vec![0u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            if let Some(px) = frame.pixel(x, y) {
                out[(y * w + x) as usize] = luma(px.r, px.g, px.b);
            }
        }
    }
    out
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    (((u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000) & 0xff) as u8
}

/// Halves each dimension, averaging each 2×2 block.
fn downsample(src: &[u8], w: i32, h: i32, factor: i32) -> (Vec<u8>, i32, i32) {
    let w2 = w / factor;
    let h2 = h / factor;
    let mut out = vec![0u8; (w2 * h2) as usize];
    for y in 0..h2 {
        for x in 0..w2 {
            let mut sum = 0u32;
            for dy in 0..factor {
                for dx in 0..factor {
                    let sx = x * factor + dx;
                    let sy = y * factor + dy;
                    sum += u32::from(src[(sy * w + sx) as usize]);
                }
            }
            out[(y * w2 + x) as usize] = (sum / (factor * factor) as u32) as u8;
        }
    }
    (out, w2, h2)
}

struct TemplateStats {
    mean: f64,
    ss: f64,
}

impl TemplateStats {
    fn new(template: &[u8], w: i32, h: i32) -> Self {
        let n = (w * h) as f64;
        let sum: f64 = template.iter().map(|v| f64::from(*v)).sum();
        let mean = sum / n;
        let ss: f64 = template
            .iter()
            .map(|v| {
                let d = f64::from(*v) - mean;
                d * d
            })
            .sum();
        Self { mean, ss }
    }
}

/// Zero-mean normalised cross correlation of `template` at `(tx, ty)`.
///
/// Returns `None` when the frame window is flatter than `min_stddev`.
#[allow(clippy::too_many_arguments)]
fn window_ncc(
    gray: &[u8],
    w: i32,
    h: i32,
    tx: i32,
    ty: i32,
    template: &[u8],
    tw: i32,
    th: i32,
    stats: &TemplateStats,
    min_stddev: f32,
) -> Option<(f32, f32)> {
    if tx < 0 || ty < 0 || tx + tw > w || ty + th > h {
        return None;
    }
    let n = f64::from(tw * th);
    let mut sum_f = 0f64;
    let mut sum_f2 = 0f64;
    let mut sum_tf = 0f64;
    for j in 0..th {
        let row = ((ty + j) * w + tx) as usize;
        for i in 0..tw {
            let f = f64::from(gray[row + i as usize]);
            let t = f64::from(template[(j * tw + i) as usize]);
            sum_f += f;
            sum_f2 += f * f;
            sum_tf += t * f;
        }
    }
    let mean_f = sum_f / n;
    let var_f = sum_f2 - sum_f * mean_f;
    if var_f <= 0.0 {
        return None;
    }
    let stddev = (var_f / n).sqrt();
    if stddev < f64::from(min_stddev) {
        return None;
    }
    let covariance = sum_tf - sum_f * stats.mean;
    if covariance <= 0.0 || stats.ss <= 0.0 {
        return Some((0.0, stddev as f32));
    }
    let denom = (var_f * stats.ss).sqrt();
    Some(((covariance / denom) as f32, stddev as f32))
}

/// Sobel gradient magnitude, clamped to `0..=255`.
fn gradient_magnitude(gray: &[u8], w: i32, h: i32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h) as usize];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let at = |dx: i32, dy: i32| i32::from(gray[((y + dy) * w + (x + dx)) as usize]);
            let gx =
                (at(1, -1) + 2 * at(1, 0) + at(1, 1)) - (at(-1, -1) + 2 * at(-1, 0) + at(-1, 1));
            let gy =
                (at(-1, 1) + 2 * at(0, 1) + at(1, 1)) - (at(-1, -1) + 2 * at(0, -1) + at(1, -1));
            let magnitude = (gx.abs() + gy.abs()) / 4;
            out[(y * w + x) as usize] = magnitude.min(255) as u8;
        }
    }
    out
}

/// Template pixels whose Sobel magnitude exceeds [`EDGE_MIN_TEMPLATE`].
fn template_edge_mask() -> Vec<bool> {
    let grad = gradient_magnitude(CROWN_TEMPLATE, TEMPLATE_W, TEMPLATE_H);
    grad.iter()
        .map(|value| i32::from(*value) >= EDGE_MIN_TEMPLATE)
        .collect()
}

fn edge_agreement(grad: &[u8], w: i32, tx: i32, ty: i32, mask: &[bool]) -> f32 {
    let mut total = 0usize;
    let mut hits = 0usize;
    for j in 0..TEMPLATE_H {
        for i in 0..TEMPLATE_W {
            if !mask[(j * TEMPLATE_W + i) as usize] {
                continue;
            }
            total += 1;
            let x = tx + i;
            let y = ty + j;
            if x < 0 || y < 0 || x >= w {
                continue;
            }
            let index = (y * w + x) as usize;
            if index < grad.len() && i32::from(grad[index]) >= EDGE_MIN_FRAME {
                hits += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        hits as f32 / total as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
