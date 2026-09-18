//! Settings model, validation and TOML persistence.
//!
//! Loading never fails hard: a broken or out-of-range file falls back to the
//! defaults *and* returns a [`ConfigWarning`] the GUI shows to the user.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::colony::RowMode;
use crate::hotkey::{Bindings, HotkeyKey};
use crate::macros::{BuildTarget, Timing};

/// Process name (exe basename) that must own the foreground window before any
/// event is injected.
pub const DEFAULT_TARGET_PROCESS: &str = "StarCraft.exe";

/// Interval bounds, in milliseconds. Every press/gap knob uses them.
pub const MIN_INTERVAL_MS: u32 = 1;
pub const MAX_INTERVAL_MS: u32 = 2000;
/// Existing default timing: 20 ms press, 20 ms gap. The minimum reliable
/// interval still requires verification against the live game and host.
/// A previously saved `press_ms`/`gap_ms` pair always wins over this default.
pub const DEFAULT_INTERVAL_MS: u32 = 20;

/// Emergency stop key, fixed by design (see [`HotkeyKey::EMERGENCY`]).
pub const EMERGENCY_HOTKEY: HotkeyKey = HotkeyKey::EMERGENCY;

/// Default trigger key: the row build runs from this single key.
pub const DEFAULT_TRIGGER_HOTKEY: HotkeyKey = HotkeyKey::F7;

/// Default key for the Spire action (scan, click, verified `A`).
pub const DEFAULT_SPIRE_ACTION_HOTKEY: HotkeyKey = HotkeyKey::Tab;
/// Default key for the F4 saved-view Colony search. F5 is deliberately not
/// used: StarCraft 1 itself binds F-keys (F2-F5 among them), so the default
/// stays on an F-key the game does not use.
pub const DEFAULT_VACANT_COLONY_HOTKEY: HotkeyKey = HotkeyKey::F6;

/// Default key for the Stargate action (scan, click each gate, verified `A`).
pub const DEFAULT_STARGATE_ACTION_HOTKEY: HotkeyKey = HotkeyKey::Tilde;

fn default_vacant_colony_hotkey(trigger: HotkeyKey, action: HotkeyKey) -> HotkeyKey {
    Bindings::new(trigger, action).vacant_colony
}

/// Fallback used when the configured row trigger already occupies the Spire
/// action's default key: a previously valid file must keep loading instead of
/// being rejected, so the action silently moves to the next free F-key.
pub const FALLBACK_SPIRE_ACTION_HOTKEY: HotkeyKey = HotkeyKey::F9;

/// The non-conflicting default for a file that does not name the Spire action
/// key yet.
fn default_spire_action_hotkey(trigger: HotkeyKey) -> HotkeyKey {
    if trigger == DEFAULT_SPIRE_ACTION_HOTKEY {
        FALLBACK_SPIRE_ACTION_HOTKEY
    } else {
        DEFAULT_SPIRE_ACTION_HOTKEY
    }
}

/// The non-conflicting default for a file that does not name the Stargate
/// action key yet. Tilde is the normal default; an occupied Tilde falls through
/// the free-key chain ([`crate::hotkey::STARGATE_FALLBACK_KEYS`]).
fn default_stargate_action_hotkey(
    trigger: HotkeyKey,
    action: HotkeyKey,
    vacant: HotkeyKey,
) -> HotkeyKey {
    crate::hotkey::STARGATE_FALLBACK_KEYS
        .into_iter()
        .find(|key| *key != trigger && *key != action && *key != vacant && *key != EMERGENCY_HOTKEY)
        .expect("four bindings cannot occupy every fallback key")
}

/// Persisted settings.
///
/// One trigger hotkey runs the row-build macro; [`BuildTarget`] picks the
/// building, and one shared press/gap pair drives the timing. Files written by
/// older versions stored two hotkeys (`colony_hotkey`/`trigger_hotkey`) and one
/// timing pair per macro; [`Config::parse`] still reads them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Config {
    /// The single key that starts the row build.
    pub trigger_hotkey: HotkeyKey,
    /// The key that starts the Spire action (one full-screen search, click each
    /// crown, verified `A`). Independent of [`Self::trigger_hotkey`] and never
    /// F8.
    ///
    /// There is no preview-only mode and no toggle for one any more: pressing
    /// this key always runs the action. Files that still store the removed
    /// `spire_scan_only` key load as usual — the legacy key is parsed and
    /// ignored, never turned into a setting.
    pub spire_action_hotkey: HotkeyKey,
    /// The key that starts the Stargate action (one full-screen search, click
    /// each gate's lower-hull core, verified `A`). Independent of the other three
    /// and never F8 or F4. Default [`DEFAULT_STARGATE_ACTION_HOTKEY`] (Tilde).
    pub stargate_action_hotkey: HotkeyKey,
    /// When enabled, tap F2 and wait for the saved view before scanning gates.
    pub stargate_recall_f2: bool,
    /// F4 saved-view Colony search (default [`DEFAULT_VACANT_COLONY_HOTKEY`]).
    /// F4 itself stays available to the game: it is never a binding here, and
    /// the macro only *presses* F4 to recall the player's saved view.
    pub vacant_colony_hotkey: HotkeyKey,
    /// Which building the row-build macro orders.
    pub build_target: BuildTarget,
    /// Order in which the row's footprints are built. The span (cursor =
    /// leftmost footprint) is the same in both modes.
    pub colony_row_mode: RowMode,
    /// When the green placement preview cannot be confirmed, click anyway and
    /// keep going instead of aborting the run.
    pub force_build: bool,
    /// How long a key/button stays pressed.
    pub press_ms: u32,
    /// Quiet gap between two events.
    pub gap_ms: u32,
    /// Foreground process (exe basename) allowed to receive input.
    pub target_process: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trigger_hotkey: DEFAULT_TRIGGER_HOTKEY,
            spire_action_hotkey: default_spire_action_hotkey(DEFAULT_TRIGGER_HOTKEY),
            stargate_action_hotkey: DEFAULT_STARGATE_ACTION_HOTKEY,
            stargate_recall_f2: false,
            vacant_colony_hotkey: DEFAULT_VACANT_COLONY_HOTKEY,
            build_target: BuildTarget::default(),
            colony_row_mode: RowMode::default(),
            force_build: true,
            press_ms: DEFAULT_INTERVAL_MS,
            gap_ms: DEFAULT_INTERVAL_MS,
            target_process: DEFAULT_TARGET_PROCESS.to_owned(),
        }
    }
}

