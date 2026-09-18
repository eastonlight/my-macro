//! Pure macro executor.
//!
//! The executor walks a [`crate::macros::plan`], asks the adapter's safety gate
//! before every single event, and guarantees that any key or button it pressed
//! is released again — including when the run is cancelled, aborted or panics.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::input::{InputAdapter, InputError};
use crate::macros::{self, Key, MacroId, PlannedStep, Primitive, Timing};

/// How often a running macro checks for cancellation while waiting.
///
/// Keeping this small is what makes disarm/F8 cancel *promptly* instead of
/// waiting out a full gap.
pub const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// How a run ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// All events were delivered and the configured delays elapsed.
    Completed,
    /// Cancelled before the next event (disarm, F8, shutdown).
    Cancelled,
    /// The safety gate refused to inject; nothing further was sent.
    Aborted { detail: String },
    /// The OS rejected an event, the run stopped there.
    Failed { detail: String },
}

/// What happened during one run, ready to be logged or displayed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunReport {
    pub macro_id: MacroId,
    pub outcome: Outcome,
    /// Number of primitives the OS accepted.
    pub steps_done: usize,
    /// Number of primitives in the plan.
    pub steps_total: usize,
    /// Orders that were issued with the forced mode although the placement
    /// preview could not be confirmed. The plan-based engine always leaves this
    /// at zero.
    pub unconfirmed: usize,
}

impl RunReport {
    pub fn failed(macro_id: MacroId, detail: impl Into<String>) -> Self {
        Self {
            macro_id,
            outcome: Outcome::Failed {
                detail: detail.into(),
            },
            steps_done: 0,
            steps_total: 0,
            unconfirmed: 0,
        }
    }
}

/// Keys/buttons this run has pressed and not yet released.
#[derive(Debug, Default)]
struct Held {
    keys: Vec<Key>,
    mouse_left: bool,
}

impl Held {
    fn note_pressed(&mut self, primitive: Primitive) {
        match primitive {
            Primitive::KeyDown(key) => {
                if !self.keys.contains(&key) {
                    self.keys.push(key);
                }
            }
            Primitive::MouseLeftDown => self.mouse_left = true,
            Primitive::KeyUp(_) | Primitive::MouseLeftUp => {}
        }
    }

    fn note_released(&mut self, primitive: Primitive) {
        match primitive {
            Primitive::KeyUp(key) => self.keys.retain(|held| *held != key),
            Primitive::MouseLeftUp => self.mouse_left = false,
            Primitive::KeyDown(_) | Primitive::MouseLeftDown => {}
        }
    }

    /// Releases everything still held. Every release is attempted even if an
    /// earlier one failed, so one bad call cannot leave a key stuck.
    fn release(&mut self, adapter: &mut dyn InputAdapter) -> Option<String> {
        let mut first_error = None;
        for key in std::mem::take(&mut self.keys) {
            if let Err(error) = adapter.key_up(key) {
                first_error.get_or_insert_with(|| error.to_string());
            }
        }
        if self.mouse_left {
            self.mouse_left = false;
            if let Err(error) = adapter.mouse_left_up() {
                first_error.get_or_insert_with(|| error.to_string());
            }
        }
        first_error
    }
}

/// Runs `macro_id` once, using `adapter`, until done or cancelled.
///
/// The function always returns a report; it never panics on user input and it
/// never leaves injected input held.
pub fn run(
    macro_id: MacroId,
    timing: Timing,
    adapter: &mut dyn InputAdapter,
    cancel: &AtomicBool,
) -> RunReport {
    let steps = macros::plan(macro_id, timing);
    let mut held = Held::default();
    let (outcome, steps_done) = execute(&steps, adapter, cancel, &mut held);

    // Whatever happened above, nothing may stay pressed. `release_all` runs last
    // as a belt-and-braces net and must not be blocked by the safety gate.
    let mut release_detail = held.release(adapter);
    if let Err(error) = adapter.release_all()
        && release_detail.is_none()
    {
        release_detail = Some(error.to_string());
    }

    let outcome = match (outcome, release_detail) {
        (outcome, None) => outcome,
        (Outcome::Completed | Outcome::Cancelled, Some(detail)) => Outcome::Failed {
            detail: format!("could not release injected input: {detail}"),
        },
        (outcome, Some(detail)) => append_release_detail(outcome, &detail),
    };

    RunReport {
        macro_id,
        outcome,
        steps_done,
        steps_total: steps.len(),
        unconfirmed: 0,
    }
}

