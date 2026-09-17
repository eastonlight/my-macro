//! Test doubles shared by the unit tests in this crate.
//!
//! Not part of the public API: it only exists so that timing, cancellation,
//! busy and registration behaviour can be tested without a Windows host.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::hotkey::{HotkeyKey, HotkeyRegistrar, HotkeySlot};
use crate::input::{InputAdapter, InputError};
use crate::macros::{Key, Primitive};

/// Blocking rendezvous used to hold a worker inside a specific action.
///
/// The worker calls [`Gate::pass`]; the test calls
/// [`Gate::wait_until_entered`] to wait for that moment and [`Gate::open`] to
/// let the worker continue. No sleeps, no flakiness.
#[derive(Default)]
pub struct Gate {
    entered: Mutex<usize>,
    cv: Condvar,
    opened: AtomicBool,
}

impl Gate {
    fn lock(&self) -> MutexGuard<'_, usize> {
        self.entered.lock().expect("gate lock")
    }

    /// Worker side: announce arrival, then wait until the test opens the gate.
    pub fn pass(&self) {
        let mut entered = self.lock();
        *entered += 1;
        self.cv.notify_all();
        while !self.opened.load(Ordering::SeqCst) {
            entered = self.cv.wait(entered).expect("gate wait");
        }
    }

    /// Test side: waits until `count` actions have entered the gate.
    pub fn wait_until_entered(&self, count: usize) -> bool {
        let mut entered = self.lock();
        let deadline = Instant::now() + Duration::from_secs(5);
        while *entered < count {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            let (guard, timeout) = self.cv.wait_timeout(entered, left).expect("gate timeout");
            entered = guard;
            if timeout.timed_out() && *entered < count {
                return false;
            }
        }
        true
    }

    /// Test side: releases every waiting worker.
    pub fn open(&self) {
        self.opened.store(true, Ordering::SeqCst);
        self.cv.notify_all();
    }
}

/// State shared between a test and its [`FakeInputAdapter`].
#[derive(Default)]
pub struct FakeState {
    events: Mutex<Vec<Primitive>>,
    safety_error: Mutex<Option<String>>,
    refuse_safety_after: Mutex<Option<(usize, String)>>,
    fail_at: Mutex<Option<(usize, String)>>,
    panic_at: Mutex<Option<usize>>,
    cancel_after: Mutex<Option<(usize, Arc<AtomicBool>)>>,
    gate: Mutex<Option<Arc<Gate>>>,
    sent: AtomicUsize,
    release_all_calls: AtomicUsize,
    release_events: Mutex<Vec<Primitive>>,
}

impl FakeState {
    /// Every primitive the adapter accepted, in order.
    pub fn events(&self) -> Vec<Primitive> {
        self.events.lock().expect("events lock").clone()
    }

    /// Number of times the engine asked for the final safety release.
    pub fn release_all_calls(&self) -> usize {
        self.release_all_calls.load(Ordering::SeqCst)
    }

    /// What the safety release actually sent, mirroring the real adapter, which
    /// lifts every injectable key plus the left button.
    pub fn release_events(&self) -> Vec<Primitive> {
        self.release_events.lock().expect("release lock").clone()
    }

    /// Makes every safety check fail.
    pub fn refuse_safety(&self, detail: &str) {
        *self.safety_error.lock().expect("safety lock") = Some(detail.to_owned());
    }

    /// Makes the safety check fail only after `after` accepted events.
    pub fn refuse_safety_after(&self, after: usize, detail: &str) {
        *self.refuse_safety_after.lock().expect("safety lock") = Some((after, detail.to_owned()));
    }

    /// Makes the `at`-th event (1 based) fail with `detail`.
    pub fn fail_at(&self, at: usize, detail: &str) {
        *self.fail_at.lock().expect("fail lock") = Some((at, detail.to_owned()));
    }

    /// Makes the `at`-th event (1 based) panic.
    pub fn panic_at(&self, at: usize) {
        *self.panic_at.lock().expect("panic lock") = Some(at);
    }

    /// Requests cancellation right after the `after`-th event was handled.
    pub fn arm_cancel_after(&self, after: usize, flag: Arc<AtomicBool>) {
        *self.cancel_after.lock().expect("cancel lock") = Some((after, flag));
    }

    /// Installs a gate that blocks every action until the test opens it.
    pub fn set_gate(&self, gate: Arc<Gate>) {
        *self.gate.lock().expect("gate slot") = Some(gate);
    }

    /// Test-side escape hatch for hanging tests.
    pub fn gate(&self) -> Option<Arc<Gate>> {
        self.gate.lock().expect("gate slot").clone()
    }

