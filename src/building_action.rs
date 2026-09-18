//! Shared building action executor: one capture, click each detected building,
//! verify the selection panel, press `A` once.
//!
//! Deliberately **not** a build row macro. It issues no build order and reports
//! no building: for each detection found by [`crate::building_vision`] it
//! clicks the detection centre once, checks the *selection panel* against the
//! profile's real portrait, and only then presses `A` once. The Spire and
//! Stargate actions are thin wrappers around this executor with their own
//! [`BuildingProfile`].
//!
//! Safety rules this module implements:
//!
//! * exactly **one** full-screen capture and **one** full-screen search per
//!   run; the per-target check reads only the small profile portrait ROI
//!   (`roi_captures`),
//! * the adapter's safety gate runs before *every* cursor move, mouse down/up
//!   and `A` down/up (foreground process, window identity and 1920×1080
//!   geometry at screen `(0, 0)`, held modifiers),
//! * when the selection panel cannot be confidently read as the profile's
//!   building the click is reported as `Skipped` and `A` is **not** sent —
//!   verification cannot be turned off,
//! * a focus/window change aborts instead of reusing the stale snapshot
//!   coordinates, and every key/button this run pressed is released again.
//!
//! Everything talks to [`DesktopAdapter`], so the whole flow is exercised on
//! any host with a fake capture/input adapter.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::building_vision::{
    self, BuildingDetection, BuildingProfile, CLIENT_HEIGHT, CLIENT_WIDTH,
};
use crate::frame::Point;
use crate::input::{DesktopAdapter, InputError};
use crate::macros::{Key, Timing};

/// Bounded selection-panel reads before a click counts as unverified.
pub const VERIFY_ATTEMPTS: usize = 3;
/// Small settle after the last event of one target.
///
/// Nothing reads the game after the `A` key-up, so the next target's cursor
/// move only has to stay queued *behind* that tap. The input queue already
/// guarantees the order, so this is a hygiene gap rather than a frame wait —
/// the configured `gap` would only add dead time between targets.
const ORDERING_GAP: Duration = Duration::from_millis(5);
/// Ceiling on the settle between moving the cursor and clicking.
///
/// `SetCursorPos` applies synchronously, so the click that follows is already
/// delivered at the new position; the configured `gap` is capped here because a
/// long wait per target would only add dead time.
const MOVE_SETTLE: Duration = Duration::from_millis(12);
/// Ceiling on the settle between two selection-panel reads.
///
/// The panel needs a rendered frame to reflect the click, so a retry must never
/// be *shorter* than the configured `gap`; this is an upper bound so a user who
/// configures a very large `gap` does not pay it on every retry of a target
/// whose panel simply is not the profile's building.
const VERIFY_RETRY_GAP: Duration = Duration::from_millis(24);
/// Camera settle after an optional function-key recall before the one scan.
const PRE_CAPTURE_SETTLE: Duration = Duration::from_millis(200);

/// How one action run ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuildingActionOutcome {
    /// The scan finished and every target was either acted on or reported as
    /// skipped.
    Completed,
    /// Cancelled (F8/disarm) before the next event.
    Cancelled,
    /// The safety gate or a bad capture stopped the run. Nothing further was
    /// sent, and a stale snapshot is never reused.
    Aborted { detail: String },
    /// The OS refused an injected event.
    Failed { detail: String },
}

impl BuildingActionOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// What happened to one detected building.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildingTargetDisposition {
    /// The click was verified against the profile portrait and `A` was sent once.
    Acted,
    /// The click landed but the selection could not be confirmed, so `A` was
    /// withheld.
    Skipped { reason: String },
}

/// Per-target detail.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingTargetReport {
    /// Centre the click was aimed at.
    pub center: Point,
    /// Detector score of the building.
    pub score: f32,
    pub disposition: BuildingTargetDisposition,
}

