//! Row-build compatibility wrapper that caps live F6/F7 runs at ten drones.
//!
//! The original state machine in `colony.rs` remains unchanged. Before it
//! starts, selections of 11 or 12 drones are trimmed to ten by Shift-clicking
//! the last occupied wireframe portraits. This keeps the existing group-9
//! protocol internally consistent: the state machine really sees ten selected
//! drones instead of merely pretending a 12-drone group is a 10-drone group.

#[path = "colony.rs"]
mod inner;

pub use inner::{
    CAPTURE_TIMEOUT, EDGE_MARGIN, HUD_TOP, SNAP_SLACK, ColonyReport, RowError, RowMode, RowPlan,
    plan_row,
};

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::engine::{CANCEL_POLL_INTERVAL, Outcome};
use crate::frame::Point;
use crate::input::{DesktopAdapter, InputError};
use crate::macros::{BuildTarget, Key, Timing};
use crate::vision::{self, SelectionRead};

/// Maximum number of selected drones the live row-build macro will consume.
///
/// StarCraft can still have 11 or 12 drones selected; the wrapper simply
/// removes the last one or two portraits from the selection before handing the
/// run to the original row state machine.
pub const MAX_ROW_BUILD_DRONES: u8 = 10;

/// Runs the original row-build macro after reducing an 11/12-drone selection
/// to ten drones.
pub fn run_row(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    mode: RowMode,
    target: BuildTarget,
    force: bool,
) -> ColonyReport {
    match trim_selection_if_needed(adapter, cancel, timing, mode, target) {
        Ok(()) => inner::run_row(adapter, cancel, timing, mode, target, force),
        Err((outcome, detected)) => {
            let outcome = match (outcome, adapter.release_all()) {
                (outcome @ (Outcome::Failed { .. } | Outcome::Aborted { .. }), _) => outcome,
                (_, Err(error)) => Outcome::Failed {
                    detail: format!("could not release injected input: {error}"),
                },
                (outcome, Ok(())) => outcome,
            };
            ColonyReport {
                outcome,
                target,
                orders_issued: 0,
                detected,
                unconfirmed: 0,
            }
        }
    }
}

/// Leaves selections of ten or fewer untouched. For 11/12, remove the last
/// occupied portraits until exactly ten remain, verifying the HUD after each
/// removal before the original state machine is allowed to run.
fn trim_selection_if_needed(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    mode: RowMode,
    target: BuildTarget,
) -> Result<(), (Outcome, u8)> {
    if cancel.load(Ordering::SeqCst) {
        return Err((Outcome::Cancelled, 0));
    }

    guard(adapter, cancel).map_err(|outcome| (outcome, 0))?;
    let anchor = adapter
        .cursor_position()
        .map_err(|error| (failed(error), 0))?;

    // Preserve the original macro's safety rule: never click a HUD portrait if
    // the starting cursor itself is not a valid world anchor.
    if plan_row(anchor, 1, mode, target).is_err() {
        return Ok(());
    }

    adapter
        .move_cursor(anchor)
        .map_err(|error| (failed(error), 0))?;
    let frame = adapter
        .capture_client()
        .map_err(|error| (failed(error), 0))?;
    let detected = match vision::detect_selection(&frame) {
        SelectionRead::Drones { count } => count,
        // Let the original state machine produce its existing detailed error
        // for rejected/single selections.
        SelectionRead::SingleDrone | SelectionRead::Rejected(_) => return Ok(()),
    };

    if detected <= MAX_ROW_BUILD_DRONES {
        return Ok(());
    }

    // Validate the row we actually intend to build before changing selection.
    // If ten buildings do not fit, fail without deselecting any drones.
    if let Err(error) = plan_row(anchor, MAX_ROW_BUILD_DRONES, mode, target) {
        return Err((
            Outcome::Aborted {
                detail: error.report_detail(),
            },
            detected,
        ));
    }

    let mut current = detected;
    while current > MAX_ROW_BUILD_DRONES {
        let last_slot = current - 1;
        adapter
            .move_cursor(vision::slot_center(last_slot))
            .map_err(|error| (failed(error), detected))?;
        shift_click(adapter, cancel, timing).map_err(|outcome| (outcome, detected))?;

        current -= 1;
        adapter
            .move_cursor(anchor)
            .map_err(|error| (failed(error), detected))?;
        expect_count(adapter, cancel, timing, current)
            .map_err(|outcome| (outcome, detected))?;
    }

    // The inner implementation snapshots the cursor as its row anchor, so put
    // it back exactly where the user pressed the hotkey.
    adapter
        .move_cursor(anchor)
        .map_err(|error| (failed(error), detected))?;
    Ok(())
}

fn failed(error: InputError) -> Outcome {
    Outcome::Failed {
        detail: error.to_string(),
    }
}

fn guard(adapter: &mut dyn DesktopAdapter, cancel: &AtomicBool) -> Result<(), Outcome> {
    if cancel.load(Ordering::SeqCst) {
        return Err(Outcome::Cancelled);
    }
    adapter.safety_check().map_err(failed)
}

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

fn expect_count(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    expected: u8,
) -> Result<(), Outcome> {
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Outcome::Cancelled);
        }

        let frame = adapter.capture_client().map_err(failed)?;
        let read = vision::detect_selection(&frame);
        if matches!(read, SelectionRead::Drones { count } if count == expected) {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(Outcome::Aborted {
                detail: format!(
                    "could not reduce the selected drone group to {expected} before the row build (last read: {read:?})"
                ),
            });
        }

        wait(timing.gap.max(Duration::from_millis(20)), cancel)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_row_build_limit_is_ten() {
        assert_eq!(MAX_ROW_BUILD_DRONES, 10);
        assert!(11 > MAX_ROW_BUILD_DRONES);
        assert!(12 > MAX_ROW_BUILD_DRONES);
    }
}
