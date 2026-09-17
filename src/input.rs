//! The seam between the macro engine and the operating system.
//!
//! The engine only ever talks to an [`InputAdapter`]. The Windows
//! implementation lives in `crate::windows::adapter`; tests use a fake.

use crate::frame::{Frame, Point, Rect};
use crate::macros::Key;

/// A reason why an input event could not be delivered, or why the safety gate
/// refused to act.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputError {
    /// The OS refused to inject the event (for example `SendInput` returned 0).
    Injection(String),
    /// The pre-action safety gate refused: wrong foreground window, a held
    /// modifier, or a process that cannot be queried.
    Unsafe(String),
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Injection(detail) => write!(f, "input injection failed: {detail}"),
            Self::Unsafe(detail) => write!(f, "safety check refused: {detail}"),
        }
    }
}

impl std::error::Error for InputError {}

/// Sends low level input events.
///
/// Implementations must be safe to move to the worker thread that runs a macro.
pub trait InputAdapter: Send {
    fn key_down(&mut self, key: Key) -> Result<(), InputError>;
    fn key_up(&mut self, key: Key) -> Result<(), InputError>;
    fn mouse_left_down(&mut self) -> Result<(), InputError>;
    fn mouse_left_up(&mut self) -> Result<(), InputError>;

    /// Releases only keys and buttons successfully pressed by this adapter
    /// that have not yet been successfully released. Never release unrelated
    /// user input, especially after the initial safety check refused to act.
    ///
    /// Must never be blocked by [`Self::safety_check`]: releasing a key is
    /// always allowed, including after the user cancelled or switched windows.
    /// Implementations must be idempotent, because the engine also calls this
    /// when nothing is held.
    fn release_all(&mut self) -> Result<(), InputError>;

    /// Checked before *every* injected event.
    ///
    /// Returns [`InputError::Unsafe`] when injecting now would be wrong, for
    /// example because the foreground window does not belong to the configured
    /// game process, or because Ctrl/Alt/Shift/Win is physically held down.
    fn safety_check(&mut self) -> Result<(), InputError>;
}

/// A modifier key this tool may press *for itself* as part of a chord.
///
/// The safety gate refuses to inject while a modifier is physically held, so
/// an adapter that presses its own Ctrl or Shift must remember it owns that
/// modifier and exclude it from that check. Everything else stays blocked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Modifier {
    Control,
    Shift,
}

impl Modifier {
    pub const ALL: [Self; 2] = [Self::Control, Self::Shift];

    /// The physical key names `GetAsyncKeyState` reports for this modifier
    /// when the *left* instance was injected. The right-hand variants are not
    /// listed, so a user holding right Ctrl/Shift is still refused.
    pub const fn held_names(self) -> &'static [&'static str] {
        match self {
            Self::Control => &["left ctrl", "ctrl"],
            Self::Shift => &["left shift", "shift"],
        }
    }
}

/// Physical modifiers that are held but do **not** belong to this tool.
///
/// `held` is the full list of physically down modifier names; `owned` are the
/// modifiers this adapter injected itself. The result is what the safety gate
/// must still refuse on.
pub fn unowned_modifiers(held: &[&'static str], owned: &[Modifier]) -> Vec<&'static str> {
    held.iter()
        .filter(|name| !owned.iter().any(|m| m.held_names().contains(name)))
        .copied()
        .collect()
}

/// The extra desktop powers the F6 colony-row macro needs on top of injecting
/// input: reading and moving the mouse cursor, and capturing the game client.
///
/// Kept as a separate trait so the pure row state machine can run against a
/// fake on any host while only the Windows adapter touches Win32.
pub trait DesktopAdapter: InputAdapter {
    /// Current cursor position in physical screen pixels.
    fn cursor_position(&mut self) -> Result<Point, InputError>;
    /// Moves the cursor; the adapter runs the safety gate first.
    fn move_cursor(&mut self, point: Point) -> Result<(), InputError>;
    /// Captures the game client area as a [`Frame`].
    fn capture_client(&mut self) -> Result<Frame, InputError>;
    /// Captures only `rect` (physical screen pixels) as a [`Frame`] whose
    /// origin is `(rect.x, rect.y)`.
    ///
    /// Used for the bounded selection-panel verification after a click, so a
    /// per-target check never re-runs the full-screen scan. Implementations
    /// must run the same safety gate as [`Self::capture_client`].
    fn capture_region(&mut self, rect: Rect) -> Result<Frame, InputError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owned_modifier_does_not_block_our_own_chord() {
        let held = ["left ctrl", "ctrl"];
        assert!(unowned_modifiers(&held, &[Modifier::Control]).is_empty());
    }

    #[test]
    fn a_user_held_modifier_still_blocks() {
        assert_eq!(unowned_modifiers(&["left shift"], &[]), vec!["left shift"]);
        assert_eq!(
            unowned_modifiers(&["right ctrl"], &[Modifier::Control]),
            vec!["right ctrl"]
        );
        assert_eq!(
            unowned_modifiers(&["left ctrl", "right shift"], &[Modifier::Control]),
            vec!["right shift"]
        );
    }

    #[test]
    fn owning_one_modifier_does_not_excuse_the_other() {
        assert_eq!(
            unowned_modifiers(&["left shift"], &[Modifier::Control]),
            vec!["left shift"]
        );
    }
}
