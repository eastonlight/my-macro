//! The Spire action: one capture, click each detected crown, verify, press `A`.
//!
//! This is the Spire wrapper around the shared
//! [`crate::building_action`] executor, which documents the flow and the safety
//! rules (one full capture and one full search per run, a safety gate before
//! every event, bounded selection-panel reads, no `A` without a verified
//! selection, cancellation, cleanup). The Stargate action uses the same
//! executor with its own profile.
//!
//! Everything talks to [`DesktopAdapter`], so the whole flow is exercised on
//! any host with a fake capture/input adapter.

use std::sync::atomic::AtomicBool;

use crate::building_action::{self, BuildingActionReport};
use crate::input::DesktopAdapter;
use crate::macros::Timing;

pub use crate::building_action::{
    BuildingActionOutcome, BuildingScanError, BuildingScanReport, BuildingTargetDisposition,
    BuildingTargetReport, VERIFY_ATTEMPTS,
};

/// How one Spire action run ended.
pub type SpireActionOutcome = BuildingActionOutcome;
/// What happened to one detected crown.
pub type TargetDisposition = BuildingTargetDisposition;
/// Per-target detail.
pub type SpireTargetReport = BuildingTargetReport;
/// Everything one Spire action run did, ready to log or display.
pub type SpireActionReport = BuildingActionReport;
/// A read-only Spire scan result.
pub type SpireScanReport = BuildingScanReport;
/// Why a Spire scan-only pass produced no trustworthy result.
pub type SpireScanError = BuildingScanError;

/// A report for a Spire run that never started (internal failure).
pub fn failed(detail: impl Into<String>) -> SpireActionReport {
    BuildingActionReport::failed(detail, "Spire")
}

/// Scan-only mode: one capture and one full-screen search, **zero** injected
/// events. Use it to preview where the Spire action would click.
pub fn scan_once(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
) -> Result<SpireScanReport, SpireScanError> {
    building_action::scan_once(adapter, cancel, &crate::spire_vision::PROFILE)
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
    building_action::run(adapter, cancel, timing, &crate::spire_vision::PROFILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building_action::guard;
    use crate::frame::{Frame, Point, Rect};
    use crate::input::{InputAdapter, InputError};
    use crate::macros::Key;
    use crate::spire_vision::PORTRAIT_ROI;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

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