/// Why a settings file or an edited value was rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    Io(String),
    Parse(String),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(detail) => write!(f, "settings file i/o failed: {detail}"),
            Self::Parse(detail) => write!(f, "settings file could not be parsed: {detail}"),
            Self::Invalid(detail) => write!(f, "settings are not usable: {detail}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Non-fatal problem found while loading; the defaults were used instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigWarning {
    Read { detail: String },
    Parse { detail: String },
    Invalid { detail: String },
}

impl ConfigWarning {
    pub fn detail(&self) -> &str {
        match self {
            Self::Read { detail } | Self::Parse { detail } | Self::Invalid { detail } => detail,
        }
    }
}

/// Result of [`Config::load`]: usable settings, plus at most one warning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Loaded {
    pub config: Config,
    pub warning: Option<ConfigWarning>,
}

impl Config {
    /// Rejects values that would produce unusable or dangerous behaviour.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (field, value) in [("press_ms", self.press_ms), ("gap_ms", self.gap_ms)] {
            validate_interval(field, value)?;
        }
        if self.trigger_hotkey == EMERGENCY_HOTKEY {
            return Err(ConfigError::Invalid(format!(
                "trigger_hotkey is {}, which is reserved for the emergency stop",
                EMERGENCY_HOTKEY
            )));
        }
        if self.spire_action_hotkey == EMERGENCY_HOTKEY {
            return Err(ConfigError::Invalid(format!(
                "spire_action_hotkey is {}, which is reserved for the emergency stop",
                EMERGENCY_HOTKEY
            )));
        }
        if self.stargate_action_hotkey == EMERGENCY_HOTKEY {
            return Err(ConfigError::Invalid(format!(
                "stargate_action_hotkey is {}, which is reserved for the emergency stop",
                EMERGENCY_HOTKEY
            )));
        }
        if self.spire_action_hotkey == self.trigger_hotkey {
            return Err(ConfigError::Invalid(format!(
                "trigger_hotkey and spire_action_hotkey are both {}; they must differ",
                self.trigger_hotkey
            )));
        }
        for (field, key) in [
            ("trigger_hotkey", self.trigger_hotkey),
            ("spire_action_hotkey", self.spire_action_hotkey),
            ("stargate_action_hotkey", self.stargate_action_hotkey),
            ("vacant_colony_hotkey", self.vacant_colony_hotkey),
        ] {
            if key == HotkeyKey::F4 {
                return Err(ConfigError::Invalid(format!(
                    "{field}: F4 is reserved for the game's saved camera view"
                )));
            }
        }
        if [
            EMERGENCY_HOTKEY,
            self.trigger_hotkey,
            self.spire_action_hotkey,
        ]
        .contains(&self.vacant_colony_hotkey)
        {
            return Err(ConfigError::Invalid(
                "vacant_colony_hotkey must differ from the other hotkeys and F8".to_owned(),
            ));
        }
        if [
            EMERGENCY_HOTKEY,
            self.trigger_hotkey,
            self.spire_action_hotkey,
            self.vacant_colony_hotkey,
        ]
        .contains(&self.stargate_action_hotkey)
        {
            return Err(ConfigError::Invalid(
                "stargate_action_hotkey must differ from the other hotkeys and F8".to_owned(),
            ));
        }
        validate_target_process(&self.target_process)?;
        Ok(())
    }

    /// The one timing both hotkeys use, already validated by
    /// [`Self::validate`].
    pub fn timing(&self) -> Timing {
        Timing::from_millis(self.press_ms, self.gap_ms)
    }

    /// Mutable shared press/gap pair, used by the GUI sliders.
    pub fn intervals_mut(&mut self) -> (&mut u32, &mut u32) {
        (&mut self.press_ms, &mut self.gap_ms)
    }

    pub fn bindings(&self) -> Bindings {
        Bindings::new(self.trigger_hotkey, self.spire_action_hotkey)
            .with_vacant_colony(self.vacant_colony_hotkey)
            .with_stargate_action(self.stargate_action_hotkey)
    }

    /// Parses one settings file, migrating the legacy shared timing if present.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let file: SettingsFile =
            toml::from_str(text).map_err(|error| ConfigError::Parse(error.to_string()))?;
        let mut config = file.into_config()?;
        if config.trigger_hotkey == HotkeyKey::F6 && config.spire_action_hotkey == HotkeyKey::F7 {
            config.trigger_hotkey = DEFAULT_TRIGGER_HOTKEY;
            config.spire_action_hotkey = DEFAULT_SPIRE_ACTION_HOTKEY;
        }
        // Migrate the immediately previous defaults so an existing install
        // receives the requested swap instead of retaining Tilde/F7 forever.
        // Any other explicit pair is a custom binding and remains untouched.
        if config.trigger_hotkey == HotkeyKey::Tilde
            && config.stargate_action_hotkey == HotkeyKey::F7
        {
            config.trigger_hotkey = HotkeyKey::F7;
            config.stargate_action_hotkey = HotkeyKey::Tilde;
        }
        config.validate()?;
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String, ConfigError> {
        self.validate()?;
        toml::to_string(self).map_err(|error| ConfigError::Parse(error.to_string()))
    }

    /// Reads `path`. A missing file is normal and yields the defaults silently.
    pub fn load(path: &Path) -> Loaded {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Loaded {
                    config: Self::default(),
                    warning: None,
                };
            }
            Err(error) => {
                return Loaded {
                    config: Self::default(),
                    warning: Some(ConfigWarning::Read {
                        detail: error.to_string(),
                    }),
                };
            }
        };
        match Self::parse(&text) {
            Ok(config) => Loaded {
                config,
                warning: None,
            },
            Err(ConfigError::Parse(detail)) => Loaded {
                config: Self::default(),
                warning: Some(ConfigWarning::Parse { detail }),
            },
            Err(other) => Loaded {
                config: Self::default(),
                warning: Some(ConfigWarning::Invalid {
                    detail: other.to_string(),
                }),
            },
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = self.to_toml()?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::Io(e.to_string()))?;
        }
        std::fs::write(path, text).map_err(|e| ConfigError::Io(e.to_string()))
    }
}