/// Everything one action run did, ready to log or display.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingActionReport {
    /// Short building class name this run acted on ("Spire", "Stargate").
    pub label: &'static str,
    pub outcome: BuildingActionOutcome,
    /// Buildings the single full-screen scan found.
    pub detections: usize,
    /// Per-target outcomes, in click order.
    pub targets: Vec<BuildingTargetReport>,
    /// Targets for which `A` was sent.
    pub acted: usize,
    /// Targets whose selection could not be verified.
    pub skipped: usize,
    /// Full-screen captures performed (always `0` or `1`).
    pub full_captures: usize,
    /// Small selection-panel ROI captures performed (bounded per target).
    pub roi_captures: usize,
    /// Time spent on the one full-screen capture.
    pub capture_ms: u128,
    /// Time spent on the one full-screen detection.
    pub detect_ms: u128,
}

impl BuildingActionReport {
    pub fn new(outcome: BuildingActionOutcome, label: &'static str) -> Self {
        Self {
            label,
            outcome,
            detections: 0,
            targets: Vec::new(),
            acted: 0,
            skipped: 0,
            full_captures: 0,
            roi_captures: 0,
            capture_ms: 0,
            detect_ms: 0,
        }
    }

    pub fn failed(detail: impl Into<String>, label: &'static str) -> Self {
        Self::new(
            BuildingActionOutcome::Failed {
                detail: detail.into(),
            },
            label,
        )
    }

    /// English one-line summary. The GUI adds its own localized framing; this
    /// wording never claims a building was produced.
    pub fn summary(&self) -> String {
        let base = format!(
            "{} action: {} found, {} command(s) sent, {} skipped, {} full capture(s), {} ROI capture(s), capture {} ms, detect {} ms",
            self.label,
            self.detections,
            self.acted,
            self.skipped,
            self.full_captures,
            self.roi_captures,
            self.capture_ms,
            self.detect_ms
        );
        match &self.outcome {
            BuildingActionOutcome::Completed => base,
            BuildingActionOutcome::Cancelled => format!("{base} (cancelled)"),
            BuildingActionOutcome::Aborted { detail } => format!("{base} (aborted: {detail})"),
            BuildingActionOutcome::Failed { detail } => format!("{base} (failed: {detail})"),
        }
    }
}

/// A read-only scan: what one capture and one search found, with timings.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingScanReport {
    pub detections: Vec<BuildingDetection>,
    pub count: usize,
    /// Time spent on the one full-screen capture.
    pub capture_ms: u128,
    /// Time spent on the one full-screen detection.
    pub detect_ms: u128,
    /// Full-screen captures performed (always 1 on success).
    pub full_captures: usize,
}

/// Why a scan-only pass produced no trustworthy result.
///
/// Finding nothing is **not** an error: `Ok` with `count == 0` means a
/// supported, non-blank game frame was searched and no building was found.
/// These variants mean the *frame itself* could not be trusted, so reporting
/// "0 found" would be a false negative instead of a measurement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuildingScanError {
    /// Cancelled (F8/disarm) before or during capture/detection.
    Cancelled,
    /// The adapter could not deliver a capture: the safety gate refused it
    /// (wrong foreground window, held modifier) or the OS failed.
    Adapter(InputError),
    /// A capture arrived but is blank or of an unsupported size/origin, so no
    /// detection result may be derived from it.
    Unusable { detail: String },
    /// Unexpected internal failure (a panic inside the scan worker). Nothing
    /// was injected.
    Internal { detail: String },
}

impl std::fmt::Display for BuildingScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("the scan was cancelled"),
            Self::Adapter(error) => write!(f, "{error}"),
            Self::Unusable { detail } => f.write_str(detail),
            Self::Internal { detail } => f.write_str(detail),
        }
    }
}

impl std::error::Error for BuildingScanError {}

