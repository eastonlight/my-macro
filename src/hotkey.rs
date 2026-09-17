//! Hotkey model and the transactional registration protocol.
//!
//! The protocol lives here (instead of in the Win32 module) so that the
//! "never leave a half-registered arm behind" rule is covered by tests. The
//! Win32 implementation only has to answer one `RegisterHotKey` /
//! `UnregisterHotKey` call per binding.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Keys a user may bind to a macro. F8 stays reserved.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum HotkeyKey {
    #[serde(rename = "Tilde")]
    Tilde,
    #[serde(rename = "Tab")]
    Tab,
    #[serde(rename = "F1")]
    F1,
    #[serde(rename = "F2")]
    F2,
    #[serde(rename = "F3")]
    F3,
    #[serde(rename = "F4")]
    F4,
    #[serde(rename = "F5")]
    F5,
    #[serde(rename = "F6")]
    F6,
    #[serde(rename = "F7")]
    F7,
    #[serde(rename = "F8")]
    F8,
    #[serde(rename = "F9")]
    F9,
    #[serde(rename = "F10")]
    F10,
    #[serde(rename = "F11")]
    F11,
    #[serde(rename = "F12")]
    F12,
}

impl HotkeyKey {
    pub const ALL: [Self; 14] = [
        Self::Tilde,
        Self::Tab,
        Self::F1,
        Self::F2,
        Self::F3,
        Self::F4,
        Self::F5,
        Self::F6,
        Self::F7,
        Self::F8,
        Self::F9,
        Self::F10,
        Self::F11,
        Self::F12,
    ];

    /// Emergency key. Fixed by design: the panic button must not depend on
    /// configuration that may itself be wrong.
    pub const EMERGENCY: Self = Self::F8;

    /// Display name; matches the serde spelling used in the config file.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tilde => "Tilde",
            Self::Tab => "Tab",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
        }
    }

    /// Win32 virtual key code used by RegisterHotKey.
    pub const fn virtual_key(self) -> u16 {
        match self {
            Self::Tilde => 0xC0,
            Self::Tab => 0x09,
            Self::F1 => 0x70,
            Self::F2 => 0x71,
            Self::F3 => 0x72,
            Self::F4 => 0x73,
            Self::F5 => 0x74,
            Self::F6 => 0x75,
            Self::F7 => 0x76,
            Self::F8 => 0x77,
            Self::F9 => 0x78,
            Self::F10 => 0x79,
            Self::F11 => 0x7A,
            Self::F12 => 0x7B,
        }
    }
}

impl fmt::Display for HotkeyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The roles a hotkey can have. The numeric id is what Win32 delivers back in
/// `WM_HOTKEY.wParam`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HotkeySlot {
    /// The single row-build trigger; [`BuildTarget`] decides what it builds.
    Trigger,
    /// The Spire action: scan the screen, click each Spire, verified `A`.
    SpireAction,
    /// The emergency stop. Fixed to F8 and never configurable.
    Emergency,
}

impl HotkeySlot {
    pub const ALL: [Self; 3] = [Self::Trigger, Self::SpireAction, Self::Emergency];

    pub const fn id(self) -> i32 {
        match self {
            Self::Trigger => 1,
            Self::SpireAction => 2,
            Self::Emergency => 3,
        }
    }

    pub const fn from_id(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::Trigger),
            2 => Some(Self::SpireAction),
            3 => Some(Self::Emergency),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Trigger => "trigger",
            Self::SpireAction => "spire action",
            Self::Emergency => "emergency",
        }
    }
}

/// The complete set of hotkeys registered while the app is armed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bindings {
    pub trigger: HotkeyKey,
    pub spire_action: HotkeyKey,
    pub emergency: HotkeyKey,
}

impl Bindings {
    /// The configurable row trigger and Spire action, plus the fixed emergency
    /// stop.
    pub const fn new(trigger: HotkeyKey, spire_action: HotkeyKey) -> Self {
        Self {
            trigger,
            spire_action,
            emergency: HotkeyKey::EMERGENCY,
        }
    }