/// Raw on-disk shape.
///
/// Kept separate from [`Config`] because older files stored the two intervals
/// once per macro (`colony_press_ms`/`colony_gap_ms` and
/// `spire_press_ms`/`spire_gap_ms`) instead of the single shared
/// `press_ms`/`gap_ms` pair. Exactly one schema must be present: a file that
/// mixes them, fills only half of one, or reports two different per-macro
/// pairs is rejected instead of silently losing settings.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsFile {
    /// The single trigger key. Missing in files written before the two row
    /// hotkeys were merged, where `colony_hotkey` (or `spire_hotkey`) carried
    /// the same role.
    #[serde(default)]
    trigger_hotkey: Option<HotkeyKey>,
    /// The Spire action key. Missing in files written before the action
    /// existed; [`SettingsFile::into_config`] then picks a free default.
    #[serde(default)]
    spire_action_hotkey: Option<HotkeyKey>,
    /// The Stargate action key. Missing in files written before this action
    /// existed; [`SettingsFile::into_config`] then picks a free default.
    #[serde(default)]
    stargate_action_hotkey: Option<HotkeyKey>,
    #[serde(default)]
    stargate_recall_f2: Option<bool>,
    #[serde(default)]
    vacant_colony_hotkey: Option<HotkeyKey>,
    /// Removed key, accepted only so an older file still loads.
    ///
    /// Files written while the preview-only mode existed store
    /// `spire_scan_only = true` (or `false`). Both values are parsed and then
    /// deliberately ignored: [`SettingsFile::into_config`] never reads this
    /// field into [`Config`] and [`Config::to_toml`] never writes it again, so
    /// a stored `true` cannot keep the action from running. The field must
    /// stay declared because the file shape denies unknown keys.
    #[serde(default)]
    spire_scan_only: Option<bool>,
    /// Older spelling: the Creep Colony hotkey.
    #[serde(default)]
    colony_hotkey: Option<HotkeyKey>,
    /// Older spelling: the Spire hotkey.
    #[serde(default)]
    spire_hotkey: Option<HotkeyKey>,
    /// Missing in files written before the build target was selectable, which
    /// means the Creep Colony.
    #[serde(default)]
    build_target: Option<BuildTarget>,
    /// Missing in files written before the row order was configurable, which
    /// means the default left-to-right order.
    #[serde(default)]
    colony_row_mode: Option<RowMode>,
    /// Missing in files written before the forced mode existed, which kept the
    /// run going anyway.
    #[serde(default)]
    force_build: Option<bool>,
    /// Shared value: how long a key/button stays pressed.
    #[serde(default)]
    press_ms: Option<u32>,
    /// Shared value: quiet gap between two events.
    #[serde(default)]
    gap_ms: Option<u32>,
    /// Older per-macro spelling, only accepted while both pairs are equal.
    #[serde(default)]
    colony_press_ms: Option<u32>,
    #[serde(default)]
    colony_gap_ms: Option<u32>,
    #[serde(default)]
    spire_press_ms: Option<u32>,
    #[serde(default)]
    spire_gap_ms: Option<u32>,
    target_process: String,
}

impl SettingsFile {
    fn into_config(self) -> Result<Config, ConfigError> {
        // The removed `spire_scan_only` key is read here only so it counts as
        // consumed, and then dropped: the action key always runs the action
        // now, whatever an older file says. Nothing else in this file is
        // affected, so a legacy `true` or `false` cannot reset the settings.
        let _legacy_scan_only = self.spire_scan_only;

        let shared = self.press_ms.is_some() || self.gap_ms.is_some();
        let per_macro = self.colony_press_ms.is_some()
            || self.colony_gap_ms.is_some()
            || self.spire_press_ms.is_some()
            || self.spire_gap_ms.is_some();

        let (press_ms, gap_ms) = match (shared, per_macro) {
            (true, true) => {
                return Err(ConfigError::Invalid(
                    "settings mix the shared press_ms/gap_ms with the per-macro \
                     colony_press_ms/colony_gap_ms and spire_press_ms/spire_gap_ms; keep only \
                     the shared press_ms/gap_ms pair"
                        .to_owned(),
                ));
            }
            (true, false) => {
                let (Some(press_ms), Some(gap_ms)) = (self.press_ms, self.gap_ms) else {
                    return Err(ConfigError::Invalid(
                        "press_ms and gap_ms must both be set; one is missing".to_owned(),
                    ));
                };
                (press_ms, gap_ms)
            }
            (false, true) => {
                let (Some(colony_press_ms), Some(colony_gap_ms)) =
                    (self.colony_press_ms, self.colony_gap_ms)
                else {
                    return Err(ConfigError::Invalid(
                        "per-macro timing is incomplete: colony_press_ms, colony_gap_ms, \
                         spire_press_ms and spire_gap_ms must all be set"
                            .to_owned(),
                    ));
                };
                let (Some(spire_press_ms), Some(spire_gap_ms)) =
                    (self.spire_press_ms, self.spire_gap_ms)
                else {
                    return Err(ConfigError::Invalid(
                        "per-macro timing is incomplete: colony_press_ms, colony_gap_ms, \
                         spire_press_ms and spire_gap_ms must all be set"
                            .to_owned(),
                    ));
                };
                if colony_press_ms != spire_press_ms || colony_gap_ms != spire_gap_ms {
                    return Err(ConfigError::Invalid(format!(
                        "the per-macro timing differs (colony {colony_press_ms}/{colony_gap_ms} ms, \
                         spire {spire_press_ms}/{spire_gap_ms} ms) but this version has one shared \
                         timing; set press_ms = {colony_press_ms} and gap_ms = {colony_gap_ms} \
                         explicitly, or delete the per-macro keys"
                    )));
                }
                (colony_press_ms, colony_gap_ms)
            }
            (false, false) => {
                return Err(ConfigError::Invalid(
                    "no timing values found: set press_ms and gap_ms".to_owned(),
                ));
            }
        };

        let trigger = self
            .trigger_hotkey
            .or(self.colony_hotkey)
            .or(self.spire_hotkey)
            .unwrap_or(DEFAULT_TRIGGER_HOTKEY);
        let action = self
            .spire_action_hotkey
            .unwrap_or_else(|| default_spire_action_hotkey(trigger));
        let vacant = self
            .vacant_colony_hotkey
            .unwrap_or_else(|| default_vacant_colony_hotkey(trigger, action));
        let stargate = self
            .stargate_action_hotkey
            .unwrap_or_else(|| default_stargate_action_hotkey(trigger, action, vacant));
        Ok(Config {
            trigger_hotkey: trigger,
            spire_action_hotkey: action,
            stargate_action_hotkey: stargate,
            stargate_recall_f2: self.stargate_recall_f2.unwrap_or(false),
            vacant_colony_hotkey: vacant,
            build_target: self.build_target.unwrap_or_default(),
            colony_row_mode: self.colony_row_mode.unwrap_or_default(),
            force_build: self.force_build.unwrap_or(true),
            press_ms,
            gap_ms,
            target_process: self.target_process,
        })
    }
}

