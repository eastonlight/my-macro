//! Thread supervisor for macro runs.
//!
//! Guarantees:
//!
//! * at most one run at a time — a second trigger is rejected with
//!   [`StartError::Busy`] instead of queueing or overlapping;
//! * the worker is always joined again, so no thread is left behind (also on
//!   drop, i.e. when the GUI window is closed);
//! * cancellation is cooperative and prompt, because the engine polls the flag
//!   every few milliseconds;
//! * the worker calls `release_all` even if the engine panics, so a crash can
//!   never leave a key or mouse button stuck in the game.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use crate::colony::{self, RowMode};
use crate::engine::{self, RunReport};
use crate::hotkey::HotkeySlot;
use crate::input::{DesktopAdapter, InputAdapter};
use crate::macros::{BuildTarget, MacroId, Timing};
use crate::spire_action::{self, SpireActionReport, SpireScanError, SpireScanReport};

/// What a registered hotkey does while the app is armed.
///
/// There is exactly one trigger: it starts the row-build macro, and the
/// configured [`BuildTarget`] decides which building it orders. The emergency
/// key stops a run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotkeyAction {
    /// Start the row-build macro.
    StartRowBuild,
    /// Start the Spire action (scan, click each crown, verified `A`).
    StartSpireAction,
    /// Stop the running macro immediately.
    EmergencyStop,
}

/// The pure routing rule the GUI applies to one hotkey slot.
pub const fn action_for(slot: HotkeySlot) -> HotkeyAction {
    match slot {
        HotkeySlot::Trigger => HotkeyAction::StartRowBuild,
        HotkeySlot::SpireAction => HotkeyAction::StartSpireAction,
        HotkeySlot::Emergency => HotkeyAction::EmergencyStop,
    }
}

/// Why a run could not be started.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartError {
    /// A macro is still running. Triggers are dropped, never queued.
    Busy,
    /// The worker thread could not be created (out of resources).
    Thread { detail: String },
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("a macro is already running"),
            Self::Thread { detail } => write!(f, "could not start the macro thread: {detail}"),
        }
    }
}

impl std::error::Error for StartError {}

/// Everything that finished since the last call.
///
/// At most one field can be set, because one worker slot serves all three
/// runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FinishedReports {
    pub row: Option<RunReport>,
    pub spire_action: Option<SpireActionReport>,
    pub spire_scan: Option<Result<SpireScanReport, SpireScanError>>,
}

impl FinishedReports {
    /// True when nothing had finished.
    pub const fn is_empty(&self) -> bool {
        self.row.is_none() && self.spire_action.is_none() && self.spire_scan.is_none()
    }
}

/// Owns the macro worker thread and the shared cancel flag.
pub struct MacroRunner {
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    reports: Receiver<RunReport>,
    report_sender: Sender<RunReport>,
    spire_reports: Receiver<SpireActionReport>,
    spire_report_sender: Sender<SpireActionReport>,
    spire_scan_reports: Receiver<Result<SpireScanReport, SpireScanError>>,
    spire_scan_report_sender: Sender<Result<SpireScanReport, SpireScanError>>,
}