fn append_release_detail(outcome: Outcome, detail: &str) -> Outcome {
    match outcome {
        Outcome::Aborted { detail: abort } => Outcome::Aborted {
            detail: format!("{abort}; release also failed: {detail}"),
        },
        Outcome::Failed { detail: failure } => Outcome::Failed {
            detail: format!("{failure}; release also failed: {detail}"),
        },
        other => other,
    }
}

fn execute(
    steps: &[PlannedStep],
    adapter: &mut dyn InputAdapter,
    cancel: &AtomicBool,
    held: &mut Held,
) -> (Outcome, usize) {
    let total = steps.len();
    let mut steps_done = 0;

    for (index, step) in steps.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return (Outcome::Cancelled, steps_done);
        }
        // Foreground/modifier safety is re-checked before *every* event: the
        // player may alt-tab or hold Shift in the millisecond between two steps.
        if let Err(error) = adapter.safety_check() {
            return (outcome_from_error(error), steps_done);
        }
        // The OS gate can take time; an emergency arriving inside it must
        // still prevent this event, not wait until after the next key-down.
        if cancel.load(Ordering::SeqCst) {
            return (Outcome::Cancelled, steps_done);
        }

        let primitive = step.primitive;
        let result = match primitive {
            Primitive::KeyDown(key) => adapter.key_down(key),
            Primitive::KeyUp(key) => adapter.key_up(key),
            Primitive::MouseLeftDown => adapter.mouse_left_down(),
            Primitive::MouseLeftUp => adapter.mouse_left_up(),
        };
        match result {
            Ok(()) => {
                held.note_released(primitive);
                held.note_pressed(primitive);
                steps_done += 1;
            }
            Err(error) => {
                let detail = format!(
                    "{} while sending step {}/{} ({})",
                    error,
                    index + 1,
                    total,
                    primitive.describe()
                );
                return (outcome_from_error_detail(error, detail), steps_done);
            }
        }

        if !sleep_unless_cancelled(step.delay_after, cancel) {
            return (Outcome::Cancelled, steps_done);
        }
    }

    (Outcome::Completed, steps_done)
}

fn outcome_from_error(error: InputError) -> Outcome {
    let detail = error.to_string();
    outcome_from_error_detail(error, detail)
}

fn outcome_from_error_detail(error: InputError, detail: String) -> Outcome {
    match error {
        // A refused safety gate is an expected, handled stop.
        InputError::Unsafe(_) => Outcome::Aborted { detail },
        InputError::Injection(_) => Outcome::Failed { detail },
    }
}

