//! Pure description of the two supported StarCraft macros.
//!
//! Nothing here touches the operating system: this module only states *which*
//! events have to be sent, and how long to wait between them.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The only keys this application is able to inject.
///
/// Deliberately a closed set: no configuration can turn this tool into a
/// general purpose keyboard/mouse automator. `B/C/V/S` are the build-menu
/// keys; `Ctrl`, `Shift` and `9` exist only for the scratch control group, and
/// `Esc` only to leave a placement mode the game did not close after a click.
/// The modifier chord never leaves a modifier held on its own.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Key {
    B,
    C,
    V,
    S,
    /// Left Ctrl, used only for the scratch-group chord (Ctrl+9) and never
    /// left held past the chord.
    Control,
    /// Left Shift, used only for the "remove one portrait" click.
    Shift,
    /// Digit 9, the one scratch control group this tool is allowed to touch.
    Nine,
    /// Escape, tapped once when forced mode could not confirm that a click left
    /// placement mode, so the next drone starts from a clean state.
    Escape,
    /// `A`, the one action key the Spire action presses after a verified crown
    /// selection. Never sent before the selection panel confirms the Spire.
    A,
    /// Recall the user's optional Stargate camera location.
    F2,
    /// Recall the user's saved Colony camera location; never combined with Shift.
    F4,
}

impl Key {
    /// Every key this tool may press, used when releasing held input.
    pub const ALL: [Self; 11] = [
        Self::B,
        Self::C,
        Self::V,
        Self::S,
        Self::Control,
        Self::Shift,
        Self::Nine,
        Self::Escape,
        Self::A,
        Self::F2,
        Self::F4,
    ];

    /// Set 1 ("XT") hardware scan code, the value `SendInput` expects together
    /// with `KEYEVENTF_SCANCODE`.
    ///
    /// Scan codes address *physical* key positions, so the injected key is the
    /// same key regardless of the active layout (for example Korean 2-set).
    pub const fn scan_code(self) -> u16 {
        match self {
            Self::B => 0x30,
            Self::C => 0x2E,
            Self::V => 0x2F,
            Self::S => 0x1F,
            Self::Control => 0x1D,
            Self::Shift => 0x2A,
            Self::Nine => 0x0A,
            Self::Escape => 0x01,
            Self::A => 0x1E,
            Self::F2 => 0x3C,
            Self::F4 => 0x3E,
        }
    }

    /// Name used in logs and diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            Self::B => "B",
            Self::C => "C",
            Self::V => "V",
            Self::S => "S",
            Self::Control => "Ctrl",
            Self::Shift => "Shift",
            Self::Nine => "9",
            Self::Escape => "Esc",
            Self::A => "A",
            Self::F2 => "F2",
            Self::F4 => "F4",
        }
    }
}

/// A single low level input event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Primitive {
    KeyDown(Key),
    KeyUp(Key),
    MouseLeftDown,
    MouseLeftUp,
}

impl Primitive {
    /// English diagnostic text. The GUI adds its own localized framing.
    pub fn describe(self) -> String {
        match self {
            Self::KeyDown(key) => format!("key down {}", key.name()),
            Self::KeyUp(key) => format!("key up {}", key.name()),
            Self::MouseLeftDown => "left button down".to_owned(),
            Self::MouseLeftUp => "left button up".to_owned(),
        }
    }
}

/// The supported macros.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacroId {
    /// `B` -> `C` -> left click, builds a Creep Colony.
    CreepColony,
    /// `V` -> `S` -> left click, builds a Spire.
    Spire,
}

impl MacroId {
    pub const ALL: [Self; 2] = [Self::CreepColony, Self::Spire];

    /// The two build-menu keys, pressed in order before the click.
    pub const fn build_keys(self) -> [Key; 2] {
        match self {
            Self::CreepColony => [Key::B, Key::C],
            Self::Spire => [Key::V, Key::S],
        }
    }

    /// Stable identifier used in logs.
    pub const fn name(self) -> &'static str {
        match self {
            Self::CreepColony => "CreepColony",
            Self::Spire => "Spire",
        }
    }
}

/// Which building the unified row-build macro orders.
///
/// Both trigger hotkeys run the same row macro; this enum is the only axis
/// that differs between the two buildings: the two build-menu keys and the
/// footprint of one building.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BuildTarget {
    /// `B`,`C` — square 2x2 footprint (the default).
    #[default]
    Colony,
    /// `V`,`S` — same calibrated footprint as the Creep Colony.
    Spire,
}

