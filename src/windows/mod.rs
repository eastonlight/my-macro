//! Windows-only integration layer.
//!
//! * [`adapter`] — `SendInput` scan-code taps and the foreground/modifier safety gate.
//! * [`listener`] — `RegisterHotKey` message pump thread.
//! * [`app`] — the eframe/egui application.

pub mod adapter;
pub mod app;
pub mod listener;

pub use adapter::SendInputAdapter;
pub use app::run;
