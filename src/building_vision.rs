//! Shared local detector and selection verifier for one calibrated building
//! class (Spire, Stargate, ...).
//!
//! The single supported profile is the 1920×1080 Remastered client at screen
//! origin `(0, 0)`. Everything here is pure and host testable: it reads a
//! [`Frame`] and returns positions.
//!
//! Matching is *shape* matching, not a colour blob:
//!
//! * a small grayscale hull template is extracted from a calibration
//!   screenshot and embedded with `include_bytes!` (see each fixture README for
//!   provenance);
//! * a coarse-to-fine normalised cross correlation finds candidate positions;
//! * a candidate survives only when the window has real contrast (frame
//!   stddev), the correlation is high, and the template's edges coincide with
//!   real screen edges (edge agreement) — flat terrain, HUD borders and colour
//!   blobs cannot pass all three;
//! * detections are deduplicated by non-maximum suppression.
//!
//! Everything a building class changes — template, click offset, search
//! viewport, portrait and thresholds — lives in [`BuildingProfile`], so a new
//! class is a data change, not a new copy of the algorithm.
//!
//! Every committed screenshot is a **single calibration scene**, not evidence
//! of generalisation. Thresholds were chosen so the real buildings pass and the
//! recorded distractors do not.

use std::time::Instant;

use crate::frame::{Frame, Point, Rect};

/// Client width of the only supported profile.
pub const CLIENT_WIDTH: i32 = 1920;
/// Client height of the only supported profile.
pub const CLIENT_HEIGHT: i32 = 1080;

/// Minimum contrast (0..255 luma stddev) of the coarse-stage window.
const COARSE_MIN_STDDEV: f32 = 4.0;
/// Minimum contrast (0..255 luma stddev) of the mid-stage window.
const MID_MIN_STDDEV: f32 = 8.0;
/// Maximum coarse-stage candidates kept per scan.
const MAX_COARSE_CANDIDATES: usize = 128;
/// Maximum mid-stage candidates kept per scan.
const MAX_MID_CANDIDATES: usize = 96;
/// Sobel magnitude (0..255 scale) that counts as a template edge.
const EDGE_MIN_TEMPLATE: i32 = 48;
/// Sobel magnitude (0..255 scale) that counts as a screen edge.
const EDGE_MIN_FRAME: i32 = 36;

/// One calibrated building class: template, click geometry, portrait and the
/// thresholds that separate the real building from its distractors.
#[derive(Clone, Copy, Debug)]
pub struct BuildingProfile {
    /// Short class name used in reports ("Spire", "Stargate").
    pub label: &'static str,
    /// Grayscale hull template (`template_w * template_h` bytes).
    pub template: &'static [u8],
    pub template_w: i32,
    pub template_h: i32,
    /// Where inside the template the click lands.
    pub click_offset: Point,
    /// Candidate click centres must fall inside this rectangle.
    pub safe_viewport: Rect,
    /// Screen rectangle of the single-unit information-panel portrait.
    pub portrait_roi: Rect,
    /// Portrait silhouette mask (`255` = sprite pixel).
    pub portrait_mask: &'static [u8],
    /// Score above which a full-resolution candidate is reported.
    pub min_ncc: f32,
    /// Minimum frame window contrast (0..255 luma stddev) to accept a match.
    pub min_frame_stddev: f32,
    /// Minimum fraction of template edge pixels that coincide with screen edges.
    pub min_edge_agreement: f32,
    /// Coarse-stage accept threshold.
    pub coarse_min_ncc: f32,
    /// Mid-stage accept threshold.
    pub mid_min_ncc: f32,
    /// Two detections closer than this are the same building.
    pub nms_radius: i32,
    /// Upper bound on reported buildings in one scene.
    pub max_detections: usize,
    /// A portrait pixel exists when any channel is above this; the panel is black.
    pub portrait_channel_min: u8,
    /// Selection verification thresholds (shape overlap with the reference portrait).
    pub verify_coverage_min: f32,
    pub verify_iou_min: f32,
}

