use super::*;
use crate::frame::{Rect, Rgb};
use crate::input::InputAdapter;
use crate::test_support::Gate;
use crate::vision::synthetic::{
    PREVIEW_RED, paint_morph_colony, paint_preview, paint_single, paint_slot,
};
use std::collections::HashSet;
use std::sync::{Arc, atomic::Ordering};

/// Dim terrain so a bounded probe region is never a blank capture; painted on
/// a 4-pixel grid, which is coarser than the module's 24-pixel sample grid and
/// finer than its 3-pixel preview scan.
const TERRAIN: Rgb = Rgb::new(30, 30, 30);
/// Terrain of a "the camera moved" frame: far enough from [`TERRAIN`] that the
/// scene-change comparison must notice it.
const MOVED_TERRAIN: Rgb = Rgb::new(90, 90, 90);

/// What the fake HUD shows after a world order, while the ordered drone is
/// still the selected unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AfterOrderPanel {
    /// The drone reached the site and the panel becomes a morphing Colony.
    MorphingColony,
    /// The panel keeps showing the ordered drone (traveling or never starting).
    StillDrone,
    /// A lone wireframe portrait: the group state changed, not a start.
    SingleWireframe,
    /// A two-drone group: a smaller control group, not a start.
    TwoDrones,
    /// Another single unit's panel, without the morph bar.
    OtherUnit,
}

/// One observed fake-side effect, so tests can prove the per-drone order of
/// "issue order" and "see the morph panel".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    Order(Point),
    MorphPanel,
    DronePanel,
    UnconfirmedPanel,
}

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
    /// How far the game snaps the preview away from the cursor, modelling the
    /// map's tile grid. The click still lands on the preview's footprint.
    preview_offset: Point,
    /// What the panel shows after a world order, and how many full-client
    /// reads it takes before the morph panel appears (travel time).
    panel: AfterOrderPanel,
    morph_delay: usize,
    panel_reads: usize,
    /// True while the single selected unit is the drone that was just ordered
    /// (the recall or a new portrait click clears it).
    ordered_selected: bool,
    events: Vec<Event>,
    /// World reads so far, used to script a temporary obstruction.
    region_reads: usize,
    transient_red: Option<(Point, usize)>,
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

    /// What the fake HUD shows after a world order.
    pub(crate) fn after_order_panel(mut self, panel: AfterOrderPanel) -> Self {
        self.panel = panel;
        self
    }

    /// Full-client reads after an order before the morph panel appears.
    pub(crate) fn morph_after_reads(mut self, reads: usize) -> Self {
        self.morph_delay = reads;
        self
    }

    /// Models a unit that temporarily blocks one probe point: its preview is
    /// red until `reads` world captures have happened.
    pub(crate) fn blocked_until_reads(mut self, point: Point, reads: usize) -> Self {
        self.transient_red = Some((point, reads));
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
            preview_offset: Point::new(0, 0),
            panel: AfterOrderPanel::MorphingColony,
            morph_delay: 0,
            panel_reads: 0,
            ordered_selected: false,
            events: Vec::new(),
            region_reads: 0,
            transient_red: None,
        }
    }

    /// Models the game snapping the 2x2 footprint to its tile grid.
    pub(crate) fn snapped_by(mut self, dx: i32, dy: i32) -> Self {
        self.preview_offset = Point::new(dx, dy);
        self
    }

    /// The point where this fake actually draws the preview.
    fn preview_center(&self) -> Point {
        self.cursor
            .offset(self.preview_offset.x, self.preview_offset.y)
    }

    /// True while a scripted temporary obstruction covers the current probe.
    fn transient_block(&self) -> bool {
        let Some((point, reads)) = self.transient_red else {
            return false;
        };
        self.cursor == point && self.region_reads < reads
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
                // A recall selects the remaining drones, never the ordered one.
                self.ordered_selected = false;
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
            self.ordered_selected = false;
        } else {
            assert!(self.preview && !self.preview_missing);
            let center = self.preview_center();
            assert!(
                self.orders.len() < self.capacity
                    && !self.blocked.contains(&center)
                    && !self.transient_block()
            );
            assert!(safe_footprint(self.cursor));
            // The building lands where the preview is drawn, not where the
            // cursor happens to be inside the footprint.
            self.orders.push(center);
            self.events.push(Event::Order(center));
            self.panel_reads = 0;
            self.ordered_selected = true;
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
        self.region_reads += 1;
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
        if self.selected == 1 && self.ordered_selected {
            self.panel_reads += 1;
            if self.panel_reads > self.morph_delay {
                match self.panel {
                    AfterOrderPanel::MorphingColony => {
                        paint_morph_colony(&mut frame);
                        self.events.push(Event::MorphPanel);
                        self.morphed = true;
                    }
                    AfterOrderPanel::StillDrone => {
                        paint_single(&mut frame, true);
                        self.events.push(Event::DronePanel);
                    }
                    AfterOrderPanel::SingleWireframe => {
                        paint_slot(&mut frame, 0, true);
                        self.events.push(Event::UnconfirmedPanel);
                    }
                    AfterOrderPanel::TwoDrones => {
                        for slot in 0..2 {
                            paint_slot(&mut frame, slot, true);
                        }
                        self.events.push(Event::UnconfirmedPanel);
                    }
                    AfterOrderPanel::OtherUnit => {
                        paint_single(&mut frame, false);
                        self.events.push(Event::UnconfirmedPanel);
                    }
                }
            } else {
                paint_single(&mut frame, true);
                self.events.push(Event::DronePanel);
            }
        } else if self.selected == 1 {
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
        // A started building changes its own world footprint, not just HUD.
        if self.morphed
            && let Some(center) = self.orders.last()
        {
            for y in center.y - 24..center.y + 24 {
                for x in center.x - 24..center.x + 24 {
                    frame.set_pixel(x - origin.x, y - origin.y, Rgb::new(130, 90, 70));
                }
            }
        }
        if self.preview && !self.preview_missing {
            let center = self.preview_center();
            let local = center.offset(-origin.x, -origin.y);
            if (0..frame.width() as i32).contains(&local.x)
                && (0..frame.height() as i32).contains(&local.y)
            {
                let green = self.orders.len() < self.capacity
                    && !self.blocked.contains(&center)
                    && !self.transient_block();
                paint_preview(&mut frame, local, green, PITCH);
            }
        }
        frame
    }
}

