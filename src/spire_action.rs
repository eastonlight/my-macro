//! The Spire action: one capture, click each detected crown, verify, press `A`.
//!
//! This is deliberately **not** the build-Spire row macro. It issues no build
//! order and reports no building: for each Spire crown found by
//! [`crate::spire_vision`] it clicks the crown once, checks the *selection
//! panel* against the real Spire portrait, and only then presses `A` once.
//!
//! Safety rules this module implements:
//!
//! * exactly **one** full-screen capture and **one** full-screen search per
//!   run; the per-target check reads only the small
//!   [`crate::spire_vision::PORTRAIT_ROI`] (`roi_captures`),
//! * the adapter's safety gate runs before *every* cursor move, mouse down/up
//!   and `A` down/up (foreground process, window identity and 1920×1080
//!   geometry, held modifiers),
//! * when the selection panel cannot be confidently read as a Spire the click
//!   is reported as `Skipped` and `A` is **not** sent — verification cannot be
//!   turned off,
//! * a focus/window change aborts instead of reusing the stale snapshot
//!   coordinates, and every key/button this run pressed is released again.
//!
//! Everything talks to [`DesktopAdapter`], so the whole flow is exercised on
//! any host with a fake capture/input adapter.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::frame::Point;
use crate::input::{DesktopAdapter, InputError};
use crate::macros::{Key, Timing};
use crate::spire_vision::{self, PORTRAIT_ROI};

/// Bounded selection-panel reads before a click counts as unverified.
pub const VERIFY_ATTEMPTS: usize = 3;
/// Cancellation poll interval while waiting.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(5);
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
/// whose panel simply is not a Spire.
const VERIFY_RETRY_GAP: Duration = Duration::from_millis(24);

/// How one Spire action run ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpireActionOutcome {
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

impl SpireActionOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// What happened to one detected crown.
#[derive(Clone, Debug, PartialEq)]
pub enum TargetDisposition {
    /// The click was verified against the Spire portrait and `A` was sent once.
    Acted,
    /// The click landed but the selection could not be confirmed as a Spire, so
    /// `A` was withheld.
    Skipped { reason: String },
}

/// Per-target detail.
#[derive(Clone, Debug, PartialEq)]
pub struct SpireTargetReport {
    /// Crown centre the click was aimed at.
    pub center: Point,
    /// Detector score of the crown.
    pub score: f32,
    pub disposition: TargetDisposition,
}

/// Everything one Spire action run did, ready to log or display.
#[derive(Clone, Debug, PartialEq)]
pub struct SpireActionReport {
    pub outcome: SpireActionOutcome,
    /// Spires the single full-screen scan found.
    pub detections: usize,
    /// Per-target outcomes, in click order.
    pub targets: Vec<SpireTargetReport>,
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

impl SpireActionReport {
    pub fn new(outcome: SpireActionOutcome) -> Self {
        Self {
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

    pub fn failed(detail: impl Into<String>) -> Self {
        Self::new(SpireActionOutcome::Failed {
            detail: detail.into(),
        })
    }

    /// English one-line summary. The GUI adds its own localized framing; this
    /// wording never claims a building was produced.
    pub fn summary(&self) -> String {
        let base = format!(
            "Spire action: {} found, {} command(s) sent, {} skipped, {} full capture(s), {} ROI capture(s), capture {} ms, detect {} ms",
            self.detections,
            self.acted,
            self.skipped,
            self.full_captures,
            self.roi_captures,
            self.capture_ms,
            self.detect_ms
        );
        match &self.outcome {
            SpireActionOutcome::Completed => base,
            SpireActionOutcome::Cancelled => format!("{base} (cancelled)"),
            SpireActionOutcome::Aborted { detail } => format!("{base} (aborted: {detail})"),
            SpireActionOutcome::Failed { detail } => format!("{base} (failed: {detail})"),
        }
    }
}

/// A read-only scan: what one capture and one search found, with timings.
#[derive(Clone, Debug, PartialEq)]
pub struct SpireScanReport {
    pub detections: Vec<spire_vision::SpireDetection>,
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
/// supported, non-blank game frame was searched and no crown was found. These
/// variants mean the *frame itself* could not be trusted, so reporting
/// "0 spires" would be a false negative instead of a measurement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpireScanError {
    /// Cancelled (F8/disarm) before or during capture/detection.
    Cancelled,
    /// The adapter could not deliver a capture: the safety gate refused it
    /// (wrong foreground window, held modifier) or the OS failed.
    Adapter(InputError),
    /// A capture arrived but is blank or of an unsupported size, so no
    /// detection result may be derived from it.
    Unusable { detail: String },
    /// Unexpected internal failure (a panic inside the scan worker). Nothing
    /// was injected.
    Internal { detail: String },
}

impl std::fmt::Display for SpireScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("the scan was cancelled"),
            Self::Adapter(error) => write!(f, "{error}"),
            Self::Unusable { detail } => f.write_str(detail),
            Self::Internal { detail } => f.write_str(detail),
        }
    }
}

