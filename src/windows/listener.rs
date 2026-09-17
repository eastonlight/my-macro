//! Global hotkey listener.
//!
//! `RegisterHotKey` delivers `WM_HOTKEY` to the message queue of the thread that
//! registered it, so this module owns a small message-pump thread. The thread:
//!
//! * creates its queue *before* the handshake, so the owner can always stop it;
//! * registers every binding transactionally (`register_all`) and reports the
//!   result back, including the failure detail, without leaving partial bindings;
//! * unregisters everything when it exits;
//! * uses `MOD_NOREPEAT`, so holding a key does not produce a stream of events.
//!
//! The emergency slot additionally sets the runner's cancel flag directly. That
//! is what makes F8 stop a running macro immediately, without waiting for the
//! GUI thread to pick up the event.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_HOTKEY, WM_QUIT,
};

use crate::hotkey::{self, Bindings, HotkeyError, HotkeyKey, HotkeyRegistrar, HotkeySlot};

/// How long the GUI waits for the listener to confirm its registrations.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Wakes the GUI thread so it can process a queued hotkey promptly.
pub type Wake = Arc<dyn Fn() + Send + Sync + 'static>;

/// Owns the message-pump thread and the registered bindings.
pub struct HotkeyListener {
    thread: Option<JoinHandle<()>>,
    thread_id: u32,
    stop: Arc<AtomicBool>,
    events: Receiver<HotkeySlot>,
}

impl HotkeyListener {
    /// Registers every binding, or fails cleanly.
    ///
    /// `cancel` belongs to the macro runner; the emergency key sets it directly
    /// so that F8 does not depend on the GUI repaint loop.
    pub fn start(
        bindings: Bindings,
        wake: Wake,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, HotkeyError> {
        let (report_tx, report_rx) = mpsc::channel::<Result<u32, HotkeyError>>();
        let (event_tx, events) = mpsc::channel::<HotkeySlot>();
        let stop = Arc::new(AtomicBool::new(false));

        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("oh-my-macro-hotkeys".to_owned())
            .spawn(move || pump(bindings, report_tx, event_tx, wake, cancel, thread_stop))
            .map_err(|error| HotkeyError::Listener {
                detail: error.to_string(),
            })?;

        match report_rx.recv_timeout(HANDSHAKE_TIMEOUT) {
            Ok(Ok(thread_id)) => Ok(Self {
                thread: Some(thread),
                thread_id,
                stop,
                events,
            }),
            Ok(Err(error)) => {
                // Registering failed and the thread already rolled back and
                // exited; joining here keeps no thread behind.
                let _ = thread.join();
                Err(error)
            }
            Err(RecvTimeoutError::Timeout) => {
                // `RegisterHotKey` does not block, so this is a safety valve. The
                // thread checks `stop` right after registering and releases its
                // bindings, so joining cannot leave hotkeys armed.
                stop.store(true, Ordering::SeqCst);
                let _ = thread.join();
                Err(HotkeyError::Listener {
                    detail: format!(
                        "no answer within {}s while registering hotkeys",
                        HANDSHAKE_TIMEOUT.as_secs()
                    ),
                })
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Err(HotkeyError::Listener {
                    detail: "hotkey thread stopped before confirming registration".to_owned(),
                })
            }
        }
    }

    /// Non-blocking: one queued hotkey, if any.
    pub fn try_recv(&self) -> Option<HotkeySlot> {
        self.events.try_recv().ok()
    }

    /// Unregisters every binding and joins the thread.
    pub fn stop(&mut self) -> Result<(), HotkeyError> {
        self.stop.store(true, Ordering::SeqCst);
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        // The queue exists (the handshake only succeeds after queue creation),
        // so this always reaches the pump.
        // SAFETY: posting a message to a live thread's queue.
        unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
        match thread.join() {
            Ok(()) => Ok(()),
            Err(_) => Err(HotkeyError::Listener {
                detail: "hotkey thread panicked; bindings were released by the OS".to_owned(),
            }),
        }
    }
}

impl Drop for HotkeyListener {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Message pump of the listener thread.
fn pump(
    bindings: Bindings,
    report: Sender<Result<u32, HotkeyError>>,
    events: Sender<HotkeySlot>,
    wake: Wake,
    cancel: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    // SAFETY: message queue and hotkey calls for this thread only.
    unsafe {
        // Create this thread's message queue before the handshake: until the
        // queue exists, `PostThreadMessageW` would fail and the owner could
        // never stop this thread.
        let mut message = MSG::default();
        PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
        let thread_id = GetCurrentThreadId();

        let mut registrar = WinHotkeyRegistrar;
        if let Err(error) = hotkey::register_all(&mut registrar, bindings) {
            let _ = report.send(Err(error));
            return;
        }
        if report.send(Ok(thread_id)).is_err() || stop.load(Ordering::SeqCst) {
            let _ = hotkey::unregister_all(&mut registrar, bindings);
            return;
        }

        loop {
            let result = GetMessageW(&mut message, std::ptr::null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if message.message != WM_HOTKEY {
                continue;
            }
            let Some(slot) = HotkeySlot::from_id(message.wParam as i32) else {
                continue;
            };
            if slot == HotkeySlot::Emergency {
                // Immediate stop: do not wait for the GUI event loop.
                cancel.store(true, Ordering::SeqCst);
            }
            if events.send(slot).is_err() {
                break;
            }
            wake();
        }

        let _ = hotkey::unregister_all(&mut registrar, bindings);
    }
}

/// One `RegisterHotKey`/`UnregisterHotKey` pair bound to the current thread.
struct WinHotkeyRegistrar;

impl HotkeyRegistrar for WinHotkeyRegistrar {
    fn register(&mut self, slot: HotkeySlot, key: HotkeyKey) -> Result<(), String> {
        // SAFETY: null hwnd associates the hotkey with the calling thread, which
        // is the thread running the pump above.
        let ok = unsafe {
            RegisterHotKey(
                std::ptr::null_mut(),
                slot.id(),
                MOD_NOREPEAT,
                u32::from(key.virtual_key()),
            )
        };
        if ok == 0 {
            return Err(format!("RegisterHotKey failed (GetLastError={})", unsafe {
                GetLastError()
            }));
        }
        Ok(())
    }

    fn unregister(&mut self, slot: HotkeySlot, _key: HotkeyKey) -> Result<(), String> {
        // SAFETY: releasing a hotkey that was registered by this thread.
        let ok = unsafe { UnregisterHotKey(std::ptr::null_mut(), slot.id()) };
        if ok == 0 {
            return Err(format!(
                "UnregisterHotKey failed (GetLastError={})",
                unsafe { GetLastError() }
            ));
        }
        Ok(())
    }
}
