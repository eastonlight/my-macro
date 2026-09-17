//! The row-build macro (F6/F7): a pure planner plus a pure state machine.
//!
//! Both trigger hotkeys run this one macro. It first reads the selection HUD
//! (see [`crate::vision`]), then builds one building per selected drone in a
//! horizontal row to the right of the cursor:
//!
//! * `2..=12` drones: the drones are saved into **scratch control group 9**
//!   (overwriting it), then each iteration selects one drone by clicking the
//!   last occupied wireframe portrait, issues the configured build keys and
//!   clicks the verified green placement preview. The group is recalled
//!   afterwards so the issued (travelling) drone can be dropped from the group
//!   before the next order.
//! * exactly one drone: `B`/`C` or `V`/`S` then click at the cursor.
//!
//! The only difference between a Creep Colony row and a Spire row is the
//! [`BuildTarget`]: its two build keys and its footprint. The order in which
//! the footprints of that row are built is configurable ([`RowMode`]). Both
//! modes cover the same span: the cursor's footprint stays the leftmost one and
//! the last footprint is `count - 1` footprints further right. Only the
//! sequence of the footprints differs.
//!
//! Everything here talks to [`DesktopAdapter`], so the whole state machine is
//! exercised on any host with a fake capture/input adapter. It never spawns a
//! thread; [`crate::runner::MacroRunner`] owns the single tracked worker.
//!
//! Counts are reported as **orders issued**, never as completed buildings.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::engine::{CANCEL_POLL_INTERVAL, Outcome, RunReport};
use crate::frame::Point;
use crate::input::{DesktopAdapter, InputError};
use crate::macros::{BuildTarget, Key, Timing};
use crate::vision::{self, MAX_DRONES, Placement, SelectionRead};

/// Margin kept from the client edges so a click can never scroll the camera.
pub const EDGE_MARGIN: i32 = 48;
/// Coarse reference for where the bottom HUD starts.
///
/// The real boundary is *not* a flat line: [`crate::play_area::is_playable_point`]
/// models the piecewise Zerg console contour that owns the actual decision. This
/// constant is kept only as an approximate landmark for tests and messages, and
/// is deliberately lower than the real contour so it can never be mistaken for
/// the authority on playable space.
pub const HUD_TOP: i32 = 680;
/// Slack between the cursor and the snapped footprint centre: the game snaps
/// the preview to a tile grid, so one tile of play is allowed.
pub const SNAP_SLACK: i32 = 32;
/// Upper bound for one awaited screen transition in strict mode.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Forced mode confirms a placement preview only this long before it clicks
/// anyway. About five StarCraft frames: enough for a preview that is merely a
/// frame late, short enough that a preview this tool cannot confirm never
/// stalls the whole row for [`CAPTURE_TIMEOUT`].
const FORCE_CONFIRM_TIMEOUT: Duration = Duration::from_millis(200);
/// Upper bound on consecutive red-preview reads; strict mode already refuses
/// on the first read, so this is only a defensive cap.
const INVALID_PREVIEW_READS: usize = 3;

/// Order in which the footprints of one planned row are built.
///
/// The row's span is the same in both modes: the cursor's footprint is the
/// leftmost footprint and every target sits exactly one footprint further
/// right.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RowMode {
    /// `0, 1, 2, ...`: walk the row from left to right (the default).
    #[default]
    LeftToRight,
    /// `0, count-1, 1, count-2, 2, ...`: alternate the remaining outermost
    /// footprints, converging on the middle (leftmost, rightmost, second
    /// leftmost, second rightmost, ...).
    EndsInward,
    /// 6 columns x 2 rows: 1..6 on the bottom row, 7..12 on the row above.
    Grid6x2,
}

impl RowMode {
    /// Every selectable mode, in the order the GUI offers them.
    pub const ALL: [Self; 3] = [Self::LeftToRight, Self::EndsInward, Self::Grid6x2];

    /// Stable config-file spelling, used by the serde implementation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LeftToRight => "left_to_right",
            Self::EndsInward => "ends_inward",
            Self::Grid6x2 => "grid_6x2",
        }
    }

    /// Parses [`Self::as_str`]. An unknown string is rejected, never guessed.
    // The name is part of the config boundary; the return type is deliberately
    // `Option` (the caller maps it to a serde error), so the `FromStr` trait
    // would not fit the signature.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == value)
    }

    /// Footprint index (`0` = leftmost) placed at placement step `step` in a
    /// row of `count` footprints. `step` must be less than `count`.
    pub fn footprint_index(self, step: u8, count: u8) -> u8 {
        match self {
            Self::LeftToRight => step,
            Self::EndsInward => {
                let outer = step / 2;
                if step.is_multiple_of(2) {
                    outer
                } else {
                    count - 1 - outer
                }
            }
            Self::Grid6x2 => step % 6,
        }
    }
}

impl Serialize for RowMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RowMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match Self::from_str(&value) {
            Some(mode) => Ok(mode),
            None => Err(serde::de::Error::custom(format!(
                "unknown colony row mode '{value}'; expected '{}', '{}' or '{}'",
                Self::LeftToRight.as_str(),
                Self::EndsInward.as_str(),
                Self::Grid6x2.as_str()
            ))),
        }
    }
}

/// A planned row of building footprints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowPlan {
    /// Cursor position when the hotkey was pressed; also the footprint the row
    /// starts at, i.e. the leftmost footprint in both [`RowMode`]s.
    pub anchor: Point,
    /// Number of buildings in the row (`1..=12`).
    pub count: u8,
    /// Which building is ordered, and therefore the spacing used below.
    pub target: BuildTarget,
    /// Footprint centres in placement order, one footprint apart.
    pub targets: Vec<Point>,
}

/// Why a row cannot be built at this anchor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowError {
    /// The cursor is outside the safe playable area (screen edge or HUD).
    AnchorOutsideSafeArea,
    /// Somewhere in the row would fall outside the safe playable area.
    RowDoesNotFit {
        /// How many buildings were requested.
        count: u8,
        /// How many still fit from the same cursor position.
        fits: u8,
    },
}

impl RowError {
    /// English detail appended to the localized status line, with the counts a
    /// refusal actually saw.
    pub fn report_detail(self) -> String {
        match self {
            Self::RowDoesNotFit { count, fits } => format!(
                "{} of {count} buildings fit in a row from this cursor position; move the \
                 cursor left, lower the count, or build the row higher up the screen",
                fits
            ),
            other => other.detail().to_owned(),
        }
    }

    /// English detail appended to the localized status line.
    pub const fn detail(self) -> &'static str {
        match self {
            Self::AnchorOutsideSafeArea => {
                "point the cursor at the build spot inside the play area (not the HUD or a \
                 screen edge)"
            }
            Self::RowDoesNotFit { .. } => {
                "the requested row does not fit without reaching the HUD or the screen edge; \
                 move the cursor left or select fewer drones"
            }
        }
    }
}

/// Plans the row for `target` and refuses anything that would touch the HUD or
/// an edge.
///
/// Every decision is made on the **clickpoint**, never on the footprint box: a
/// cursor point is what clicks UI or scrolls the camera, and the game decides on
/// its own whether a building that extends past an edge may be placed there.
/// The horizontal bound is therefore the calibrated
/// [`crate::play_area::is_playable_point`] edge guard, exactly like the vertical
/// HUD contour check — the old `48 px + half footprint + slack` inset wasted
/// 152 px on the right of the screen and refused long rows that the game accepts.
///
/// A row whose later clickpoint leaves the play area is wholly refused before
/// input. Strict preview recognition near a clipped screen boundary can still
/// refuse while forced mode clicks.
pub fn plan_row(
    anchor: Point,
    count: u8,
    mode: RowMode,
    target: BuildTarget,
) -> Result<RowPlan, RowError> {
    let count = count.clamp(1, MAX_DRONES);
    let footprint = target.footprint_px();
    if !crate::play_area::is_playable_point(anchor) {
        return Err(RowError::AnchorOutsideSafeArea);
    }
    let targets: Vec<Point> = (0..count)
        .map(|step| match mode {
            RowMode::Grid6x2 => {
                let column = step % 6;
                let row = step / 6;
                anchor.offset(footprint * i32::from(column), -footprint * i32::from(row))
            }
            _ => {
                let index = mode.footprint_index(step, count);
                anchor.offset(footprint * i32::from(index), 0)
            }
        })
        .collect();
    // Counted across the whole span, not "the first blocked step": the number of
    // footprints that still fit is a property of the row's span, while the step
    // order differs between [`RowMode`]s. This also keeps the refusal identical
    // for both modes.
    let fits = targets
        .iter()
        .filter(|point| crate::play_area::is_playable_point(**point))
        .count();
    if fits < usize::from(count) {
        // Report how many still fit so the status line can say what works
        // instead of only refusing.
        return Err(RowError::RowDoesNotFit {
            count,
            fits: u8::try_from(fits).unwrap_or(0),
        });
    }
    Ok(RowPlan {
        anchor,
        count,
        target,
        targets,
    })
}