/// Sleeps for `duration`, waking up to poll `cancel` every
/// [`CANCEL_POLL_INTERVAL`]. Returns `false` as soon as cancellation is seen.
pub(crate) fn sleep_unless_cancelled(duration: Duration, cancel: &AtomicBool) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        let remaining = deadline - now;
        std::thread::sleep(remaining.min(CANCEL_POLL_INTERVAL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FakeInputAdapter, FakeState, Gate};
    use std::sync::Arc;

    fn timing() -> Timing {
        Timing::from_millis(1, 1)
    }

    fn run_with(state: Arc<FakeState>) -> RunReport {
        let mut adapter = FakeInputAdapter::from_state(Arc::clone(&state));
        let cancel = AtomicBool::new(false);
        run(MacroId::CreepColony, timing(), &mut adapter, &cancel)
    }

    #[test]
    fn zero_duration_wait_still_observes_cancellation() {
        assert!(!sleep_unless_cancelled(
            Duration::ZERO,
            &AtomicBool::new(true)
        ));
        assert!(sleep_unless_cancelled(
            Duration::ZERO,
            &AtomicBool::new(false)
        ));
    }

    #[test]
    fn events_reach_the_adapter_in_exact_order() {
        let state = Arc::new(FakeState::default());
        let report = run_with(Arc::clone(&state));

        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.steps_done, 6);
        assert_eq!(report.steps_total, 6);
        assert_eq!(
            state.events(),
            vec![
                Primitive::KeyDown(Key::B),
                Primitive::KeyUp(Key::B),
                Primitive::KeyDown(Key::C),
                Primitive::KeyUp(Key::C),
                Primitive::MouseLeftDown,
                Primitive::MouseLeftUp,
            ]
        );
        // Exactly the planned events, plus the closing safety net.
        assert_eq!(state.release_all_calls(), 1);
    }

    #[test]
    fn a_safety_stop_happens_before_the_first_event_and_releases_nothing() {
        let state = Arc::new(FakeState::default());
        state.refuse_safety("foreground window belongs to 'notepad.exe'");
        let report = run_with(Arc::clone(&state));

        match report.outcome {
            Outcome::Aborted { detail } => assert!(detail.contains("notepad.exe"), "{detail}"),
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert_eq!(report.steps_done, 0);
        assert_eq!(state.events(), Vec::new());
    }

    #[test]
    fn a_safety_stop_mid_run_does_not_send_more_events() {
        let state = Arc::new(FakeState::default());
        state.refuse_safety_after(3, "modifier keys are held: Ctrl");
        let report = run_with(Arc::clone(&state));

        assert!(matches!(report.outcome, Outcome::Aborted { .. }));
        assert_eq!(report.steps_done, 3);
        // Key C was already pressed when the gate refused, so it is released
        // before the run stops.
        assert_eq!(
            state.events(),
            vec![
                Primitive::KeyDown(Key::B),
                Primitive::KeyUp(Key::B),
                Primitive::KeyDown(Key::C),
                Primitive::KeyUp(Key::C),
            ]
        );
        // The plan stopped after event 3, so C's own key up was the safety release.
        assert_eq!(state.release_all_calls(), 1);
    }

    #[test]
    fn cancellation_before_start_sends_nothing() {
        let state = Arc::new(FakeState::default());
        let mut adapter = FakeInputAdapter::from_state(Arc::clone(&state));
        let cancel = AtomicBool::new(true);
        let report = run(MacroId::Spire, timing(), &mut adapter, &cancel);

        assert_eq!(report.outcome, Outcome::Cancelled);
        assert_eq!(report.steps_done, 0);
        assert_eq!(state.events(), Vec::new());
    }

    #[test]
    fn cancellation_after_a_key_down_still_releases_that_key() {
        let state = Arc::new(FakeState::default());
        let cancel = Arc::new(AtomicBool::new(false));
        // Cancel from inside the adapter, right after the first key down: this
        // reproduces the "F8 pressed while B is held" race deterministically.
        state.arm_cancel_after(1, Arc::clone(&cancel));
        let mut adapter = FakeInputAdapter::from_state(Arc::clone(&state));
        let report = run(MacroId::CreepColony, timing(), &mut adapter, &cancel);

        assert_eq!(report.outcome, Outcome::Cancelled);
        assert_eq!(report.steps_done, 1);
        // The very first event was the key down ...
        assert_eq!(state.events().first(), Some(&Primitive::KeyDown(Key::B)));
        // ... and it was released again even though the run was cancelled.
        assert!(state.events().contains(&Primitive::KeyUp(Key::B)));
        assert_eq!(state.release_all_calls(), 1);
    }

    #[test]
    fn cancellation_between_two_keys_releases_the_first_one() {
        let state = Arc::new(FakeState::default());
        let cancel = Arc::new(AtomicBool::new(false));
        state.arm_cancel_after(3, Arc::clone(&cancel));
        let mut adapter = FakeInputAdapter::from_state(Arc::clone(&state));
        let report = run(MacroId::CreepColony, timing(), &mut adapter, &cancel);

        assert_eq!(report.outcome, Outcome::Cancelled);
        assert_eq!(
            state.events(),
            vec![
                Primitive::KeyDown(Key::B),
                Primitive::KeyUp(Key::B),
                Primitive::KeyDown(Key::C),
                // release of the still held C, issued after cancellation
                Primitive::KeyUp(Key::C),
            ]
        );
    }

    #[test]
    fn cancellation_during_an_extra_long_gap_is_prompt() {
        let state = Arc::new(FakeState::default());
        let gate = Arc::new(Gate::default());
        state.set_gate(Arc::clone(&gate));
        let cancel = Arc::new(AtomicBool::new(false));

        let adapter = FakeInputAdapter::from_state(Arc::clone(&state));
        let worker_cancel = Arc::clone(&cancel);
        let worker = std::thread::spawn(move || {
            let mut adapter = adapter;
            let started = Instant::now();
            // 1000 ms hold time: the worker is inside the delay after the key down,
            // so without the 5 ms cancel poll this run would last a second.
            let report = run(
                MacroId::CreepColony,
                Timing::from_millis(1000, 1000),
                &mut adapter,
                &worker_cancel,
            );
            (report, started.elapsed())
        });

        assert!(
            gate.wait_until_entered(1),
            "worker never reached the action"
        );
        gate.open();
        // The worker has now injected the key down and is inside the hold delay.
        std::thread::sleep(Duration::from_millis(30));
        cancel.store(true, Ordering::SeqCst);

        let (report, elapsed) = worker.join().expect("worker join");
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert_eq!(report.steps_done, 1);
        assert!(
            elapsed < Duration::from_millis(500),
            "cancel during a gap took {elapsed:?}"
        );
    }

    #[test]
    fn an_injection_failure_stops_the_run_and_is_reported() {
        let state = Arc::new(FakeState::default());
        state.fail_at(4, "SendInput returned 0 (GetLastError=5)");
        let report = run_with(Arc::clone(&state));

        match report.outcome {
            Outcome::Failed { detail } => {
                assert!(detail.contains("GetLastError=5"), "{detail}");
                assert!(detail.contains("step 4/6"), "{detail}");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert_eq!(report.steps_done, 3);
        // Key C was pressed but not released by the plan, so the executor freed it.
        assert!(state.events().contains(&Primitive::KeyUp(Key::C)));
    }

    #[test]
    fn a_failing_release_is_reported_and_does_not_stop_the_other_releases() {
        let state = Arc::new(FakeState::default());
        state.fail_at(2, "blocked"); // the key-up of B fails
        let report = run_with(Arc::clone(&state));

        match report.outcome {
            Outcome::Failed { detail } => assert!(detail.contains("blocked"), "{detail}"),
            other => panic!("unexpected outcome: {other:?}"),
        }
        // The failed up was retried during the final release, together with
        // the `release_all` net.
        assert_eq!(state.release_all_calls(), 1);
        assert!(state.events().contains(&Primitive::KeyUp(Key::B)));
    }

    #[test]
    fn the_plan_is_executed_with_the_configured_delays() {
        let state = Arc::new(FakeState::default());
        let mut adapter = FakeInputAdapter::from_state(Arc::clone(&state));
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let report = run(
            MacroId::Spire,
            Timing::from_millis(20, 20),
            &mut adapter,
            &cancel,
        );
        assert_eq!(report.outcome, Outcome::Completed);
        // 6 events, 5 gaps of 20 ms plus the final 20 ms pause.
        assert!(
            started.elapsed() >= Duration::from_millis(120),
            "run finished too early: {:?}",
            started.elapsed()
        );
    }
}