impl std::error::Error for SpireScanError {}

/// Scan-only mode: one capture and one full-screen search, **zero** injected
/// events. Use it to preview where the action would click.
///
/// Runs the same frame gates as [`run`] before it reports anything: a blank or
/// unsupported capture is [`SpireScanError::Unusable`], never "0 spires". A
/// cancelled pass (`cancel` already latched) returns before it captures
/// anything.
pub fn scan_once(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
) -> Result<SpireScanReport, SpireScanError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(SpireScanError::Cancelled);
    }

    let started = Instant::now();
    let frame = adapter.capture_client().map_err(SpireScanError::Adapter)?;
    let capture_ms = started.elapsed().as_millis();
    if cancel.load(Ordering::SeqCst) {
        return Err(SpireScanError::Cancelled);
    }

    if frame.is_blank() {
        return Err(SpireScanError::Unusable {
            detail: "the capture is blank; the game window is minimized or not rendering"
                .to_owned(),
        });
    }
    if !spire_vision::supported_profile(&frame) {
        return Err(SpireScanError::Unusable {
            detail: format!(
                "unsupported client size {}x{}; only {}x{} is supported",
                frame.width(),
                frame.height(),
                spire_vision::CLIENT_WIDTH,
                spire_vision::CLIENT_HEIGHT
            ),
        });
    }

    let scan = spire_vision::detect_spires(&frame);
    if cancel.load(Ordering::SeqCst) {
        return Err(SpireScanError::Cancelled);
    }
    let count = scan.count();
    Ok(SpireScanReport {
        detections: scan.detections,
        count,
        capture_ms,
        detect_ms: scan.detect_ms,
        full_captures: 1,
    })
}