/// How a row-build run ended, plus the counts the GUI reports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColonyReport {
    pub outcome: Outcome,
    /// Which building was ordered.
    pub target: BuildTarget,
    /// Buildings for which the build keys and a click were actually issued.
    pub orders_issued: u8,
    /// Drones the HUD reported before the first order (`0` if unreadable).
    pub detected: u8,
    /// Orders issued although the green placement preview could not be
    /// confirmed (only possible with the forced mode).
    pub unconfirmed: u8,
}

impl ColonyReport {
    fn new(
        outcome: Outcome,
        target: BuildTarget,
        orders_issued: u8,
        detected: u8,
        unconfirmed: u8,
    ) -> Self {
        Self {
            outcome,
            target,
            orders_issued,
            detected,
            unconfirmed,
        }
    }

    /// Converts to the engine report the GUI already knows how to display.
    /// `steps_done` is orders issued, `steps_total` is the detected count, and
    /// the macro id says which building was ordered.
    pub fn into_run_report(self) -> RunReport {
        RunReport {
            macro_id: self.target.macro_id(),
            outcome: self.outcome,
            steps_done: usize::from(self.orders_issued),
            steps_total: usize::from(self.detected),
            unconfirmed: usize::from(self.unconfirmed),
        }
    }
}

/// Runs one row-build macro for `target`.
///
/// Never panics on adapter errors, always releases what it pressed, and returns
/// a report for every path.
pub fn run_row(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    mode: RowMode,
    target: BuildTarget,
    force: bool,
) -> ColonyReport {
    let mut counts = Counts::default();
    let outcome = match run_inner(adapter, cancel, timing, mode, target, force, &mut counts) {
        Ok(()) => Outcome::Completed,
        Err(outcome) => outcome,
    };
    let outcome = match (outcome, adapter.release_all()) {
        (outcome @ (Outcome::Failed { .. } | Outcome::Aborted { .. }), _) => outcome,
        (_, Err(error)) => Outcome::Failed {
            detail: format!("could not release injected input: {error}"),
        },
        (outcome, Ok(())) => outcome,
    };
    ColonyReport::new(
        outcome,
        target,
        counts.issued,
        counts.detected,
        counts.unconfirmed,
    )
}

#[derive(Default)]
struct Counts {
    issued: u8,
    detected: u8,
    unconfirmed: u8,
}

fn run_inner(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    mode: RowMode,
    target: BuildTarget,
    force: bool,
    counts: &mut Counts,
) -> Result<(), Outcome> {
    // 1. Snapshot the anchor before any portrait click can move the cursor.
    if cancel.load(Ordering::SeqCst) {
        return Err(Outcome::Cancelled);
    }
    guard(adapter, cancel)?;
    let anchor = adapter.cursor_position().map_err(failed)?;
    plan_row(anchor, 1, mode, target).map_err(aborted_row)?;

    // 2. Park the pointer at the anchor (a safe world position, so no hover
    //    tooltip can cover the selection HUD) and read the selection.
    adapter.move_cursor(anchor).map_err(failed)?;
    let read = await_selection(adapter, cancel, timing, |_| true)?;
    let count = match read {
        SelectionRead::Rejected(reason) => {
            return Err(Outcome::Aborted {
                detail: reason.detail().to_owned(),
            });
        }
        SelectionRead::SingleDrone => 1,
        SelectionRead::Drones { count } => count,
    };
    counts.detected = count;
    let plan = plan_row(anchor, count, mode, target).map_err(aborted_row)?;

    // 3. One drone keeps the original behaviour: build at the cursor.
    if count == 1 {
        adapter.move_cursor(plan.anchor).map_err(failed)?;
        place_building(adapter, cancel, timing, target, plan.anchor, force, counts)?;
        counts.issued = 1;
        return Ok(());
    }

    // 4. Save the group into scratch control group 9 (overwrites any group 9).
    chord(adapter, cancel, timing, Key::Control, Key::Nine)?;

    let mut pool = count;
    while pool > 1 {
        let last_slot = pool - 1;
        let target_point = plan.targets[usize::from(count - pool)];

        // Select the last occupied portrait; that leaves exactly one drone.
        adapter
            .move_cursor(vision::slot_center(last_slot))
            .map_err(failed)?;
        click(adapter, cancel, timing)?;

        // Confirm the click really selected a single drone before the build
        // keys, so a misread can never issue a build order to a whole group.
        adapter.move_cursor(target_point).map_err(failed)?;
        expect_single(adapter, cancel, timing)?;
        place_building(
            adapter,
            cancel,
            timing,
            plan.target,
            target_point,
            force,
            counts,
        )?;
        counts.issued += 1;

        // Recall group 9. If the issued drone already morphed it has left the
        // group; if it is still travelling it is selected again.
        tap(adapter, cancel, timing, Key::Nine)?;
        let recalled = expect_recall(adapter, cancel, timing, pool)?;

        if recalled == pool {
            // Still travelling: remove exactly that portrait, then re-save.
            adapter
                .move_cursor(vision::slot_center(last_slot))
                .map_err(failed)?;
            shift_click(adapter, cancel, timing)?;
            chord(adapter, cancel, timing, Key::Control, Key::Nine)?;
            // Validate the new pool of `pool - 1` before trusting it.
            adapter.move_cursor(anchor).map_err(failed)?;
            expect_count(adapter, cancel, timing, pool - 1)?;
        } else {
            // Already morphed (`pool - 1`) or a single information panel at
            // `pool == 2`: do NOT remove another portrait, just re-save.
            chord(adapter, cancel, timing, Key::Control, Key::Nine)?;
        }
        pool -= 1;
    }

    // 5. The last drone is shown as a single information panel.
    let last_target = plan.targets[usize::from(count - 1)];
    adapter.move_cursor(last_target).map_err(failed)?;
    place_building(
        adapter,
        cancel,
        timing,
        plan.target,
        last_target,
        force,
        counts,
    )?;
    counts.issued += 1;
    Ok(())
}

fn aborted_row(error: RowError) -> Outcome {
    Outcome::Aborted {
        detail: error.report_detail(),
    }
}

fn failed(error: InputError) -> Outcome {
    Outcome::Failed {
        detail: error.to_string(),
    }
}

/// Runs the adapter's safety gate before an injected event. The engine does
/// this for plan-based macros; the colony row calls the adapter directly, so it
/// must do the same before every key press and click.
fn guard(adapter: &mut dyn DesktopAdapter, cancel: &AtomicBool) -> Result<(), Outcome> {
    if cancel.load(Ordering::SeqCst) {
        return Err(Outcome::Cancelled);
    }
    adapter.safety_check().map_err(failed)
}

/// Injectable key hold / gap, polled for cancellation.
fn wait(duration: Duration, cancel: &AtomicBool) -> Result<(), Outcome> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if cancel.load(Ordering::SeqCst) {
            return Err(Outcome::Cancelled);
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
) -> Result<(), Outcome> {
    guard(adapter, cancel)?;
    adapter.key_down(key).map_err(failed)?;
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.key_up(key).map_err(failed)?;
    wait(timing.gap, cancel)
}