impl BuildTarget {
    /// Every selectable target, in the order the GUI offers them.
    pub const ALL: [Self; 2] = [Self::Colony, Self::Spire];

    /// Stable config-file spelling, used by the serde implementation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Colony => "colony",
            Self::Spire => "spire",
        }
    }

    /// Parses [`Self::as_str`]. An unknown string is rejected, never guessed.
    // The name is part of the config boundary; the return type is deliberately
    // `Option` (the caller maps it to a serde error), so the `FromStr` trait
    // would not fit the signature.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|target| target.as_str() == value)
    }

    /// Training-name spelling used in logs and diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Colony => "CreepColony",
            Self::Spire => "Spire",
        }
    }

    /// The two build-menu keys, pressed in order before each placement click.
    pub const fn build_keys(self) -> [Key; 2] {
        match self {
            Self::Colony => [Key::B, Key::C],
            Self::Spire => [Key::V, Key::S],
        }
    }

    /// Footprint width in client pixels, and therefore the exact
    /// centre-to-centre spacing of a row (`64`/`96` logical px at 1080p).
    pub const fn footprint_px(self) -> i32 {
        match self {
            Self::Colony => 144,
            Self::Spire => 144,
        }
    }

    /// Half of [`Self::footprint_px`], used for the safe-area math.
    pub const fn half_px(self) -> i32 {
        self.footprint_px() / 2
    }

    /// Run-report key of the macro that orders this building.
    pub const fn macro_id(self) -> MacroId {
        match self {
            Self::Colony => MacroId::CreepColony,
            Self::Spire => MacroId::Spire,
        }
    }
}

impl Serialize for BuildTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BuildTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match Self::from_str(&value) {
            Some(target) => Ok(target),
            None => Err(serde::de::Error::custom(format!(
                "unknown build target '{value}'; expected '{}' or '{}'",
                Self::Colony.as_str(),
                Self::Spire.as_str()
            ))),
        }
    }
}

/// Timing knobs, already validated by [`crate::config::Config::validate`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timing {
    /// How long a key stays pressed, and how long a mouse button stays down.
    pub press: Duration,
    /// Quiet gap after a completed event, before the next one starts.
    pub gap: Duration,
}

impl Timing {
    pub const fn from_millis(press_ms: u32, gap_ms: u32) -> Self {
        Self {
            press: Duration::from_millis(press_ms as u64),
            gap: Duration::from_millis(gap_ms as u64),
        }
    }
}

/// One primitive plus the pause that follows it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlannedStep {
    pub primitive: Primitive,
    pub delay_after: Duration,
}

