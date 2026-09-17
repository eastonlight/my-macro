//! Public-API smoke tests.
//!
//! The detailed behaviour tests live next to the code they cover (`src/*.rs`).
//! This file exercises the same guarantees through the exported surface only,
//! the way an embedder would see them.

use std::time::Duration;

use oh_my_macro::colony::{RowError, RowMode, plan_row};
use oh_my_macro::config::{Config, ConfigError, MAX_INTERVAL_MS, MIN_INTERVAL_MS};
use oh_my_macro::frame::Point;
use oh_my_macro::hotkey::{Bindings, HotkeyKey, HotkeySlot};
use oh_my_macro::macros::{self, BuildTarget, Key, MacroId, Primitive, Timing};
use oh_my_macro::runner::{HotkeyAction, action_for};

#[test]
fn creep_colony_is_b_c_then_a_click_at_the_current_mouse_position() {
    let primitives: Vec<_> = macros::plan(MacroId::CreepColony, Timing::from_millis(50, 50))
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
fn spire_is_v_s_then_a_click_at_the_current_mouse_position() {
    let primitives: Vec<_> = macros::plan(MacroId::Spire, Timing::from_millis(50, 50))
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
fn the_default_configuration_is_f6_with_20ms_intervals_for_both() {
    let config = Config::default();
    assert_eq!(config.trigger_hotkey, HotkeyKey::F6);
    assert_eq!(config.build_target, BuildTarget::Colony);
    assert_eq!(config.press_ms, 20);
    assert_eq!(config.gap_ms, 20);
    assert_eq!(config.timing(), Timing::from_millis(20, 20));
    assert_eq!(config.validate(), Ok(()));

    let bindings = Bindings::new(config.trigger_hotkey, config.spire_action_hotkey);
    assert_eq!(bindings.get(HotkeySlot::Trigger), HotkeyKey::F6);
    assert_eq!(bindings.get(HotkeySlot::SpireAction), HotkeyKey::F7);
    assert_eq!(bindings.get(HotkeySlot::Emergency), HotkeyKey::F8);
}

#[test]
fn both_trigger_hotkeys_route_to_the_same_row_build() {
    // The GUI routes hotkeys through this pure rule, so the two trigger keys
    // can never drift into two different macros again.
    assert_eq!(action_for(HotkeySlot::Trigger), HotkeyAction::StartRowBuild);
    assert_eq!(
        action_for(HotkeySlot::Emergency),
        HotkeyAction::EmergencyStop
    );
}

#[test]
fn both_hotkeys_share_one_timing_pair() {
    let mut config = Config {
        press_ms: 40,
        gap_ms: 70,
        ..Config::default()
    };
    config.validate().expect("valid settings");
    assert_eq!(config.timing(), Timing::from_millis(40, 70));

    let (press_ms, gap_ms) = config.intervals_mut();
    *press_ms = 90;
    *gap_ms = 20;
    assert_eq!(config.timing(), Timing::from_millis(90, 20));

    let delays: Vec<_> = macros::plan(MacroId::CreepColony, config.timing())
        .into_iter()
        .map(|step| step.delay_after)
        .collect();
    assert_eq!(
        delays,
        vec![
            Duration::from_millis(90),
            Duration::from_millis(20),
            Duration::from_millis(90),
            Duration::from_millis(20),
            Duration::from_millis(90),
            Duration::from_millis(20),
        ]
    );
}

#[test]
fn the_build_target_picks_the_keys_and_the_row_spacing() {
    assert_eq!(BuildTarget::Colony.build_keys(), [Key::B, Key::C]);
    assert_eq!(BuildTarget::Spire.build_keys(), [Key::V, Key::S]);
    assert_eq!(BuildTarget::Colony.footprint_px(), 144);
    assert_eq!(BuildTarget::Spire.footprint_px(), 216);

    let anchor = Point::new(400, 400);
    let colony = plan_row(anchor, 3, RowMode::LeftToRight, BuildTarget::Colony).expect("fits");
    let spire = plan_row(anchor, 3, RowMode::LeftToRight, BuildTarget::Spire).expect("fits");

    assert_eq!(colony.targets[1], Point::new(544, 400));
    assert_eq!(spire.targets[1], Point::new(616, 400));
    // A wider footprint fits fewer buildings in the same playable width.
    assert!(
        plan_row(
            Point::new(160, 400),
            12,
            RowMode::LeftToRight,
            BuildTarget::Colony
        )
        .is_ok()
    );
    assert_eq!(
        plan_row(
            Point::new(200, 400),
            9,
            RowMode::LeftToRight,
            BuildTarget::Spire
        ),
        Err(RowError::RowDoesNotFit { count: 9, fits: 8 })
    );
}

#[test]
fn a_shared_timing_file_survives_the_round_trip() {
    let shared = r#"
trigger_hotkey = "F6"
press_ms = 80
gap_ms = 30
target_process = "StarCraft.exe"
"#;
    let config = Config::parse(shared).expect("the shared pair is the current schema");
    assert_eq!(config.timing(), Timing::from_millis(80, 30));
    assert_eq!(config.build_target, BuildTarget::Colony);
    assert_eq!(config.colony_row_mode, RowMode::LeftToRight);

    let text = config.to_toml().expect("serialize migrated settings");
    for key in [
        "trigger_hotkey",
        "trigger_hotkey",
        "build_target",
        "colony_row_mode",
        "press_ms",
        "gap_ms",
        "target_process",
    ] {
        assert!(text.contains(&format!("{key} =")), "missing {key}: {text}");
    }
    assert_eq!(Config::parse(&text).expect("reparse"), config);
}

#[test]
fn equal_per_macro_timing_migrates_but_different_pairs_are_rejected() {
    let equal = r#"
trigger_hotkey = "F6"
colony_press_ms = 80
colony_gap_ms = 30
spire_press_ms = 80
spire_gap_ms = 30
target_process = "StarCraft.exe"
"#;
    let config = Config::parse(equal).expect("equal per-macro pairs migrate");
    assert_eq!(config.timing(), Timing::from_millis(80, 30));

    let different = equal
        .replace("spire_press_ms = 80", "spire_press_ms = 90")
        .replace("spire_gap_ms = 30", "spire_gap_ms = 15");
    assert!(
        matches!(Config::parse(&different), Err(ConfigError::Invalid(_))),
        "two different pairs have no shared value; do not pick one"
    );
}

#[test]
fn mixing_shared_and_per_macro_timing_is_rejected() {
    let mixed = r#"
trigger_hotkey = "F6"
press_ms = 80
gap_ms = 30
colony_press_ms = 10
target_process = "StarCraft.exe"
"#;
    assert!(matches!(Config::parse(mixed), Err(ConfigError::Invalid(_))));
}

#[test]
fn interval_bounds_are_enforced_for_the_shared_pair() {
    let mut config = Config {
        press_ms: MIN_INTERVAL_MS,
        gap_ms: MAX_INTERVAL_MS,
        ..Config::default()
    };
    assert_eq!(config.validate(), Ok(()));

    config.press_ms = MAX_INTERVAL_MS + 1;
    assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));

    config.press_ms = MIN_INTERVAL_MS;
    config.gap_ms = 0;
    assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
}

#[test]
fn settings_round_trip_through_toml() {
    let config = Config {
        trigger_hotkey: HotkeyKey::F10,
        build_target: BuildTarget::Spire,
        colony_row_mode: RowMode::EndsInward,
        press_ms: 60,
        gap_ms: 40,
        ..Config::default()
    };
    let text = config.to_toml().expect("serialize settings");
    assert_eq!(Config::parse(&text).expect("parse settings"), config);
}