    fn on_event(&self, index: usize) {
        if let Some(gate) = self.gate() {
            gate.pass();
        }
        // Copy out of the lock before panicking so a test-induced panic cannot
        // poison the mutex for the assertions that follow.
        let panic_at = *self.panic_at.lock().expect("panic lock");
        if panic_at == Some(index) {
            panic!("fake adapter panics at event {index}");
        }
        let cancel_after = self.cancel_after.lock().expect("cancel lock").clone();
        if let Some((after, flag)) = cancel_after
            && after <= index
        {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn record(&self, primitive: Primitive) -> Result<(), InputError> {
        let index = self.sent.fetch_add(1, Ordering::SeqCst) + 1;
        self.on_event(index);
        if let Some((at, detail)) = self.fail_at.lock().expect("fail lock").as_ref()
            && *at == index
        {
            return Err(InputError::Injection(detail.clone()));
        }
        self.events.lock().expect("events lock").push(primitive);
        Ok(())
    }

    fn check_safety(&self) -> Result<(), InputError> {
        let accepted = self.events.lock().expect("events lock").len();
        if let Some((after, detail)) = self
            .refuse_safety_after
            .lock()
            .expect("safety lock")
            .as_ref()
            && accepted >= *after
        {
            return Err(InputError::Unsafe(detail.clone()));
        }
        if let Some(detail) = self.safety_error.lock().expect("safety lock").as_ref() {
            return Err(InputError::Unsafe(detail.clone()));
        }
        Ok(())
    }
}

/// Records instead of injecting.
pub struct FakeInputAdapter {
    state: Arc<FakeState>,
}

impl FakeInputAdapter {
    pub fn new() -> Self {
        Self::from_state(Arc::new(FakeState::default()))
    }

    pub fn from_state(state: Arc<FakeState>) -> Self {
        Self { state }
    }
}

impl Default for FakeInputAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl InputAdapter for FakeInputAdapter {
    fn key_down(&mut self, key: Key) -> Result<(), InputError> {
        self.state.record(Primitive::KeyDown(key))
    }

    fn key_up(&mut self, key: Key) -> Result<(), InputError> {
        self.state.record(Primitive::KeyUp(key))
    }

    fn mouse_left_down(&mut self) -> Result<(), InputError> {
        self.state.record(Primitive::MouseLeftDown)
    }

    fn mouse_left_up(&mut self) -> Result<(), InputError> {
        self.state.record(Primitive::MouseLeftUp)
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        self.state.release_all_calls.fetch_add(1, Ordering::SeqCst);
        let mut released = self.state.release_events.lock().expect("release lock");
        released.extend(Key::ALL.into_iter().map(Primitive::KeyUp));
        released.push(Primitive::MouseLeftUp);
        Ok(())
    }

    fn safety_check(&mut self) -> Result<(), InputError> {
        self.state.check_safety()
    }
}

/// In-memory `RegisterHotKey` replacement.
#[derive(Default)]
pub struct FakeRegistrar {
    registered: Vec<(HotkeySlot, HotkeyKey)>,
    unregistered: Vec<(HotkeySlot, HotkeyKey)>,
    live: Vec<(HotkeySlot, HotkeyKey)>,
    fail_register: Vec<(HotkeySlot, String)>,
    fail_unregister: Vec<(HotkeySlot, String)>,
}

impl FakeRegistrar {
    pub fn fail_on(&mut self, slot: HotkeySlot, detail: &str) {
        self.fail_register.push((slot, detail.to_owned()));
    }

    pub fn fail_unregister(&mut self, slot: HotkeySlot, detail: &str) {
        self.fail_unregister.push((slot, detail.to_owned()));
    }

    pub fn registered(&self) -> Vec<(HotkeySlot, HotkeyKey)> {
        self.registered.clone()
    }

    pub fn unregistered(&self) -> Vec<(HotkeySlot, HotkeyKey)> {
        self.unregistered.clone()
    }

    /// Bindings that are still registered, i.e. that would really fire.
    pub fn live(&self) -> Vec<(HotkeySlot, HotkeyKey)> {
        self.live.clone()
    }
}

impl HotkeyRegistrar for FakeRegistrar {
    fn register(&mut self, slot: HotkeySlot, key: HotkeyKey) -> Result<(), String> {
        if let Some((_, detail)) = self.fail_register.iter().find(|(s, _)| *s == slot) {
            return Err(detail.clone());
        }
        self.registered.push((slot, key));
        self.live.push((slot, key));
        Ok(())
    }

    fn unregister(&mut self, slot: HotkeySlot, key: HotkeyKey) -> Result<(), String> {
        if let Some((_, detail)) = self.fail_unregister.iter().find(|(s, _)| *s == slot) {
            return Err(detail.clone());
        }
        self.unregistered.push((slot, key));
        self.live.retain(|(s, _)| *s != slot);
        Ok(())
    }
}

/// Unique path inside the system temp directory.
pub fn temp_path(name: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "oh-my-macro-test-{}-{unique}-{name}",
        std::process::id()
    ))
}
