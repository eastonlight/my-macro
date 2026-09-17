//! F4 saved-view Colony placement. Candidates are probes, not inferred free land:
//! only a fresh, stable green placement overlay authorizes a world click.
//! Uses all 2–12 drones, scratch group 9, and the shared worker/cancel contract.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::colony::CAPTURE_TIMEOUT;
use crate::colony::controls::{
    chord, click, expect_count, expect_recall, expect_single, guard, shift_click, tap, wait,
};
use crate::engine::Outcome;
use crate::frame::{Frame, Point, Rect};
use crate::input::{DesktopAdapter, InputError};
use crate::macros::{BuildTarget, Key, Timing};
use crate::play_area;
use crate::vision::{self, Placement, SelectionRead};

const PITCH: i32 = 144;
const TILE: i32 = PITCH / 2;
/// How far the game may snap the preview from the probe point. A 2x2 building
/// is snapped to the map's tile grid, so the preview centre can sit a whole
/// tile away from the cursor; the click is sent at the cursor either way, so
/// this only has to reject a *different* footprint, not measure the snap.
const SNAP: i32 = TILE;
const PARK: Point = Point::new(960, 396);
/// Whole-run search budget. A full probe pass is a few seconds, so this is a
/// backstop, not the normal exit.
const SEARCH_BUDGET: Duration = Duration::from_secs(20);
/// Upper bound on probes per run, so a dense view cannot keep the worker busy
/// for long. It is a backstop: the time budget normally stops a long search
/// first, and candidates that are certainly already built on cost no probe.
const MAX_PROBES: usize = 192;
/// If not a single green preview is confirmed in this many probes, the
/// detector is not matching this game/skin; stop and say so instead of
/// sweeping the whole screen and looking hung.
const NO_CONFIRM_GIVE_UP: usize = 24;
const CAMERA_SETTLE: Duration = Duration::from_millis(200);
/// Bounds for the pause between the two confirming reads of one probe. It
/// follows the configured gap (so the user's own pacing applies) but never
/// drops below a fraction of a game frame nor exceeds one: too short and the
/// preview has not been drawn yet, too long and a full-screen search crawls.
const PREVIEW_SETTLE_FLOOR: Duration = Duration::from_millis(16);
const PREVIEW_SETTLE_CEIL: Duration = Duration::from_millis(32);

/// Pause between the two reads that confirm one candidate's preview.
fn preview_settle(timing: Timing) -> Duration {
    timing
        .gap
        .max(PREVIEW_SETTLE_FLOOR)
        .min(PREVIEW_SETTLE_CEIL)
}
/// Half-size of one probe read. It covers the detector's 176 px search window
/// plus the preview square and its ring, and it is read with the cheap
/// composed-screen copy instead of a full 1920x1080 window render.
const PROBE_HALF: i32 = 240;
/// Everything within this distance of a probe (or of an already ordered
/// building) is ignored by the scene-change comparison, because our own
/// preview and newly placed buildings legitimately change those pixels.
const SKIP_RADIUS: i32 = 150;

#[cfg(test)]
#[path = "vacant_colony_tests.rs"]
pub(crate) mod tests;

/// The candidate list and the wall-clock budget of one search.
#[derive(Clone, Copy, Debug)]
struct SearchPlan<'a> {
    points: &'a [Point],
    budget: Duration,
}

/// Live counters of a running search, so the GUI can show that the sweep is
/// making progress instead of looking stuck. Only the worker writes them.
#[derive(Debug, Default)]
pub struct VacantProgress {
    probes: AtomicUsize,
    orders: AtomicUsize,
    confirmed: AtomicUsize,
}

impl VacantProgress {
    /// Resets the counters for a new run.
    pub fn reset(&self) {
        self.probes.store(0, Ordering::SeqCst);
        self.orders.store(0, Ordering::SeqCst);
        self.confirmed.store(0, Ordering::SeqCst);
    }

    /// Candidates probed so far.
    pub fn probes(&self) -> usize {
        self.probes.load(Ordering::SeqCst)
    }

    /// Orders issued so far (not completed buildings).
    pub fn orders(&self) -> usize {
        self.orders.load(Ordering::SeqCst)
    }

