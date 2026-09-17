use super::*;
use crate::frame::{Rect, Rgb};
use crate::input::InputAdapter;
use crate::test_support::Gate;
use crate::vision::synthetic::{PREVIEW_RED, paint_preview, paint_single, paint_slot};
use std::collections::HashSet;
use std::sync::{Arc, atomic::Ordering};

/// Dim terrain so a bounded probe region is never a blank capture; painted on
/// a 4-pixel grid, which is coarser than the module's 24-pixel sample grid and
/// finer than its 3-pixel preview scan.
const TERRAIN: Rgb = Rgb::new(30, 30, 30);
/// Terrain of a "the camera moved" frame: far enough from [`TERRAIN`] that the
/// scene-change comparison must notice it.
const MOVED_TERRAIN: Rgb = Rgb::new(90, 90, 90);

/// Test desktop drives the real HUD/preview detectors, not mocked verdicts.
/// Available to runner tests to verify the same worker and cleanup path.
pub(crate) struct Desktop {
    pub cancel: Arc<AtomicBool>,
    selected: u8,
    saved: u8,
    cursor: Point,
    held: HashSet<Key>,
    mouse: bool,
    preview: bool,
    keys: Vec<Key>,
    orders: Vec<Point>,
    removed: usize,
    released: bool,
    pub panic_capture: bool,
    captures: usize,
    cancel_capture: Option<usize>, // set through `cancel_after_captures`
    /// Runner-test hook: the worker blocks in this capture until the test
    /// opens the gate, so the test can latch F8 from the outside.
    hold: Option<(usize, Arc<Gate>)>,
    lose_focus_capture: Option<usize>,
    unsafe_now: bool,
    fail_key: Option<Key>,
    fail_release: bool,
    cancel_on_click: bool,
    stuck_preview: bool,
    morphed: bool,
    capacity: usize,
    blocked: Vec<Point>,
    preview_missing: bool,
    move_before_click: bool,
    wrong_profile: bool,
    drone: bool,
}
impl Desktop {
    /// Requests cancellation right after capture number `capture`. Only for
    /// unit tests that own the cancel flag; runner tests use `hold_capture`.
    pub(crate) fn cancel_after_captures(mut self, capture: usize) -> Self {
        self.cancel_capture = Some(capture);
        self
    }

    /// Blocks inside capture number `capture` until `gate` is opened.
    pub(crate) fn hold_capture(mut self, capture: usize, gate: Arc<Gate>) -> Self {
        self.hold = Some((capture, gate));
        self
    }