/// Scan-only mode: one capture and one full-screen search, **zero** injected
/// events. Use it to preview where the action would click.
///
/// Runs the same frame gates as [`run`] before it reports anything: a blank or
/// unsupported capture is [`BuildingScanError::Unusable`], never "0 found". A
/// cancelled pass (`cancel` already latched) returns before it captures
/// anything.
pub fn scan_once(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    profile: &BuildingProfile,
) -> Result<BuildingScanReport, BuildingScanError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(BuildingScanError::Cancelled);
    }
    if !profile.is_valid() {
        return Err(BuildingScanError::Unusable {
            detail: "invalid building calibration profile".to_owned(),
        });
    }

    let started = Instant::now();
    let frame = adapter
        .capture_client()
        .map_err(BuildingScanError::Adapter)?;
    let capture_ms = started.elapsed().as_millis();
    if cancel.load(Ordering::SeqCst) {
        return Err(BuildingScanError::Cancelled);
    }

    if frame.is_blank() {
        return Err(BuildingScanError::Unusable {
            detail: "the capture is blank; the game window is minimized or not rendering"
                .to_owned(),
        });
    }
    if !building_vision::supported_client(&frame) {
        return Err(BuildingScanError::Unusable {
            detail: format!(
                "unsupported client size {}x{} at ({}, {}); only {}x{} at (0, 0) is supported",
                frame.width(),
                frame.height(),
                frame.origin().x,
                frame.origin().y,
                CLIENT_WIDTH,
                CLIENT_HEIGHT
            ),
        });
    }

    let scan = building_vision::detect_buildings(&frame, profile);
    if cancel.load(Ordering::SeqCst) {
        return Err(BuildingScanError::Cancelled);
    }
    let count = scan.count();
    Ok(BuildingScanReport {
        detections: scan.detections,
        count,
        capture_ms,
        detect_ms: scan.detect_ms,
        full_captures: 1,
    })
}

/// Runs the action once for `profile`.
///
/// Never panics on adapter errors, always releases what it pressed, and returns
/// a report for every path.
pub fn run(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    profile: &BuildingProfile,
) -> BuildingActionReport {
    run_after_key(adapter, cancel, timing, profile, None)
}

/// Runs the same action after optionally tapping one guarded key and waiting
/// for the recalled camera view to settle before the single capture.
pub fn run_after_key(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    profile: &BuildingProfile,
    before_capture: Option<Key>,
) -> BuildingActionReport {
    let mut report = BuildingActionReport::new(BuildingActionOutcome::Completed, profile.label);
    let outcome = match run_inner(
        adapter,
        cancel,
        timing,
        profile,
        before_capture,
        &mut report,
    ) {
        Ok(()) => BuildingActionOutcome::Completed,
        Err(outcome) => outcome,
    };
    report.outcome = match (outcome, adapter.release_all()) {
        (
            outcome
            @ (BuildingActionOutcome::Failed { .. } | BuildingActionOutcome::Aborted { .. }),
            _,
        ) => outcome,
        (_, Err(error)) => BuildingActionOutcome::Failed {
            detail: format!("could not release injected input: {error}"),
        },
        (outcome, Ok(())) => outcome,
    };
    report
}