/// Test budgets: a full search window, but only a few seconds of construction
/// wait so a never-starting fake cannot slow the suite down.
fn budgets() -> Budgets {
    Budgets {
        search: SEARCH_BUDGET,
        construction: Duration::from_secs(2),
        run: RUN_BUDGET,
    }
}

/// Short construction window for the tests that exercise the timeout.
fn short_construction() -> Budgets {
    Budgets {
        construction: Duration::from_millis(50),
        ..budgets()
    }
}

fn execute(fake: &mut Desktop, points: &[Point]) -> VacantColonyReport {
    execute_with_progress(fake, points).0
}

/// Same, but the caller also gets the live counters the GUI reads.
fn execute_with_progress(
    fake: &mut Desktop,
    points: &[Point],
) -> (VacantColonyReport, VacantProgress) {
    execute_with_budgets(fake, points, budgets())
}

fn execute_with_budgets(
    fake: &mut Desktop,
    points: &[Point],
    budgets: Budgets,
) -> (VacantColonyReport, VacantProgress) {
    let cancel = Arc::clone(&fake.cancel);
    let progress = VacantProgress::default();
    let report = run_with_search(
        fake,
        &cancel,
        Timing::from_millis(0, 0),
        points,
        budgets,
        &progress,
    );
    assert!(
        fake.released && fake.held.is_empty() && !fake.mouse,
        "owned input cleanup"
    );
    (report, progress)
}
const TWO: [Point; 2] = [Point::new(900, 400), Point::new(1044, 400)];

/// The preview detector reports the painted square's bounding-box centre,
/// which can sit one pixel from the painted centre.
fn assert_near(actual: Point, expected: Point, context: &str) {
    assert!(
        (actual.x - expected.x).abs() <= 1 && (actual.y - expected.y).abs() <= 1,
        "{context}: {actual:?} vs {expected:?}"
    );
}