impl MacroRunner {
    pub fn new() -> Self {
        let (report_sender, reports) = std::sync::mpsc::channel();
        let (spire_report_sender, spire_reports) = std::sync::mpsc::channel();
        let (spire_scan_report_sender, spire_scan_reports) = std::sync::mpsc::channel();
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
            reports,
            report_sender,
            spire_reports,
            spire_report_sender,
            spire_scan_reports,
            spire_scan_report_sender,
        }
    }

    /// Shared cancel flag.
    ///
    /// The hotkey listener keeps a clone of this so that F8 can stop a running
    /// macro immediately, without waiting for the GUI to react.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    pub fn is_running(&mut self) -> bool {
        self.worker.is_some()
    }

    /// Starts one run of `macro_id`.
    ///
    /// The adapter is created by the caller (on the GUI thread) and moved into
    /// the worker, which keeps adapter construction failures synchronous.
    pub fn try_start(
        &mut self,
        macro_id: MacroId,
        timing: Timing,
        adapter: Box<dyn InputAdapter>,
    ) -> Result<(), StartError> {
        if self.worker.is_some() {
            return Err(StartError::Busy);
        }

        // Cancellation is latched until explicit rearming, not cleared by a
        // trigger that may have been queued just before the emergency hotkey.
        let cancel = Arc::clone(&self.cancel);
        let sender = self.report_sender.clone();
        let handle = thread::Builder::new()
            .name("oh-my-macro-runner".to_owned())
            .spawn(move || {
                let mut adapter = adapter;
                let report = run_guarded(macro_id, timing, adapter.as_mut(), &cancel);
                // The GUI may already be gone; a closed channel is not an error.
                let _ = sender.send(report);
            })
            .map_err(|error| StartError::Thread {
                detail: error.to_string(),
            })?;
        self.worker = Some(handle);
        Ok(())
    }

    /// Starts one row-build run for `target` in the configured footprint
    /// order.
    ///
    /// Uses the same single worker slot as [`Self::try_start`], so F8, disarm
    /// and window close cancel it exactly like a plan-based macro and no
    /// untracked thread is ever left behind.
    pub fn try_start_row_build(
        &mut self,
        timing: Timing,
        mode: RowMode,
        target: BuildTarget,
        force: bool,
        adapter: Box<dyn DesktopAdapter>,
    ) -> Result<(), StartError> {
        if self.worker.is_some() {
            return Err(StartError::Busy);
        }

        let cancel = Arc::clone(&self.cancel);
        let sender = self.report_sender.clone();
        let handle = thread::Builder::new()
            .name("oh-my-macro-runner".to_owned())
            .spawn(move || {
                let mut adapter = adapter;
                let report =
                    row_build_guarded(adapter.as_mut(), &cancel, timing, mode, target, force);
                let _ = sender.send(report);
            })
            .map_err(|error| StartError::Thread {
                detail: error.to_string(),
            })?;
        self.worker = Some(handle);
        Ok(())
    }

    /// Starts one Spire-action run.
    ///
    /// Shares the single worker slot with [`Self::try_start`] and
    /// [`Self::try_start_row_build`], so F8, disarm and window close cancel it
    /// exactly like the row macro and no two runs can overlap.
    pub fn try_start_spire_action(
        &mut self,
        timing: Timing,
        adapter: Box<dyn DesktopAdapter>,
    ) -> Result<(), StartError> {
        if self.worker.is_some() {
            return Err(StartError::Busy);
        }

        let cancel = Arc::clone(&self.cancel);
        let sender = self.spire_report_sender.clone();
        let handle = thread::Builder::new()
            .name("oh-my-macro-runner".to_owned())
            .spawn(move || {
                let mut adapter = adapter;
                let report = spire_action_guarded(adapter.as_mut(), &cancel, timing);
                let _ = sender.send(report);
            })
            .map_err(|error| StartError::Thread {
                detail: error.to_string(),
            })?;
        self.worker = Some(handle);
        Ok(())
    }

    /// Starts one scan-only Spire preview pass.
    ///
    /// Read-only by construction: one capture and one full-screen search on the
    /// worker thread, **zero** injected events. It shares the single worker
    /// slot with the row build and the Spire action, so F8, disarm and window
    /// close cancel it exactly like them and no two runs can overlap.
    pub fn try_start_spire_scan(
        &mut self,
        adapter: Box<dyn DesktopAdapter>,
    ) -> Result<(), StartError> {
        if self.worker.is_some() {
            return Err(StartError::Busy);
        }

        let cancel = Arc::clone(&self.cancel);
        let sender = self.spire_scan_report_sender.clone();
        let handle = thread::Builder::new()
            .name("oh-my-macro-runner".to_owned())
            .spawn(move || {
                let mut adapter = adapter;
                let report = spire_scan_guarded(adapter.as_mut(), &cancel);
                let _ = sender.send(report);
            })
            .map_err(|error| StartError::Thread {
                detail: error.to_string(),
            })?;
        self.worker = Some(handle);
        Ok(())
    }

    /// Asks the running macro to stop. Returns immediately.
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Cancels and joins the worker, discarding any report it produced.
    pub fn cancel_and_join(&mut self) {
        self.request_cancel();
        self.join_worker();
        while self.reports.try_recv().is_ok() {}
        while self.spire_reports.try_recv().is_ok() {}
        while self.spire_scan_reports.try_recv().is_ok() {}
    }

    /// Explicitly starts a new armed session after retiring the old worker.
    pub fn rearm(&mut self) {
        self.cancel_and_join();
        self.cancel.store(false, Ordering::SeqCst);
    }

    /// Takes every finished report and hands them back, newest per channel.
    ///
    /// **Order matters:** the worker slot stays occupied until its report is
    /// consumed, so the GUI must call this *before* it routes any queued
    /// hotkey. Routing first would reject a rapid repeat trigger with
    /// [`StartError::Busy`] although the previous run had already finished, and
    /// the stale report would then overwrite that message.
    ///
    /// It deliberately does not touch the cancel latch, does not revive or
    /// clear an active job, and never invents a report: a job whose report has
    /// not arrived yet keeps the slot busy.
    pub fn drain_finished(&mut self) -> FinishedReports {
        FinishedReports {
            row: self.poll_report(),
            spire_action: self.poll_spire_report(),
            spire_scan: self.poll_spire_scan_report(),
        }
    }

    /// Returns the finished run's report, if there is one.
    ///
    /// Must be called regularly (once per frame) to free the worker slot.
    pub fn poll_report(&mut self) -> Option<RunReport> {
        match self.reports.try_recv() {
            Ok(report) => {
                // The worker sends first and returns right after, so joining
                // here is bounded and cannot block on a sleeping macro.
                self.join_worker();
                Some(report)
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }

    /// Returns the finished Spire action's report, if there is one.
    ///
    /// Separate from [`Self::poll_report`] because the Spire action has its own
    /// diagnostics (capture/detection timings, per-target dispositions). Also
    /// frees the shared worker slot, so the GUI must poll this as well.
    pub fn poll_spire_report(&mut self) -> Option<SpireActionReport> {
        match self.spire_reports.try_recv() {
            Ok(report) => {
                self.join_worker();
                Some(report)
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }

    /// Returns the finished scan-only pass's result, if there is one.
    ///
    /// A scan that found no crown is `Ok` (a real measurement); a refusal or an
    /// unusable frame is `Err`, so the GUI never renders "0 spires" for a
    /// capture that could not be trusted. Also frees the shared worker slot.
    pub fn poll_spire_scan_report(&mut self) -> Option<Result<SpireScanReport, SpireScanError>> {
        match self.spire_scan_reports.try_recv() {
            Ok(report) => {
                self.join_worker();
                Some(report)
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }

    fn join_worker(&mut self) {
        if let Some(handle) = self.worker.take() {
            // A panicking worker is already reported through its RunReport; the
            // join result carries no extra information worth surfacing.
            let _ = handle.join();
        }
    }
}

impl Default for MacroRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MacroRunner {
    fn drop(&mut self) {
        // Closing the window must not leave a macro half executed or a thread
        // behind.
        self.cancel_and_join();
    }
}

/// Runs the engine, and releases injected input even if it panics.
fn run_guarded(
    macro_id: MacroId,
    timing: Timing,
    adapter: &mut dyn InputAdapter,
    cancel: &AtomicBool,
) -> RunReport {
    let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine::run(macro_id, timing, adapter, cancel)
    }));
    match guarded {
        Ok(report) => report,
        Err(_) => {
            // Last line of defence against a stuck key inside the game.
            let _ = adapter.release_all();
            RunReport::failed(macro_id, "internal panic while running the macro")
        }
    }
}

/// Runs the row build, releasing injected input even if it panics.
fn row_build_guarded(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
    mode: RowMode,
    target: BuildTarget,
    force: bool,
) -> RunReport {
    let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        colony::run_row(adapter, cancel, timing, mode, target, force)
    }));
    match guarded {
        Ok(report) => report.into_run_report(),
        Err(_) => {
            let _ = adapter.release_all();
            RunReport::failed(
                target.macro_id(),
                "internal panic while running the row build",
            )
        }
    }
}

