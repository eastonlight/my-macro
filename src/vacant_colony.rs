//! F4 saved-view Colony placement. Candidates are probes, not inferred free land:
//! only a fresh, stable green placement overlay authorizes a world click, and
//! the next drone is only touched after the ordered one is positively seen
//! starting to morph. Uses all 2–12 drones, scratch group 9, and the shared
//! worker/cancel contract.

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
use crate::vision::{self, ConstructionStart, Placement, SelectionRead};

const PITCH: i32 = 144;
const TILE: i32 = PITCH / 2;
/// How far the game may snap the preview from the probe point. A 2x2 building
/// is snapped to the map's tile grid, so the preview centre can sit a whole
/// tile away from the cursor; the click is sent at the cursor either way, so
/// this only has to reject a *different* footprint, not measure the snap.
const SNAP: i32 = TILE;
const PARK: Point = Point::new(960, 396);
/// Budget outside the per-order construction waits. Cooperative: checked
/// between operations, not a hard deadline for blocking OS capture calls.
const SEARCH_BUDGET: Duration = Duration::from_secs(60);
/// How long one issued order may take to show positive construction-start
/// evidence before the run stops with a partial report. The drone has to
/// travel to the site before it can morph, so this is per order, not per probe.
const CONSTRUCTION_BUDGET: Duration = Duration::from_secs(30);
/// Cooperative whole-run backstop: the search budget plus one construction
/// window for every supported drone. Like the other budgets this is checked
/// between captures, so it is not a hard wall-clock guarantee.
const RUN_BUDGET: Duration = Duration::from_secs(60 + 30 * 12);
/// Hard cap on probes across the whole run, so a dense view and many drones
/// cannot keep the worker busy forever. It is sized for the retry policy (a
/// full candidate pass plus its delayed revisits) and is a backstop: the time
/// budget and the per-drone cap normally stop a long search first, and
/// candidates that are certainly inside an existing footprint cost no probe.
const MAX_PROBES: usize = 384;
/// Hard cap on probes spent searching for one drone, across both passes.
const MAX_PROBES_PER_DRONE: usize = 128;
/// A candidate may fail two probes before it is treated as blocked for this
/// run: the search pass that found it and one delayed revisit. A drone passing
/// through only costs one attempt, so an earlier candidate is never dropped
/// permanently after a single transient failure.
const MAX_ATTEMPTS: u8 = 2;
/// Search passes per drone: the ordered scan, then a delayed revisit of that
/// drone's failures before giving up.
const MAX_PASSES: usize = 2;
/// Probes kept for the delayed revisit pass of one drone. Without a reserve
/// the ordered scan could spend the whole per-drone budget and no retry would
/// ever be reachable; the reserve is small so the scan still walks a long list
/// (a view whose first free spot is far to the right is not cut short).
const REVISIT_RESERVE: usize = 8;
/// Bounded no-preview search including delayed retries. Missing green may
/// mean blocked terrain/resources or a detector mismatch; it proves neither.
const NO_CONFIRM_GIVE_UP: usize = 24;
/// Consecutive positive panel reads, with a stable site view between them,
/// required before the run may touch the next drone.
const CONSTRUCTION_CONFIRM_READS: usize = 2;
/// How often the construction wait re-reads the selection panel. The morph
/// panel does not change fast, and a full-client capture is expensive.
const CONSTRUCTION_POLL: Duration = Duration::from_millis(100);
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

/// Pause between construction-start panel reads, never a busy loop.
fn construction_poll(timing: Timing) -> Duration {
    timing.gap.max(CONSTRUCTION_POLL)
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

/// The finite budgets of one run, separated so tests can shrink them.
#[derive(Clone, Copy, Debug)]
struct Budgets {
    /// Wall-clock budget for all candidate probing (cooperative).
    search: Duration,
    /// Wall-clock wait per issued order for positive construction evidence.
    construction: Duration,
    /// Whole-run cooperative backstop.
    run: Duration,
}

impl Budgets {
    const DEFAULT: Self = Self {
        search: SEARCH_BUDGET,
        construction: CONSTRUCTION_BUDGET,
        run: RUN_BUDGET,
    };
}

/// The candidate reading order and the budgets of one run.
#[derive(Clone, Copy, Debug)]
struct Plan<'a> {
    points: &'a [Point],
    budgets: Budgets,
}