#[test]
fn candidates_are_unique_bounded_safe_and_read_from_the_lower_left() {
    let points = candidates();
    assert!(points.len() < 250 && points.len() > 100);
    assert!(points.iter().all(|point| safe_footprint(*point)));
    let unique = {
        let mut sorted = points.clone();
        sorted.sort_by_key(|point| (point.x, point.y));
        sorted.dedup();
        sorted.len()
    };
    assert_eq!(unique, points.len(), "no candidate twice");
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        assert!(
            b.y < a.y || (b.y == a.y && b.x > a.x),
            "reading order must go left to right, then up: {a:?} -> {b:?}"
        );
    }
    // The first row is the lowest row the footprint allows, scanned left to
    // right; the next row is above it.
    let (first, second) = (points[0], points[1]);
    assert_eq!(second.y, first.y);
    assert!(second.x > first.x);
    let next_row = points
        .iter()
        .find(|point| point.y < first.y)
        .expect("a row above the first");
    assert!(next_row.y < first.y);

    // The order can still fit all twelve buildings.
    let mut packed = Vec::new();
    for point in &points {
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
fn same_view_ignores_the_hud_and_minimap_but_sees_world_changes() {
    // Two full frames with identical world terrain but different HUD/minimap
    // pixels: only the world samples may decide, so this must pass even though
    // the panels change completely.
    let world = |hud: Rgb| {
        let mut frame = Frame::blank(1920, 1080);
        for y in (0..1080).step_by(4) {
            for x in (0..1920).step_by(4) {
                frame.set_pixel(x, y, TERRAIN);
            }
        }
        for y in (700..1080).step_by(4) {
            for x in (0..520).step_by(4) {
                frame.set_pixel(x, y, hud); // minimap console
            }
        }
        for y in (880..1080).step_by(4) {
            for x in (600..1160).step_by(4) {
                frame.set_pixel(x, y, hud); // selection panel
            }
        }
        frame
    };
    let before = world(TERRAIN);
    let after = world(MOVED_TERRAIN);
    assert!(
        same_view(&before, &after, &[]),
        "HUD and minimap changes are not camera moves"
    );

    // A real world change above the HUD skyline is still refused.
    let mut moved_world = before.clone();
    for y in (100..600).step_by(4) {
        for x in (100..1800).step_by(4) {
            moved_world.set_pixel(x, y, MOVED_TERRAIN);
        }
    }
    assert!(
        !same_view(&before, &moved_world, &[]),
        "a world change must be seen"
    );
}
#[test]
fn uses_two_drones_and_recalls_f4_without_overwriting_it() {
    let mut fake = Desktop::new(2);
    let report = execute(&mut fake, &TWO);
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(report.orders.len(), 2);
    assert_eq!(
        report.starts, report.orders,
        "both drones were seen starting"
    );
    assert_eq!(report.detected, 2);
    assert_eq!(fake.keys.iter().filter(|k| **k == Key::F4).count(), 2);
    // The morph was confirmed before the group was recalled, so the game had
    // already dropped the ordered drone and no portrait removal was needed.
    assert_eq!(fake.removed, 0);
    assert_eq!(fake.keys.first(), Some(&Key::F4));
}
#[test]
fn all_twelve_drones_are_used_without_the_single_row_cap() {
    let mut fake = Desktop::new(12);
    let report = execute(&mut fake, &candidates());
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(report.orders.len(), 12);
    assert_eq!(report.starts.len(), 12);
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
    // The blocked point is probed once per drone search that reaches it.
    assert_eq!(report.probes, 4);
    assert_eq!(report.starts.len(), 2);
}
#[test]
fn an_earlier_candidate_is_revisited_after_a_transient_obstruction() {
    // A unit passes over the first candidate while the first drone scans, so
    // that drone takes the second spot. The next drone must come back to the
    // earlier candidate instead of dropping it permanently.
    let mut fake = Desktop::new(2).blocked_until_reads(TWO[0], 2);
    let report = execute(&mut fake, &TWO);
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(fake.orders, [TWO[1], TWO[0]]);
    assert_eq!(report.starts.len(), 2);
}
#[test]
fn a_blocked_candidate_is_revisited_once_before_it_is_dropped() {
    // Nothing else is free: the search must revisit its own failure once
    // before giving up, and the obstruction is gone by then.
    let mut fake = Desktop::new(2)
        .blocked_until_reads(TWO[0], 2)
        .after_order_panel(AfterOrderPanel::MorphingColony);
    fake.blocked = vec![TWO[1]];
    // The default construction window: this test is about the retry, and the
    // fake shows the morph panel immediately.
    let report = execute(&mut fake, &TWO);
    assert_eq!(report.orders.len(), 1, "{report:?}");
    assert_near(report.orders[0], TWO[0], "ordered the revisited candidate");
    assert_eq!(report.starts.len(), 1);
    assert_near(
        report.starts[0],
        TWO[0],
        "start evidence for the same order",
    );
    // The first drone probes both candidates, revisits the obstructed one,
    // then the second drone spends one probe on the still-blocked candidate.
    assert_eq!(report.probes, 4, "one revisit of the blocked first pass");
}
#[test]
fn a_free_spot_far_to_the_right_is_still_reached_in_the_first_pass() {
    // The whole lower row up to x=1488 is blocked, so the only free spot sits
    // six tiles further right. The ordered scan must reach it in one pass:
    // reserving probes for the delayed revisit must not truncate the reading
    // order, or a view with space would report "no free position".
    let blocked: Vec<Point> = (0..20).map(|i| Point::new(120 + i * 72, 400)).collect();
    let free = Point::new(1776, 400);
    assert!(blocked.iter().all(|point| safe_footprint(*point)));
    assert!(safe_footprint(free));
    let mut fake = Desktop::new(2);
    fake.blocked = blocked.clone();
    let points: Vec<Point> = blocked.into_iter().chain([free]).collect();
    let report = execute(&mut fake, &points);
    assert_eq!(fake.orders.len(), 1, "{report:?}");
    assert_near(fake.orders[0], free, "the far free spot was found");
    assert_eq!(report.starts.len(), 1, "{report:?}");
    // 21 probes in the first pass reach index 20 (the free spot); the second
    // drone then scans the same blocked row and its single revisit fails.
    assert_eq!(report.probes, 21 + 20, "{report:?}");
}

#[test]
fn no_creep_or_no_free_space_never_forces_a_click() {
    let mut fake = Desktop::new(2);
    fake.capacity = 0;
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert!(report.orders.is_empty());
    assert!(report.starts.is_empty());
    // Two candidates, each probed in the first pass and once revisited.
    assert_eq!(report.probes, 4);
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
    assert_eq!(report.starts.len(), 1);
    assert_eq!(report.detected, 2);
    // The second drone was never ordered, so "starts" cannot equal "detected".
    assert!(report.starts.len() < report.detected as usize);
}
#[test]
fn stuck_preview_stops_after_one_order_without_retrying_the_click() {
    let mut fake = Desktop::new(2);
    fake.stuck_preview = true;
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert_eq!(report.orders.len(), 1);
    assert!(report.starts.is_empty(), "no start evidence was shown");
    assert_eq!(fake.orders.len(), 1, "no re-click of the stuck order");
    assert_eq!(fake.keys.last(), Some(&Key::Escape));
}
#[test]
fn a_never_starting_drone_stops_with_a_partial_report_and_keeps_the_reservation() {
    let mut fake = Desktop::new(2).after_order_panel(AfterOrderPanel::StillDrone);
    let (report, progress) = execute_with_budgets(&mut fake, &TWO, short_construction());
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    // The issued order and its reservation survive the failed confirmation.
    assert_eq!(report.orders.len(), 1, "{report:?}");
    assert_near(report.orders[0], TWO[0], "the reservation is kept");
    assert_eq!(fake.orders, [TWO[0]]);
    assert!(report.starts.is_empty());
    assert_eq!(progress.orders(), 1);
    assert_eq!(progress.starts(), 0);
    // One B/C pair only: the next drone was never touched, and the click was
    // never retried.
    assert_eq!(fake.keys.iter().filter(|k| **k == Key::B).count(), 1);
    assert_eq!(fake.keys.iter().filter(|k| **k == Key::C).count(), 1);
}
#[test]
fn departure_preview_closure_and_smaller_groups_are_not_start_evidence() {
    for panel in [
        AfterOrderPanel::StillDrone,
        AfterOrderPanel::SingleWireframe,
        AfterOrderPanel::TwoDrones,
        AfterOrderPanel::OtherUnit,
    ] {
        let mut fake = Desktop::new(2).after_order_panel(panel);
        let (report, _) = execute_with_budgets(&mut fake, &TWO, short_construction());
        assert!(
            matches!(report.outcome, Outcome::Aborted { .. }),
            "{panel:?}: {:?}",
            report.outcome
        );
        assert_eq!(report.orders.len(), 1, "{panel:?}");
        assert!(report.starts.is_empty(), "{panel:?}: no positive start");
        assert_eq!(fake.orders.len(), 1, "{panel:?}");
        assert!(
            !fake.keys.contains(&Key::Escape),
            "{panel:?}: the preview was already closed"
        );
    }
}
#[test]
fn no_second_order_before_the_first_morph_is_confirmed() {
    // The fake keeps showing the selected drone for two reads after the order
    // (travel time), then the morph panel. The second drone must not be
    // ordered before two consecutive morph reads are seen.
    let mut fake = Desktop::new(2).morph_after_reads(2);
    let report = execute(&mut fake, &TWO);
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(report.starts.len(), 2);

    let first_order = fake
        .events
        .iter()
        .position(|event| matches!(event, Event::Order(_)))
        .expect("first order");
    let second_order = fake
        .events
        .iter()
        .skip(first_order + 1)
        .position(|event| matches!(event, Event::Order(_)))
        .map(|index| index + first_order + 1)
        .expect("second order");
    let morphs = fake
        .events
        .iter()
        .enumerate()
        .filter(|(index, event)| {
            *index > first_order && *index < second_order && **event == Event::MorphPanel
        })
        .count();
    assert!(
        morphs >= CONSTRUCTION_CONFIRM_READS,
        "the second order arrived after {morphs} morph reads: {:?}",
        fake.events
    );
    let drone_reads = fake
        .events
        .iter()
        .enumerate()
        .filter(|(index, event)| {
            *index > first_order && *index < second_order && **event == Event::DronePanel
        })
        .count();
    assert!(
        drone_reads >= 2,
        "the wait must actually observe the traveling drone first: {:?}",
        fake.events
    );
}
#[test]
fn twelve_drones_confirm_each_morph_before_the_next_order() {
    let mut fake = Desktop::new(12);
    let report = execute(&mut fake, &candidates());
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(report.orders.len(), 12);
    assert_eq!(report.starts.len(), 12);
    // Every order except the last is followed by at least two morph reads
    // before the next order.
    let orders: Vec<usize> = fake
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event, Event::Order(_)))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(orders.len(), 12);
    for pair in orders.windows(2) {
        let morphs = fake.events[pair[0] + 1..pair[1]]
            .iter()
            .filter(|event| **event == Event::MorphPanel)
            .count();
        assert!(
            morphs >= CONSTRUCTION_CONFIRM_READS,
            "order at {} was followed by {morphs} morph reads",
            pair[0]
        );
    }
}
#[test]
fn construction_wait_cancellation_and_focus_loss_are_bounded() {
    // Capture 10 is the first construction panel read after the order.
    let mut fake = Desktop::new(2).cancel_after_captures(10);
    let report = execute_with_budgets(&mut fake, &TWO, short_construction()).0;
    assert_eq!(report.outcome, Outcome::Cancelled, "{report:?}");
    assert_eq!(report.orders.len(), 1, "the issued order is kept");
    assert_near(report.orders[0], TWO[0], "the issued order is kept");
    assert!(report.starts.is_empty());

    let mut fake = Desktop::new(2);
    fake.lose_focus_capture = Some(11);
    let report = execute_with_budgets(&mut fake, &TWO, short_construction()).0;
    assert!(
        matches!(report.outcome, Outcome::Failed { .. }),
        "{report:?}"
    );
    assert_eq!(report.orders.len(), 1);
    assert_near(
        report.orders[0],
        TWO[0],
        "the reservation survives focus loss",
    );
    assert!(report.starts.is_empty());
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
    // Captures 1-7 are the reads before the first world click: the initial
    // selection, the saved view, the F4 count, the single-drone check, the
    // baseline, and the two preview frames.
    for capture in [1, 5, 6, 7] {
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
        Budgets {
            search: Duration::ZERO,
            ..budgets()
        },
        &VacantProgress::default(),
    );
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert!(report.orders.is_empty() && fake.released);
    assert!(!fake.keys.contains(&Key::B));
}