/// Runs the Spire action once.
///
/// Never panics on adapter errors, always releases what it pressed, and returns
/// a report for every path.
pub fn run(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
) -> SpireActionReport {
    let mut report = SpireActionReport::new(SpireActionOutcome::Completed);
    let outcome = match run_inner(adapter, cancel, timing, &mut report) {
        Ok(()) => SpireActionOutcome::Completed,
        Err(outcome) => outcome,
    };
    report.outcome = match (outcome, adapter.release_all()) {
        (outcome @ (SpireActionOutcome::Failed { .. } | SpireActionOutcome::Aborted { .. }), _) => {
            outcome
        }
        (_, Err(error)) => SpireActionOutcome::Failed {
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
    report: &mut SpireActionReport,
) -> Result<(), SpireActionOutcome> {
    if cancel.load(Ordering::SeqCst) {
        return Err(SpireActionOutcome::Cancelled);
    }

    // Exactly one full-screen capture and one full-screen search per run.
    let started = Instant::now();
    let frame = adapter.capture_client().map_err(map_error)?;
    report.capture_ms = started.elapsed().as_millis();
    report.full_captures = 1;
    if cancel.load(Ordering::SeqCst) {
        return Err(SpireActionOutcome::Cancelled);
    }

    if frame.is_blank() {
        return Err(SpireActionOutcome::Aborted {
            detail: "the capture is blank; the game window is minimized or not rendering"
                .to_owned(),
        });
    }
    if !spire_vision::supported_profile(&frame) {
        return Err(SpireActionOutcome::Aborted {
            detail: format!(
                "unsupported client size {}x{}; only {}x{} is supported",
                frame.width(),
                frame.height(),
                spire_vision::CLIENT_WIDTH,
                spire_vision::CLIENT_HEIGHT
            ),
        });
    }

    let scan = spire_vision::detect_spires(&frame);
    report.detect_ms = scan.detect_ms;
    report.detections = scan.count();
    if cancel.load(Ordering::SeqCst) {
        return Err(SpireActionOutcome::Cancelled);
    }

    for detection in &scan.detections {
        if cancel.load(Ordering::SeqCst) {
            return Err(SpireActionOutcome::Cancelled);
        }
        // The cursor move is itself an action in the game, so the safety gate
        // runs before it exactly like before a key or click.
        guard(adapter, cancel)?;
        adapter.move_cursor(detection.center).map_err(map_error)?;
        wait(timing.gap.min(MOVE_SETTLE), cancel)?;
        click(adapter, cancel, timing)?;

        let disposition = if verify_selection(adapter, cancel, timing, report)? {
            tap(adapter, cancel, timing, Key::A)?;
            report.acted += 1;
            TargetDisposition::Acted
        } else {
            report.skipped += 1;
            TargetDisposition::Skipped {
                reason: "selection panel did not show the Spire portrait; A withheld".to_owned(),
            }
        };
        report.targets.push(SpireTargetReport {
            center: detection.center,
            score: detection.score,
            disposition,
        });
    }
    Ok(())
}

/// Reads the selection panel a bounded number of times; true only when it
/// positively shows the Spire portrait.
fn verify_selection(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    report: &mut SpireActionReport,
) -> Result<bool, SpireActionOutcome> {
    for attempt in 0..VERIFY_ATTEMPTS {
        guard(adapter, cancel)?;
        let roi = adapter.capture_region(PORTRAIT_ROI).map_err(map_error)?;
        report.roi_captures += 1;
        if spire_vision::verify_spire_selection(&roi).accepted {
            return Ok(true);
        }
        if attempt + 1 < VERIFY_ATTEMPTS {
            wait(VERIFY_RETRY_GAP.min(timing.gap), cancel)?;
        }
    }
    Ok(false)
}

fn map_error(error: InputError) -> SpireActionOutcome {
    match error {
        // A refused gate is an expected, handled stop: the snapshot coordinates
        // must not be used after a focus/window change.
        InputError::Unsafe(_) => SpireActionOutcome::Aborted {
            detail: error.to_string(),
        },
        InputError::Injection(_) => SpireActionOutcome::Failed {
            detail: error.to_string(),
        },
    }
}

/// Runs the adapter's safety gate before an injected event.
fn guard(adapter: &mut dyn DesktopAdapter, cancel: &AtomicBool) -> Result<(), SpireActionOutcome> {
    if cancel.load(Ordering::SeqCst) {
        return Err(SpireActionOutcome::Cancelled);
    }
    adapter.safety_check().map_err(map_error)?;
    // F8 may arrive while the OS gate is inspecting the foreground process.
    if cancel.load(Ordering::SeqCst) {
        return Err(SpireActionOutcome::Cancelled);
    }
    Ok(())
}

/// Injectable key hold / gap, polled for cancellation.
fn wait(duration: Duration, cancel: &AtomicBool) -> Result<(), SpireActionOutcome> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if cancel.load(Ordering::SeqCst) {
            return Err(SpireActionOutcome::Cancelled);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(left.min(CANCEL_POLL_INTERVAL));
    }
    Ok(())
}

fn tap(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    key: Key,
) -> Result<(), SpireActionOutcome> {
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
) -> Result<(), SpireActionOutcome> {
    guard(adapter, cancel)?;
    adapter.mouse_left_down().map_err(map_error)?;
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.mouse_left_up().map_err(map_error)?;
    wait(timing.gap, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Frame, Rect};
    use crate::input::InputAdapter;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn fixture(name: &str) -> Frame {
        let path = PathBuf::from("tests/fixtures/spire-screen-1080").join(name);
        Frame::from_png(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
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
        Frame::new(w as u32, h as u32, Point::new(x, y), rgba).expect("crop")
    }

    fn blit(source: &Frame) -> Frame {
        let mut frame = Frame::blank(1920, 1080);
        for y in 0..source.height() as i32 {
            for x in 0..source.width() as i32 {
                if let Some(px) = source.pixel(x, y) {
                    frame.set_pixel(600 + x, 300 + y, px);
                }
            }
        }
        frame
    }

    struct Fake {
        screen: Frame,
        portrait: Frame,
        blank_panel: Frame,
        /// The first `verified_clicks` clicks read as a selected Spire.
        verified_clicks: usize,
        clicks: usize,
        events: Vec<String>,
        held: Vec<Key>,
        left_held: bool,
        full_captures: usize,
        roi_captures: usize,
        release_all_calls: usize,
        safety_calls: usize,
        /// When set, `capture_client` fails with this error instead of
        /// returning the screen (the Win32 gate refuses before any pixel is
        /// read).
        capture_error: Option<InputError>,
        fail_safety_after: Option<usize>,
        cancel_after_safety: Option<usize>,
        cancel_after_capture: bool,
        fail_key_down: bool,
        cancel: Arc<AtomicBool>,
    }

    impl Fake {
        fn on_screen(screen: Frame) -> Self {
            let portrait = crop(
                &screen,
                PORTRAIT_ROI.x,
                PORTRAIT_ROI.y,
                PORTRAIT_ROI.w,
                PORTRAIT_ROI.h,
            );
            Self {
                screen,
                portrait,
                blank_panel: Frame::blank(PORTRAIT_ROI.w as u32, PORTRAIT_ROI.h as u32),
                verified_clicks: usize::MAX,
                clicks: 0,
                events: Vec::new(),
                held: Vec::new(),
                left_held: false,
                full_captures: 0,
                roi_captures: 0,
                release_all_calls: 0,
                safety_calls: 0,
                capture_error: None,
                fail_safety_after: None,
                cancel_after_safety: None,
                cancel_after_capture: false,
                fail_key_down: false,
                cancel: Arc::new(AtomicBool::new(false)),
            }
        }

        fn cancel_flag(&self) -> Arc<AtomicBool> {
            Arc::clone(&self.cancel)
        }

        fn events(&self) -> &[String] {
            &self.events
        }

        fn count(&self, needle: &str) -> usize {
            self.events.iter().filter(|event| *event == needle).count()
        }

        fn any_starts_with(&self, prefix: &str) -> bool {
            self.events.iter().any(|event| event.starts_with(prefix))
        }

        fn safety(&mut self) -> Result<(), InputError> {
            self.safety_calls += 1;
            if let Some(after) = self.cancel_after_safety
                && self.safety_calls >= after
            {
                self.cancel.store(true, Ordering::SeqCst);
            }
            if let Some(after) = self.fail_safety_after
                && self.safety_calls > after
            {
                return Err(InputError::Unsafe("test: focus changed".to_owned()));
            }
            Ok(())
        }
    }

    impl InputAdapter for Fake {
        fn key_down(&mut self, key: Key) -> Result<(), InputError> {
            if self.fail_key_down {
                return Err(InputError::Injection("test: SendInput refused".to_owned()));
            }
            self.events.push(format!("key_down({})", key.name()));
            if !self.held.contains(&key) {
                self.held.push(key);
            }
            Ok(())
        }

        fn key_up(&mut self, key: Key) -> Result<(), InputError> {
            self.events.push(format!("key_up({})", key.name()));
            self.held.retain(|held| *held != key);
            Ok(())
        }

        fn mouse_left_down(&mut self) -> Result<(), InputError> {
            self.events.push("down".to_owned());
            self.left_held = true;
            Ok(())
        }

        fn mouse_left_up(&mut self) -> Result<(), InputError> {
            self.events.push("up".to_owned());
            self.left_held = false;
            self.clicks += 1;
            Ok(())
        }

        fn release_all(&mut self) -> Result<(), InputError> {
            self.release_all_calls += 1;
            let held = std::mem::take(&mut self.held);
            for key in held {
                self.events.push(format!("release({})", key.name()));
            }
            if self.left_held {
                self.events.push("release(mouse)".to_owned());
                self.left_held = false;
            }
            Ok(())
        }

        fn safety_check(&mut self) -> Result<(), InputError> {
            self.safety()
        }
    }

    impl DesktopAdapter for Fake {
        fn cursor_position(&mut self) -> Result<Point, InputError> {
            Ok(Point::new(0, 0))
        }

        fn move_cursor(&mut self, point: Point) -> Result<(), InputError> {
            self.events.push(format!("move({},{})", point.x, point.y));
            Ok(())
        }

        fn capture_client(&mut self) -> Result<Frame, InputError> {
            if let Some(error) = &self.capture_error {
                return Err(error.clone());
            }
            self.full_captures += 1;
            if self.cancel_after_capture {
                self.cancel.store(true, Ordering::SeqCst);
            }
            Ok(self.screen.clone())
        }

        fn capture_region(&mut self, _rect: Rect) -> Result<Frame, InputError> {
            self.roi_captures += 1;
            if self.clicks > 0 && self.clicks <= self.verified_clicks {
                Ok(self.portrait.clone())
            } else {
                Ok(self.blank_panel.clone())
            }
        }
    }

    fn timing() -> Timing {
        Timing::from_millis(1, 1)
    }

    #[test]
    fn cancellation_during_capture_is_reported_without_detection_or_input() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        fake.cancel_after_capture = true;
        let cancel = fake.cancel_flag();
        assert_eq!(
            scan_once(&mut fake, &cancel),
            Err(SpireScanError::Cancelled)
        );
        assert_eq!(fake.full_captures, 1);
        assert!(fake.events.is_empty());

        cancel.store(false, Ordering::SeqCst);
        let report = run(&mut fake, &cancel, timing());
        assert_eq!(report.outcome, SpireActionOutcome::Cancelled);
        assert_eq!(report.detections, 0);
        assert!(fake.events.is_empty());
        assert_eq!(fake.release_all_calls, 1);
    }

    #[test]
    fn cancellation_inside_the_safety_gate_prevents_the_next_event() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        fake.cancel_after_safety = Some(1);
        let cancel = fake.cancel_flag();
        assert_eq!(
            guard(&mut fake, &cancel),
            Err(SpireActionOutcome::Cancelled)
        );
        assert!(fake.events.is_empty());
    }

    #[test]
    fn every_detected_crown_is_clicked_verified_and_gets_exactly_one_a() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert_eq!(report.outcome, SpireActionOutcome::Completed);
        assert_eq!(report.detections, 4);
        assert_eq!(report.acted, 4);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.full_captures, 1, "exactly one full-screen capture");
        assert_eq!(report.roi_captures, 4, "one ROI read per verified target");
        assert_eq!(fake.count("down"), 4);
        assert_eq!(fake.count("up"), 4);
        assert_eq!(fake.count("key_down(A)"), 4, "A once per verified target");
        assert_eq!(fake.count("key_up(A)"), 4);
        // The A is only ever sent after the click: move, down, up, A.
        let first_a = fake
            .events()
            .iter()
            .position(|event| event == "key_down(A)")
            .expect("A");
        assert_eq!(
            &fake.events()[first_a - 3..first_a],
            ["move(790,158)", "down", "up"]
        );
    }

    #[test]
    fn a_partly_matching_scene_skips_only_the_unverified_targets() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        fake.verified_clicks = 2;
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert_eq!(report.outcome, SpireActionOutcome::Completed);
        assert_eq!(report.acted, 2);
        assert_eq!(report.skipped, 2);
        assert_eq!(fake.count("key_down(A)"), 2);
        assert_eq!(report.roi_captures, 2 + 2 * VERIFY_ATTEMPTS);
    }

    #[test]
    fn an_unverified_selection_never_receives_a() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        fake.verified_clicks = 0;
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert_eq!(report.outcome, SpireActionOutcome::Completed);
        assert_eq!(report.acted, 0);
        assert_eq!(report.skipped, 4);
        assert!(!fake.any_starts_with("key_down(A)"), "{:?}", fake.events());
        assert_eq!(report.roi_captures, 4 * VERIFY_ATTEMPTS);
    }

    #[test]
    fn no_detections_means_no_injected_events() {
        let negative = blit(&{
            let path = PathBuf::from("tests/fixtures/remastered-1080/drones-5.png");
            Frame::from_png(&path).expect("fixture")
        });
        let mut fake = Fake::on_screen(negative);
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert_eq!(report.outcome, SpireActionOutcome::Completed);
        assert_eq!(report.detections, 0);
        assert!(fake.events().is_empty(), "{:?}", fake.events());
        assert_eq!(report.full_captures, 1);
    }

    #[test]
    fn a_blank_capture_aborts_before_any_event() {
        let mut fake = Fake::on_screen(Frame::blank(1920, 1080));
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert!(matches!(report.outcome, SpireActionOutcome::Aborted { .. }));
        assert!(fake.events().is_empty());
    }

    #[test]
    fn a_wrong_profile_aborts_before_any_event() {
        let mut fake = Fake::on_screen(Frame::blank(1280, 720));
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert!(matches!(report.outcome, SpireActionOutcome::Aborted { .. }));
        assert_eq!(report.detections, 0);
        assert!(fake.events().is_empty());
    }

    #[test]
    fn a_cancel_before_the_scan_performs_no_capture_and_no_event() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        fake.cancel.store(true, Ordering::SeqCst);
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert_eq!(report.outcome, SpireActionOutcome::Cancelled);
        assert_eq!(report.full_captures, 0);
        assert!(fake.events().is_empty());
    }

    #[test]
    fn a_cancel_during_the_action_releases_everything_and_sends_no_a() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        fake.cancel_after_safety = Some(2);
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert_eq!(report.outcome, SpireActionOutcome::Cancelled);
        assert_eq!(report.acted, 0);
        assert!(!fake.any_starts_with("key_down(A)"), "{:?}", fake.events());
        assert_eq!(fake.release_all_calls, 1);
    }

    #[test]
    fn an_injection_failure_still_releases_every_owned_key() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        fake.fail_key_down = true;
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert!(
            matches!(report.outcome, SpireActionOutcome::Failed { .. }),
            "{:?}",
            report.outcome
        );
        assert_eq!(fake.release_all_calls, 1, "owned input must be released");
        assert!(fake.held.is_empty());
    }

    #[test]
    fn a_focus_change_after_the_snapshot_aborts_instead_of_clicking_stale_points() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        // The first safety check (before the cursor move) passes; every later
        // one fails, as if the game lost focus mid-run.
        fake.fail_safety_after = Some(1);
        let cancel = fake.cancel_flag();
        let report = run(&mut fake, &cancel, timing());
        assert!(
            matches!(report.outcome, SpireActionOutcome::Aborted { .. }),
            "{:?}",
            report.outcome
        );
        assert!(!fake.any_starts_with("down"), "{:?}", fake.events());
        assert!(!fake.any_starts_with("key_down"), "{:?}", fake.events());
        assert_eq!(fake.release_all_calls, 1);
    }

    #[test]
    fn scan_only_reports_locations_without_any_input() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        let cancel = fake.cancel_flag();
        let report = scan_once(&mut fake, &cancel).expect("scan");
        assert_eq!(report.count, 4);
        assert_eq!(report.full_captures, 1);
        assert_eq!(report.detections.len(), 4);
        assert!(fake.events().is_empty(), "scan-only must inject nothing");
        assert_eq!(fake.roi_captures, 0, "scan-only must not read any ROI");
        // "No held input": the preview never presses and therefore never has
        // to release anything.
        assert!(fake.held.is_empty(), "{:?}", fake.held);
        assert!(!fake.left_held);
        assert_eq!(fake.release_all_calls, 0);
    }

    #[test]
    fn a_scan_only_pass_never_touches_the_cursor_or_the_safety_gate() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        let cancel = fake.cancel_flag();
        scan_once(&mut fake, &cancel).expect("scan");
        assert_eq!(fake.safety_calls, 0, "no event means no gate call");
        assert_eq!(fake.full_captures, 1);
        assert!(!fake.any_starts_with("move"), "{:?}", fake.events());
    }

    #[test]
    fn a_scan_only_pass_cancelled_before_the_capture_scans_nothing() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        let cancel = fake.cancel_flag();
        cancel.store(true, Ordering::SeqCst);
        assert_eq!(
            scan_once(&mut fake, &cancel),
            Err(SpireScanError::Cancelled)
        );
        assert_eq!(fake.full_captures, 0, "nothing may be captured after F8");
        assert_eq!(fake.roi_captures, 0);
        assert!(fake.events().is_empty(), "{:?}", fake.events());
    }

    #[test]
    fn a_blank_capture_is_not_reported_as_zero_spires() {
        let mut fake = Fake::on_screen(Frame::blank(1920, 1080));
        let cancel = fake.cancel_flag();
        let error = scan_once(&mut fake, &cancel).expect_err("blank frame must not be a result");
        assert!(
            matches!(error, SpireScanError::Unusable { .. }),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("blank"),
            "the reason must be visible: {error}"
        );
        assert!(fake.events().is_empty(), "{:?}", fake.events());
        assert!(fake.held.is_empty());
    }

    #[test]
    fn an_unsupported_client_size_is_refused_instead_of_guessed() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        // A 1280x720 window must never be stretched into the calibrated grid.
        let smaller = {
            let source = fixture("screen.png");
            crop(&source, 0, 0, 1280, 720)
        };
        fake.screen = smaller;
        let cancel = fake.cancel_flag();
        let error = scan_once(&mut fake, &cancel).expect_err("only 1920x1080 is supported");
        match error {
            SpireScanError::Unusable { detail } => {
                assert!(detail.contains("1280x720"), "{detail}");
                assert!(detail.contains("1920x1080"), "{detail}");
            }
            other => panic!("expected an unusable-frame error, got {other:?}"),
        }
        assert!(fake.events().is_empty(), "{:?}", fake.events());
    }

    #[test]
    fn a_gate_refusal_is_reported_as_an_adapter_refusal() {
        let mut fake = Fake::on_screen(fixture("screen.png"));
        fake.capture_error = Some(InputError::Unsafe(
            "foreground window belongs to 'notepad.exe'".to_owned(),
        ));
        let cancel = fake.cancel_flag();
        let error = scan_once(&mut fake, &cancel).expect_err("the gate refused");
        match error {
            SpireScanError::Adapter(InputError::Unsafe(detail)) => {
                assert!(detail.contains("notepad.exe"), "{detail}");
            }
            other => panic!("expected an adapter refusal, got {other:?}"),
        }
        assert!(fake.events().is_empty(), "{:?}", fake.events());
        assert!(fake.held.is_empty());
    }

    #[test]
    fn a_supported_frame_without_crowns_truthfully_reports_zero() {
        // Non-blank, correctly sized, and genuinely without a Spire: "0 found"
        // is a measurement here, unlike on a blank or unsupported frame.
        let mut frame = Frame::blank(1920, 1080);
        for x in 0..64 {
            frame.set_pixel(x, 0, crate::frame::Rgb::new(255, 255, 255));
        }
        let mut fake = Fake::on_screen(frame);
        let cancel = fake.cancel_flag();
        let report = scan_once(&mut fake, &cancel).expect("a good frame is a result");
        assert_eq!(report.count, 0);
        assert_eq!(report.full_captures, 1);
        assert!(fake.events().is_empty(), "{:?}", fake.events());
    }
}