/// Runs the Spire action, releasing injected input even if it panics.
fn spire_action_guarded(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
    timing: Timing,
) -> SpireActionReport {
    let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        spire_action::run(adapter, cancel, timing)
    }));
    match guarded {
        Ok(report) => report,
        Err(_) => {
            let _ = adapter.release_all();
            SpireActionReport::failed("internal panic while running the Spire action")
        }
    }
}

/// Runs the scan-only pass, releasing injected input even if it panics.
///
/// Scan-only injects nothing, but the guard is kept identical to the other two
/// workers: a panic inside the capture path must still not leave anything this
/// adapter pressed held in the game, and must surface as a report instead of a
/// dead worker with a permanently busy slot.
fn spire_scan_guarded(
    adapter: &mut dyn DesktopAdapter,
    cancel: &AtomicBool,
) -> Result<SpireScanReport, SpireScanError> {
    let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        spire_action::scan_once(adapter, cancel)
    }));
    match guarded {
        Ok(result) => result,
        Err(_) => {
            let _ = adapter.release_all();
            Err(SpireScanError::Internal {
                detail: "internal panic while scanning the screen".to_owned(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Outcome;
    use crate::frame::Frame;
    use crate::hotkey::{Bindings, HotkeyKey};
    use crate::input::{DesktopAdapter, InputError};
    use crate::macros::Key;
    use crate::test_support::{FakeInputAdapter, FakeState, Gate};
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    #[test]
    fn an_emergency_before_a_trigger_stays_latched_until_rearm() {
        let state = Arc::new(FakeState::default());
        let mut runner = MacroRunner::new();
        runner.request_cancel();
        runner
            .try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
            )
            .unwrap();
        assert_eq!(wait_for_report(&mut runner).outcome, Outcome::Cancelled);
        assert!(runner.cancel_flag().load(Ordering::SeqCst));
        runner.rearm();
        runner
            .try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::from_state(state)),
            )
            .unwrap();
        assert_eq!(wait_for_report(&mut runner).outcome, Outcome::Completed);
    }

    #[test]
    fn a_finished_worker_keeps_its_slot_until_its_report_is_consumed() {
        let mut runner = MacroRunner::new();
        runner
            .try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::new()),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !runner.worker.as_ref().unwrap().is_finished() {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        assert_eq!(
            runner.try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::new())
            ),
            Err(StartError::Busy)
        );
        assert_eq!(runner.poll_report().unwrap().outcome, Outcome::Completed);
        assert!(!runner.is_running());
    }

    #[test]
    fn cancel_and_join_discards_the_retired_workers_report() {
        let mut runner = MacroRunner::new();
        runner
            .try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::new()),
            )
            .unwrap();
        runner.cancel_and_join();
        assert!(runner.poll_report().is_none());
    }

    fn spyre_timing() -> Timing {
        Timing::from_millis(1, 1)
    }

    #[test]
    fn a_report_is_delivered_after_the_run() {
        let state = Arc::new(FakeState::default());
        let mut runner = MacroRunner::new();
        runner
            .try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
            )
            .expect("start");

        let report = wait_for_report(&mut runner);
        assert_eq!(report.macro_id, MacroId::Spire);
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.steps_done, 6);
        assert!(!runner.is_running(), "slot must be free again");
    }

    #[test]
    fn triggers_are_never_queued_or_run_concurrently() {
        let state = Arc::new(FakeState::default());
        let gate = Arc::new(Gate::default());
        state.set_gate(Arc::clone(&gate));

        let mut runner = MacroRunner::new();
        runner
            .try_start(
                MacroId::CreepColony,
                spyre_timing(),
                Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
            )
            .expect("start");

        // Wait until the worker is definitely inside its first event.
        assert!(gate.wait_until_entered(1), "worker never started");
        assert_eq!(
            runner.try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
            ),
            Err(StartError::Busy)
        );

        gate.open();
        let report = wait_for_report(&mut runner);
        assert_eq!(report.macro_id, MacroId::CreepColony);

        // A new run is possible once the previous one finished.
        runner
            .try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
            )
            .expect("start after completion");
        assert_eq!(wait_for_report(&mut runner).macro_id, MacroId::Spire);
    }

    #[test]
    fn cancel_stops_the_run_and_releases_held_input() {
        let state = Arc::new(FakeState::default());
        let gate = Arc::new(Gate::default());
        state.set_gate(Arc::clone(&gate));
        let mut runner = MacroRunner::new();
        runner
            .try_start(
                MacroId::CreepColony,
                // Long gaps so the macro is certainly still running.
                Timing::from_millis(1000, 1000),
                Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
            )
            .expect("start");

        // Wait until the worker is inside its first action, then cancel from the
        // outside: this is the "stop while a key is held" case.
        assert!(gate.wait_until_entered(1), "worker never started");
        let started = Instant::now();
        runner.request_cancel();
        gate.open();
        let report = wait_for_report(&mut runner);

        assert_eq!(report.outcome, Outcome::Cancelled);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "cancel took {:?}",
            started.elapsed()
        );
        assert!(state.events().contains(&Primitive::KeyUp(Key::B)));
        assert_eq!(state.release_all_calls(), 1, "release_all must run once");
    }

    #[test]
    fn dropping_the_runner_joins_the_worker_promptly() {
        let state = Arc::new(FakeState::default());
        let started = Instant::now();
        {
            let mut runner = MacroRunner::new();
            runner
                .try_start(
                    MacroId::Spire,
                    Timing::from_millis(1000, 1000),
                    Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
                )
                .expect("start");
            // Dropping must cancel and join instead of leaking the thread.
        }
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "drop took {:?}",
            started.elapsed()
        );
        assert_eq!(state.release_all_calls(), 1);
    }

    #[test]
    fn the_shared_cancel_flag_stops_a_running_macro() {
        let state = Arc::new(FakeState::default());
        let mut runner = MacroRunner::new();
        let cancel = runner.cancel_flag();
        runner
            .try_start(
                MacroId::CreepColony,
                Timing::from_millis(1000, 1000),
                Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
            )
            .expect("start");

        // This is the F8 fast path: the hotkey thread sets the flag directly.
        cancel.store(true, Ordering::SeqCst);
        let report = wait_for_report(&mut runner);
        assert_eq!(report.outcome, Outcome::Cancelled);
    }

    #[test]
    fn a_panicking_engine_still_releases_injected_input() {
        let state = Arc::new(FakeState::default());
        state.panic_at(2);
        let mut runner = MacroRunner::new();
        runner
            .try_start(
                MacroId::CreepColony,
                spyre_timing(),
                Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
            )
            .expect("start");

        let report = wait_for_report(&mut runner);
        assert!(matches!(report.outcome, Outcome::Failed { .. }));
        assert_eq!(state.release_all_calls(), 1);
        // The safety net lifted B even though the engine unwound before its own
        // release step.
        assert!(state.release_events().contains(&Primitive::KeyUp(Key::B)));
    }

    /// Waits for a report with a hard timeout so a broken runner fails the test
    /// instead of hanging it.
    fn wait_for_report(runner: &mut MacroRunner) -> RunReport {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(report) = runner.poll_report() {
                return report;
            }
            assert!(Instant::now() < deadline, "no report within 5s");
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Waits for a scan-only result with a hard timeout.
    fn wait_for_scan_result(
        runner: &mut MacroRunner,
    ) -> Result<crate::spire_action::SpireScanReport, SpireScanError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = runner.poll_spire_scan_report() {
                return result;
            }
            assert!(Instant::now() < deadline, "no scan result within 5s");
            thread::sleep(Duration::from_millis(1));
        }
    }

    use crate::macros::Primitive;

    /// Minimal [`DesktopAdapter`] for runner-level tests: it never touches the
    /// desktop and reports a configured failure.
    struct StubDesktop {
        fail: bool,
        gate: Option<Arc<Gate>>,
    }

    /// A desktop adapter that panics inside the capture, to prove the scan-only
    /// guard still releases injected input and reports instead of dying.
    #[derive(Default)]
    struct PanicDesktop {
        releases: Arc<AtomicUsize>,
    }

    impl InputAdapter for PanicDesktop {
        fn key_down(&mut self, _key: Key) -> Result<(), InputError> {
            Ok(())
        }
        fn key_up(&mut self, _key: Key) -> Result<(), InputError> {
            Ok(())
        }
        fn mouse_left_down(&mut self) -> Result<(), InputError> {
            Ok(())
        }
        fn mouse_left_up(&mut self) -> Result<(), InputError> {
            Ok(())
        }
        fn release_all(&mut self) -> Result<(), InputError> {
            self.releases.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn safety_check(&mut self) -> Result<(), InputError> {
            Ok(())
        }
    }

    impl DesktopAdapter for PanicDesktop {
        fn cursor_position(&mut self) -> Result<crate::frame::Point, InputError> {
            Ok(crate::frame::Point::new(0, 0))
        }
        fn move_cursor(&mut self, _point: crate::frame::Point) -> Result<(), InputError> {
            Ok(())
        }
        fn capture_client(&mut self) -> Result<Frame, InputError> {
            panic!("test: capture exploded");
        }
        fn capture_region(&mut self, _rect: crate::frame::Rect) -> Result<Frame, InputError> {
            panic!("test: capture exploded");
        }
    }

    impl InputAdapter for StubDesktop {
        fn key_down(&mut self, _key: Key) -> Result<(), InputError> {
            Ok(())
        }
        fn key_up(&mut self, _key: Key) -> Result<(), InputError> {
            Ok(())
        }
        fn mouse_left_down(&mut self) -> Result<(), InputError> {
            Ok(())
        }
        fn mouse_left_up(&mut self) -> Result<(), InputError> {
            Ok(())
        }
        fn release_all(&mut self) -> Result<(), InputError> {
            Ok(())
        }
        fn safety_check(&mut self) -> Result<(), InputError> {
            Ok(())
        }
    }

    impl DesktopAdapter for StubDesktop {
        fn cursor_position(&mut self) -> Result<crate::frame::Point, InputError> {
            if let Some(gate) = &self.gate {
                gate.pass();
            }
            if self.fail {
                return Err(InputError::Unsafe("stub is not the game".to_owned()));
            }
            Ok(crate::frame::Point::new(400, 400))
        }

        fn move_cursor(&mut self, _point: crate::frame::Point) -> Result<(), InputError> {
            Ok(())
        }

        fn capture_client(&mut self) -> Result<Frame, InputError> {
            Err(InputError::Injection("stub capture".to_owned()))
        }

        fn capture_region(&mut self, _rect: crate::frame::Rect) -> Result<Frame, InputError> {
            Err(InputError::Injection("stub capture".to_owned()))
        }
    }

    #[test]
    fn a_row_build_failure_is_reported_as_the_colony_macro() {
        let mut runner = MacroRunner::new();
        runner
            .try_start_row_build(
                spyre_timing(),
                RowMode::LeftToRight,
                BuildTarget::Colony,
                true,
                Box::new(StubDesktop {
                    fail: true,
                    gate: None,
                }),
            )
            .unwrap();
        let report = wait_for_report(&mut runner);
        assert_eq!(report.macro_id, MacroId::CreepColony);
        assert!(matches!(report.outcome, Outcome::Failed { .. }));
        assert!(!runner.is_running());
    }

    #[test]
    fn a_spire_row_build_failure_is_reported_as_the_spire_macro() {
        let mut runner = MacroRunner::new();
        runner
            .try_start_row_build(
                spyre_timing(),
                RowMode::LeftToRight,
                BuildTarget::Spire,
                true,
                Box::new(StubDesktop {
                    fail: true,
                    gate: None,
                }),
            )
            .unwrap();
        let report = wait_for_report(&mut runner);
        assert_eq!(report.macro_id, MacroId::Spire);
        assert!(matches!(report.outcome, Outcome::Failed { .. }));
        assert!(!runner.is_running());
    }

    #[test]
    fn the_spire_action_shares_the_single_slot_and_reports_separately() {
        let gate = Arc::new(Gate::default());
        let mut runner = MacroRunner::new();
        // A gated row build holds the slot.
        runner
            .try_start_row_build(
                spyre_timing(),
                RowMode::LeftToRight,
                BuildTarget::Colony,
                true,
                Box::new(StubDesktop {
                    fail: false,
                    gate: Some(Arc::clone(&gate)),
                }),
            )
            .unwrap();
        assert!(gate.wait_until_entered(1), "row build never started");
        assert_eq!(
            runner.try_start_spire_action(
                spyre_timing(),
                Box::new(StubDesktop {
                    fail: false,
                    gate: None,
                }),
            ),
            Err(StartError::Busy),
            "the Spire action must never overlap the row build"
        );
        gate.open();
        assert_eq!(wait_for_report(&mut runner).macro_id, MacroId::CreepColony);

        // The slot is free again; the Spire action reports through its own
        // channel, so a failure (the stub refuses to capture) is distinct from
        // a build.
        runner
            .try_start_spire_action(
                spyre_timing(),
                Box::new(StubDesktop {
                    fail: false,
                    gate: None,
                }),
            )
            .expect("start");
        let deadline = Instant::now() + Duration::from_secs(5);
        let report = loop {
            if let Some(report) = runner.poll_spire_report() {
                break report;
            }
            assert!(Instant::now() < deadline, "no Spire report within 5s");
            thread::sleep(Duration::from_millis(1));
        };
        assert!(matches!(
            report.outcome,
            crate::spire_action::SpireActionOutcome::Failed { .. }
        ));
        assert!(!runner.is_running());
        assert!(runner.poll_report().is_none(), "no row report was produced");
    }

    /// Waits for the active worker to finish while leaving its report queued.
    fn wait_until_worker_finished(runner: &MacroRunner) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !runner.worker.as_ref().expect("a worker").is_finished() {
            assert!(Instant::now() < deadline, "the worker never finished");
            thread::yield_now();
        }
    }

    #[test]
    fn a_repeat_trigger_is_accepted_once_the_finished_report_is_drained() {
        // Regression: the slot stays busy while a finished run's report is
        // still queued, so the GUI must drain reports before routing a queued
        // hotkey. Routing first rejected a rapid repeat trigger as Busy and the
        // old report then overwrote that message.
        let mut runner = MacroRunner::new();
        runner
            .try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::new()),
            )
            .expect("start");
        wait_until_worker_finished(&runner);
        assert_eq!(
            runner.try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::new())
            ),
            Err(StartError::Busy),
            "the finished report is still queued"
        );

        let finished = runner.drain_finished();
        assert_eq!(finished.row.expect("the row report").steps_done, 6);
        assert!(finished.spire_action.is_none());
        assert!(finished.spire_scan.is_none());

        // The same trigger now runs instead of being dropped.
        runner
            .try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::new()),
            )
            .expect("the repeat trigger must be accepted");
        assert_eq!(wait_for_report(&mut runner).outcome, Outcome::Completed);
    }

    #[test]
    fn every_report_channel_frees_the_slot_only_when_its_report_is_taken() {
        // The same rule holds for the two Spire channels, not just the row one.
        let mut runner = MacroRunner::new();
        runner
            .try_start_spire_action(
                spyre_timing(),
                Box::new(StubDesktop {
                    fail: false,
                    gate: None,
                }),
            )
            .expect("start");
        wait_until_worker_finished(&runner);
        assert_eq!(
            runner.try_start_spire_scan(Box::new(StubDesktop {
                fail: false,
                gate: None,
            })),
            Err(StartError::Busy)
        );
        let finished = runner.drain_finished();
        assert!(finished.spire_action.is_some());
        assert!(finished.row.is_none() && finished.spire_scan.is_none());

        runner
            .try_start_spire_scan(Box::new(StubDesktop {
                fail: false,
                gate: None,
            }))
            .expect("accepted after draining");
        wait_until_worker_finished(&runner);
        assert_eq!(
            runner.try_start_spire_action(
                spyre_timing(),
                Box::new(StubDesktop {
                    fail: false,
                    gate: None,
                })
            ),
            Err(StartError::Busy)
        );
        let finished = runner.drain_finished();
        assert!(matches!(
            finished.spire_scan,
            Some(Err(SpireScanError::Adapter(_)))
        ));
        assert!(runner.drain_finished().is_empty(), "only the newest result");
    }

    #[test]
    fn draining_reports_leaves_an_active_job_and_the_cancel_latch_alone() {
        let gate = Arc::new(Gate::default());
        let state = Arc::new(FakeState::default());
        state.set_gate(Arc::clone(&gate));
        let mut runner = MacroRunner::new();
        runner
            .try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::from_state(Arc::clone(&state))),
            )
            .expect("start");
        assert!(gate.wait_until_entered(1), "the job never started");

        // A job whose report has not arrived keeps its slot and is never
        // revived or cleared by draining.
        assert!(runner.drain_finished().is_empty());
        assert_eq!(
            runner.try_start(
                MacroId::Spire,
                spyre_timing(),
                Box::new(FakeInputAdapter::new())
            ),
            Err(StartError::Busy)
        );

        // Draining neither sets nor clears the latch: an emergency that arrived
        // after the job started is still honoured, and stays latched after the
        // cancelled report is consumed.
        runner.request_cancel();
        assert!(runner.drain_finished().is_empty());
        assert!(runner.cancel_flag().load(Ordering::SeqCst));
        gate.open();
        assert_eq!(wait_for_report(&mut runner).outcome, Outcome::Cancelled);
        assert!(runner.cancel_flag().load(Ordering::SeqCst));
    }

    #[test]
    fn the_scan_only_pass_shares_the_single_slot_and_reports_separately() {
        let gate = Arc::new(Gate::default());
        let mut runner = MacroRunner::new();
        // A gated row build holds the slot.
        runner
            .try_start_row_build(
                spyre_timing(),
                RowMode::LeftToRight,
                BuildTarget::Colony,
                true,
                Box::new(StubDesktop {
                    fail: false,
                    gate: Some(Arc::clone(&gate)),
                }),
            )
            .unwrap();
        assert!(gate.wait_until_entered(1), "row build never started");
        assert_eq!(
            runner.try_start_spire_scan(Box::new(StubDesktop {
                fail: false,
                gate: None,
            })),
            Err(StartError::Busy),
            "the scan-only pass must never overlap the row build"
        );
        gate.open();
        assert_eq!(wait_for_report(&mut runner).macro_id, MacroId::CreepColony);

        // The slot is free again; the scan-only result arrives on its own
        // channel (the stub refuses to capture) and no build report appears.
        runner
            .try_start_spire_scan(Box::new(StubDesktop {
                fail: false,
                gate: None,
            }))
            .expect("start");
        let result = wait_for_scan_result(&mut runner);
        assert!(
            matches!(
                result,
                Err(SpireScanError::Adapter(InputError::Injection(_)))
            ),
            "{result:?}"
        );
        assert!(!runner.is_running(), "the slot must be free again");
        assert!(runner.poll_report().is_none(), "no row report was produced");
        assert!(
            runner.poll_spire_report().is_none(),
            "no action report was produced"
        );
    }

    #[test]
    fn a_latched_cancel_is_honoured_and_rearming_clears_it() {
        let mut runner = MacroRunner::new();
        runner.request_cancel();
        runner
            .try_start_spire_scan(Box::new(StubDesktop {
                fail: false,
                gate: None,
            }))
            .expect("start");
        assert_eq!(
            wait_for_scan_result(&mut runner),
            Err(SpireScanError::Cancelled),
            "an emergency before the scan must stop it before the capture"
        );
        assert!(!runner.is_running());

        // Explicit rearming clears the latch, exactly like the row macro.
        runner.rearm();
        runner
            .try_start_spire_scan(Box::new(StubDesktop {
                fail: false,
                gate: None,
            }))
            .expect("start");
        assert!(matches!(
            wait_for_scan_result(&mut runner),
            Err(SpireScanError::Adapter(_))
        ));
    }

    #[test]
    fn cancel_and_join_discards_a_finished_scan_report() {
        let mut runner = MacroRunner::new();
        runner
            .try_start_spire_scan(Box::new(StubDesktop {
                fail: false,
                gate: None,
            }))
            .expect("start");
        runner.cancel_and_join();
        assert!(runner.poll_spire_scan_report().is_none());
    }

    #[test]
    fn a_panicking_scan_worker_releases_input_and_reports_instead_of_dying() {
        let releases = Arc::new(AtomicUsize::new(0));
        let mut runner = MacroRunner::new();
        runner
            .try_start_spire_scan(Box::new(PanicDesktop {
                releases: Arc::clone(&releases),
            }))
            .expect("start");
        match wait_for_scan_result(&mut runner) {
            Err(SpireScanError::Internal { detail }) => {
                assert!(detail.contains("panic"), "{detail}");
            }
            other => panic!("expected an internal failure, got {other:?}"),
        }
        assert_eq!(
            releases.load(Ordering::SeqCst),
            1,
            "release must be retried"
        );
        assert!(
            !runner.is_running(),
            "a panic must not wedge the worker slot"
        );
    }

    #[test]
    fn a_running_row_build_keeps_the_single_slot_and_joins_on_cancel() {
        let gate = Arc::new(Gate::default());
        let mut runner = MacroRunner::new();
        runner
            .try_start_row_build(
                spyre_timing(),
                RowMode::LeftToRight,
                BuildTarget::Colony,
                true,
                Box::new(StubDesktop {
                    fail: false,
                    gate: Some(Arc::clone(&gate)),
                }),
            )
            .unwrap();
        assert!(gate.wait_until_entered(1), "row worker never started");
        assert_eq!(
            runner.try_start_row_build(
                spyre_timing(),
                RowMode::LeftToRight,
                BuildTarget::Spire,
                true,
                Box::new(StubDesktop {
                    fail: false,
                    gate: None
                })
            ),
            Err(StartError::Busy)
        );
        runner.request_cancel();
        gate.open();
        let deadline = Instant::now() + Duration::from_secs(5);
        while runner.is_running() {
            assert!(Instant::now() < deadline, "row worker did not stop");
            let _ = runner.poll_report();
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_trigger_starts_the_row_build_and_f8_stops_it() {
        // The GUI routes both through this pure rule, so the macro can never
        // be split across bindings again.
        assert_eq!(action_for(HotkeySlot::Trigger), HotkeyAction::StartRowBuild);
        assert_eq!(
            action_for(HotkeySlot::SpireAction),
            HotkeyAction::StartSpireAction
        );
        assert_eq!(
            action_for(HotkeySlot::Emergency),
            HotkeyAction::EmergencyStop
        );

        let bindings = Bindings::new(HotkeyKey::F6, HotkeyKey::F7);
        assert_eq!(bindings.get(HotkeySlot::Trigger), HotkeyKey::F6);
        assert_eq!(bindings.get(HotkeySlot::SpireAction), HotkeyKey::F7);
        assert_ne!(bindings.get(HotkeySlot::Trigger), HotkeyKey::EMERGENCY);
        for slot in HotkeySlot::ALL {
            let expected = match slot {
                HotkeySlot::Trigger => HotkeyAction::StartRowBuild,
                HotkeySlot::SpireAction => HotkeyAction::StartSpireAction,
                HotkeySlot::Emergency => HotkeyAction::EmergencyStop,
            };
            assert_eq!(action_for(slot), expected, "{slot:?}");
        }
    }
}