#[test]
fn a_preview_snapped_by_up_to_one_tile_is_still_accepted() {
    // The game snaps a 2x2 footprint to its tile grid, so the preview can sit
    // almost a whole tile away from the probe point. That is not a different
    // footprint: the click still lands on the previewed one.
    let mut fake = Desktop::new(2).snapped_by(60, -55);
    let report = execute(&mut fake, &TWO);
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(report.orders.len(), 2);
    // The detector reports the centre of the painted square's bounding box,
    // which is at most one pixel off the painted centre.
    for (order, probe) in report.orders.iter().zip(TWO.iter()) {
        let snapped = probe.offset(60, -55);
        assert!((order.x - snapped.x).abs() <= 1, "{order:?} vs {snapped:?}");
        assert!((order.y - snapped.y).abs() <= 1, "{order:?} vs {snapped:?}");
    }
    // The snapped centres, not the probe points, keep the next order apart.
    assert!(unreserved(report.orders[1], &report.orders[..1]));
}

#[test]
fn a_green_square_more_than_a_tile_away_belongs_to_another_footprint() {
    // Beyond one tile the detected square cannot be this candidate's preview,
    // so the candidate is skipped instead of being clicked.
    let mut fake = Desktop::new(2).snapped_by(120, 0);
    let report = execute(&mut fake, &TWO);
    assert!(matches!(report.outcome, Outcome::Aborted { .. }));
    assert!(report.orders.is_empty());
    assert_eq!(report.probes, 4, "both candidates, first pass and revisit");
    // B/C were pressed for the drone, but no world click was ever sent.
    assert_eq!(fake.keys.iter().filter(|k| **k == Key::B).count(), 1);
    assert!(fake.orders.is_empty());
}