/// Live counters of a running search, so the GUI can show that the sweep is
/// making progress instead of looking stuck. Only the worker writes them.
#[derive(Debug, Default)]
pub struct VacantProgress {
    probes: AtomicUsize,
    confirmed: AtomicUsize,
    orders: AtomicUsize,
    starts: AtomicUsize,
}

impl VacantProgress {
    /// Resets the counters for a new run.
    pub fn reset(&self) {
        self.probes.store(0, Ordering::SeqCst);
        self.confirmed.store(0, Ordering::SeqCst);
        self.orders.store(0, Ordering::SeqCst);
        self.starts.store(0, Ordering::SeqCst);
    }

    /// Candidates probed so far.
    pub fn probes(&self) -> usize {
        self.probes.load(Ordering::SeqCst)
    }

    /// Candidates whose green preview was confirmed at least once.
    pub fn confirmed(&self) -> usize {
        self.confirmed.load(Ordering::SeqCst)
    }

    /// Orders issued so far (not completed buildings).
    pub fn orders(&self) -> usize {
        self.orders.load(Ordering::SeqCst)
    }

    /// Orders whose drone was positively seen starting to construct/morph.
    /// This is still not a completed building.
    pub fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VacantColonyReport {
    pub outcome: Outcome,
    pub detected: u8,
    /// Issued world clicks, NOT proof that the drone arrived or finished
    /// building. A reservation kept here stays reserved even if the
    /// construction wait then fails.
    pub orders: Vec<Point>,
    /// Orders whose drone was positively seen starting to morph. Always a
    /// subset of [`Self::orders`]; still not a completed building.
    pub starts: Vec<Point>,
    pub probes: usize,
}

impl VacantColonyReport {
    pub(crate) fn failed(detail: String) -> Self {
        Self {
            outcome: Outcome::Failed { detail },
            detected: 0,
            orders: Vec::new(),
            starts: Vec::new(),
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

/// Probe one tile apart in reading order: from the lower left, left to right,
/// then the next row up. This is finite and intentionally may miss
/// small/irregular free areas.
pub fn candidates() -> Vec<Point> {
    let mut points = Vec::new();
    for y in (108..720).step_by(TILE as usize).rev() {
        for x in (120..1800).step_by(TILE as usize) {
            let point = Point::new(x, y);
            if safe_footprint(point) {
                points.push(point);
            }
        }
    }
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
    let left = (point.x - PROBE_HALF).max(0);
    let top = (point.y - PROBE_HALF).max(0);
    let rect = Rect::new(
        left,
        top,
        (point.x + PROBE_HALF).min(1920) - left,
        (point.y + PROBE_HALF).min(1080) - top,
    );
    let frame = adapter.capture_region(rect).map_err(failed)?;
    guard(adapter, cancel)?;
    if frame.origin() != Point::new(rect.x, rect.y)
        || frame.width() != rect.w as u32
        || frame.height() != rect.h as u32
    {
        return Err(aborted(
            "the probe capture returned unexpected screen coordinates",
        ));
    }
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
/// the pixels our own preview or a freshly ordered building covers. Both
/// frames must describe the same world-space rectangle; a comparison across
/// different regions or sizes is never valid. Samples outside the calibrated
/// play area (HUD, minimap, resource bar, scroll edges) are skipped, so an
/// animating panel can never be mistaken for a camera move.
fn same_view(before: &Frame, after: &Frame, skipped: &[Point]) -> bool {
    let same_rect = before.width() == after.width()
        && before.height() == after.height()
        && before.origin() == after.origin();
    let baseline_contains_region = before.origin() == Point::new(0, 0)
        && before.width() == 1920
        && before.height() == 1080
        && after.origin().x >= 0
        && after.origin().y >= 0
        && after.origin().x + after.width() as i32 <= 1920
        && after.origin().y + after.height() as i32 <= 1080;
    if !same_rect && !baseline_contains_region {
        return false;
    }
    let origin = after.origin();
    let mut changed = 0;
    let mut total = 0;
    for y in (0..after.height() as i32).step_by(24) {
        for x in (0..after.width() as i32).step_by(24) {
            let screen = Point::new(origin.x + x, origin.y + y);
            if !play_area::is_playable_point(screen) {
                continue;
            }
            if skipped.iter().any(|skip| {
                (screen.x - skip.x).abs() < SKIP_RADIUS && (screen.y - skip.y).abs() < SKIP_RADIUS
            }) {
                continue;
            }
            let (Some(a), Some(b)) = (
                before.pixel_at_screen(screen),
                after.pixel_at_screen(screen),
            ) else {
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
        Budgets::DEFAULT,
        progress,
    )
}

fn run_with_search(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    points: &[Point],
    budgets: Budgets,
    progress: &VacantProgress,
) -> VacantColonyReport {
    let mut report = VacantColonyReport {
        outcome: Outcome::Completed,
        detected: 0,
        orders: Vec::new(),
        starts: Vec::new(),
        probes: 0,
    };
    let mut preview_at = None;
    if let Err(outcome) = run_inner(
        adapter,
        cancel,
        timing,
        &mut report,
        &mut preview_at,
        Plan { points, budgets },
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

/// Run-wide candidate state: the reading order, the bounded attempt counter
/// for delayed revisits, and the cooperative deadlines.
struct Search<'a> {
    points: &'a [Point],
    attempts: Vec<u8>,
    search_deadline: Instant,
    run_deadline: Instant,
    construction_budget: Duration,
}

impl<'a> Search<'a> {
    fn new(points: &'a [Point], budgets: Budgets) -> Self {
        Self {
            points,
            attempts: vec![0; points.len()],
            search_deadline: Instant::now() + budgets.search,
            run_deadline: Instant::now() + budgets.run,
            construction_budget: budgets.construction,
        }
    }

    /// Which cooperative deadline has passed, if any.
    fn deadline_detail(&self) -> Option<&'static str> {
        if Instant::now() >= self.search_deadline {
            Some("F4 vacant-space search time limit reached")
        } else if Instant::now() >= self.run_deadline {
            Some("the whole F4 run hit its time limit")
        } else {
            None
        }
    }

    /// Gate before one probe: the hard probe caps and the deadlines.
    fn budget_gate(
        &self,
        report: &VacantColonyReport,
        probes_this_drone: usize,
    ) -> Result<(), Outcome> {
        if report.probes >= MAX_PROBES {
            return Err(aborted(
                "the F4 search stopped after the probe limit without placing every drone",
            ));
        }
        if probes_this_drone >= MAX_PROBES_PER_DRONE {
            return Err(aborted(
                "the F4 search hit its probe limit for one drone without finding a spot",
            ));
        }
        match self.deadline_detail() {
            Some(detail) => Err(aborted(detail)),
            None => Ok(()),
        }
    }
}

fn run_inner(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    report: &mut VacantColonyReport,
    preview_at: &mut Option<Point>,
    plan: Plan<'_>,
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

    let mut search = Search::new(plan.points, plan.budgets);
    let mut pool = count;
    loop {
        if let Some(detail) = search.deadline_detail() {
            return Err(aborted(detail));
        }

        // Select one drone from the scratch group. The last remaining drone
        // is assigned directly, without a portrait click.
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

        find_and_order(
            adapter,
            cancel,
            timing,
            report,
            preview_at,
            &baseline,
            &mut search,
            progress,
        )?;

        if pool == 1 {
            return Ok(());
        }
        // Only now is the ordered drone left alone: recall the scratch group
        // and drop it from the group before the next drone is touched.
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

/// Finds one verified position for the current drone, issues the order and
/// waits for positive construction-start evidence. Two passes at most: the
/// ordered scan, then a delayed revisit of this drone's failures. Returns
/// `Ok(())` only after the ordered drone was seen starting to morph.
#[allow(clippy::too_many_arguments)]
fn find_and_order(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    report: &mut VacantColonyReport,
    preview_at: &mut Option<Point>,
    baseline: &Frame,
    search: &mut Search<'_>,
    progress: &VacantProgress,
) -> Result<(), Outcome> {
    let mut probes_this_drone = 0usize;
    let mut deferred: Vec<usize> = Vec::new();
    for pass in 0..MAX_PASSES {
        if pass > 0 {
            wait(CAMERA_SETTLE.max(timing.gap), cancel)?;
        }
        let per_pass = if pass == 0 {
            MAX_PROBES_PER_DRONE - REVISIT_RESERVE
        } else {
            REVISIT_RESERVE
        };
        let mut probes_this_pass = 0;
        let indices: Vec<usize> = if pass == 0 {
            (0..search.points.len()).collect()
        } else {
            std::mem::take(&mut deferred)
        };
        for index in indices {
            if search.attempts[index] >= MAX_ATTEMPTS {
                continue;
            }
            let point = search.points[index];
            if !possibly_free(point, &report.orders) {
                continue;
            }
            if probes_this_pass >= per_pass {
                break;
            }
            search.budget_gate(report, probes_this_drone)?;
            // A detector that matches nothing must stop this drone quickly
            // instead of sweeping the screen before the retry pass exists.
            if progress.confirmed() == 0 && probes_this_drone >= NO_CONFIRM_GIVE_UP {
                return Err(aborted(
                    "no green placement preview was confirmed after a bounded search; check                      terrain, resources and preview detection",
                ));
            }
            probes_this_drone += 1;
            probes_this_pass += 1;
            if probe_candidate(
                adapter, cancel, timing, report, preview_at, baseline, point, search, progress,
            )? {
                return Ok(());
            }
            search.attempts[index] += 1;
            deferred.push(index);
        }
        if deferred.is_empty() {
            break;
        }
    }
    if probes_this_drone >= MAX_PROBES_PER_DRONE {
        return Err(aborted("the F4 search hit its probe limit for one drone"));
    }
    Err(aborted(
        "no more verified free Colony positions in the F4 view after retry",
    ))
}

/// One candidate: confirm a fresh, stable green preview, click once (the
/// order is counted at mouse-down), confirm the preview closed, then wait for
/// the ordered drone to be positively seen morphing. Returns `true` when the
/// order was issued and construction started.
#[allow(clippy::too_many_arguments)]
fn probe_candidate(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    report: &mut VacantColonyReport,
    preview_at: &mut Option<Point>,
    baseline: &Frame,
    point: Point,
    search: &mut Search<'_>,
    progress: &VacantProgress,
) -> Result<bool, Outcome> {
    move_to(adapter, cancel, point)?;
    *preview_at = Some(point);
    report.probes += 1;
    progress.probes.store(report.probes, Ordering::SeqCst);
    wait(preview_settle(timing), cancel)?;
    let first = capture_probe(adapter, cancel, point)?;
    if !same_view(baseline, &first, &[point, PARK]) {
        return Err(aborted(
            "the camera or scene changed since the placement baseline",
        ));
    }
    let Some(center) = fresh_green(baseline, &first, point, &report.orders) else {
        return Ok(false);
    };
    progress.confirmed.fetch_add(1, Ordering::SeqCst);
    wait(preview_settle(timing), cancel)?;
    let second = capture_probe(adapter, cancel, point)?;
    if !same_view(&first, &second, &[point]) {
        return Err(aborted("camera or scene changed before placement click"));
    }
    if fresh_green(baseline, &second, point, &report.orders) != Some(center) {
        return Ok(false);
    }
    if let Some(detail) = search.deadline_detail() {
        return Err(aborted(detail));
    }
    guard(adapter, cancel)?;
    if adapter.cursor_position().map_err(failed)? != point {
        return Err(aborted(
            "the mouse moved before placement; refusing to click",
        ));
    }
    // Count at the side effect, even if F8 arrives while the button is held.
    // This is an issued order, not a completed building, and the reservation
    // survives even if the construction wait below then fails.
    guard(adapter, cancel)?;
    adapter.mouse_left_down().map_err(failed)?;
    report.orders.push(center);
    progress.orders.store(report.orders.len(), Ordering::SeqCst);
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.mouse_left_up().map_err(failed)?;
    wait(timing.gap, cancel)?;
    let wait_started = Instant::now();
    confirm_closed(adapter, cancel, timing, point)?;
    *preview_at = None;
    await_construction_start(
        adapter,
        cancel,
        timing,
        report,
        center,
        baseline,
        (wait_started + search.construction_budget).min(search.run_deadline),
        progress,
    )?;
    // Travel/morph waits have their own budget and do not consume search time.
    search.search_deadline += wait_started.elapsed();
    Ok(true)
}

/// Waits for positive evidence that the drone selected for the order above has
/// actually begun constructing/morphing. A closed preview, a departing drone,
/// a lost selection or a smaller control group are all explicitly not enough:
/// only consecutive single-unit panel reads showing the morphing Colony count,
/// and the world view at the ordered site must stay the same between them.
/// Any other outcome is a timeout that stops the run with a partial report.
#[allow(clippy::too_many_arguments)]
fn await_construction_start(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    report: &mut VacantColonyReport,
    point: Point,
    baseline: &Frame,
    deadline: Instant,
    progress: &VacantProgress,
) -> Result<(), Outcome> {
    let mut consecutive = 0usize;
    let expected_cursor = adapter.cursor_position().map_err(failed)?;
    let mut previous_site: Option<Frame> = None;
    loop {
        let panel = capture(adapter, cancel)?;
        let read = vision::detect_construction_start(&panel);
        // A bounded world read of the ordered site: the morph panel must be
        // stable while the site view stays the same camera view. This compares
        // two equivalent world-space regions and never the HUD pixels.
        let site = capture_probe(adapter, cancel, point)?;
        if Instant::now() >= deadline {
            return Err(aborted(&format!(
                "no construction start was confirmed before the wait limit (last panel read: {read:?}); the issued order remains unconfirmed"
            )));
        }
        if adapter.cursor_position().map_err(failed)? != expected_cursor {
            return Err(aborted("the mouse moved during construction confirmation"));
        }
        if !same_view(baseline, &site, &[point, PARK]) {
            return Err(aborted(
                "the camera or scene changed during construction confirmation",
            ));
        }
        if matches!(read, ConstructionStart::MorphingColony { .. })
            && site_changed(baseline, &site, point)
        {
            consecutive += 1;
            if let Some(previous) = &previous_site
                && !same_view(previous, &site, &[point])
            {
                // The view at the ordered site changed between the panel
                // reads, so this is not a stable confirmation.
                consecutive = 1;
            }
            if consecutive >= CONSTRUCTION_CONFIRM_READS {
                report.starts.push(point);
                progress.starts.store(report.starts.len(), Ordering::SeqCst);
                return Ok(());
            }
        } else {
            consecutive = 0;
        }
        previous_site = Some(site);
        wait(construction_poll(timing), cancel)?;
    }
}

/// Supporting site evidence, never sufficient without the selected Colony
/// morph panel: something must have appeared at the reserved footprint.
fn site_changed(before: &Frame, after: &Frame, center: Point) -> bool {
    let mut changed = 0;
    for y in (center.y - TILE..center.y + TILE).step_by(6) {
        for x in (center.x - TILE..center.x + TILE).step_by(6) {
            let p = Point::new(x, y);
            let (Some(a), Some(b)) = (before.pixel_at_screen(p), after.pixel_at_screen(p)) else {
                return false;
            };
            if u32::from(a.r.abs_diff(b.r))
                + u32::from(a.g.abs_diff(b.g))
                + u32::from(a.b.abs_diff(b.b))
                > 100
            {
                changed += 1;
            }
        }
    }
    changed >= 24
}

/// Confirms that placement mode closed after the order. The placement overlay
/// disappearing is not construction evidence by itself; it only proves the
/// click left placement mode.
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