/// Builds the exact event sequence for `macro_id`.
///
/// The click happens at the *current* mouse position; no coordinate is stored
/// anywhere, which keeps the macro usable with whatever the player prepared on
/// screen.
pub fn plan(macro_id: MacroId, timing: Timing) -> Vec<PlannedStep> {
    let [first, second] = macro_id.build_keys();
    let mut steps = Vec::with_capacity(6);
    for key in [first, second] {
        steps.push(PlannedStep {
            primitive: Primitive::KeyDown(key),
            delay_after: timing.press,
        });
        steps.push(PlannedStep {
            primitive: Primitive::KeyUp(key),
            delay_after: timing.gap,
        });
    }
    steps.push(PlannedStep {
        primitive: Primitive::MouseLeftDown,
        delay_after: timing.press,
    });
    steps.push(PlannedStep {
        primitive: Primitive::MouseLeftUp,
        delay_after: timing.gap,
    });
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creep_colony_plan_is_b_c_then_click() {
        let timing = Timing::from_millis(50, 50);
        let primitives: Vec<_> = plan(MacroId::CreepColony, timing)
            .into_iter()
            .map(|step| step.primitive)
            .collect();
        assert_eq!(
            primitives,
            vec![
                Primitive::KeyDown(Key::B),
                Primitive::KeyUp(Key::B),
                Primitive::KeyDown(Key::C),
                Primitive::KeyUp(Key::C),
                Primitive::MouseLeftDown,
                Primitive::MouseLeftUp,
            ]
        );
    }

    #[test]
    fn spire_plan_is_v_s_then_click() {
        let timing = Timing::from_millis(50, 50);
        let primitives: Vec<_> = plan(MacroId::Spire, timing)
            .into_iter()
            .map(|step| step.primitive)
            .collect();
        assert_eq!(
            primitives,
            vec![
                Primitive::KeyDown(Key::V),
                Primitive::KeyUp(Key::V),
                Primitive::KeyDown(Key::S),
                Primitive::KeyUp(Key::S),
                Primitive::MouseLeftDown,
                Primitive::MouseLeftUp,
            ]
        );
    }

    #[test]
    fn plan_takes_delays_from_the_configured_timing() {
        let timing = Timing::from_millis(40, 70);
        let delays: Vec<_> = plan(MacroId::CreepColony, timing)
            .into_iter()
            .map(|step| step.delay_after)
            .collect();
        assert_eq!(
            delays,
            vec![
                Duration::from_millis(40), // B down
                Duration::from_millis(70), // B up
                Duration::from_millis(40), // C down
                Duration::from_millis(70), // C up
                Duration::from_millis(40), // click down
                Duration::from_millis(70), // click up
            ]
        );
    }

    #[test]
    fn build_target_key_pairs_are_b_c_and_v_s() {
        assert_eq!(BuildTarget::Colony.build_keys(), [Key::B, Key::C]);
        assert_eq!(BuildTarget::Spire.build_keys(), [Key::V, Key::S]);
        for target in BuildTarget::ALL {
            let keys = target.build_keys();
            assert_ne!(keys[0], keys[1], "{target:?} cannot use the same key twice");
        }
    }

    #[test]
    fn both_build_targets_use_a_two_tile_footprint() {
        // 1920x1080 renders one tile (32 logical px) as 72 client px, and both
        // supported buildings are ordered on the same two-tile pitch.
        const TILE_PX: i32 = 72;
        assert_eq!(BuildTarget::Colony.footprint_px(), 2 * TILE_PX);
        assert_eq!(BuildTarget::Spire.footprint_px(), 2 * TILE_PX);
        for target in BuildTarget::ALL {
            assert_eq!(target.half_px(), target.footprint_px() / 2);
            assert!(target.half_px() < target.footprint_px());
        }
    }

    #[test]
    fn the_build_target_string_form_is_stable_and_round_trips() {
        assert_eq!(BuildTarget::default(), BuildTarget::Colony);
        assert_eq!(BuildTarget::Colony.as_str(), "colony");
        assert_eq!(BuildTarget::Spire.as_str(), "spire");
        for target in BuildTarget::ALL {
            assert_eq!(BuildTarget::from_str(target.as_str()), Some(target));
        }
        assert_eq!(BuildTarget::from_str("CreepColony"), None);
        assert_eq!(BuildTarget::from_str(""), None);
    }

    #[test]
    fn every_build_target_maps_to_its_run_report_macro() {
        assert_eq!(BuildTarget::Colony.macro_id(), MacroId::CreepColony);
        assert_eq!(BuildTarget::Spire.macro_id(), MacroId::Spire);
        for target in BuildTarget::ALL {
            assert_eq!(target.build_keys(), target.macro_id().build_keys());
            assert!(!target.name().is_empty());
        }
    }

    #[test]
    fn scan_codes_are_the_set_1_positions() {
        assert_eq!(Key::B.scan_code(), 0x30);
        assert_eq!(Key::C.scan_code(), 0x2E);
        assert_eq!(Key::V.scan_code(), 0x2F);
        assert_eq!(Key::S.scan_code(), 0x1F);
        assert_eq!(Key::Control.scan_code(), 0x1D);
        assert_eq!(Key::Shift.scan_code(), 0x2A);
        assert_eq!(Key::Nine.scan_code(), 0x0A);
        assert_eq!(Key::A.scan_code(), 0x1E);
        assert_eq!(Key::F4.scan_code(), 0x3E, "F4 recalls the saved view");
        assert_eq!(Key::Escape.scan_code(), 0x01);
    }

    #[test]
    fn all_keys_are_covered_by_the_release_list() {
        assert_eq!(Key::ALL.len(), 11);
        assert!(Key::ALL.contains(&Key::F4), "the third feature presses F4");
        assert!(
            Key::ALL.contains(&Key::F2),
            "Stargate camera recall presses F2"
        );
        for key in Key::ALL {
            assert!(!key.name().is_empty());
        }
    }
}