#[test]
fn a_search_that_never_confirms_a_preview_stops_early_with_a_diagnostic() {
    // A detector that matches nothing (wrong skin, wrong game, occluded
    // desktop) must not sweep the whole screen: it stops and says so.
    let mut fake = Desktop::new(2);
    fake.preview_missing = true;
    let report = execute(&mut fake, &candidates());
    let Outcome::Aborted { detail } = &report.outcome else {
        panic!("expected an early stop, got {:?}", report.outcome);
    };
    assert!(
        detail.contains("no green placement preview was confirmed"),
        "{detail}"
    );
    assert_eq!(
        report.probes, NO_CONFIRM_GIVE_UP,
        "it stops at the give-up probe count, not after the whole screen"
    );
    assert!(report.orders.is_empty());
}

#[test]
fn the_per_drone_probe_limit_stops_a_long_search() {
    // One order succeeds, then the view has no room left. A dense grid offers
    // far more candidates than one drone may probe, so the second search ends
    // at the per-drone limit instead of walking every point twice.
    let mut fake = Desktop::new(2);
    fake.capacity = 1;
    let dense: Vec<Point> = (180..720)
        .step_by(36)
        .flat_map(|y| (120..1800).step_by(36).map(move |x| Point::new(x, y)))
        .filter(|point| safe_footprint(*point))
        .collect();
    assert!(
        dense.len() > MAX_PROBES_PER_DRONE + 16,
        "the test needs more candidates than the limit: {}",
        dense.len()
    );
    let report = execute(&mut fake, &dense);
    let Outcome::Aborted { detail } = &report.outcome else {
        panic!("expected a bounded stop, got {:?}", report.outcome);
    };
    assert!(detail.contains("probe limit"), "{detail}");
    assert_eq!(report.orders.len(), 1, "the one real spot was still built");
    assert_eq!(report.probes, MAX_PROBES_PER_DRONE + 1);
}