/// Ctrl+9-style chord. The modifier is pressed first and released last, and
/// the adapter owns it, so the user's own modifiers still block injection.
fn chord(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    modifier: Key,
    key: Key,
) -> Result<(), Outcome> {
    guard(adapter, cancel)?;
    adapter.key_down(modifier).map_err(failed)?;
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.key_down(key).map_err(failed)?;
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.key_up(key).map_err(failed)?;
    wait(timing.gap, cancel)?;
    guard(adapter, cancel)?;
    adapter.key_up(modifier).map_err(failed)?;
    wait(timing.gap, cancel)
}

fn click(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
) -> Result<(), Outcome> {
    guard(adapter, cancel)?;
    adapter.mouse_left_down().map_err(failed)?;
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.mouse_left_up().map_err(failed)?;
    wait(timing.gap, cancel)
}

/// Shift-click removes one portrait from the current selection.
fn shift_click(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
) -> Result<(), Outcome> {
    guard(adapter, cancel)?;
    adapter.key_down(Key::Shift).map_err(failed)?;
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.mouse_left_down().map_err(failed)?;
    wait(timing.press, cancel)?;
    guard(adapter, cancel)?;
    adapter.mouse_left_up().map_err(failed)?;
    wait(timing.gap, cancel)?;
    guard(adapter, cancel)?;
    adapter.key_up(Key::Shift).map_err(failed)?;
    wait(timing.gap, cancel)
}

/// Waits (bounded) for a screenshot to match `accept`, returning the read.
fn await_selection(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    accept: impl Fn(&SelectionRead) -> bool,
) -> Result<SelectionRead, Outcome> {
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Outcome::Cancelled);
        }
        let frame = adapter.capture_client().map_err(failed)?;
        let read = vision::detect_selection(&frame);
        if accept(&read) {
            return Ok(read);
        }
        if Instant::now() >= deadline {
            return Err(Outcome::Aborted {
                detail: format!(
                    "the selection HUD did not reach the expected state in time (last read: \
                     {read:?})"
                ),
            });
        }
        wait(ui_poll(timing), cancel)?;
    }
}

/// After a portrait click the HUD must show exactly one selected drone.
fn expect_single(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
) -> Result<(), Outcome> {
    let read = await_selection(adapter, cancel, timing, |read| {
        matches!(read, SelectionRead::SingleDrone)
    })?;
    debug_assert!(matches!(read, SelectionRead::SingleDrone));
    Ok(())
}

/// After recalling group 9 the live count must be `pool` or `pool - 1`
/// (`pool == 2` may legitimately collapse to the single information panel).
fn expect_recall(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    pool: u8,
) -> Result<u8, Outcome> {
    let read = await_selection(adapter, cancel, timing, |read| match read {
        SelectionRead::Drones { count } => *count == pool || *count == pool - 1,
        SelectionRead::SingleDrone => pool == 2,
        SelectionRead::Rejected(_) => false,
    })?;
    Ok(match read {
        SelectionRead::Drones { count } => count,
        SelectionRead::SingleDrone => 1,
        SelectionRead::Rejected(_) => unreachable!("predicate excludes rejected reads"),
    })
}

/// After removing the travelling drone the pool must be exactly `count`.
fn expect_count(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    count: u8,
) -> Result<(), Outcome> {
    await_selection(adapter, cancel, timing, |read| match read {
        SelectionRead::Drones { count: found } => *found == count,
        SelectionRead::SingleDrone => count == 1,
        SelectionRead::Rejected(_) => false,
    })?;
    Ok(())
}

/// Issues the target's two build keys, waits for the green placement preview
/// near `point`, clicks once, and confirms placement mode closed.
///
/// Strict mode (`force == false`) refuses to click when no valid green preview
/// appears and aborts when the preview does not close, waiting up to
/// [`CAPTURE_TIMEOUT`] for each transition.
///
/// Forced mode (the user's choice, `force == true`) only confirms the preview
/// for [`FORCE_CONFIRM_TIMEOUT`]: a red preview is clicked immediately (it
/// cannot turn green while the cursor stays put), and a missing or ambiguous
/// preview is polled only for that brief window before the click is sent
/// anyway. If the preview never closes, the order is counted unconfirmed
/// exactly once and `Escape` is tapped once to leave placement mode — but only
/// while the last read positively shows a preview; an ambiguous read cannot
/// prove placement mode is still open.
fn place_building(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    target: BuildTarget,
    point: Point,
    force: bool,
    counts: &mut Counts,
) -> Result<(), Outcome> {
    for key in target.build_keys() {
        tap(adapter, cancel, timing, key)?;
    }

    // Strict mode stays conservative; forced mode never spends more than the
    // brief confirmation window on a preview it is going to click anyway.
    let confirm_timeout = if force {
        FORCE_CONFIRM_TIMEOUT
    } else {
        CAPTURE_TIMEOUT
    };

    let deadline = Instant::now() + confirm_timeout;
    let mut red_reads = 0usize;
    let preview = loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Outcome::Cancelled);
        }
        let frame = adapter.capture_client().map_err(failed)?;
        match vision::detect_placement(&frame, point, target.footprint_px()) {
            Placement::Valid { .. } => {
                break Placement::Valid {
                    center: Point::new(0, 0),
                };
            }
            confirmed @ (Placement::Invalid { .. } | Placement::Ambiguous) => {
                red_reads += 1;
                if !force && (red_reads >= INVALID_PREVIEW_READS || red_reads > 0) {
                    return Err(Outcome::Aborted {
                        detail: format!(
                            "the placement preview at ({}, {}) is blocked (red) or ambiguous; \
                             no click was sent",
                            point.x, point.y
                        ),
                    });
                }
                if force && matches!(confirmed, Placement::Invalid { .. }) {
                    // A red preview cannot turn green while the cursor stays
                    // put, so forced mode clicks right away instead of burning
                    // the confirmation window first.
                    break confirmed;
                }
                if Instant::now() >= deadline {
                    break confirmed;
                }
            }
            Placement::Absent => {
                if Instant::now() >= deadline {
                    break Placement::Absent;
                }
            }
        }
        wait(ui_poll(timing), cancel)?;
    };

    let preview_confirmed = matches!(preview, Placement::Valid { .. });
    if !preview_confirmed && !force {
        return Err(Outcome::Aborted {
            detail: format!(
                "no valid green placement preview appeared at ({}, {}); no click was sent",
                point.x, point.y
            ),
        });
    }

    // Forced mode clicks even without a confirmed preview: the cursor is
    // already on the planned target, so the worst case is a click the game
    // rejects.
    click(adapter, cancel, timing)?;

    // The order was placed: the preview must disappear. If it does not, the
    // click did not register. In forced mode leave placement mode with one
    // Escape so the next drone starts clean, and carry on.
    let deadline = Instant::now() + confirm_timeout;
    let mut closed = false;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Outcome::Cancelled);
        }
        let frame = adapter.capture_client().map_err(failed)?;
        let read = vision::detect_placement(&frame, point, target.footprint_px());
        if matches!(read, Placement::Absent) {
            closed = true;
            break;
        }
        if Instant::now() >= deadline {
            if !force {
                return Err(Outcome::Aborted {
                    detail: "the placement preview did not close after the click; stopping"
                        .to_owned(),
                });
            }
            // Still in placement mode: cancel it once, then continue. Only a
            // positively detected preview proves placement mode is open; an
            // ambiguous read must not turn into a blind Escape that could open
            // the game menu instead.
            if matches!(read, Placement::Valid { .. } | Placement::Invalid { .. }) {
                tap(adapter, cancel, timing, Key::Escape)?;
            }
            break;
        }
        wait(ui_poll(timing), cancel)?;
    }

    // Exactly one unconfirmed count per issued order: it was clicked without a
    // green preview, or the preview never closed after the click. An order
    // whose click was never sent (cancelled or refused) is never counted.
    if !preview_confirmed || !closed {
        counts.unconfirmed += 1;
    }
    Ok(())
}

