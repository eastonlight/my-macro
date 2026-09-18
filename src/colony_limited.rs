//! Row-build compatibility wrapper that caps long single-row runs at ten drones.
//!
//! The original state machine in `colony.rs` remains unchanged. For the two
//! single-row modes, selections of 11 or 12 drones are trimmed to ten by
//! Shift-clicking the last occupied wireframe portraits. The compact `6x2`
//! mode bypasses that cap and uses the full selected count (up to all 12).

#[path = "colony.rs"]
mod inner;

/// Shared guarded primitives; the F4 feature bypasses only the row-count cap,
/// not selection verification, cancellation or owned-input cleanup.
pub(crate) mod controls {
    pub(crate) use super::inner::{
        chord, click, expect_count, expect_recall, expect_single, guard, shift_click, tap, wait,
    };
}

pub use inner::{
    CAPTURE_TIMEOUT, ColonyReport, EDGE_MARGIN, HUD_TOP, RowError, RowMode, RowPlan, SNAP_SLACK,
    plan_row,
};

use std::sync::atomic::{AtomicBool, Ordering};

use crate::engine::Outcome;
use crate::input::{DesktopAdapter, InputError};
use crate::macros::{BuildTarget, Timing};
use crate::vision::{self, SelectionRead};
use inner::{expect_count, guard, shift_click};

/// Maximum number of selected drones consumed by the single-row modes.
pub const MAX_ROW_BUILD_DRONES: u8 = 10;

/// Runs the original row-build macro after applying the ten-drone cap only to
/// the single-row modes. `Grid6x2` always keeps the full selection.
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

/// `Grid6x2` needs no trimming: its 6-column span fits all twelve drones. The
/// single-row modes keep the existing ten-drone cap.
fn trim_selection_if_needed(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    mode: RowMode,
    target: BuildTarget,
) -> Result<(), (Outcome, u8)> {
    if mode == RowMode::Grid6x2 {
        return Ok(());
    }

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
        .capture_region(vision::SELECTION_ROI)
        .map_err(|error| (failed(error), 0))?;
    guard(adapter, cancel).map_err(|outcome| (outcome, 0))?;
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
        expect_count(adapter, cancel, timing, current).map_err(|outcome| (outcome, detected))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_row_limit_is_ten_but_grid_6x2_is_uncapped() {
        assert_eq!(MAX_ROW_BUILD_DRONES, 10);
        assert_ne!(RowMode::Grid6x2, RowMode::LeftToRight);
        assert_ne!(RowMode::Grid6x2, RowMode::EndsInward);
    }
}