fn run_inner(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    profile: &BuildingProfile,
    before_capture: Option<Key>,
    report: &mut BuildingActionReport,
) -> Result<(), BuildingActionOutcome> {
    if cancel.load(Ordering::SeqCst) {
        return Err(BuildingActionOutcome::Cancelled);
    }
    if !profile.is_valid() {
        return Err(BuildingActionOutcome::Aborted {
            detail: "invalid building calibration profile; no input was sent".to_owned(),
        });
    }
    if let Some(key) = before_capture {
        tap(adapter, cancel, timing, key)?;
        wait(PRE_CAPTURE_SETTLE, cancel)?;
    }

    // Exactly one full-screen capture and one full-screen search per run.
    let started = Instant::now();
    let frame = adapter.capture_client().map_err(map_error)?;
    report.capture_ms = started.elapsed().as_millis();
    report.full_captures = 1;
    if cancel.load(Ordering::SeqCst) {
        return Err(BuildingActionOutcome::Cancelled);
    }

    if frame.is_blank() {
        return Err(BuildingActionOutcome::Aborted {
            detail: "the capture is blank; the game window is minimized or not rendering"
                .to_owned(),
        });
    }
    if !building_vision::supported_client(&frame) {
        return Err(BuildingActionOutcome::Aborted {
            detail: format!(
                "unsupported client size {}x{} at ({}, {}); only {}x{} at (0, 0) is supported",
                frame.width(),
                frame.height(),
                frame.origin().x,
                frame.origin().y,
                CLIENT_WIDTH,
                CLIENT_HEIGHT
            ),
        });
    }

    let scan = building_vision::detect_buildings(&frame, profile);
    report.detect_ms = scan.detect_ms;
    report.detections = scan.count();
    if cancel.load(Ordering::SeqCst) {
        return Err(BuildingActionOutcome::Cancelled);
    }

    for detection in &scan.detections {
        if cancel.load(Ordering::SeqCst) {
            return Err(BuildingActionOutcome::Cancelled);
        }
        // The cursor move is itself an action in the game, so the safety gate
        // runs before it exactly like before a key or click.
        guard(adapter, cancel)?;
        adapter.move_cursor(detection.center).map_err(map_error)?;
        wait(timing.gap.min(MOVE_SETTLE), cancel)?;
        click(adapter, cancel, timing)?;

        let disposition = if verify_selection(adapter, cancel, timing, profile, report)? {
            tap(adapter, cancel, timing, Key::A)?;
            report.acted += 1;
            BuildingTargetDisposition::Acted
        } else {
            report.skipped += 1;
            BuildingTargetDisposition::Skipped {
                reason: format!(
                    "selection panel did not show the {} portrait; A withheld",
                    profile.label
                ),
            }
        };
        report.targets.push(BuildingTargetReport {
            center: detection.center,
            score: detection.score,
            disposition,
        });
    }
    Ok(())
}

/// Reads the selection panel a bounded number of times; true only when it
/// positively shows the profile's portrait.
fn verify_selection(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    profile: &BuildingProfile,
    report: &mut BuildingActionReport,
) -> Result<bool, BuildingActionOutcome> {
    for attempt in 0..VERIFY_ATTEMPTS {
        guard(adapter, cancel)?;
        let roi = adapter
            .capture_region(profile.portrait_roi)
            .map_err(map_error)?;
        report.roi_captures += 1;
        if building_vision::verify_building_selection(&roi, profile).accepted {
            return Ok(true);
        }
        if attempt + 1 < VERIFY_ATTEMPTS {
            wait(VERIFY_RETRY_GAP.min(timing.gap), cancel)?;
        }
    }
    Ok(false)
}

fn map_error(error: InputError) -> BuildingActionOutcome {
    match error {
        // A refused gate is an expected, handled stop: the snapshot coordinates
        // must not be used after a focus/window change.
        InputError::Unsafe(_) => BuildingActionOutcome::Aborted {
            detail: error.to_string(),
        },
        InputError::Injection(_) => BuildingActionOutcome::Failed {
            detail: error.to_string(),
        },
    }
}

/// Runs the adapter's safety gate before an injected event.
pub(crate) fn guard(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
) -> Result<(), BuildingActionOutcome> {
    if cancel.load(Ordering::SeqCst) {
        return Err(BuildingActionOutcome::Cancelled);
    }
    adapter.safety_check().map_err(map_error)?;
    // F8 may arrive while the OS gate is inspecting the foreground process.
    if cancel.load(Ordering::SeqCst) {
        return Err(BuildingActionOutcome::Cancelled);
    }
    Ok(())
}

/// Injectable key hold / gap, polled for cancellation.
fn wait(duration: Duration, cancel: &AtomicBool) -> Result<(), BuildingActionOutcome> {
    if crate::engine::sleep_unless_cancelled(duration, cancel) {
        Ok(())
    } else {
        Err(BuildingActionOutcome::Cancelled)
    }
}

fn tap(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    key: Key,
) -> Result<(), BuildingActionOutcome> {
    guard(adapter, cancel)?;
    adapter.key_down(key).map_err(map_error)?;
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.key_up(key).map_err(map_error)?;
    wait(ORDERING_GAP, cancel)
}

fn click(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
) -> Result<(), BuildingActionOutcome> {
    guard(adapter, cancel)?;
    adapter.mouse_left_down().map_err(map_error)?;
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.mouse_left_up().map_err(map_error)?;
    wait(timing.gap, cancel)
}