    pub(crate) fn new(count: u8) -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            selected: count,
            saved: 0,
            cursor: PARK,
            held: HashSet::new(),
            mouse: false,
            preview: false,
            keys: Vec::new(),
            orders: Vec::new(),
            removed: 0,
            released: false,
            panic_capture: false,
            captures: 0,
            cancel_capture: None,
            hold: None,
            lose_focus_capture: None,
            unsafe_now: false,
            fail_key: None,
            fail_release: false,
            cancel_on_click: false,
            stuck_preview: false,
            morphed: false,
            capacity: 12,
            blocked: Vec::new(),
            preview_missing: false,
            move_before_click: false,
            wrong_profile: false,
            drone: true,
        }
    }
}
impl InputAdapter for Desktop {
    fn key_down(&mut self, key: Key) -> Result<(), InputError> {
        assert!(
            !self.cancel.load(Ordering::SeqCst),
            "input after cancellation"
        );
        if self.fail_key == Some(key) {
            return Err(InputError::Injection("test key failure".into()));
        }
        self.keys.push(key);
        if key == Key::F4 {
            assert!(
                !self.held.contains(&Key::Shift),
                "must recall, never replace, F4"
            );
        }
        if key == Key::Nine {
            if self.held.contains(&Key::Control) {
                self.saved = self.selected;
            } else {
                self.selected = self.saved - u8::from(self.morphed && !self.orders.is_empty());
            }
        }
        if key == Key::C {
            assert_eq!(self.selected, 1, "building needs one verified drone");
            assert_eq!(self.keys[self.keys.len() - 2], Key::B);
            self.preview = true;
        }
        if key == Key::Escape {
            self.preview = false;
        }
        self.held.insert(key);
        Ok(())
    }
    fn key_up(&mut self, key: Key) -> Result<(), InputError> {
        self.held.remove(&key);
        Ok(())
    }
    fn mouse_left_down(&mut self) -> Result<(), InputError> {
        assert!(!self.cancel.load(Ordering::SeqCst));
        self.mouse = true;
        if self.cursor.y > 850 {
            assert_eq!(self.cursor, vision::slot_center(self.selected - 1));
            if self.held.contains(&Key::Shift) {
                self.selected -= 1;
                self.removed += 1;
            } else {
                self.selected = 1;
            }
        } else {
            assert!(self.preview && !self.preview_missing);
            assert!(self.orders.len() < self.capacity && !self.blocked.contains(&self.cursor));
            assert!(safe_footprint(self.cursor));
            self.orders.push(self.cursor);
            if !self.stuck_preview {
                self.preview = false;
            }
            if self.cancel_on_click {
                self.cancel.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }
    fn mouse_left_up(&mut self) -> Result<(), InputError> {
        self.mouse = false;
        Ok(())
    }
    fn release_all(&mut self) -> Result<(), InputError> {
        self.released = true;
        self.held.clear();
        self.mouse = false;
        if self.fail_release {
            Err(InputError::Injection("test cleanup failure".into()))
        } else {
            Ok(())
        }
    }
    fn safety_check(&mut self) -> Result<(), InputError> {
        if self.unsafe_now {
            Err(InputError::Unsafe("test focus loss".into()))
        } else {
            Ok(())
        }
    }
}
impl DesktopAdapter for Desktop {
    fn cursor_position(&mut self) -> Result<Point, InputError> {
        Ok(if self.move_before_click {
            self.cursor.offset(100, 0)
        } else {
            self.cursor
        })
    }
    fn move_cursor(&mut self, point: Point) -> Result<(), InputError> {
        self.cursor = point;
        Ok(())
    }
    fn capture_region(&mut self, rect: Rect) -> Result<Frame, InputError> {
        self.begin_capture();
        assert!(rect.w > 0 && rect.h > 0);
        Ok(self.paint(
            Frame::new(
                rect.w as u32,
                rect.h as u32,
                Point::new(rect.x, rect.y),
                vec![0; (rect.w * rect.h * 4) as usize],
            )
            .expect("bounded test frame"),
        ))
    }
    fn capture_client(&mut self) -> Result<Frame, InputError> {
        self.begin_capture();
        if self.wrong_profile {
            return Ok(Frame::blank(1280, 720));
        }
        let mut frame = Frame::blank(1920, 1080);
        if self.selected == 1 {
            paint_single(&mut frame, self.drone);
        } else {
            for slot in 0..self.selected {
                paint_slot(&mut frame, slot, self.drone);
            }
        }
        Ok(self.paint(frame))
    }
}

impl Desktop {
    /// Counts the read and applies the scripted cancellation/focus defects.
    fn begin_capture(&mut self) {
        assert!(!self.panic_capture, "test capture panic");
        self.captures += 1;
        if let Some((at, gate)) = &self.hold
            && *at == self.captures
        {
            gate.pass();
        }
        if self.cancel_capture == Some(self.captures) {
            self.cancel.store(true, Ordering::SeqCst);
        }
        if self.lose_focus_capture == Some(self.captures) {
            self.unsafe_now = true;
        }
    }

    /// Terrain plus the overlay this fake currently shows.
    fn paint(&self, mut frame: Frame) -> Frame {
        let origin = frame.origin();
        for y in (0..frame.height() as i32).step_by(4) {
            for x in (0..frame.width() as i32).step_by(4) {
                frame.set_pixel(x, y, TERRAIN);
            }
        }
        if self.preview && !self.preview_missing {
            let local = self.cursor.offset(-origin.x, -origin.y);
            if (0..frame.width() as i32).contains(&local.x)
                && (0..frame.height() as i32).contains(&local.y)
            {
                let green =
                    self.orders.len() < self.capacity && !self.blocked.contains(&self.cursor);
                paint_preview(&mut frame, local, green, PITCH);
            }
        }
        frame
    }
}

fn execute(fake: &mut Desktop, points: &[Point]) -> VacantColonyReport {
    let cancel = Arc::clone(&fake.cancel);
    let report = run_with_search(
        fake,
        &cancel,
        Timing::from_millis(0, 0),
        points,
        SEARCH_BUDGET,
    );
    assert!(
        fake.released && fake.held.is_empty() && !fake.mouse,
        "owned input cleanup"
    );
    report
}
const TWO: [Point; 2] = [Point::new(900, 400), Point::new(1044, 400)];

#[test]
fn candidates_are_unique_bounded_safe_and_can_fit_twelve() {
    let points = candidates();
    assert!(points.len() < 250 && points.len() > 100);
    let mut packed = Vec::new();
    for (i, point) in points.iter().enumerate() {
        assert!(safe_footprint(*point));
        assert!(!points[..i].contains(point));
        if unreserved(*point, &packed) {
            packed.push(*point);
        }
    }
    assert!(packed.len() >= 12);
    assert!(!safe_footprint(Point::new(200, 650)));
    assert!(!safe_footprint(Point::new(20, 300)));
    assert!(!safe_footprint(Point::new(1500, 100)));
    assert!(!unreserved(Point::new(500, 500), &[Point::new(600, 600)]));
}
#[test]
fn requires_fresh_green_in_all_quadrants_and_no_partial_red() {
    let before = Frame::blank(1920, 1080);
    let mut green = before.clone();
    paint_preview(&mut green, TWO[0], true, PITCH);
    let center = fresh_green(&before, &green, TWO[0], &[]).expect("fresh green");
    assert_eq!(
        fresh_green(&green, &green, TWO[0], &[]),
        None,
        "static terrain"
    );
    assert_eq!(
        fresh_green(&before, &green, TWO[0].offset(72, 0), &[]),
        None,
        "stale off-target frame"
    );
    assert_eq!(
        fresh_green(&before, &green, TWO[0], &[center]),
        None,
        "reserved footprint"
    );
    for y in 360..380 {
        for x in 860..880 {
            green.set_pixel(x, y, PREVIEW_RED);
        }
    }
    assert_eq!(
        fresh_green(&before, &green, TWO[0], &[]),
        None,
        "partial blocked tile"
    );
}
#[test]
fn scene_changes_are_refused_but_animation_near_the_preview_is_ignored() {
    let probe = |terrain: Rgb, origin: Point| {
        let mut frame = Frame::new(480, 480, origin, vec![0; 480 * 480 * 4]).expect("probe frame");
        for y in (0..480).step_by(4) {
            for x in (0..480).step_by(4) {
                frame.set_pixel(x, y, terrain);
            }
        }
        frame
    };
    let origin = Point::new(760, 160);
    let center = origin.offset(240, 240);
    let same = probe(TERRAIN, origin);
    assert!(
        same_view(&same, &same, &[center]),
        "an unchanged scene passes"
    );

    // The moved frame differs exactly on the 24-pixel sample grid, but only in
    // the ring outside the preview area, so the comparison still has to see it.
    let moved = probe(MOVED_TERRAIN, origin);
    assert!(
        !same_view(&same, &moved, &[center]),
        "a camera move must be seen"
    );

    // A different probe area or a different frame size can never be compared.
    assert!(!same_view(
        &same,
        &probe(TERRAIN, origin.offset(24, 0)),
        &[center]
    ));
    assert!(!same_view(&same, &Frame::blank(480, 240), &[center]));

    // A freshly ordered building and the probe itself are skipped: a frame that
    // only differs there still counts as the same view.
    // 240x240 local pixels is a quarter of the probe area: more than the
    // tolerance allows, so only the skip list can excuse it.
    let mut local_change = probe(TERRAIN, origin);
    for y in 0..240 {
        for x in 0..240 {
            local_change.set_pixel(x, y, MOVED_TERRAIN);
        }
    }
    let skipped = origin.offset(120, 120);
    assert!(
        !same_view(&same, &local_change, &[]),
        "not skipped: noticed"
    );
    assert!(
        same_view(&same, &local_change, &[skipped]),
        "the pixels our own overlay covers must be ignored"
    );
}

#[test]
fn uses_two_drones_and_recalls_f4_without_overwriting_it() {
    let mut fake = Desktop::new(2);
    let report = execute(&mut fake, &TWO);
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(report.orders.len(), 2);
    assert_eq!(report.detected, 2);
    assert_eq!(fake.keys.iter().filter(|k| **k == Key::F4).count(), 2);
    assert_eq!(fake.removed, 1);
    assert_eq!(fake.keys.first(), Some(&Key::F4));
}
#[test]
fn all_twelve_drones_are_used_without_the_single_row_cap() {
    let mut fake = Desktop::new(12);
    let report = execute(&mut fake, &candidates());
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(report.orders.len(), 12);
    assert_eq!(report.detected, 12);
    for (i, p) in report.orders.iter().enumerate() {
        assert!(unreserved(*p, &report.orders[..i]));
    }
}
#[test]
fn eleven_already_morphing_drones_do_not_remove_the_next_drone() {
    let mut fake = Desktop::new(11);
    fake.morphed = true;
    let report = execute(&mut fake, &candidates());
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(report.orders.len(), 11);
    assert_eq!(fake.removed, 0);
}
#[test]
fn blocked_candidate_is_skipped_without_a_world_click() {
    let mut fake = Desktop::new(2);
    fake.blocked = vec![TWO[0]];
    let points = [TWO[0], TWO[1], Point::new(1188, 400)];
    let report = execute(&mut fake, &points);
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(fake.orders, points[1..]);
    assert_eq!(report.probes, 3);
}
#[test]
fn no_creep_or_no_free_space_never_forces_a_click() {
    let mut fake = Desktop::new(2);
    fake.capacity = 0;
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert!(report.orders.is_empty());
    assert_eq!(report.probes, TWO.len());
    assert_eq!(
        fake.keys.last(),
        Some(&Key::Escape),
        "close visible blocked preview"
    );
}
#[test]
fn lack_of_resources_or_absent_preview_does_not_click_or_guess_escape() {
    let mut fake = Desktop::new(2);
    fake.preview_missing = true;
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert!(report.orders.is_empty());
    assert!(!fake.keys.contains(&Key::Escape));
}
#[test]
fn partial_run_reports_issued_orders_not_completed_buildings() {
    let mut fake = Desktop::new(2);
    fake.capacity = 1;
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert_eq!(report.orders.len(), 1);
    assert_eq!(report.detected, 2);
}
#[test]
fn stuck_preview_stops_after_one_order_without_retrying_the_click() {
    let mut fake = Desktop::new(2);
    fake.stuck_preview = true;
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert_eq!(report.orders.len(), 1);
    assert_eq!(fake.keys.last(), Some(&Key::Escape));
}
#[test]
fn invalid_selection_and_geometry_send_no_keys_or_clicks() {
    for count in [0, 1] {
        let mut fake = Desktop::new(count);
        let report = execute(&mut fake, &TWO);
        assert!(matches!(report.outcome, Outcome::Aborted { .. }));
        assert!(fake.keys.is_empty() && fake.orders.is_empty());
    }
    for wrong_profile in [false, true] {
        let mut fake = Desktop::new(2);
        fake.drone = false;
        fake.wrong_profile = wrong_profile;
        let report = execute(&mut fake, &TWO);
        assert!(matches!(report.outcome, Outcome::Aborted { .. }));
        assert!(fake.keys.is_empty() && fake.orders.is_empty());
    }
}
#[test]
fn cancellation_after_either_preview_capture_sends_no_world_click() {
    for capture in [1, 6, 7] {
        let mut fake = Desktop::new(2).cancel_after_captures(capture);
        let report = execute(&mut fake, &TWO);
        assert_eq!(report.outcome, Outcome::Cancelled, "capture {capture}");
        assert!(report.orders.is_empty());
        assert!(!fake.keys.contains(&Key::Escape));
    }
}
#[test]
fn focus_loss_after_verification_blocks_click_and_escape() {
    let mut fake = Desktop::new(2);
    fake.lose_focus_capture = Some(7);
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Failed { .. }));
    assert!(report.orders.is_empty());
    assert!(!fake.keys.contains(&Key::Escape));
}
#[test]
fn emergency_during_world_mouse_down_counts_once_and_releases_button() {
    let mut fake = Desktop::new(2);
    fake.cancel_on_click = true;
    let report = execute(&mut fake, &TWO);
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert_eq!(report.orders.len(), 1);
}
#[test]
fn mouse_interference_does_not_click_at_the_wrong_location() {
    let mut fake = Desktop::new(2);
    fake.move_before_click = true;
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert!(report.orders.is_empty());
}
#[test]
fn injection_and_cleanup_errors_are_reported() {
    let mut fake = Desktop::new(2);
    fake.fail_key = Some(Key::C);
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Failed { .. }));
    assert!(report.orders.is_empty());
    let mut fake = Desktop::new(0);
    fake.fail_release = true;
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Failed { .. }));
}
#[test]
fn expired_search_budget_is_reported_without_a_build_order() {
    let mut fake = Desktop::new(2);
    let cancel = Arc::clone(&fake.cancel);
    let report = run_with_search(
        &mut fake,
        &cancel,
        Timing::from_millis(0, 0),
        &TWO,
        Duration::ZERO,
    );
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert!(report.orders.is_empty() && fake.released);
    assert!(!fake.keys.contains(&Key::B));
}