    pub const fn get(self, slot: HotkeySlot) -> HotkeyKey {
        match slot {
            HotkeySlot::Trigger => self.trigger,
            HotkeySlot::SpireAction => self.spire_action,
            HotkeySlot::Emergency => self.emergency,
        }
    }

    /// Slots in registration order, emergency last.
    pub const fn slots(self) -> [(HotkeySlot, HotkeyKey); 3] {
        [
            (HotkeySlot::Trigger, self.trigger),
            (HotkeySlot::SpireAction, self.spire_action),
            (HotkeySlot::Emergency, self.emergency),
        ]
    }
}

/// A registration or unregistration problem, ready to be shown in the UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HotkeyError {
    Register {
        slot: HotkeySlot,
        key: HotkeyKey,
        /// English diagnostic detail; may mention a failed rollback.
        detail: String,
    },
    /// The listener thread could not be started or did not answer in time.
    Listener { detail: String },
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Register { slot, key, detail } => {
                write!(f, "failed to register {key} for {}: {detail}", slot.label())
            }
            Self::Listener { detail } => write!(f, "hotkey listener failed: {detail}"),
        }
    }
}

impl std::error::Error for HotkeyError {}

/// One `RegisterHotKey`/`UnregisterHotKey` call.
pub trait HotkeyRegistrar {
    fn register(&mut self, slot: HotkeySlot, key: HotkeyKey) -> Result<(), String>;
    fn unregister(&mut self, slot: HotkeySlot, key: HotkeyKey) -> Result<(), String>;
}

/// Registers every binding, or none at all.
///
/// If any registration fails, the bindings registered so far are released
/// again, so a failed arm cannot leave a partial set behind. A failed rollback
/// is reported inside the returned error instead of being swallowed.
pub fn register_all(
    registrar: &mut dyn HotkeyRegistrar,
    bindings: Bindings,
) -> Result<(), HotkeyError> {
    let mut done: Vec<(HotkeySlot, HotkeyKey)> = Vec::new();
    for (slot, key) in bindings.slots() {
        match registrar.register(slot, key) {
            Ok(()) => done.push((slot, key)),
            Err(detail) => {
                let mut detail = detail;
                for (undo_slot, undo_key) in done.iter().rev() {
                    if let Err(rollback) = registrar.unregister(*undo_slot, *undo_key) {
                        detail.push_str(&format!(
                            "; rollback of {} for {} also failed: {rollback}",
                            undo_key,
                            undo_slot.label()
                        ));
                    }
                }
                return Err(HotkeyError::Register { slot, key, detail });
            }
        }
    }
    Ok(())
}