fn validate_interval(field: &str, value: u32) -> Result<(), ConfigError> {
    if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&value) {
        return Err(ConfigError::Invalid(format!(
            "{field} must be between {MIN_INTERVAL_MS} and {MAX_INTERVAL_MS}, got {value}"
        )));
    }
    Ok(())
}

/// The target must be a bare exe basename: a window title substring is not
/// accepted, because "any window that mentions StarCraft" is not a safety
/// boundary.
fn validate_target_process(target: &str) -> Result<(), ConfigError> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::Invalid("target_process is empty".to_owned()));
    }
    if trimmed.contains(['\\', '/', ':']) {
        return Err(ConfigError::Invalid(format!(
            "target_process must be an exe file name without a path, got '{trimmed}'"
        )));
    }
    if !trimmed.to_ascii_lowercase().ends_with(".exe") {
        return Err(ConfigError::Invalid(format!(
            "target_process must end with .exe, got '{trimmed}'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::HotkeySlot;
    use crate::test_support::temp_path;

    /// Current schema. It has no `build_target` and no `colony_row_mode`, so
    /// the same text also covers files written before those keys existed.
    const SAMPLE: &str = r#"
trigger_hotkey = "F6"
press_ms = 50
gap_ms = 50
target_process = "StarCraft.exe"
"#;

    /// Oldest file: the shared press/gap pair, nothing else.
    const LEGACY: &str = r#"
trigger_hotkey = "F6"
press_ms = 80
gap_ms = 30
target_process = "StarCraft.exe"
"#;

    /// Previous schema: one pair per macro, and both pairs are equal.
    const PER_MACRO_EQUAL: &str = r#"
trigger_hotkey = "F6"
colony_press_ms = 80
colony_gap_ms = 30
spire_press_ms = 80
spire_gap_ms = 30
target_process = "StarCraft.exe"
"#;

    /// Previous schema with two different pairs: no longer representable.
    const PER_MACRO_DIFFERENT: &str = r#"
trigger_hotkey = "F6"
colony_press_ms = 40
colony_gap_ms = 70
spire_press_ms = 90
spire_gap_ms = 15
target_process = "StarCraft.exe"
"#;

    /// The exact key set `to_toml` is required to write, in field order.
    ///
    /// `spire_scan_only` is deliberately absent: the key is read from older
    /// files but never written again, so a removed feature cannot come back
    /// through a saved file.
    const WRITTEN_KEYS: [&str; 11] = [
        "trigger_hotkey",
        "spire_action_hotkey",
        "stargate_action_hotkey",
        "stargate_recall_f2",
        "vacant_colony_hotkey",
        "build_target",
        "colony_row_mode",
        "force_build",
        "press_ms",
        "gap_ms",
        "target_process",
    ];

    fn written_keys(text: &str) -> Vec<&str> {
        text.lines()
            .filter_map(|line| line.split('=').next().map(str::trim))
            .filter(|key| !key.is_empty())
            .collect()
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let config = Config::default();
        assert_eq!(config.trigger_hotkey, HotkeyKey::F7);
        assert_eq!(config.spire_action_hotkey, HotkeyKey::Tab);
        assert_eq!(config.stargate_action_hotkey, HotkeyKey::Tilde);
        assert_eq!(config.vacant_colony_hotkey, HotkeyKey::F6);
        assert_eq!(config.build_target, BuildTarget::Colony);
        assert_eq!(config.colony_row_mode, RowMode::LeftToRight);
        assert!(config.force_build, "the forced mode is the default");
        assert_eq!(config.press_ms, 20);
        assert_eq!(config.gap_ms, 20);
        assert_eq!(config.target_process, "StarCraft.exe");
        assert_eq!(config.validate(), Ok(()));
        assert_eq!(config.timing(), Timing::from_millis(20, 20));
    }

    #[test]
    fn a_previously_saved_timing_keeps_its_value_instead_of_the_new_default() {
        // The default moved from 50 ms to 20 ms; a file that already stores
        // 50/50 must keep it until the user edits or resets it.
        let config = Config::parse(SAMPLE).expect("a saved pair still loads");
        assert_eq!((config.press_ms, config.gap_ms), (50, 50));
        let text = config.to_toml().expect("serialize");
        assert!(text.contains("press_ms = 50"), "{text}");
        assert!(text.contains("gap_ms = 50"), "{text}");
    }

    #[test]
    fn the_shared_pair_is_the_one_the_gui_edits() {
        let mut config = Config::default();
        {
            let (press_ms, gap_ms) = config.intervals_mut();
            *press_ms = 40;
            *gap_ms = 70;
        }
        assert_eq!(config.press_ms, 40);
        assert_eq!(config.gap_ms, 70);
        assert_eq!(config.timing(), Timing::from_millis(40, 70));
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn to_toml_writes_exactly_the_documented_keys() {
        let text = Config::default().to_toml().expect("serialize");
        assert_eq!(written_keys(&text), WRITTEN_KEYS.to_vec(), "{text}");
        assert!(text.contains("\"colony\""), "{text}");
        assert!(text.contains("\"left_to_right\""), "{text}");
    }

    #[test]
    fn toml_round_trip_preserves_every_field() {
        let config = Config {
            trigger_hotkey: HotkeyKey::F11,
            spire_action_hotkey: HotkeyKey::F12,
            stargate_action_hotkey: HotkeyKey::F10,
            vacant_colony_hotkey: HotkeyKey::F9,
            stargate_recall_f2: true,
            build_target: BuildTarget::Spire,
            colony_row_mode: RowMode::EndsInward,
            force_build: false,
            press_ms: 35,
            gap_ms: 80,
            target_process: "StarCraft.exe".to_owned(),
        };
        let text = config.to_toml().expect("serialize");
        assert_eq!(written_keys(&text), WRITTEN_KEYS.to_vec(), "{text}");
        assert_eq!(Config::parse(&text).expect("parse"), config);
    }

    #[test]
    fn the_build_target_is_written_and_read_back_as_a_stable_string() {
        let default_text = Config::default().to_toml().expect("serialize");
        assert!(
            default_text.contains("build_target = \"colony\""),
            "{default_text}"
        );

        let spire = Config {
            build_target: BuildTarget::Spire,
            ..Config::default()
        };
        let text = spire.to_toml().expect("serialize");
        assert!(text.contains("build_target = \"spire\""), "{text}");
        assert_eq!(Config::parse(&text).expect("parse"), spire);
    }

    #[test]
    fn the_row_mode_is_written_and_read_back_as_a_stable_string() {
        let default_text = Config::default().to_toml().expect("serialize");
        assert!(
            default_text.contains("colony_row_mode = \"left_to_right\""),
            "{default_text}"
        );

        let ends_inward = Config {
            colony_row_mode: RowMode::EndsInward,
            ..Config::default()
        };
        let text = ends_inward.to_toml().expect("serialize");
        assert!(text.contains("colony_row_mode = \"ends_inward\""), "{text}");
        assert_eq!(Config::parse(&text).expect("parse"), ends_inward);
    }

    #[test]
    fn a_file_without_the_row_mode_key_loads_as_left_to_right() {
        assert!(!SAMPLE.contains("colony_row_mode"), "{SAMPLE}");
        let config = Config::parse(SAMPLE).expect("files written before the mode must still load");
        assert_eq!(config.colony_row_mode, RowMode::LeftToRight);
    }

    #[test]
    fn a_file_without_the_build_target_key_loads_as_a_colony() {
        assert!(!SAMPLE.contains("build_target"), "{SAMPLE}");
        assert!(!LEGACY.contains("build_target"), "{LEGACY}");
        for text in [SAMPLE, LEGACY] {
            let config = Config::parse(text).expect("files written before the target must load");
            assert_eq!(config.build_target, BuildTarget::Colony);
        }
    }

    #[test]
    fn an_unknown_row_mode_is_rejected_instead_of_guessed() {
        let text = format!("{SAMPLE}colony_row_mode = \"middle_out\"\n");
        match Config::parse(&text) {
            Err(ConfigError::Parse(detail)) => {
                assert!(detail.contains("middle_out"), "{detail}")
            }
            other => panic!("expected a parse error, got {other:?}"),
        }

        let path = temp_path("row-mode.toml");
        std::fs::write(&path, text).expect("write");
        let loaded = Config::load(&path);
        assert_eq!(loaded.config, Config::default());
        assert!(matches!(loaded.warning, Some(ConfigWarning::Parse { .. })));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unknown_build_target_is_rejected_instead_of_guessed() {
        let text = format!("{SAMPLE}build_target = \"hatchery\"\n");
        match Config::parse(&text) {
            Err(ConfigError::Parse(detail)) => {
                assert!(detail.contains("hatchery"), "{detail}");
                assert!(detail.contains("colony"), "{detail}");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }

        let path = temp_path("build-target.toml");
        std::fs::write(&path, text).expect("write");
        let loaded = Config::load(&path);
        assert_eq!(loaded.config, Config::default());
        assert!(matches!(loaded.warning, Some(ConfigWarning::Parse { .. })));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_fields_are_rejected_so_typos_are_visible() {
        let text = SAMPLE.replace("gap_ms", "gapp_ms");
        let error = Config::parse(&text).expect_err("typo must be rejected");
        assert!(matches!(error, ConfigError::Parse(_)), "got {error:?}");
    }

    #[test]
    fn out_of_range_timing_is_rejected_for_the_shared_pair() {
        for (field, config) in [
            (
                "press_ms",
                Config {
                    press_ms: MAX_INTERVAL_MS + 1,
                    ..Config::default()
                },
            ),
            (
                "gap_ms",
                Config {
                    gap_ms: 0,
                    ..Config::default()
                },
            ),
        ] {
            match config.validate() {
                Err(ConfigError::Invalid(detail)) => {
                    assert!(detail.contains(field), "expected {field} in '{detail}'")
                }
                other => panic!("{field} should be rejected, got {other:?}"),
            }
        }

        // Both bounds are inclusive.
        let at_the_edges = Config {
            press_ms: MAX_INTERVAL_MS,
            gap_ms: MIN_INTERVAL_MS,
            ..Config::default()
        };
        assert_eq!(at_the_edges.validate(), Ok(()));
    }

    #[test]
    fn the_emergency_key_cannot_be_reused() {
        let emergency = HotkeyKey::EMERGENCY;
        let config = Config {
            trigger_hotkey: emergency,
            ..Config::default()
        };
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn target_process_must_be_an_exe_basename() {
        for bad in [
            "",
            "   ",
            "StarCraft",
            "C:\\Games\\StarCraft.exe",
            "StarCraft.exe*",
        ] {
            let config = Config {
                target_process: bad.to_owned(),
                ..Config::default()
            };
            assert!(
                matches!(config.validate(), Err(ConfigError::Invalid(_))),
                "expected '{bad}' to be rejected"
            );
        }
        let config = Config {
            target_process: "starcraft.exe".to_owned(),
            ..Config::default()
        };
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn a_missing_file_loads_defaults_without_a_warning() {
        let path = temp_path("missing.toml");
        let _ = std::fs::remove_file(&path);
        let loaded = Config::load(&path);
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.warning, None);
    }

    #[test]
    fn an_invalid_file_falls_back_to_defaults_with_a_visible_warning() {
        let path = temp_path("invalid.toml");
        std::fs::write(&path, "press_ms = 99999\n").expect("write");
        let loaded = Config::load(&path);
        // Defaults are usable ...
        assert_eq!(loaded.config, Config::default());
        // ... and the problem is reported, not swallowed.
        assert!(matches!(loaded.warning, Some(ConfigWarning::Parse { .. })));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_out_of_range_file_falls_back_to_defaults_with_a_visible_warning() {
        let path = temp_path("range.toml");
        let text = SAMPLE.replace("gap_ms = 50", "gap_ms = 0");
        std::fs::write(&path, text).expect("write");
        let loaded = Config::load(&path);
        assert_eq!(loaded.config, Config::default());
        match loaded.warning {
            Some(ConfigWarning::Invalid { detail }) => {
                assert!(detail.contains("gap_ms"), "{detail}")
            }
            other => panic!("unexpected warning: {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_shared_timing_file_loads_into_the_shared_pair() {
        let config = Config::parse(LEGACY).expect("the shared schema is the current one");
        assert_eq!(config.timing(), Timing::from_millis(80, 30));
        assert_eq!(config.trigger_hotkey, HotkeyKey::F6);
        assert_eq!(config.target_process, "StarCraft.exe");
    }

    #[test]
    fn equal_per_macro_timing_migrates_to_the_shared_pair() {
        let config = Config::parse(PER_MACRO_EQUAL).expect("equal pairs must still load");
        assert_eq!(config.press_ms, 80);
        assert_eq!(config.gap_ms, 30);
        assert_eq!(config.timing(), Timing::from_millis(80, 30));
        assert_eq!(config.build_target, BuildTarget::Colony);
    }

    #[test]
    fn differing_per_macro_timing_is_rejected_instead_of_picking_one() {
        match Config::parse(PER_MACRO_DIFFERENT) {
            Err(ConfigError::Invalid(detail)) => {
                assert!(detail.contains("colony 40/70"), "{detail}");
                assert!(detail.contains("spire 90/15"), "{detail}");
            }
            other => panic!("expected an ambiguous-timing error, got {other:?}"),
        }

        let path = temp_path("per-macro-different.toml");
        std::fs::write(&path, PER_MACRO_DIFFERENT).expect("write");
        let loaded = Config::load(&path);
        assert_eq!(loaded.config, Config::default(), "defaults, never a guess");
        assert!(matches!(
            loaded.warning,
            Some(ConfigWarning::Invalid { .. })
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mixing_shared_and_per_macro_timing_is_rejected_as_ambiguous() {
        let mixed = format!("{LEGACY}colony_press_ms = 10\n");
        match Config::parse(&mixed) {
            Err(ConfigError::Invalid(detail)) => {
                assert!(detail.contains("press_ms"), "{detail}")
            }
            other => panic!("expected an ambiguity error, got {other:?}"),
        }
    }

    #[test]
    fn partially_filled_per_macro_timing_is_rejected() {
        let text = PER_MACRO_EQUAL.replace("spire_gap_ms = 30\n", "");
        match Config::parse(&text) {
            Err(ConfigError::Invalid(detail)) => {
                assert!(detail.contains("incomplete"), "{detail}")
            }
            other => panic!("expected an incomplete-schema error, got {other:?}"),
        }

        // Half of the shared pair is just as unusable.
        let lone_press = LEGACY.replace("gap_ms = 30\n", "");
        match Config::parse(&lone_press) {
            Err(ConfigError::Invalid(detail)) => {
                assert!(detail.contains("gap_ms"), "{detail}")
            }
            other => panic!("expected an incomplete-shared error, got {other:?}"),
        }
    }

    #[test]
    fn a_file_without_any_timing_is_rejected() {
        let text = SAMPLE
            .lines()
            .filter(|line| !line.contains("_ms ="))
            .collect::<Vec<_>>()
            .join("\n");
        match Config::parse(&text) {
            Err(ConfigError::Invalid(detail)) => {
                assert!(detail.contains("no timing"), "{detail}")
            }
            other => panic!("expected a missing-timing error, got {other:?}"),
        }
    }

    #[test]
    fn a_migrated_file_is_saved_in_the_new_schema() {
        let path = temp_path("migrated.toml");
        let config = Config::parse(PER_MACRO_EQUAL).expect("equal pairs must still load");
        config.save(&path).expect("save");
        let text = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(written_keys(&text), WRITTEN_KEYS.to_vec(), "{text}");
        assert!(text.contains("press_ms = 80"), "{text}");
        assert!(text.contains("gap_ms = 30"), "{text}");
        // The per-macro keys must not be written again.
        assert!(!text.contains("colony_press_ms"), "{text}");
        assert!(!text.contains("spire_gap_ms"), "{text}");
        let loaded = Config::load(&path);
        assert_eq!(loaded.config, config);
        assert_eq!(loaded.warning, None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_then_load_round_trips_and_creates_directories() {
        let path = temp_path("nested").join("config.toml");
        let config = Config {
            build_target: BuildTarget::Spire,
            press_ms: 120,
            gap_ms: 30,
            ..Config::default()
        };
        config.save(&path).expect("save");
        let loaded = Config::load(&path);
        assert_eq!(loaded.config, config);
        assert_eq!(loaded.warning, None);
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn saving_refuses_invalid_values() {
        let path = temp_path("refuse.toml");
        let config = Config {
            gap_ms: MAX_INTERVAL_MS + 5,
            ..Config::default()
        };
        assert!(config.save(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn the_default_keys_are_f7_tab_f6_and_tilde_with_f8_reserved() {
        let config = Config::default();
        assert_eq!(config.trigger_hotkey, HotkeyKey::F7);
        assert_eq!(config.spire_action_hotkey, HotkeyKey::Tab);
        assert_eq!(config.bindings().get(HotkeySlot::Trigger), HotkeyKey::F7);
        assert_eq!(
            config.bindings().get(HotkeySlot::SpireAction),
            HotkeyKey::Tab
        );
        assert_eq!(
            config.bindings().get(HotkeySlot::VacantColony),
            HotkeyKey::F6
        );
        assert_eq!(
            config.bindings().get(HotkeySlot::StargateAction),
            HotkeyKey::Tilde
        );
        assert_eq!(config.bindings().emergency, HotkeyKey::F8);
        assert!(config.force_build, "the forced mode is the default");
    }

    #[test]
    fn a_file_without_the_spire_action_key_gets_the_default_action_key() {
        assert!(!SAMPLE.contains("spire_action_hotkey"), "{SAMPLE}");
        let config = Config::parse(SAMPLE).expect("an older file must still load");
        assert_eq!(config.spire_action_hotkey, HotkeyKey::Tab);
    }

    #[test]
    fn a_legacy_scan_only_key_is_ignored_instead_of_resetting_anything() {
        // Files written while the removed preview-only mode existed store the
        // key with either value. Both must load, must not change any other
        // setting, and must not keep the action key from running the action.
        for value in ["true", "false"] {
            let text = format!("{SAMPLE}spire_scan_only = {value}\n");
            let config = Config::parse(&text).unwrap_or_else(|error| {
                panic!("spire_scan_only = {value} must still load: {error}")
            });
            assert_eq!(config.trigger_hotkey, HotkeyKey::F6);
            assert_eq!(config.spire_action_hotkey, HotkeyKey::Tab);
            assert_eq!((config.press_ms, config.gap_ms), (50, 50));
            assert_eq!(config.build_target, BuildTarget::Colony);
            assert_eq!(config.colony_row_mode, RowMode::LeftToRight);
            assert!(config.force_build);
            assert_eq!(config.target_process, "StarCraft.exe");
            assert_eq!(config.validate(), Ok(()));
            // No preview-only state survives the load, so nothing can branch
            // on the stored value any more.
            assert_eq!(
                config,
                Config::parse(SAMPLE).expect("the same file without the key")
            );
        }
    }

    #[test]
    fn the_removed_scan_only_key_is_never_written_again() {
        assert!(
            !WRITTEN_KEYS.contains(&"spire_scan_only"),
            "{WRITTEN_KEYS:?}"
        );
        let legacy = Config::parse(&format!("{SAMPLE}spire_scan_only = true\n")).expect("load");
        let saved = legacy.to_toml().expect("serialize");
        assert_eq!(written_keys(&saved), WRITTEN_KEYS.to_vec(), "{saved}");
        assert!(!saved.contains("spire_scan_only"), "{saved}");
        // Saving and loading the rewritten text keeps every remaining value.
        let reloaded = Config::parse(&saved).expect("parse the rewritten file");
        assert_eq!(reloaded, legacy);
        assert_eq!((reloaded.press_ms, reloaded.gap_ms), (50, 50));
        assert_eq!(reloaded.spire_action_hotkey, HotkeyKey::Tab);
    }

    #[test]
    fn a_file_without_the_stargate_action_key_gets_a_free_default() {
        // The current default is Tilde. `SAMPLE` leaves Tilde free, so an
        // older file receives the normal default without becoming invalid.
        assert!(!SAMPLE.contains("stargate_action_hotkey"), "{SAMPLE}");
        let config = Config::parse(SAMPLE).expect("an older file must still load");
        assert_eq!(config.stargate_action_hotkey, HotkeyKey::Tilde);
        assert_eq!(config.validate(), Ok(()));

        assert_eq!(Config::default().stargate_action_hotkey, HotkeyKey::Tilde);
        // An explicit key always wins.
        let text = format!("{SAMPLE}stargate_action_hotkey = \"F11\"\n");
        assert_eq!(
            Config::parse(&text)
                .expect("explicit key")
                .stargate_action_hotkey,
            HotkeyKey::F11
        );
    }

    #[test]
    fn the_stargate_key_must_be_distinct_from_every_other_binding() {
        for key in [
            HotkeyKey::F7,
            HotkeyKey::Tab,
            HotkeyKey::F6,
            HotkeyKey::EMERGENCY,
        ] {
            let config = Config {
                stargate_action_hotkey: key,
                ..Config::default()
            };
            let error = config.validate().expect_err("duplicate must be rejected");
            assert!(
                error.to_string().contains("stargate_action_hotkey"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_row_trigger_on_f7_keeps_the_file_valid_with_the_default_action_key() {
        // F7 used to be a valid row binding (the legacy `spire_hotkey`), so a
        // previously valid file must keep loading; the absent action key gets
        // the normal default rather than failing validation.
        let text = SAMPLE.replace("trigger_hotkey = \"F6\"\n", "trigger_hotkey = \"F7\"\n");
        let config = Config::parse(&text).expect("previously valid file");
        assert_eq!(config.trigger_hotkey, HotkeyKey::F7);
        assert_eq!(config.spire_action_hotkey, HotkeyKey::Tab);
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn the_third_key_defaults_to_f6_and_moves_out_of_the_way() {
        // The default never lands on F5: StarCraft 1 uses it itself.
        assert_eq!(DEFAULT_VACANT_COLONY_HOTKEY, HotkeyKey::F6);
        assert_ne!(DEFAULT_VACANT_COLONY_HOTKEY, HotkeyKey::F5);

        // An older file without the key gets the free default. `SAMPLE` already
        // binds the row trigger to F6, so the third key moves to the next free
        // F-key instead of making the file invalid.
        let config = Config::parse(SAMPLE).expect("older file");
        assert_eq!(config.trigger_hotkey, HotkeyKey::F6);
        assert_eq!(config.vacant_colony_hotkey, HotkeyKey::F7);
        assert_eq!(config.validate(), Ok(()));

        // Both F6 and F7 taken: the chain continues to F9, which is also free
        // in the game.
        let text = SAMPLE.replace(
            "trigger_hotkey = \"F6\"\n",
            "trigger_hotkey = \"F6\"\nspire_action_hotkey = \"F7\"\n",
        );
        let config = Config::parse(&text).expect("previously valid file");
        assert_eq!(config.vacant_colony_hotkey, HotkeyKey::F9);
        assert_eq!(config.validate(), Ok(()));

        // The defaults (Tilde/Tab) leave F6 for the third key.
        assert_eq!(Config::default().vacant_colony_hotkey, HotkeyKey::F6);

        // A file that already names the key keeps exactly that key.
        let text = format!("{SAMPLE}vacant_colony_hotkey = \"F11\"\n");
        assert_eq!(
            Config::parse(&text)
                .expect("explicit key")
                .vacant_colony_hotkey,
            HotkeyKey::F11
        );
    }

    #[test]
    fn the_game_s_f4_view_key_can_never_be_bound() {
        // F4 belongs to the game's saved camera view; every binding rejects it.
        for config in [
            Config {
                trigger_hotkey: HotkeyKey::F4,
                ..Config::default()
            },
            Config {
                spire_action_hotkey: HotkeyKey::F4,
                ..Config::default()
            },
            Config {
                vacant_colony_hotkey: HotkeyKey::F4,
                ..Config::default()
            },
            Config {
                stargate_action_hotkey: HotkeyKey::F4,
                ..Config::default()
            },
        ] {
            let error = config.validate().expect_err("F4 must be rejected");
            assert!(error.to_string().contains("F4"), "{error}");
        }
        // F4 is still offered by the GUI's key list so the rejection is
        // reachable and visible, not silently impossible.
        assert!(HotkeyKey::ALL.contains(&HotkeyKey::F4));
    }

    #[test]
    fn the_third_key_must_differ_from_the_other_two_and_from_f8() {
        for key in [HotkeyKey::Tilde, HotkeyKey::Tab, HotkeyKey::EMERGENCY] {
            let config = Config {
                trigger_hotkey: HotkeyKey::Tilde,
                spire_action_hotkey: HotkeyKey::Tab,
                vacant_colony_hotkey: key,
                ..Config::default()
            };
            let error = config.validate().expect_err("duplicate must be rejected");
            assert!(
                error.to_string().contains("vacant_colony_hotkey"),
                "{error}"
            );
        }
    }

    #[test]
    fn the_third_key_is_written_and_read_back() {
        let text = Config::default().to_toml().expect("serialize");
        assert!(text.contains("vacant_colony_hotkey = \"F6\""), "{text}");
        assert_eq!(written_keys(&text), WRITTEN_KEYS.to_vec(), "{text}");
    }

    #[test]
    fn an_explicit_duplicate_action_key_is_rejected_with_a_clear_message() {
        let text = format!("{SAMPLE}spire_action_hotkey = \"F6\"\n");
        let error = Config::parse(&text).expect_err("duplicate must fail");
        match error {
            ConfigError::Invalid(detail) => {
                assert!(detail.contains("spire_action_hotkey"), "{detail}");
                assert!(detail.contains("F6"), "{detail}");
            }
            other => panic!("expected an invalid-settings error, got {other:?}"),
        }
    }

    #[test]
    fn the_spire_action_key_cannot_be_the_emergency_key() {
        let config = Config {
            spire_action_hotkey: HotkeyKey::EMERGENCY,
            ..Config::default()
        };
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn the_legacy_spire_key_still_migrates_to_the_row_trigger() {
        // The old `spire_hotkey` named the *row* macro, so it must not be
        // repurposed as the new action key.
        let text = SAMPLE.replace(
            "trigger_hotkey = \"F6\"\n",
            "colony_hotkey = \"F5\"\nspire_hotkey = \"F7\"\n",
        );
        let config = Config::parse(&text).expect("legacy file");
        assert_eq!(config.trigger_hotkey, HotkeyKey::F5);
        assert_eq!(config.spire_action_hotkey, HotkeyKey::Tab);
    }

    #[test]
    fn the_trigger_key_migrates_from_the_old_two_hotkey_files() {
        // Old files named the two row hotkeys separately; the colony key wins
        // and the file keeps loading.
        let two_keys = SAMPLE.replace(
            "trigger_hotkey = \"F6\"\n",
            "colony_hotkey = \"F11\"\nspire_hotkey = \"F12\"\n",
        );
        assert_eq!(
            Config::parse(&two_keys).expect("parse").trigger_hotkey,
            HotkeyKey::F11
        );
        // A file with no hotkey key at all still loads with the default.
        let none = SAMPLE.replace("trigger_hotkey = \"F6\"\n", "");
        assert_eq!(
            Config::parse(&none).expect("no hotkey key").trigger_hotkey,
            HotkeyKey::F7
        );
    }
}