/// Gap between screenshot attempts: the configured gap, but never a busy loop.
fn ui_poll(timing: Timing) -> Duration {
    timing.gap.max(Duration::from_millis(20))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Frame;
    use crate::input::{InputAdapter, InputError};
    use crate::macros::MacroId;
    use crate::vision::{CLIENT_HEIGHT, CLIENT_WIDTH, FOOTPRINT, synthetic};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum DroneStatus {
        Idle,
        Traveling,
        Morphed,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Preview {
        Green,
        Red,
        /// Both colours in one frame, which [`vision::detect_placement`] reports
        /// as [`Placement::Ambiguous`].
        Ambiguous,
        Absent,
    }

    struct Game {
        drones: Vec<DroneStatus>,
        selected: Vec<u8>,
        group9: Vec<u8>,
        build: Option<u8>,
        preview: Preview,
        /// Screen size of the preview the fake game paints, so the Spire path is
        /// exercised with a 3×3 sized overlay instead of a colony overlay.
        preview_footprint: i32,
        /// When false, clicks in placement mode are swallowed and the preview
        /// stays open, like a game that missed the click.
        placement_click_registers: bool,
        captures_since_order: u32,
        morph_delay: u32,
        cursor: Point,
        held: Vec<Key>,
        left_held: bool,
        nine_with_ctrl: bool,
        /// First key of a build pair that is currently latched (`B` or `V`).
        build_key_latched: Option<Key>,
        log: Vec<String>,
        orders: Vec<Point>,
        shifts: usize,
        saves: usize,
        recalls: usize,
        /// Number of captures the engine asked for.
        captures: usize,
        /// Number of `Escape` taps, i.e. placement-mode cleanups.
        escapes: usize,
        ops: usize,
        cancel: Arc<AtomicBool>,
        cancel_after: Option<usize>,
        fail_after: Option<usize>,
    }

    impl Game {
        fn new(count: u8) -> Self {
            Self {
                drones: vec![DroneStatus::Idle; count as usize],
                selected: (0..count).collect(),
                group9: Vec::new(),
                build: None,
                preview: Preview::Green,
                preview_footprint: vision::FOOTPRINT,
                placement_click_registers: true,
                captures_since_order: 0,
                morph_delay: 1_000,
                cursor: Point::new(400, 400),
                held: Vec::new(),
                left_held: false,
                nine_with_ctrl: false,
                build_key_latched: None,
                log: Vec::new(),
                orders: Vec::new(),
                shifts: 0,
                saves: 0,
                recalls: 0,
                captures: 0,
                escapes: 0,
                ops: 0,
                cancel: Arc::new(AtomicBool::new(false)),
                cancel_after: None,
                fail_after: None,
            }
        }

        fn slot_under_cursor(&self) -> Option<usize> {
            (0..self.selected.len()).find(|slot| {
                let center = vision::slot_center(*slot as u8);
                (center.x - self.cursor.x).abs() <= 40 && (center.y - self.cursor.y).abs() <= 40
            })
        }

        fn morph_tick(&mut self) {
            self.captures_since_order += 1;
            if self.captures_since_order >= self.morph_delay
                && let Some(traveling) = self
                    .drones
                    .iter()
                    .position(|status| *status == DroneStatus::Traveling)
            {
                self.drones[traveling] = DroneStatus::Morphed;
            }
        }

        fn render(&mut self) -> Frame {
            self.morph_tick();
            let mut frame = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
            match self.selected.len() {
                0 => {}
                1 => synthetic::paint_single(&mut frame, true),
                count => {
                    for slot in 0..count {
                        synthetic::paint_slot(&mut frame, slot as u8, true);
                    }
                }
            }
            if self.build.is_some() {
                match self.preview {
                    Preview::Green => synthetic::paint_preview(
                        &mut frame,
                        self.cursor,
                        true,
                        self.preview_footprint,
                    ),
                    Preview::Red => synthetic::paint_preview(
                        &mut frame,
                        self.cursor,
                        false,
                        self.preview_footprint,
                    ),
                    Preview::Ambiguous => {
                        // Two non-overlapping preview squares, both inside the
                        // detector's search window, so it sees green and red at
                        // once. One footprint apart keeps their rings clean.
                        let offset = self.preview_footprint / 2 + 8;
                        synthetic::paint_preview(
                            &mut frame,
                            Point::new(self.cursor.x, self.cursor.y - offset),
                            true,
                            self.preview_footprint,
                        );
                        synthetic::paint_preview(
                            &mut frame,
                            Point::new(self.cursor.x, self.cursor.y + offset),
                            false,
                            self.preview_footprint,
                        );
                    }
                    Preview::Absent => {}
                }
            }
            frame
        }

        /// Counts one adapter operation, applying the configured cancel and
        /// failure hooks so the tests can interrupt a run deterministically.
        fn note(&mut self) -> Result<(), InputError> {
            self.ops += 1;
            if let Some(fail_at) = self.fail_after
                && self.ops >= fail_at
            {
                return Err(InputError::Unsafe(
                    "test: foreground window changed".to_owned(),
                ));
            }
            if let Some(after) = self.cancel_after
                && self.ops >= after
            {
                self.cancel.store(true, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    struct FakeDesktop {
        game: Arc<Mutex<Game>>,
    }

    impl FakeDesktop {
        fn new(count: u8) -> Self {
            Self {
                game: Arc::new(Mutex::new(Game::new(count))),
            }
        }

        /// A fake game whose placement preview has the selected building's size,
        /// so the Spire path meets a 216 px overlay instead of a colony one.
        fn for_target(count: u8, target: BuildTarget) -> Self {
            let fake = Self::new(count);
            fake.with(|game| game.preview_footprint = target.footprint_px());
            fake
        }

        fn cancel_flag(&self) -> Arc<AtomicBool> {
            self.with(|game| Arc::clone(&game.cancel))
        }

        fn with<R>(&self, f: impl FnOnce(&mut Game) -> R) -> R {
            let mut game = self.game.lock().expect("game lock");
            f(&mut game)
        }

        fn events(&self) -> Vec<String> {
            self.with(|game| game.log.clone())
        }

        fn orders(&self) -> Vec<Point> {
            self.with(|game| game.orders.clone())
        }

        fn held(&self) -> Vec<Key> {
            self.with(|game| game.held.clone())
        }

        fn set_cursor(&self, point: Point) {
            self.with(|game| game.cursor = point);
        }

        fn set_preview(&self, preview: Preview) {
            self.with(|game| game.preview = preview);
        }

        fn set_placement_click_registers(&self, registers: bool) {
            self.with(|game| game.placement_click_registers = registers);
        }

        fn set_morph_delay(&self, captures: u32) {
            self.with(|game| game.morph_delay = captures);
        }

        fn cancel_after(&self, ops: usize) {
            self.with(|game| game.cancel_after = Some(ops));
        }

        fn fail_after(&self, ops: usize) {
            self.with(|game| game.fail_after = Some(ops));
        }

        fn apply(&self) -> Result<(), InputError> {
            let mut game = self.game.lock().expect("game lock");
            game.note()
        }
    }

    impl DesktopAdapter for FakeDesktop {
        fn cursor_position(&mut self) -> Result<Point, InputError> {
            self.apply()?;
            Ok(self.with(|game| game.cursor))
        }

        fn move_cursor(&mut self, point: Point) -> Result<(), InputError> {
            self.apply()?;
            self.with(|game| game.cursor = point);
            Ok(())
        }

        fn capture_client(&mut self) -> Result<Frame, InputError> {
            self.apply()?;
            Ok(self.with(|game| {
                game.captures += 1;
                game.render()
            }))
        }

        fn capture_region(&mut self, rect: crate::frame::Rect) -> Result<Frame, InputError> {
            let frame = self.capture_client()?;
            let mut rgba = Vec::with_capacity((rect.w * rect.h * 4) as usize);
            for y in 0..rect.h {
                for x in 0..rect.w {
                    let pixel = frame
                        .pixel(rect.x + x, rect.y + y)
                        .unwrap_or(crate::frame::Rgb::new(0, 0, 0));
                    rgba.extend_from_slice(&[pixel.r, pixel.g, pixel.b, 255]);
                }
            }
            Frame::new(
                rect.w as u32,
                rect.h as u32,
                Point::new(rect.x, rect.y),
                rgba,
            )
            .ok_or_else(|| InputError::Injection("test crop size mismatch".to_owned()))
        }
    }

    impl InputAdapter for FakeDesktop {
        fn key_down(&mut self, key: Key) -> Result<(), InputError> {
            self.apply()?;
            self.with(|game| {
                if !game.held.contains(&key) {
                    game.held.push(key);
                }
                if key == Key::Nine && game.held.contains(&Key::Control) {
                    game.nine_with_ctrl = true;
                }
                if matches!(key, Key::B | Key::V) {
                    game.build_key_latched = Some(key);
                }
                if key == Key::Escape {
                    game.escapes += 1;
                    // Escape leaves placement mode, so a swallowed click cannot
                    // leave the fake game latched in it.
                    game.build = None;
                }
            });
            Ok(())
        }

        fn key_up(&mut self, key: Key) -> Result<(), InputError> {
            self.apply()?;
            self.with(|game| {
                game.held.retain(|held| *held != key);
                if let Some(first) = game.build_key_latched {
                    let second = match first {
                        Key::B => Key::C,
                        _ => Key::S,
                    };
                    if key == second {
                        game.build_key_latched = None;
                        let pair = format!(
                            "{}{}",
                            first.name().to_ascii_lowercase(),
                            second.name().to_ascii_lowercase()
                        );
                        if game.selected.len() == 1 {
                            game.build = Some(game.selected[0]);
                            game.log.push(pair);
                        } else {
                            game.log.push(format!("{pair}-on-group"));
                        }
                    }
                }
                if key == Key::Nine {
                    if game.nine_with_ctrl {
                        game.nine_with_ctrl = false;
                        game.saves += 1;
                        game.group9 = game.selected.clone();
                        game.log.push(format!("save9({})", game.selected.len()));
                    } else {
                        game.recalls += 1;
                        let group = game.group9.clone();
                        game.selected = group
                            .into_iter()
                            .filter(|id| game.drones[*id as usize] != DroneStatus::Morphed)
                            .collect();
                        game.log.push(format!("recall9({})", game.selected.len()));
                    }
                }
            });
            Ok(())
        }

        fn mouse_left_down(&mut self) -> Result<(), InputError> {
            self.apply()?;
            self.with(|game| game.left_held = true);
            Ok(())
        }

        fn mouse_left_up(&mut self) -> Result<(), InputError> {
            self.apply()?;
            self.with(|game| {
                game.left_held = false;
                if let Some(id) = game.build.take() {
                    if !game.placement_click_registers {
                        // The click was swallowed: placement mode stays open.
                        game.build = Some(id);
                        game.log.push("click-swallowed".to_owned());
                        return;
                    }
                    game.drones[id as usize] = DroneStatus::Traveling;
                    game.captures_since_order = 0;
                    let target = game.cursor;
                    game.orders.push(target);
                    game.log.push(format!("place({},{})", target.x, target.y));
                } else if game.held.contains(&Key::Shift) {
                    if let Some(slot) = game.slot_under_cursor() {
                        game.selected.remove(slot);
                        game.shifts += 1;
                        game.log.push(format!("shift({slot})"));
                    }
                } else if let Some(slot) = game.slot_under_cursor() {
                    game.selected = vec![game.selected[slot]];
                    game.log.push(format!("select({slot})"));
                }
            });
            Ok(())
        }

        fn release_all(&mut self) -> Result<(), InputError> {
            self.with(|game| {
                game.held.clear();
                game.left_held = false;
                game.build = None;
            });
            Ok(())
        }

        fn safety_check(&mut self) -> Result<(), InputError> {
            self.apply()
        }
    }

    fn timing() -> Timing {
        Timing::from_millis(1, 1)
    }

    fn run(count: u8, target: BuildTarget) -> (FakeDesktop, ColonyReport) {
        let mut adapter = FakeDesktop::for_target(count, target);
        let cancel = adapter.cancel_flag();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            target,
            true,
        );
        (adapter, report)
    }

    fn run_mode(count: u8, mode: RowMode, target: BuildTarget) -> (FakeDesktop, ColonyReport) {
        let mut adapter = FakeDesktop::for_target(count, target);
        let cancel = adapter.cancel_flag();
        let report = run_row(&mut adapter, &cancel, timing(), mode, target, true);
        (adapter, report)
    }

    /// Highest `count` whose row still fits inside the safe area, derived from
    /// the same public constants `plan_row` uses (an independent oracle).
    /// How many footprints still fit to the right of `anchor_x`.
    ///
    /// The planner's rule is the play-area edge guard (24 px), so the helper
    /// mirrors exactly that instead of a footprint-based margin.
    fn max_count_that_fits(target: BuildTarget, anchor_x: i32) -> u8 {
        let min_x = crate::play_area::EDGE_GUARD;
        let max_x = CLIENT_WIDTH as i32 - crate::play_area::EDGE_GUARD;
        assert!(
            anchor_x >= min_x,
            "{anchor_x} is left of the safe area of {target:?}"
        );
        let steps = (max_x - anchor_x) / target.footprint_px();
        u8::try_from(steps + 1).expect("count fits in u8")
    }

    #[test]
    fn the_colony_footprint_is_the_calibrated_one() {
        // 144 px is the live-verified Creep Colony spacing of the supported
        // profile; the enum must not drift away from that calibration.
        assert_eq!(BuildTarget::Colony.footprint_px(), FOOTPRINT);
        assert_eq!(BuildTarget::Colony.half_px(), FOOTPRINT / 2);
    }

    #[test]
    fn plan_row_spaces_colony_footprints_by_144_to_the_right() {
        let plan = plan_row(
            Point::new(400, 400),
            3,
            RowMode::LeftToRight,
            BuildTarget::Colony,
        )
        .expect("fits");
        assert_eq!(plan.target, BuildTarget::Colony);
        assert_eq!(
            plan.targets,
            vec![
                Point::new(400, 400),
                Point::new(544, 400),
                Point::new(688, 400)
            ]
        );
    }

    #[test]
    fn plan_row_spaces_spire_footprints_by_144_to_the_right() {
        let plan = plan_row(
            Point::new(400, 400),
            3,
            RowMode::LeftToRight,
            BuildTarget::Spire,
        )
        .expect("fits");
        assert_eq!(plan.target, BuildTarget::Spire);
        assert_eq!(
            plan.targets,
            vec![
                Point::new(400, 400),
                Point::new(544, 400),
                Point::new(688, 400)
            ]
        );
    }

    /// Documented `EndsInward` footprint index order (0 = leftmost) for every
    /// supported drone count.
    const ENDS_INWARD_ORDERS: [(u8, &[u8]); 11] = [
        (2, &[0, 1]),
        (3, &[0, 2, 1]),
        (4, &[0, 3, 1, 2]),
        (5, &[0, 4, 1, 3, 2]),
        (6, &[0, 5, 1, 4, 2, 3]),
        (7, &[0, 6, 1, 5, 2, 4, 3]),
        (8, &[0, 7, 1, 6, 2, 5, 3, 4]),
        (9, &[0, 8, 1, 7, 2, 6, 3, 5, 4]),
        (10, &[0, 9, 1, 8, 2, 7, 3, 6, 4, 5]),
        (11, &[0, 10, 1, 9, 2, 8, 3, 7, 4, 6, 5]),
        (12, &[0, 11, 1, 10, 2, 9, 3, 8, 4, 7, 5, 6]),
    ];

    #[test]
    fn both_modes_place_the_same_span_for_every_target_and_count() {
        // The anchor is inside the safe area of both targets (a Spire needs a
        // wider margin because its 3x3 footprint is checked with its centre).
        let anchor = Point::new(200, 400);
        for target in BuildTarget::ALL {
            let footprint = target.footprint_px();
            let fit = max_count_that_fits(target, anchor.x);
            for (count, inward_order) in ENDS_INWARD_ORDERS {
                let left = plan_row(anchor, count, RowMode::LeftToRight, target);
                let inward = plan_row(anchor, count, RowMode::EndsInward, target);
                if count > fit {
                    // A row that does not fit is refused identically in both
                    // modes, before anything is injected.
                    assert_eq!(
                        left,
                        Err(RowError::RowDoesNotFit { count, fits: fit }),
                        "{target:?}, count {count}"
                    );
                    assert_eq!(inward, left, "{target:?}, count {count}");
                    continue;
                }
                let left = left.expect("fits");
                let inward = inward.expect("fits");
                assert_eq!(left.count, count);
                assert_eq!(inward.count, count);

                let expected_left: Vec<Point> = (0..count)
                    .map(|index| anchor.offset(footprint * i32::from(index), 0))
                    .collect();
                let expected_inward: Vec<Point> = inward_order
                    .iter()
                    .map(|index| anchor.offset(footprint * i32::from(*index), 0))
                    .collect();
                assert_eq!(
                    left.targets, expected_left,
                    "{target:?} left to right, count {count}"
                );
                assert_eq!(
                    inward.targets, expected_inward,
                    "{target:?} ends inward, count {count}"
                );

                // Same span, same targets, only the order differs. The cursor
                // keeps the leftmost footprint in both modes and every target
                // sits on the target's own grid.
                let mut left_sorted = left.targets.clone();
                left_sorted.sort_by_key(|target| target.x);
                let mut inward_sorted = inward.targets.clone();
                inward_sorted.sort_by_key(|target| target.x);
                assert_eq!(left_sorted, inward_sorted, "span, count {count}");
                assert_eq!(left.targets[0], anchor, "count {count}");
                assert_eq!(inward.targets[0], anchor, "count {count}");
                assert_eq!(
                    left_sorted.last().expect("target").x,
                    anchor.x + footprint * (i32::from(count) - 1),
                    "{target:?}, count {count}"
                );

                for plan in [&left, &inward] {
                    let offsets: Vec<i32> = plan
                        .targets
                        .iter()
                        .map(|point| {
                            let steps = (point.x - anchor.x) / footprint;
                            assert_eq!(
                                *point,
                                anchor.offset(footprint * steps, 0),
                                "target off the {footprint} px grid of {target:?}"
                            );
                            steps
                        })
                        .collect();
                    let mut sorted = offsets.clone();
                    sorted.sort_unstable();
                    assert_eq!(
                        sorted,
                        (0..i32::from(count)).collect::<Vec<_>>(),
                        "every footprint of the span is used exactly once, {target:?}, count {count}"
                    );
                }
            }
        }
    }

    #[test]
    fn eleven_and_twelve_colony_rows_fit_left_of_the_edge_guard() {
        // Regression: the horizontal check used to subtract
        // `48 + half footprint + slack` (152 px for a colony), so a cursor at
        // x >= ~330 refused eleven and twelve drones although the row fits on
        // screen. The clickpoint rule must accept them.
        for (anchor_x, count) in [(300, 12), (311, 12), (450, 11), (455, 11), (599, 10)] {
            let anchor = Point::new(anchor_x, 400);
            for mode in RowMode::ALL {
                let plan = plan_row(anchor, count, mode, BuildTarget::Colony)
                    .unwrap_or_else(|error| panic!("{anchor:?} count {count}: {error:?}"));
                assert_eq!(plan.count, count);
                let last_x = plan.targets.iter().map(|point| point.x).max().unwrap_or(0);
                assert!(
                    last_x < crate::play_area::CLIENT_WIDTH - crate::play_area::EDGE_GUARD,
                    "the widest footprint at {last_x} must stay inside the play area"
                );
            }
        }

        // One step further right the same row genuinely cannot fit, and the
        // refusal reports how many still would.
        assert_eq!(
            plan_row(
                Point::new(320, 400),
                12,
                RowMode::LeftToRight,
                BuildTarget::Colony
            ),
            Err(RowError::RowDoesNotFit {
                count: 12,
                fits: 11
            })
        );
        assert_eq!(
            plan_row(
                Point::new(460, 400),
                11,
                RowMode::LeftToRight,
                BuildTarget::Colony
            ),
            Err(RowError::RowDoesNotFit {
                count: 11,
                fits: 10
            })
        );
    }

    #[test]
    fn both_targets_share_the_same_two_tile_row_capacity() {
        // Both buildings are ordered on the same two-tile pitch now, so from
        // x=200 the full twelve-building row fits for either target
        // (11 * 144 px = 1584 px, ending at 1784, inside the 24 px edge guard).
        let anchor = Point::new(200, 400);
        for target in BuildTarget::ALL {
            assert_eq!(max_count_that_fits(target, anchor.x), 12);
        }

        let colony = plan_row(anchor, 12, RowMode::EndsInward, BuildTarget::Colony).expect("fits");
        let spire = plan_row(anchor, 12, RowMode::EndsInward, BuildTarget::Spire).expect("fits");
        assert_eq!(
            colony.targets[0], spire.targets[0],
            "both rows start at the cursor"
        );
        assert_eq!(
            colony.targets, spire.targets,
            "the same pitch must plan the same footprints"
        );

        // One more building than the span allows is refused, not clipped.
        assert_eq!(
            plan_row(
                Point::new(400, 400),
                12,
                RowMode::LeftToRight,
                BuildTarget::Colony
            ),
            Err(RowError::RowDoesNotFit {
                count: 12,
                fits: 11
            })
        );
        for mode in [RowMode::LeftToRight, RowMode::EndsInward] {
            let colony = plan_row(anchor, 12, mode, BuildTarget::Colony).expect("fits");
            let spire = plan_row(anchor, 12, mode, BuildTarget::Spire).expect("fits");
            assert_eq!(colony.targets, spire.targets, "same pitch, {mode:?}");
        }
    }

    #[test]
    fn a_single_building_ignores_the_row_mode() {
        let anchor = Point::new(400, 400);
        for target in BuildTarget::ALL {
            for mode in RowMode::ALL {
                assert_eq!(
                    plan_row(anchor, 1, mode, target).expect("fits").targets,
                    vec![anchor]
                );
            }
        }
    }

    #[test]
    fn the_prevalidation_is_the_same_for_both_single_row_modes_and_targets() {
        // Only the two single-row modes cover the same span; `Grid6x2` lays the
        // row out in two columns-wise rows and is covered separately below.
        for target in BuildTarget::ALL {
            for mode in [RowMode::LeftToRight, RowMode::EndsInward] {
                // Console points and a screen-edge point: refused identically
                // for every target and row order, before any input.
                for anchor in [
                    Point::new(200, 740),
                    Point::new(400, HUD_TOP - 10),
                    Point::new(5, 400),
                ] {
                    assert_eq!(
                        plan_row(anchor, 3, mode, target),
                        Err(RowError::AnchorOutsideSafeArea),
                        "{anchor:?} in {mode:?} for {target:?}"
                    );
                }
                assert_eq!(
                    plan_row(Point::new(1790, 400), 2, mode, target),
                    Err(RowError::RowDoesNotFit { count: 2, fits: 1 }),
                    "{mode:?} for {target:?}"
                );
            }
        }
        // The widest row each target can still place, in both single-row modes.
        for mode in [RowMode::LeftToRight, RowMode::EndsInward] {
            for target in BuildTarget::ALL {
                assert!(plan_row(Point::new(160, 400), 12, mode, target).is_ok());
                assert!(plan_row(Point::new(400, 400), 12, mode, target).is_err());
            }
        }
    }

    #[test]
    fn the_row_mode_string_form_is_stable_and_round_trips() {
        assert_eq!(RowMode::default(), RowMode::LeftToRight);
        assert_eq!(RowMode::LeftToRight.as_str(), "left_to_right");
        assert_eq!(RowMode::EndsInward.as_str(), "ends_inward");
        for mode in RowMode::ALL {
            assert_eq!(RowMode::from_str(mode.as_str()), Some(mode));
        }
        assert_eq!(RowMode::from_str("leftToRight"), None);
        assert_eq!(RowMode::from_str(""), None);
    }

    #[test]
    fn plan_row_refuses_the_hud_and_the_screen_edge() {
        // These points are inside the real console: the minimap console at
        // (200, 740) and the central console at (400, HUD_TOP - 10), which the
        // calibrated skyline refuses with its margin.
        for target in BuildTarget::ALL {
            assert!(plan_row(Point::new(200, 740), 1, RowMode::LeftToRight, target).is_err());
            assert_eq!(
                plan_row(
                    Point::new(400, HUD_TOP - 10),
                    1,
                    RowMode::LeftToRight,
                    target
                ),
                Err(RowError::AnchorOutsideSafeArea)
            );
            assert_eq!(
                plan_row(Point::new(5, 400), 1, RowMode::LeftToRight, target),
                Err(RowError::AnchorOutsideSafeArea)
            );
            assert_eq!(
                plan_row(Point::new(1790, 400), 2, RowMode::LeftToRight, target),
                Err(RowError::RowDoesNotFit { count: 2, fits: 1 })
            );
            assert!(plan_row(Point::new(200, 400), 1, RowMode::LeftToRight, target).is_ok());
        }
    }

    #[test]
    fn plan_row_accepts_visible_top_and_lower_central_points_for_both_targets() {
        for target in BuildTarget::ALL {
            // Top visible playfield: anchor at y=30 (previously refused because min_y was 152/188)
            let top_anchor = Point::new(400, 30);
            assert!(
                plan_row(top_anchor, 1, RowMode::LeftToRight, target).is_ok(),
                "top visible playfield at {top_anchor:?} should be accepted for {target:?}"
            );

            // Lower central playfield: anchor at (800, 720) (previously refused because max_y was 528/492)
            let lower_center = Point::new(800, 720);
            assert!(
                plan_row(lower_center, 1, RowMode::LeftToRight, target).is_ok(),
                "lower central corridor at {lower_center:?} should be accepted for {target:?}"
            );

            // True HUD points must be rejected:
            // Minimap area
            assert_eq!(
                plan_row(Point::new(200, 740), 1, RowMode::LeftToRight, target),
                Err(RowError::AnchorOutsideSafeArea)
            );
            // Minimap claw/horn
            assert_eq!(
                plan_row(Point::new(60, 650), 1, RowMode::LeftToRight, target),
                Err(RowError::AnchorOutsideSafeArea)
            );
            // Command card area
            assert_eq!(
                plan_row(Point::new(1600, 720), 1, RowMode::LeftToRight, target),
                Err(RowError::AnchorOutsideSafeArea)
            );
            // Resource bar in top right
            assert_eq!(
                plan_row(Point::new(1600, 20), 1, RowMode::LeftToRight, target),
                Err(RowError::AnchorOutsideSafeArea)
            );
        }
    }

    #[test]
    fn plan_row_refuses_when_later_target_crosses_hud_contour() {
        // Anchor at (1200, 720) is in the central corridor and is playable.
        // For Colony (footprint 144): target 0 = (1200, 720).
        // Target 1 = (1344, 720). At x=1344, command card skyline is 710, so (1344, 720) is inside HUD!
        // The entire row must be wholly refused before input.
        assert_eq!(
            plan_row(
                Point::new(1200, 720),
                2,
                RowMode::LeftToRight,
                BuildTarget::Colony
            ),
            Err(RowError::RowDoesNotFit { count: 2, fits: 1 })
        );
        // Single building at (1200, 720) fits:
        assert!(
            plan_row(
                Point::new(1200, 720),
                1,
                RowMode::LeftToRight,
                BuildTarget::Colony
            )
            .is_ok()
        );
    }

    #[test]
    fn an_ends_inward_run_issues_orders_in_that_order() {
        let (adapter, report) = run_mode(5, RowMode::EndsInward, BuildTarget::Colony);
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.target, BuildTarget::Colony);
        assert_eq!(report.detected, 5);
        assert_eq!(report.orders_issued, 5);
        assert_eq!(
            adapter.orders(),
            vec![
                Point::new(400, 400),
                Point::new(976, 400),
                Point::new(544, 400),
                Point::new(832, 400),
                Point::new(688, 400),
            ]
        );
        assert!(adapter.held().is_empty(), "no key may stay held");
    }

    #[test]
    fn a_spire_row_presses_v_s_and_spaces_the_orders_by_144() {
        // The fake desktop is keyed on the build pair and on the target the
        // run planned, so this pins the V,S order and the 144 px spacing, and
        // its synthetic preview is painted at the Spire's 144 px size.
        let (fake, report) = run(3, BuildTarget::Spire);
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.target, BuildTarget::Spire);
        assert_eq!(report.detected, 3);
        assert_eq!(report.orders_issued, 3);
        assert_eq!(
            report.into_run_report().macro_id,
            MacroId::Spire,
            "the report must say which building was ordered"
        );
        assert_eq!(
            fake.orders(),
            vec![
                Point::new(400, 400),
                Point::new(544, 400),
                Point::new(688, 400),
            ],
            "spire footprint is 144 px wide now"
        );
        assert_eq!(
            fake.events().join(" "),
            "save9(3) select(2) vs place(400,400) recall9(3) shift(2) save9(2) \
             select(1) vs place(544,400) recall9(2) shift(1) save9(1) vs place(688,400)"
        );
        assert!(fake.held().is_empty(), "no key may stay held");
    }

    #[test]
    fn five_drones_shrink_group_nine_one_portrait_at_a_time() {
        let (fake, report) = run(5, BuildTarget::Colony);
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.detected, 5);
        assert_eq!(report.orders_issued, 5);
        assert_eq!(
            report.unconfirmed, 0,
            "every green preview was confirmed before its click"
        );
        assert_eq!(
            fake.events().join(" "),
            "save9(5) select(4) bc place(400,400) recall9(5) shift(4) save9(4) \
             select(3) bc place(544,400) recall9(4) shift(3) save9(3) \
             select(2) bc place(688,400) recall9(3) shift(2) save9(2) \
             select(1) bc place(832,400) recall9(2) shift(1) save9(1) bc place(976,400)"
        );
        assert_eq!(
            fake.orders(),
            vec![
                Point::new(400, 400),
                Point::new(544, 400),
                Point::new(688, 400),
                Point::new(832, 400),
                Point::new(976, 400),
            ]
        );
        assert!(fake.held().is_empty(), "no key may stay held");
    }

    #[test]
    fn an_already_morphed_drone_is_not_shift_clicked_again() {
        let mut adapter = FakeDesktop::new(3);
        adapter.set_morph_delay(1);
        let cancel = adapter.cancel_flag();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.orders_issued, 3);
        assert_eq!(adapter.with(|game| game.shifts), 0);
        assert_eq!(
            adapter.events().join(" "),
            "save9(3) select(2) bc place(400,400) recall9(2) save9(2) \
             select(1) bc place(544,400) recall9(1) save9(1) bc place(688,400)"
        );
    }

    #[test]
    fn two_drones_collapse_to_the_single_panel_and_still_build_both() {
        let mut adapter = FakeDesktop::new(2);
        adapter.set_morph_delay(1);
        let cancel = adapter.cancel_flag();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.orders_issued, 2);
        assert_eq!(adapter.with(|game| game.shifts), 0);
        assert_eq!(
            adapter.orders(),
            vec![Point::new(400, 400), Point::new(544, 400)]
        );
    }

    #[test]
    fn a_single_drone_builds_once_at_the_cursor_without_group_nine() {
        for target in BuildTarget::ALL {
            let (fake, report) = run(1, target);
            assert_eq!(report.outcome, Outcome::Completed, "{target:?}");
            assert_eq!(report.target, target);
            assert_eq!(report.detected, 1);
            assert_eq!(report.orders_issued, 1);
            assert_eq!(
                report.unconfirmed, 0,
                "a confirmed green preview is never counted unconfirmed"
            );
            assert_eq!(fake.with(|game| game.saves), 0, "no scratch group needed");
            assert_eq!(fake.orders(), vec![Point::new(400, 400)]);
            let expected = if target == BuildTarget::Spire {
                "vs place(400,400)"
            } else {
                "bc place(400,400)"
            };
            assert_eq!(fake.events().join(" "), expected, "{target:?}");
        }
    }

    #[test]
    fn a_row_that_does_not_fit_is_refused_before_injecting_anything() {
        // From x=400 a twelve-building row (11 * 144 px) runs past the edge
        // guard, for both targets, because they share the pitch.
        for (count, target) in [(12, BuildTarget::Colony), (12, BuildTarget::Spire)] {
            let mut adapter = FakeDesktop::new(count);
            adapter.set_cursor(Point::new(400, 400));
            let cancel = adapter.cancel_flag();
            let report = run_row(
                &mut adapter,
                &cancel,
                timing(),
                RowMode::LeftToRight,
                target,
                true,
            );
            assert!(
                matches!(report.outcome, Outcome::Aborted { .. }),
                "{target:?}, count {count}"
            );
            assert_eq!(report.orders_issued, 0);
            assert_eq!(
                report.detected, count,
                "the refusal happens after the HUD read, before any injected event"
            );
            assert!(adapter.orders().is_empty());
            assert!(
                adapter.events().is_empty(),
                "{}",
                adapter.events().join(" ")
            );
        }
    }

    #[test]
    fn an_anchor_over_the_hud_is_refused_without_capturing() {
        for target in BuildTarget::ALL {
            let mut adapter = FakeDesktop::new(3);
            adapter.set_cursor(Point::new(400, 1000));
            let cancel = adapter.cancel_flag();
            let report = run_row(
                &mut adapter,
                &cancel,
                timing(),
                RowMode::LeftToRight,
                target,
                false,
            );
            assert!(
                matches!(report.outcome, Outcome::Aborted { .. }),
                "{target:?}"
            );
            assert_eq!(report.detected, 0);
        }
    }

    #[test]
    fn a_red_preview_is_never_clicked_in_strict_mode() {
        for target in BuildTarget::ALL {
            let mut adapter = FakeDesktop::new(2);
            adapter.set_preview(Preview::Red);
            let cancel = adapter.cancel_flag();
            let report = run_row(
                &mut adapter,
                &cancel,
                timing(),
                RowMode::LeftToRight,
                target,
                false,
            );
            assert!(
                matches!(report.outcome, Outcome::Aborted { .. }),
                "{target:?}"
            );
            assert_eq!(report.orders_issued, 0);
            assert_eq!(
                report.unconfirmed, 0,
                "nothing was clicked, nothing counted"
            );
            assert_eq!(adapter.with(|game| game.escapes), 0);
            assert!(adapter.orders().is_empty());
            assert!(adapter.held().is_empty());
        }
    }

    #[test]
    fn a_missing_preview_is_never_clicked_in_strict_mode() {
        for target in BuildTarget::ALL {
            let mut adapter = FakeDesktop::new(2);
            adapter.set_preview(Preview::Absent);
            let cancel = adapter.cancel_flag();
            let report = run_row(
                &mut adapter,
                &cancel,
                timing(),
                RowMode::LeftToRight,
                target,
                false,
            );
            assert!(
                matches!(report.outcome, Outcome::Aborted { .. }),
                "{target:?}"
            );
            assert_eq!(
                report.unconfirmed, 0,
                "nothing was clicked, nothing counted"
            );
            assert_eq!(adapter.with(|game| game.escapes), 0);
            assert!(adapter.orders().is_empty());
        }
    }

    #[test]
    fn force_mode_confirms_a_missing_preview_only_briefly_and_counts_once() {
        let mut adapter = FakeDesktop::new(2);
        adapter.set_preview(Preview::Absent);
        let cancel = adapter.cancel_flag();
        let started = Instant::now();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        let elapsed = started.elapsed();
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.orders_issued, 2);
        assert_eq!(report.unconfirmed, 2, "one count per forced order");
        assert!(report.unconfirmed <= report.orders_issued);
        assert_eq!(adapter.orders().len(), 2);
        assert!(
            elapsed < CAPTURE_TIMEOUT,
            "forced mode spent {elapsed:?} on a preview it clicks anyway"
        );
        let captures = adapter.with(|game| game.captures);
        assert!(
            captures <= 40,
            "forced mode polled the preview {captures} times"
        );
    }

    #[test]
    fn force_mode_clicks_a_red_preview_without_waiting_for_the_timeout() {
        let mut adapter = FakeDesktop::new(2);
        adapter.set_preview(Preview::Red);
        let cancel = adapter.cancel_flag();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.orders_issued, 2);
        assert_eq!(report.unconfirmed, 2);
        assert_eq!(adapter.orders().len(), 2);
        // A red preview cannot turn green while the cursor stays put, so there
        // is nothing to poll for: every capture is one of the fixed state
        // checks, not a retry of the preview.
        let captures = adapter.with(|game| game.captures);
        assert!(captures <= 10, "red preview was polled {captures} times");
        assert_eq!(adapter.with(|game| game.escapes), 0);
    }

    #[test]
    fn force_mode_bounds_an_ambiguous_preview_to_the_brief_window() {
        let mut adapter = FakeDesktop::new(2);
        adapter.set_preview(Preview::Ambiguous);
        let cancel = adapter.cancel_flag();
        let started = Instant::now();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        let elapsed = started.elapsed();
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.orders_issued, 2);
        assert_eq!(report.unconfirmed, 2);
        assert_eq!(adapter.orders().len(), 2);
        assert!(
            elapsed < CAPTURE_TIMEOUT,
            "forced mode spent {elapsed:?} on an ambiguous preview"
        );
        let captures = adapter.with(|game| game.captures);
        assert!(
            captures <= 40,
            "ambiguous preview was polled {captures} times"
        );
    }

    #[test]
    fn a_forced_click_that_never_closes_is_counted_once_and_escaped_once() {
        let mut adapter = FakeDesktop::new(2);
        adapter.set_preview(Preview::Red);
        adapter.set_placement_click_registers(false);
        let cancel = adapter.cancel_flag();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.orders_issued, 2);
        assert_eq!(
            report.unconfirmed, 2,
            "one per issued order, never the pre- and post-click count"
        );
        assert!(
            adapter.orders().is_empty(),
            "the fake game swallowed both clicks"
        );
        assert_eq!(
            adapter.with(|game| game.escapes),
            2,
            "one Escape per unclosed placement"
        );
    }

    #[test]
    fn an_ambiguous_preview_never_triggers_a_blind_escape() {
        let mut adapter = FakeDesktop::new(1);
        adapter.set_preview(Preview::Ambiguous);
        adapter.set_placement_click_registers(false);
        let cancel = adapter.cancel_flag();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.orders_issued, 1);
        assert_eq!(report.unconfirmed, 1);
        assert_eq!(
            adapter.with(|game| game.escapes),
            0,
            "an ambiguous read cannot prove placement mode is open"
        );
        assert!(adapter.orders().is_empty());
    }

    #[test]
    fn cancelling_during_the_forced_confirmation_never_counts_an_unsent_order() {
        // The 12th adapter call is the first preview capture of the forced
        // confirmation window: cancel there, before the forced click.
        let mut adapter = FakeDesktop::new(1);
        adapter.set_preview(Preview::Absent);
        adapter.cancel_after(12);
        let cancel = adapter.cancel_flag();
        let started = Instant::now();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert_eq!(report.orders_issued, 0);
        assert_eq!(report.unconfirmed, 0, "an unsent order is never counted");
        assert!(adapter.orders().is_empty());
        assert!(adapter.held().is_empty());
        assert!(
            started.elapsed() < CAPTURE_TIMEOUT,
            "cancellation waited for the strict timeout"
        );
    }

    #[test]
    fn cancelling_before_start_does_not_even_move_or_capture() {
        let mut adapter = FakeDesktop::new(5);
        let cancel = adapter.cancel_flag();
        cancel.store(true, Ordering::SeqCst);
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert_eq!(adapter.with(|game| game.ops), 0);
        assert!(adapter.events().is_empty());
    }

    #[test]
    fn cancelling_mid_run_stops_and_releases_everything() {
        let mut adapter = FakeDesktop::new(5);
        adapter.cancel_after(8);
        let cancel = adapter.cancel_flag();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert!(adapter.held().is_empty());
        assert!(report.orders_issued <= 5);
    }

    #[test]
    fn a_safety_refusal_stops_without_further_clicks() {
        let mut adapter = FakeDesktop::new(3);
        adapter.fail_after(6);
        let cancel = adapter.cancel_flag();
        let report = run_row(
            &mut adapter,
            &cancel,
            timing(),
            RowMode::LeftToRight,
            BuildTarget::Colony,
            true,
        );
        assert!(matches!(report.outcome, Outcome::Failed { .. }));
        assert!(adapter.held().is_empty());
    }
}