    /// Candidates whose green preview was confirmed at least once.
    pub fn confirmed(&self) -> usize {
        self.confirmed.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VacantColonyReport {
    pub outcome: Outcome,
    pub detected: u8,
    /// Issued clicks, NOT proof that the drone arrived or finished building.
    pub orders: Vec<Point>,
    pub probes: usize,
}

impl VacantColonyReport {
    pub(crate) fn failed(detail: String) -> Self {
        Self {
            outcome: Outcome::Failed { detail },
            detected: 0,
            orders: Vec::new(),
            probes: 0,
        }
    }
}

fn aborted(detail: &str) -> Outcome {
    Outcome::Aborted {
        detail: detail.to_owned(),
    }
}
fn failed(error: InputError) -> Outcome {
    Outcome::Failed {
        detail: error.to_string(),
    }
}

/// Unlike row building, require the whole footprint plus a small margin to
/// stay in the calibrated world area. Never probe HUD or scroll borders.
fn safe_footprint(center: Point) -> bool {
    let radius = PITCH / 2 + 12;
    (center.x - radius..=center.x + radius).all(|x| {
        play_area::is_playable_point(Point::new(x, center.y - radius))
            && play_area::is_playable_point(Point::new(x, center.y + radius))
    })
}

/// Authoritative spacing rule: two 2x2 buildings must be at least one pitch
/// apart in one axis. Applied to the *snapped* centre the detector found.
fn unreserved(center: Point, reserved: &[Point]) -> bool {
    reserved
        .iter()
        .all(|other| (other.x - center.x).abs() >= PITCH || (other.y - center.y).abs() >= PITCH)
}

/// Cheap pre-filter for the probe grid. The game may snap the footprint by up
/// to one tile, so a probe point that is close to an existing building could
/// still become a different, valid footprint: only points that are certainly
/// the same footprint are skipped without a read. Without this margin the
/// pre-filter silently dropped valid spots and the search looked as if it found
/// nothing even in an open area.
fn possibly_free(point: Point, reserved: &[Point]) -> bool {
    let certainly_taken = PITCH - SNAP;
    reserved.iter().all(|other| {
        (other.x - point.x).abs() >= certainly_taken || (other.y - point.y).abs() >= certainly_taken
    })
}

/// Probe one tile apart, centre-out. Even-grid cells first give compact 2-tile
/// packing; the other tile phases can discover narrower gaps between objects.
/// This is finite and intentionally may miss small/irregular free areas.
pub fn candidates() -> Vec<Point> {
    let mut points = Vec::new();
    for y in (108..720).step_by(TILE as usize) {
        for x in (120..1800).step_by(TILE as usize) {
            let point = Point::new(x, y);
            if safe_footprint(point) {
                points.push(point);
            }
        }
    }
    points.sort_by_key(|p| {
        let phase = ((p.x - 120) / TILE) % 2 + 2 * (((p.y - 108) / TILE) % 2);
        (phase, (p.x - PARK.x).abs() + (p.y - PARK.y).abs(), p.y, p.x)
    });
    points
}

/// One bounded read around a candidate. Uses the adapter's region capture, so
/// the per-candidate cost stays small enough for a full-screen search.
fn capture_probe(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    point: Point,
) -> Result<Frame, Outcome> {
    guard(adapter, cancel)?;
    let rect = Rect::new(
        point.x - PROBE_HALF,
        point.y - PROBE_HALF,
        PROBE_HALF * 2,
        PROBE_HALF * 2,
    );
    let frame = adapter.capture_region(rect).map_err(failed)?;
    guard(adapter, cancel)?;
    // A dark frame is accepted: black unexplored terrain, space and shadows are
    // normal, and rejecting them would abort on the very maps this feature is
    // meant to build on. A capture that cannot be trusted shows up as "no
    // preview was ever confirmed" instead (see NO_CONFIRM_GIVE_UP).
    Ok(frame)
}

/// Full-client read, used for the calibrated HUD and the per-drone view check.
/// Refuses anything but a 1920x1080 client at the origin.
fn capture(adapter: &mut dyn DesktopAdapter, cancel: &AtomicBool) -> Result<Frame, Outcome> {
    guard(adapter, cancel)?;
    let frame = adapter.capture_client().map_err(failed)?;
    guard(adapter, cancel)?;
    if frame.origin() != Point::new(0, 0)
        || frame.width() != 1920
        || frame.height() != 1080
        || frame.is_blank()
    {
        return Err(aborted(
            "F4 placement requires a visible 1920x1080 client at (0,0)",
        ));
    }
    Ok(frame)
}

fn move_to(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    point: Point,
) -> Result<(), Outcome> {
    guard(adapter, cancel)?;
    adapter.move_cursor(point).map_err(failed)
}

/// Coarse scene-change refusal between two captures of the same area (not an
/// ownership or bookmark detector). Tolerates ordinary animation and ignores
/// the pixels our own preview or a freshly ordered building covers.
fn same_view(before: &Frame, after: &Frame, skipped: &[Point]) -> bool {
    if before.width() != after.width()
        || before.height() != after.height()
        || before.origin() != after.origin()
    {
        return false;
    }
    let origin = before.origin();
    let mut changed = 0;
    let mut total = 0;
    for y in (0..before.height() as i32).step_by(24) {
        for x in (0..before.width() as i32).step_by(24) {
            let screen = Point::new(origin.x + x, origin.y + y);
            if skipped.iter().any(|skip| {
                (screen.x - skip.x).abs() < SKIP_RADIUS && (screen.y - skip.y).abs() < SKIP_RADIUS
            }) {
                continue;
            }
            let (Some(a), Some(b)) = (before.pixel(x, y), after.pixel(x, y)) else {
                return false;
            };
            total += 1;
            if a.r.abs_diff(b.r) as u32 + a.g.abs_diff(b.g) as u32 + a.b.abs_diff(b.b) as u32 > 100
            {
                changed += 1;
            }
        }
    }
    total >= 30 && changed * 100 <= total * 15
}

/// Reject static green terrain, partially red previews, off-target or unsafe
/// snapped centres. Colour deltas must appear in every quadrant of the square.
fn fresh_green(before: &Frame, after: &Frame, point: Point, reserved: &[Point]) -> Option<Point> {
    let Placement::Valid { center } = vision::detect_placement(after, point, PITCH) else {
        return None;
    };
    if (center.x - point.x).abs() > SNAP
        || (center.y - point.y).abs() > SNAP
        || !safe_footprint(center)
        || !unreserved(center, reserved)
    {
        return None;
    }
    let mut green = [0usize; 4];
    let mut red = 0usize;
    for y in (center.y - TILE..center.y + TILE).step_by(3) {
        for x in (center.x - TILE..center.x + TILE).step_by(3) {
            let p = Point::new(x, y);
            let (Some(old), Some(new)) = (before.pixel_at_screen(p), after.pixel_at_screen(p))
            else {
                return None;
            };
            if new.is_preview_red() && !old.is_preview_red() {
                red += 1;
            }
            if new.is_preview_green() && !old.is_preview_green() {
                let quadrant = usize::from(x >= center.x) + 2 * usize::from(y >= center.y);
                green[quadrant] += 1;
            }
        }
    }
    (red <= 3 && green.into_iter().all(|count| count >= 35)).then_some(center)
}

/// Runs one finite search. Always releases owned keys/buttons; cancellation and
/// focus loss do not send cleanup Escape into a different window.
pub fn run(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    progress: &VacantProgress,
) -> VacantColonyReport {
    progress.reset();
    run_with_search(
        adapter,
        cancel,
        timing,
        &candidates(),
        SEARCH_BUDGET,
        progress,
    )
}

fn run_with_search(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    points: &[Point],
    budget: Duration,
    progress: &VacantProgress,
) -> VacantColonyReport {
    let mut report = VacantColonyReport {
        outcome: Outcome::Completed,
        detected: 0,
        orders: Vec::new(),
        probes: 0,
    };
    let mut preview_at = None;
    if let Err(outcome) = run_inner(
        adapter,
        cancel,
        timing,
        &mut report,
        &mut preview_at,
        SearchPlan { points, budget },
        progress,
    ) {
        report.outcome = outcome;
    }
    if let Some(point) = preview_at {
        // Best effort only when a placement overlay is positively visible.
        if let Ok(frame) = capture_probe(adapter, cancel, point)
            && matches!(
                vision::detect_placement(&frame, point, PITCH),
                Placement::Valid { .. } | Placement::Invalid { .. }
            )
        {
            let _ = tap(adapter, cancel, timing, Key::Escape);
        }
    }
    if let Err(error) = adapter.release_all() {
        report.outcome = failed(error);
    }
    report
}

fn run_inner(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    report: &mut VacantColonyReport,
    preview_at: &mut Option<Point>,
    plan: SearchPlan<'_>,
    progress: &VacantProgress,
) -> Result<(), Outcome> {
    // Read the group before F4 or Ctrl+9. No count cap and no force mode.
    move_to(adapter, cancel, PARK)?;
    let initial = capture(adapter, cancel)?;
    let SelectionRead::Drones {
        count: count @ 2..=12,
    } = vision::detect_selection(&initial)
    else {
        return Err(aborted(
            "select 2-12 drones before starting F4 Colony placement",
        ));
    };
    report.detected = count;
    tap(adapter, cancel, timing, Key::F4)?;
    wait(CAMERA_SETTLE, cancel)?;
    let saved_view = capture(adapter, cancel)?;
    expect_count(adapter, cancel, timing, count)?;
    chord(adapter, cancel, timing, Key::Control, Key::Nine)?;

    let deadline = Instant::now() + plan.budget;
    let mut next = 0;
    let mut pool = count;
    loop {
        if Instant::now() >= deadline {
            return Err(aborted("F4 vacant-space search time limit reached"));
        }
        if pool > 1 {
            move_to(adapter, cancel, vision::slot_center(pool - 1))?;
            click(adapter, cancel, timing)?;
        }
        move_to(adapter, cancel, PARK)?;
        expect_single(adapter, cancel, timing)?;
        // A group recall can move the camera on some setups. Always restore
        // the same user bookmark before reusing any reserved screen positions.
        if pool != count {
            tap(adapter, cancel, timing, Key::F4)?;
            wait(CAMERA_SETTLE, cancel)?;
        }
        let baseline = capture(adapter, cancel)?;
        let mut skips = report.orders.clone();
        skips.push(PARK);
        if !same_view(&saved_view, &baseline, &skips) {
            return Err(aborted(
                "the F4 view changed; refusing stale screen coordinates",
            ));
        }
        if vision::detect_selection(&baseline) != SelectionRead::SingleDrone {
            return Err(aborted(
                "the single-drone selection changed after recalling F4",
            ));
        }
        *preview_at = Some(PARK);
        for key in BuildTarget::Colony.build_keys() {
            tap(adapter, cancel, timing, key)?;
        }

        let mut placed = false;
        while let Some(&point) = plan.points.get(next) {
            next += 1;
            if !possibly_free(point, &report.orders) {
                continue;
            }
            if Instant::now() >= deadline {
                return Err(aborted("F4 vacant-space search time limit reached"));
            }
            if report.probes >= MAX_PROBES {
                return Err(aborted(
                    "the F4 search stopped after the probe limit without placing every drone",
                ));
            }
            move_to(adapter, cancel, point)?;
            *preview_at = Some(point);
            report.probes += 1;
            progress.probes.store(report.probes, Ordering::SeqCst);
            wait(preview_settle(timing), cancel)?;
            let first = capture_probe(adapter, cancel, point)?;
            let Some(center) = fresh_green(&baseline, &first, point, &report.orders) else {
                if progress.confirmed() == 0 && report.probes >= NO_CONFIRM_GIVE_UP {
                    return Err(aborted(
                        "no green placement preview was confirmed in the probes so far; the \
                         preview detector may not match this game or skin. The row macro's forced \
                         mode clicks without confirmation, this feature refuses to",
                    ));
                }
                continue;
            };
            progress.confirmed.fetch_add(1, Ordering::SeqCst);
            wait(preview_settle(timing), cancel)?;
            let second = capture_probe(adapter, cancel, point)?;
            if !same_view(&first, &second, &[point]) {
                return Err(aborted("camera or scene changed before placement click"));
            }
            if fresh_green(&baseline, &second, point, &report.orders) != Some(center) {
                continue;
            }
            if Instant::now() >= deadline {
                return Err(aborted("F4 vacant-space search time limit reached"));
            }
            guard(adapter, cancel)?;
            if adapter.cursor_position().map_err(failed)? != point {
                return Err(aborted(
                    "the mouse moved before placement; refusing to click",
                ));
            }
            // Count at the side effect, even if F8 arrives while the button is
            // held. This is an issued order, not a completed building.
            guard(adapter, cancel)?;
            adapter.mouse_left_down().map_err(failed)?;
            report.orders.push(center);
            progress.orders.store(report.orders.len(), Ordering::SeqCst);
            wait(timing.press, cancel)?;
            guard(adapter, cancel)?;
            adapter.mouse_left_up().map_err(failed)?;
            wait(timing.gap, cancel)?;
            confirm_closed(adapter, cancel, timing, point)?;
            *preview_at = None;
            placed = true;
            break;
        }
        if !placed {
            return Err(aborted(
                "no more verified free Colony positions in the F4 view",
            ));
        }
        if pool == 1 {
            return Ok(());
        }
        tap(adapter, cancel, timing, Key::Nine)?;
        let recalled = expect_recall(adapter, cancel, timing, pool)?;
        if recalled == pool {
            move_to(adapter, cancel, vision::slot_center(pool - 1))?;
            shift_click(adapter, cancel, timing)?;
            move_to(adapter, cancel, PARK)?;
            // Verify before overwriting group 9 with the reduced pool.
            expect_count(adapter, cancel, timing, pool - 1)?;
        }
        chord(adapter, cancel, timing, Key::Control, Key::Nine)?;
        pool -= 1;
    }
}

fn confirm_closed(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    point: Point,
) -> Result<(), Outcome> {
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let mut absent = 0;
    loop {
        wait(preview_settle(timing), cancel)?;
        let frame = capture_probe(adapter, cancel, point)?;
        if vision::detect_placement(&frame, point, PITCH) == Placement::Absent {
            absent += 1;
        } else {
            absent = 0;
        }
        if absent >= 2 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(aborted(
                "placement did not close after the order; stopping, not retrying the click",
            ));
        }
    }
}