#[test]
fn the_probe_limits_are_gated_before_every_probe() {
    let search = Search::new(&TWO, budgets());
    let mut report = VacantColonyReport {
        outcome: Outcome::Completed,
        detected: 2,
        orders: Vec::new(),
        starts: Vec::new(),
        probes: MAX_PROBES,
    };
    let Outcome::Aborted { detail } = search.budget_gate(&report, 0).unwrap_err() else {
        panic!("the run cap must abort");
    };
    assert!(detail.contains("probe limit"), "{detail}");

    report.probes = 0;
    let Outcome::Aborted { detail } = search
        .budget_gate(&report, MAX_PROBES_PER_DRONE)
        .unwrap_err()
    else {
        panic!("the per-drone cap must abort");
    };
    assert!(detail.contains("probe limit"), "{detail}");

    search.budget_gate(&report, 0).expect("under both caps");
}

/// Existing, previously captured game images only. No live desktop access.
fn real_preview_fixture(name: &str) -> Frame {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/vacant-preview-1080")
        .join(name);
    let image = Frame::from_png(&path).expect("real preview fixture");
    let mut rgba = Vec::new();
    for y in 0..image.height() as i32 {
        for x in 0..image.width() as i32 {
            let p = image.pixel(x, y).expect("fixture pixel");
            rgba.extend([p.r, p.g, p.b, 255]);
        }
    }
    Frame::new(image.width(), image.height(), Point::new(96, 300), rgba)
        .expect("translated fixture")
}