/// One confirmed building.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildingDetection {
    /// Click centre of the building.
    pub center: Point,
    /// Zero-mean normalised cross correlation of the template (`~1.0` is a
    /// perfect match).
    pub score: f32,
    /// Fraction of template edge pixels matched on screen.
    pub edge_agreement: f32,
    /// Contrast of the matched window; flat areas are rejected on this.
    pub frame_stddev: f32,
}

/// Result of one full-screen scan.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingScan {
    pub detections: Vec<BuildingDetection>,
    /// Wall time of the full detection pass.
    pub detect_ms: u128,
    /// Full-resolution positions the last stage scored.
    pub evaluated: usize,
    /// False when the frame is not the supported 1920×1080 client at `(0, 0)`;
    /// no detection is attempted then.
    pub supported_profile: bool,
}

impl BuildingScan {
    pub fn count(&self) -> usize {
        self.detections.len()
    }
}

/// Shape overlap of a captured portrait ROI with the reference portrait.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionVerification {
    /// Fraction of reference portrait pixels also present in the capture.
    pub coverage: f32,
    /// Intersection over union of the two silhouettes.
    pub iou: f32,
    /// True when the capture really shows the profile's portrait.
    pub accepted: bool,
}

/// True when `frame` is the supported 1920×1080 client at screen origin
/// `(0, 0)`. The origin is part of the calibrated profile: the detector
/// reports frame-local coordinates that are only clickable if the client sits
/// at the desktop origin, so a moved window must fail closed.
pub fn supported_client(frame: &Frame) -> bool {
    frame.width() as i32 == CLIENT_WIDTH
        && frame.height() as i32 == CLIENT_HEIGHT
        && frame.origin() == Point::new(0, 0)
}

/// Template rectangle for a detection whose click centre is `center`.
pub fn template_rect(center: Point, profile: &BuildingProfile) -> Rect {
    Rect::new(
        center.x - profile.click_offset.x,
        center.y - profile.click_offset.y,
        profile.template_w,
        profile.template_h,
    )
}

