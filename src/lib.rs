//! oh-my-macro: a small, Windows desktop macro helper for StarCraft 1.
//!
//! Layout:
//!
//! * [`macros`] — pure description of the two supported macros and their timing plan.
//! * [`engine`] — pure executor: walks a plan, honours cancellation, releases held input.
//! * [`runner`] — thread supervisor: one macro at a time, reports, prompt cancel, no leaks.
//! * [`config`] — settings model, validation, TOML persistence.
//! * [`hotkey`] — hotkey model plus the (testable) transactional registration protocol.
//! * [`input`] — the [`input::InputAdapter`] seam between the engine and the OS.
//! * [`ui_text`] — Korean/English UI strings.
//! * [`font`] — Korean font discovery helpers.
//! * `windows` — the Win32 adapter (SendInput, RegisterHotKey) and the egui application.
//!
//! Everything except `windows` is host independent, which is what allows the
//! pure tests in this crate to run on Linux.

pub mod colony;
pub mod config;
pub mod engine;
pub mod font;
pub mod frame;
pub mod hotkey;
pub mod input;
pub mod macros;
pub mod play_area;
pub mod runner;
pub mod spire_action;
pub mod spire_vision;
pub mod ui_text;
pub mod vision;

#[cfg(windows)]
pub mod windows;

#[cfg(test)]
pub(crate) mod test_support;