#[test]
fn actual_green_preview_is_not_lost_to_a_nearby_coloured_building() {
    let before = real_preview_fixture("before.png");
    let after = real_preview_fixture("green-near-building.png");
    let cursor = Point::new(336, 540);
    // Reproduces the old refusal on an actual visible green 144x144 preview.
    assert_eq!(
        vision::detect_placement(&after, cursor, PITCH),
        Placement::Absent
    );
    assert_eq!(
        vision::detect_local_placement(&after, cursor, PITCH),
        Placement::Valid {
            center: Point::new(323, 537)
        }
    );
    assert_eq!(
        fresh_green(&before, &after, cursor, &[]),
        Some(Point::new(323, 537))
    );
    assert!(same_view(&before, &after, &[cursor, PARK]));
}

#[test]
fn actual_static_scene_and_reused_preview_are_not_fresh_green() {
    let before = real_preview_fixture("before.png");
    let after = real_preview_fixture("green-near-building.png");
    let cursor = Point::new(336, 540);
    assert_eq!(fresh_green(&before, &before, cursor, &[]), None);
    assert_eq!(fresh_green(&after, &after, cursor, &[]), None);
    assert_eq!(
        fresh_green(&before, &after, cursor, &[Point::new(323, 537)]),
        None
    );
    assert_eq!(
        vision::detect_local_placement(&before, cursor, PITCH),
        Placement::Absent
    );
}

#[test]
fn actual_preview_with_a_new_red_patch_is_still_refused() {
    let before = real_preview_fixture("before.png");
    let mut after = real_preview_fixture("green-near-building.png");
    // Nine newly red sampled pixels inside an otherwise valid real preview.
    for y in 485..500 {
        for x in 280..295 {
            after.set_pixel(x - 96, y - 300, PREVIEW_RED);
        }
    }
    assert_eq!(
        fresh_green(&before, &after, Point::new(336, 540), &[]),
        None
    );
}

#[test]
fn the_live_counters_track_the_search() {
    let mut fake = Desktop::new(2);
    let (report, progress) = execute_with_progress(&mut fake, &TWO);
    assert_eq!(report.outcome, Outcome::Completed, "{report:?}");
    assert_eq!(progress.probes(), report.probes);
    assert_eq!(progress.orders(), report.orders.len());
    assert_eq!(progress.starts(), report.starts.len());
    assert_eq!(progress.confirmed(), report.orders.len());
    assert_eq!(progress.starts(), report.orders.len());
    assert!(progress.probes() >= report.orders.len());

    // A fresh run resets them, so the GUI never shows the previous run's numbers.
    let mut fake = Desktop::new(0);
    let (_, progress) = execute_with_progress(&mut fake, &TWO);
    assert_eq!(
        (
            progress.probes(),
            progress.orders(),
            progress.starts(),
            progress.confirmed()
        ),
        (0, 0, 0, 0)
    );
}