/// Scans one frame for `profile`'s buildings. This is the single expensive pass.
pub fn detect_buildings(frame: &Frame, profile: &BuildingProfile) -> BuildingScan {
    let started = Instant::now();
    let template_w = profile.template_w;
    let template_h = profile.template_h;
    let viewport = profile.safe_viewport;
    if !supported_client(frame) {
        return BuildingScan {
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
    let (t2, tw2, th2) = downsample(profile.template, template_w, template_h, 2);
    let (t4, tw4, th4) = downsample(&t2, tw2, th2, 2);

    let t_stats = TemplateStats::new(profile.template, template_w, template_h);
    let t2_stats = TemplateStats::new(&t2, tw2, th2);
    let t4_stats = TemplateStats::new(&t4, tw4, th4);

    // Coarse stage: every 4th pixel, cheap correlation inside the safe viewport.
    let min_cx = ((viewport.x - profile.click_offset.x).max(0) + 3) / 4;
    let max_cx = ((viewport.x + viewport.w - profile.click_offset.x).min(w - template_w) - 1) / 4;
    let min_cy = ((viewport.y - profile.click_offset.y).max(0) + 3) / 4;
    let max_cy = ((viewport.y + viewport.h - profile.click_offset.y).min(h - template_h) - 1) / 4;

    let mut coarse_raw: Vec<(i32, i32, f32)> = Vec::new();
    for cy in min_cy..=max_cy {
        for cx in min_cx..=max_cx {
            let tx = cx * 4;
            let ty = cy * 4;
            let center = Point::new(tx + profile.click_offset.x, ty + profile.click_offset.y);
            if !point_in_viewport(center) {
                continue;
            }
            if let Some((ncc, _)) = window_ncc(
                &g4,
                w4,
                h4,
                cx,
                cy,
                &t4,
                tw4,
                th4,
                &t4_stats,
                COARSE_MIN_STDDEV,
            ) && ncc >= profile.coarse_min_ncc
            {
                coarse_raw.push((tx, ty, ncc));
            }
        }
    }
    coarse_raw.sort_by(|a, b| b.2.total_cmp(&a.2));

    // Diverse coarse candidate retention: suppress immediate neighbors (within
    // 16px) so redundant points from one building do not displace others.
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
                if mx < 0 || my < 0 || mx + template_w > w || my + template_h > h {
                    continue;
                }
                if let Some((ncc, _)) = window_ncc(
                    &g2,
                    w2,
                    h2,
                    mx >> 1,
                    my >> 1,
                    &t2,
                    tw2,
                    th2,
                    &t2_stats,
                    MID_MIN_STDDEV,
                ) && ncc >= profile.mid_min_ncc
                {
                    mid_raw.push((mx, my, ncc));
                }
            }
        }
    }
    mid_raw.sort_by(|a, b| b.2.total_cmp(&a.2));

    // Diverse mid candidate retention: keep local maxima with a radius of 24px.
    // This prevents clustered sub-pixel variants from choking out distinct
    // buildings.
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
    let edge_mask = template_edge_mask(profile);
    let mut full: Vec<(i32, i32, f32, f32, f32)> = Vec::new();
    let mut evaluated = 0;
    for (tx, ty, _) in &mid {
        for dy in -3..=3 {
            for dx in -3..=3 {
                let (fx, fy) = (tx + dx, ty + dy);
                if fx < 0 || fy < 0 || fx + template_w > w || fy + template_h > h {
                    continue;
                }
                let center = Point::new(fx + profile.click_offset.x, fy + profile.click_offset.y);
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
                    profile.template,
                    template_w,
                    template_h,
                    &t_stats,
                    profile.min_frame_stddev,
                ) else {
                    continue;
                };
                if ncc < profile.min_ncc {
                    continue;
                }
                let agreement =
                    edge_agreement(&grad, w, fx, fy, template_w, template_h, &edge_mask);
                if agreement < profile.min_edge_agreement {
                    continue;
                }
                full.push((fx, fy, ncc.max(0.0), agreement, stddev));
            }
        }
    }
    full.sort_by(|a, b| b.2.total_cmp(&a.2));

    // Non-maximum suppression: keep the best score per cluster.
    let mut detections: Vec<BuildingDetection> = Vec::new();
    for (tx, ty, score, agreement, stddev) in full {
        let center = Point::new(tx + profile.click_offset.x, ty + profile.click_offset.y);
        if detections.iter().any(|kept| {
            (kept.center.x - center.x).abs() <= profile.nms_radius
                && (kept.center.y - center.y).abs() <= profile.nms_radius
        }) {
            continue;
        }
        detections.push(BuildingDetection {
            center,
            score,
            edge_agreement: agreement,
            frame_stddev: stddev,
        });
        if detections.len() >= profile.max_detections {
            break;
        }
    }
    detections.sort_by(|a, b| {
        a.center
            .y
            .cmp(&b.center.y)
            .then(a.center.x.cmp(&b.center.x))
    });

    BuildingScan {
        detections,
        detect_ms: started.elapsed().as_millis(),
        evaluated,
        supported_profile: true,
    }
}

/// Scores a captured selection-panel ROI against the profile's reference
/// portrait.
///
/// The comparison is on the silhouette (any channel above the profile's black
/// threshold), so it is independent of the player's team colour and of the HP
/// text.
pub fn verify_building_selection(roi: &Frame, profile: &BuildingProfile) -> SelectionVerification {
    let (rw, rh) = (profile.portrait_roi.w, profile.portrait_roi.h);
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
            let wants = profile.portrait_mask[index] != 0;
            let has = match roi.pixel(x, y) {
                Some(px) => px.r.max(px.g).max(px.b) > profile.portrait_channel_min,
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
        accepted: coverage >= profile.verify_coverage_min && iou >= profile.verify_iou_min,
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

/// Rec.601 luma, the grayscale every profile asset is stored in.
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
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
fn template_edge_mask(profile: &BuildingProfile) -> Vec<bool> {
    let grad = gradient_magnitude(profile.template, profile.template_w, profile.template_h);
    grad.iter()
        .map(|value| i32::from(*value) >= EDGE_MIN_TEMPLATE)
        .collect()
}

fn edge_agreement(grad: &[u8], w: i32, tx: i32, ty: i32, tw: i32, th: i32, mask: &[bool]) -> f32 {
    let mut total = 0usize;
    let mut hits = 0usize;
    for j in 0..th {
        for i in 0..tw {
            if !mask[(j * tw + i) as usize] {
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