/// Releases every binding, collecting the failures instead of stopping early.
pub fn unregister_all(registrar: &mut dyn HotkeyRegistrar, bindings: Bindings) -> Vec<HotkeyError> {
    let mut errors = Vec::new();
    for (slot, key) in bindings.slots().into_iter().rev() {
        if let Err(detail) = registrar.unregister(slot, key) {
            errors.push(HotkeyError::Register { slot, key, detail });
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeRegistrar;

    #[test]
    fn labels_match_the_config_spelling() {
        #[derive(Deserialize, Serialize)]
        struct Holder {
            key: HotkeyKey,
        }

        for key in HotkeyKey::ALL {
            let encoded = toml::to_string(&Holder { key }).expect("serialize hotkey");
            assert!(
                encoded.contains(&format!("\"{}\"", key.label())),
                "{key:?} was written as '{encoded}'"
            );
            let decoded: Holder = toml::from_str(&encoded).expect("deserialize hotkey");
            assert_eq!(decoded.key, key);
        }
    }

    #[test]
    fn virtual_keys_are_vk_f1_to_vk_f12() {
        assert_eq!(HotkeyKey::F1.virtual_key(), 0x70);
        assert_eq!(HotkeyKey::F6.virtual_key(), 0x75);
        assert_eq!(HotkeyKey::F12.virtual_key(), 0x7B);
    }

    #[test]
    fn slot_ids_round_trip() {
        for slot in HotkeySlot::ALL {
            assert_eq!(HotkeySlot::from_id(slot.id()), Some(slot));
        }
        assert_eq!(HotkeySlot::from_id(99), None);
    }

    #[test]
    fn emergency_slot_is_fixed_to_f8() {
        let bindings = Bindings::new(HotkeyKey::F6, HotkeyKey::F7);
        assert_eq!(bindings.get(HotkeySlot::Emergency), HotkeyKey::F8);
    }

    #[test]
    fn successful_arm_registers_the_trigger_and_the_emergency_key() {
        let mut registrar = FakeRegistrar::default();
        let bindings = Bindings::new(HotkeyKey::F6, HotkeyKey::F7);
        assert_eq!(register_all(&mut registrar, bindings), Ok(()));
        assert_eq!(
            registrar.registered(),
            vec![
                (HotkeySlot::Trigger, HotkeyKey::F6),
                (HotkeySlot::SpireAction, HotkeyKey::F7),
                (HotkeySlot::Emergency, HotkeyKey::F8),
            ]
        );
        assert!(registrar.unregistered().is_empty());
    }

    #[test]
    fn a_failing_registration_rolls_back_the_earlier_bindings() {
        let mut registrar = FakeRegistrar::default();
        registrar.fail_on(HotkeySlot::Emergency, "already in use by another program");
        let bindings = Bindings::new(HotkeyKey::F6, HotkeyKey::F7);

        let error = register_all(&mut registrar, bindings).expect_err("emergency must fail");
        match &error {
            HotkeyError::Register { slot, key, detail } => {
                assert_eq!(*slot, HotkeySlot::Emergency);
                assert_eq!(*key, HotkeyKey::F8);
                assert!(detail.contains("already in use"), "detail was {detail}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        // No binding may survive a failed arm.
        assert_eq!(
            registrar.unregistered(),
            vec![
                (HotkeySlot::SpireAction, HotkeyKey::F7),
                (HotkeySlot::Trigger, HotkeyKey::F6),
            ]
        );
        assert_eq!(registrar.live(), Vec::new());
    }

    #[test]
    fn a_failed_rollback_is_reported_instead_of_hidden() {
        let mut registrar = FakeRegistrar::default();
        registrar.fail_on(HotkeySlot::Emergency, "already in use");
        registrar.fail_unregister(HotkeySlot::Trigger, "no such binding");

        let error = register_all(&mut registrar, Bindings::new(HotkeyKey::F6, HotkeyKey::F7))
            .expect_err("emergency must fail");
        let detail = match error {
            HotkeyError::Register { detail, .. } => detail,
            other => panic!("unexpected error: {other:?}"),
        };
        assert!(detail.contains("rollback"), "detail was {detail}");
    }

    #[test]
    fn an_unregisterable_trigger_is_reported_after_a_failed_arm() {
        let mut registrar = FakeRegistrar::default();
        registrar.fail_on(HotkeySlot::Emergency, "F8 taken");
        let error = register_all(&mut registrar, Bindings::new(HotkeyKey::F6, HotkeyKey::F7))
            .expect_err("emergency must fail");
        assert!(matches!(
            error,
            HotkeyError::Register {
                slot: HotkeySlot::Emergency,
                ..
            }
        ));
        assert_eq!(
            registrar.unregistered(),
            vec![
                (HotkeySlot::SpireAction, HotkeyKey::F7),
                (HotkeySlot::Trigger, HotkeyKey::F6),
            ]
        );
    }

    #[test]
    fn unregister_all_reports_every_failure() {
        let mut registrar = FakeRegistrar::default();
        assert_eq!(
            register_all(&mut registrar, Bindings::new(HotkeyKey::F6, HotkeyKey::F7)),
            Ok(())
        );
        registrar.fail_unregister(HotkeySlot::Emergency, "gone");

        let errors = unregister_all(&mut registrar, Bindings::new(HotkeyKey::F6, HotkeyKey::F7));
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            errors.first(),
            Some(HotkeyError::Register {
                slot: HotkeySlot::Emergency,
                ..
            })
        ));
        // Everything that could be released was released; only the failed one
        // is still live, and that is exactly what the error reports.
        assert_eq!(
            registrar.live(),
            vec![(HotkeySlot::Emergency, HotkeyKey::F8)]
        );
    }
}
