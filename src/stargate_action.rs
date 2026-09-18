//! The Stargate action: one capture, click each detected Stargate, verify,
//! press `A`.
//!
//! This is the Stargate wrapper around the shared [`crate::building_action`]
//! executor, which documents the flow and the safety rules (one full capture
//! and one full search per run, a safety gate before every event, bounded
//! selection-panel reads, no `A` without a verified selection, cancellation,
//! cleanup). The whole action was requested as "click each Stargate, then press
//! `A` once per verified selection": `A` is sent exactly as pressed on the
//! keyboard, with no attempt to substitute a presumed production hotkey.
//!
//! Everything talks to [`DesktopAdapter`], so the whole flow is exercised on
//! any host with a fake capture/input adapter.

use std::sync::atomic::AtomicBool;

use crate::building_action::{self, BuildingActionReport};
use crate::input::DesktopAdapter;
use crate::macros::Timing;

pub use crate::building_action::{
    BuildingActionOutcome, BuildingTargetDisposition, BuildingTargetReport, VERIFY_ATTEMPTS,
};

/// How one Stargate action run ended.
pub type StargateActionOutcome = BuildingActionOutcome;
/// What happened to one detected Stargate.
pub type StargateTargetDisposition = BuildingTargetDisposition;
/// Per-target detail.
pub type StargateTargetReport = BuildingTargetReport;
/// Everything one Stargate action run did, ready to log or display.
pub type StargateActionReport = BuildingActionReport;

/// A report for a Stargate run that never started (internal failure).
pub fn failed(detail: impl Into<String>) -> StargateActionReport {
    BuildingActionReport::failed(detail, "Stargate")
}

/// Runs the Stargate action once.
///
/// Never panics on adapter errors, always releases what it pressed, and returns
/// a report for every path.
pub fn run(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
) -> StargateActionReport {
    building_action::run(adapter, cancel, timing, &crate::stargate_vision::PROFILE)
}
