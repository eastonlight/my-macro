//! Localized UI strings.
//!
//! Korean is the primary language. The GUI only uses [`Labels::korean`] when a
//! Hangul capable font was found (see [`crate::font`]); otherwise it switches to
//! [`Labels::english`] instead of rendering boxes.
//!
//! Engine/adapter detail strings stay English on purpose: they carry data from
//! the operating system (`SendInput` codes, process names) and are appended
//! after the localized framing.
//!
//! Both label sets describe the same states, so [`Labels::start_error`],
//! [`Labels::outcome`] and friends stay identical in structure and differ only
//! in wording and language.

use crate::colony::RowMode;
use crate::config::{ConfigError, ConfigWarning};
use crate::engine::{Outcome, RunReport};
use crate::frame::Point;
use crate::hotkey::{HotkeyError, HotkeySlot};
use crate::input::InputError;
use crate::macros::{BuildTarget, MacroId};
use crate::runner::StartError;
use crate::spire_action::{SpireActionOutcome, SpireActionReport, SpireScanError, SpireScanReport};
use crate::stargate_action::StargateActionReport;

/// UI language actually in use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lang {
    Korean,
    English,
}

/// How the GUI presents one result: neutral information, a success, or a
/// problem.
///
/// Kept here next to the strings so a status line can never *look* like a
/// success: a scan-only preview is information by definition, and a pass that
/// sent no `A` is never a green check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoticeLevel {
    Info,
    Ok,
    Err,
}

/// The compact, pre-localized result one Spire card shows until the next run.
///
/// Built on the GUI thread from a finished report: drawing stays trivial and
/// the wording is testable without a window. Only the newest result is kept —
/// there is no history or activity log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpirePreview {
    pub level: NoticeLevel,
    /// One line: what ran and what it did.
    pub headline: String,
    /// Detection positions of this run, if it produced any.
    pub positions: Option<String>,
    /// Capture/detection timings of this run.
    pub timings: Option<String>,
    /// Extra honesty lines (skipped targets, "A commands are not upgrades").
    pub notes: Vec<String>,
}

/// Level for one finished row build, mirroring [`Labels::outcome`].
pub fn run_notice_level(report: &RunReport) -> NoticeLevel {
    match report.outcome {
        Outcome::Completed => NoticeLevel::Ok,
        Outcome::Cancelled | Outcome::Aborted { .. } | Outcome::Failed { .. } => NoticeLevel::Err,
    }
}

/// Level for one finished Spire action.
///
/// `Ok` requires at least one verified `A` **and** no skipped target: a partial
/// or empty pass is reported as information, never as success.
pub fn spire_action_notice_level(report: &SpireActionReport) -> NoticeLevel {
    match &report.outcome {
        SpireActionOutcome::Completed if report.acted > 0 && report.skipped == 0 => NoticeLevel::Ok,
        SpireActionOutcome::Completed | SpireActionOutcome::Cancelled => NoticeLevel::Info,
        SpireActionOutcome::Aborted { .. } | SpireActionOutcome::Failed { .. } => NoticeLevel::Err,
    }
}

/// All user visible strings.
#[derive(Clone, Copy, Debug)]
pub struct Labels {
    pub lang: Lang,

    pub app_title: &'static str,
    pub status_heading: &'static str,
    pub status_armed: &'static str,
    pub status_disarmed: &'static str,
    pub status_running: &'static str,
    pub status_idle: &'static str,
    pub status_emergency: &'static str,
    pub status_not_armed: &'static str,
    pub arm_button: &'static str,
    pub disarm_button: &'static str,
    pub arm_hint: &'static str,

    pub hotkeys_heading: &'static str,
    pub colony_label: &'static str,
    pub spire_label: &'static str,
    pub emergency_label: &'static str,
    pub emergency_hint: &'static str,
    pub hotkeys_locked_hint: &'static str,

    pub timing_heading: &'static str,
    pub press_label: &'static str,
    pub gap_label: &'static str,
    pub timing_hint: &'static str,
    pub target_label: &'static str,
    pub target_hint: &'static str,
    pub interval_range_hint: &'static str,

    pub sequence_label: &'static str,
    pub trigger_label: &'static str,
    pub trigger_single_hint: &'static str,
    pub timing_text_hint: &'static str,
    pub force_build_checkbox: &'static str,
    pub force_build_hint: &'static str,
    pub unconfirmed_suffix: &'static str,
    pub mouse_click_label: &'static str,
    pub row_build_title: &'static str,
    pub build_spire_checkbox: &'static str,
    pub build_target_label: &'static str,
    pub colony_target_name: &'static str,
    pub spire_target_name: &'static str,
    pub row_build_hint: &'static str,
    pub row_mode_label: &'static str,
    pub row_mode_left_to_right: &'static str,
    pub row_mode_ends_inward: &'static str,
    pub row_mode_grid_6x2: &'static str,
    pub spire_action_title: &'static str,
    pub spire_action_hotkey_label: &'static str,
    pub spire_action_sequence_label: &'static str,
    /// The read-only first step of one action run, shown in the sequence row
    /// before the first click: exactly one full-screen search.
    pub spire_search_step_label: &'static str,
    pub spire_action_hint: &'static str,
    /// Caveat under the Spire card: what a detection and a confirmed selection
    /// panel do **not** prove (type only, not ownership or upgrade availability).
    pub spire_confirm_scope_note: &'static str,
    pub spire_preview_heading: &'static str,
    pub spire_preview_empty: &'static str,
    pub spire_detected_label: &'static str,
    pub spire_positions_label: &'static str,
    pub spire_capture_label: &'static str,
    pub spire_detect_label: &'static str,
    pub spire_roi_label: &'static str,
    pub spire_scan_label: &'static str,
    /// Tag of the retained read-only scan diagnostic (no longer reachable from
    /// the action card, which always runs the action).
    pub spire_scan_only_tag: &'static str,
    pub spire_scan_only_unsent: &'static str,
    pub spire_a_sent_label: &'static str,
    pub spire_skipped_label: &'static str,
    pub spire_a_not_upgrade: &'static str,
    pub spire_skipped_note: &'static str,
    pub spire_zero_note: &'static str,
    pub stargate_action_title: &'static str,
    pub stargate_action_hint: &'static str,
    pub stargate_confirm_scope_note: &'static str,
    pub stargate_recall_f2_checkbox: &'static str,
    pub stargate_recall_f2_hint: &'static str,
    pub stargate_skipped_note: &'static str,
    /// Label of the retained read-only scan diagnostic (see
    /// [`Labels::spire_scan_preview`]); the action card itself only ever shows
    /// [`Labels::status_running`], because the action key always runs the full
    /// action.
    pub status_scanning: &'static str,
    pub arm_hint_invalid: &'static str,
    pub advanced_heading: &'static str,

    pub config_heading: &'static str,
    pub config_path_label: &'static str,
    pub save_button: &'static str,
    pub reload_button: &'static str,
    pub reset_button: &'static str,
    pub saved_ok: &'static str,
    pub save_failed: &'static str,

    pub warn_read: &'static str,
    pub warn_parse: &'static str,
    pub warn_invalid: &'static str,

    pub hotkey_register_failed: &'static str,
    pub hotkey_listener_failed: &'static str,
    pub hotkey_slot_trigger: &'static str,
    pub hotkey_slot_spire_action: &'static str,
    pub hotkey_slot_stargate_action: &'static str,
    pub hotkey_slot_emergency: &'static str,

    pub diag_heading: &'static str,
    pub diag_foreground: &'static str,
    pub diag_target_ok: &'static str,
    pub diag_target_not: &'static str,
    pub diag_unknown: &'static str,

    pub outcome_completed: &'static str,
    pub outcome_cancelled: &'static str,
    pub outcome_aborted: &'static str,
    pub outcome_failed: &'static str,
    pub steps_word: &'static str,
    pub busy_note: &'static str,

    pub notes_heading: &'static str,
    pub vacant_colony_sequence_label: &'static str,
    pub vacant_colony_search_step_label: &'static str,
    pub vacant_colony_confirm_step_label: &'static str,
    pub vacant_colony_caveat: &'static str,
    pub note_select_drone: &'static str,
    pub note_chat: &'static str,
    pub note_online: &'static str,
    pub note_language_fallback: &'static str,
}

impl Labels {
    pub const fn korean() -> Self {
        Self {
            lang: Lang::Korean,
            app_title: "oh-my-macro",
            status_heading: "상태",
            status_armed: "사용 중",
            status_disarmed: "중지됨",
            status_running: "실행 중",
            status_idle: "대기",
            status_emergency: "정지 F8",
            status_not_armed: "중지 상태입니다",
            arm_button: "사용 시작",
            disarm_button: "사용 중지",
            arm_hint: "사용 시작 후 단축키가 동작합니다. F8로 언제든 정지할 수 있습니다.",

            hotkeys_heading: "단축키",
            colony_label: "크립 콜로니 (Creep Colony)",
            spire_label: "둥지탑 (Spire)",
            emergency_label: "정지",
            emergency_hint: "F8은 고정 정지 키입니다.",
            hotkeys_locked_hint: "사용 중에는 단축키를 바꿀 수 없습니다.",

            timing_heading: "입력 타이밍",
            press_label: "누름 (ms)",
            gap_label: "간격 (ms)",
            timing_hint: "기본 20ms입니다. 게임이 입력을 놓치면 50ms 전후로 늘려보세요.",
            target_label: "대상 프로세스 (exe 파일명)",
            target_hint: "창 제목이 아니라 실행 파일 이름입니다. 예: StarCraft.exe",
            interval_range_hint: "범위",

            sequence_label: "입력 순서",
            trigger_label: "트리거 단축키",
            trigger_single_hint: "이 단축키로 실행합니다. F8은 정지입니다.",
            timing_text_hint: "1~2000ms 숫자를 직접 입력하세요.",
            force_build_checkbox: "미리보기 확인 실패 시에도 강행",
            force_build_hint: "미리보기가 안 보여도 클릭을 계속합니다.",
            unconfirmed_suffix: "건은 미확인(강행)",
            mouse_click_label: "좌클릭",
            row_build_title: "드론 줄짓기 건설",
            build_spire_checkbox: "둥지탑(Spire) 건설 — 끄면 크립 콜로니",
            build_target_label: "건물",
            colony_target_name: "크립 콜로니 (2x2)",
            spire_target_name: "둥지탑 (2x2)",
            row_build_hint: "선택한 드론 2~12기로 커서 오른쪽에 한 줄 또는 6x2로 건설합니다. 그룹 9 사용 · 1920x1080 전용.",
            row_mode_label: "배치 순서",
            row_mode_left_to_right: "왼쪽→오른쪽",
            row_mode_ends_inward: "양끝→가운데",
            row_mode_grid_6x2: "6x2 (1~6 아래 / 7~12 위)",
            spire_action_title: "스파이어 감지",
            spire_action_hotkey_label: "단축키",
            spire_action_sequence_label: "동작",
            spire_search_step_label: "탐색",
            spire_action_hint: "화면의 스파이어를 클릭하고, 선택 확인 후에만 A를 누릅니다. 1920x1080 전용.",
            spire_confirm_scope_note: "소유권과 업그레이드 가능 여부는 확인하지 않습니다.",
            spire_preview_heading: "마지막 결과",
            spire_preview_empty: "아직 실행 결과가 없습니다.",
            spire_detected_label: "곳 감지",
            spire_positions_label: "감지 위치",
            spire_capture_label: "탐색용 캡처",
            spire_detect_label: "감지",
            spire_roi_label: "ROI 캡처",
            spire_scan_label: "스파이어 스캔",
            spire_scan_only_tag: "스캔만",
            spire_scan_only_unsent: "클릭·A를 보내지 않았습니다",
            spire_a_sent_label: "A 전송",
            spire_skipped_label: "건너뜀",
            spire_a_not_upgrade: "표시된 수는 완성된 업그레이드가 아니라 보낸 A 명령 수입니다.",
            spire_skipped_note: "선택 패널이 스파이어로 확인되지 않은 위치는 A를 보내지 않고 건너뛰었습니다.",
            spire_zero_note: "0곳은 없다는 증명이 아닙니다 — 게임 창이 전면에서 렌더링 중인지 확인하세요.",
            stargate_action_title: "스타게이트 감지",
            stargate_action_hint: "화면의 스타게이트를 클릭하고, 선택 확인 후에만 A를 누릅니다. 1920x1080 전용.",
            stargate_confirm_scope_note: "소유권과 업그레이드 가능 여부는 확인하지 않습니다.",
            stargate_recall_f2_checkbox: "실행 전에 F2 화면 호출",
            stargate_recall_f2_hint: "F2 화면으로 이동한 뒤 스타게이트를 탐색합니다.",
            stargate_skipped_note: "스타게이트로 확인되지 않으면 A를 보내지 않습니다.",
            status_scanning: "스캔 중",
            arm_hint_invalid: "설정을 확인하세요. 단축키가 겹치면 시작할 수 없습니다.",
            advanced_heading: "설정",

            config_heading: "설정 파일",
            config_path_label: "경로",
            save_button: "저장",
            reload_button: "다시 읽기",
            reset_button: "기본값으로",
            saved_ok: "설정을 저장했습니다.",
            save_failed: "설정 저장 실패",

            warn_read: "설정 파일을 읽을 수 없어 기본값을 사용합니다",
            warn_parse: "설정 파일이 올바르지 않아 기본값을 사용합니다",
            warn_invalid: "설정 값이 범위를 벗어나 기본값을 사용합니다",

            hotkey_register_failed: "단축키 등록 실패 — 다른 프로그램이 이미 사용 중일 수 있습니다",
            hotkey_listener_failed: "단축키 감시를 시작하지 못했습니다",
            hotkey_slot_trigger: "줄짓기 실행",
            hotkey_slot_spire_action: "스파이어 동작",
            hotkey_slot_stargate_action: "스타게이트 동작",
            hotkey_slot_emergency: "정지",

            diag_heading: "안전 점검",
            diag_foreground: "현재 전면 창 프로세스",
            diag_target_ok: "대상 확인됨 — 입력을 보낼 수 있습니다",
            diag_target_not: "대상 아님 — 입력을 보내지 않습니다",
            diag_unknown: "확인할 수 없음",

            outcome_completed: "완료",
            outcome_cancelled: "취소됨",
            outcome_aborted: "중단됨",
            outcome_failed: "오류",
            steps_word: "단계",
            busy_note: "이미 실행 중입니다. 새 입력은 대기열에 쌓지 않고 무시합니다.",

            notes_heading: "사용 전 확인",
            vacant_colony_sequence_label: "한 번 실행",
            vacant_colony_search_step_label: "빈자리 탐색·검증",
            vacant_colony_confirm_step_label: "변이 시작 확인",
            vacant_colony_caveat: "확인된 빈자리에만 건설합니다. 최대 탐색 60초.",
            note_select_drone: "줄짓기: 드론 2~12기를 선택하고 커서를 첫 위치에 둡니다. F6 기능은 게임의 F4 저장 화면을 사용합니다.",
            note_chat: "게임 채팅 입력 중에는 사용하지 마세요.",
            note_online: "온라인 경기에서는 이용 규정을 확인하세요.",
            note_language_fallback: "한글 글꼴을 찾지 못해 영어로 표시합니다.",
        }
    }

    pub const fn english() -> Self {
        Self {
            lang: Lang::English,
            app_title: "oh-my-macro",
            status_heading: "Status",
            status_armed: "Active",
            status_disarmed: "Stopped",
            status_running: "Running",
            status_idle: "Idle",
            status_emergency: "Stop F8",
            status_not_armed: "ignored: stopped",
            arm_button: "Start",
            disarm_button: "Stop",
            arm_hint: "Press Start to enable hotkeys. Press F8 anytime to stop.",

            hotkeys_heading: "Hotkeys",
            colony_label: "Creep Colony",
            spire_label: "Spire",
            emergency_label: "Stop",
            emergency_hint: "F8 is the fixed stop key.",
            hotkeys_locked_hint: "Hotkeys cannot be changed while active.",

            timing_heading: "Input timing",
            press_label: "Hold (ms)",
            gap_label: "Gap (ms)",
            timing_hint: "The 20 ms default is fast; increase toward 50 ms if the game misses input.",
            target_label: "Target process (exe basename)",
            target_hint: "Exe file name, not a window title, e.g. StarCraft.exe",
            interval_range_hint: "range",

            sequence_label: "Sequence",
            trigger_label: "Trigger Hotkey",
            trigger_single_hint: "This hotkey runs the macro. F8 stops it.",
            timing_text_hint: "Enter a number from 1 to 2000 ms.",
            force_build_checkbox: "Keep going without a confirmed preview",
            force_build_hint: "Continue clicking when the preview is not visible.",
            unconfirmed_suffix: "unconfirmed (forced)",
            mouse_click_label: "Click",
            row_build_title: "Drone row build",
            build_spire_checkbox: "Build a Spire (unchecked: Creep Colony)",
            build_target_label: "Building",
            colony_target_name: "Creep Colony (2x2)",
            spire_target_name: "Spire (2x2)",
            row_build_hint: "Builds a row or 6x2 grid to the right with 2-12 selected drones. Uses group 9 - 1920x1080 only.",
            row_mode_label: "Row order",
            row_mode_left_to_right: "Left to right",
            row_mode_ends_inward: "Ends inward",
            row_mode_grid_6x2: "6x2 (1-6 bottom / 7-12 top)",
            spire_action_title: "Spire detection",
            spire_action_hotkey_label: "Hotkey",
            spire_action_sequence_label: "Action",
            spire_search_step_label: "Scan",
            spire_action_hint: "Clicks visible Spires and presses A only after verifying the selection. 1920x1080 only.",
            spire_confirm_scope_note: "Ownership and upgrade availability are not checked.",
            spire_preview_heading: "Last result",
            spire_preview_empty: "No result yet.",
            spire_detected_label: "detected",
            spire_positions_label: "positions",
            spire_capture_label: "scan capture",
            spire_detect_label: "detect",
            spire_roi_label: "ROI capture",
            spire_scan_label: "spire scan",
            spire_scan_only_tag: "scan only",
            spire_scan_only_unsent: "no click or A was sent",
            spire_a_sent_label: "A sent",
            spire_skipped_label: "skipped",
            spire_a_not_upgrade: "The count is A commands sent, not completed upgrades.",
            spire_skipped_note: "Positions whose selection panel was not confirmed as the Spire were skipped without A.",
            spire_zero_note: "0 found is not proof that there is none - check that the game window is in front and rendering.",
            stargate_action_title: "Stargate detection",
            stargate_action_hint: "Clicks visible Stargates and presses A only after verifying the selection. 1920x1080 only.",
            stargate_confirm_scope_note: "Ownership and upgrade availability are not checked.",
            stargate_recall_f2_checkbox: "Recall F2 before running",
            stargate_recall_f2_hint: "Recall the F2 view before scanning Stargates.",
            stargate_skipped_note: "No A is sent unless the Stargate is confirmed.",
            status_scanning: "Scanning",
            arm_hint_invalid: "Check the settings. Conflicting hotkeys prevent Start.",
            advanced_heading: "Settings",

            config_heading: "Settings file",
            config_path_label: "Path",
            save_button: "Save",
            reload_button: "Reload",
            reset_button: "Restore defaults",
            saved_ok: "Settings saved.",
            save_failed: "Could not save settings",

            warn_read: "Could not read the settings file, using defaults",
            warn_parse: "Settings file is not valid, using defaults",
            warn_invalid: "Settings values are out of range, using defaults",

            hotkey_register_failed: "Hotkey registration failed — another program may already use it",
            hotkey_listener_failed: "Could not start the hotkey listener",
            hotkey_slot_trigger: "row build",
            hotkey_slot_spire_action: "spire action",
            hotkey_slot_stargate_action: "stargate action",
            hotkey_slot_emergency: "stop",

            diag_heading: "Safety check",
            diag_foreground: "Foreground process",
            diag_target_ok: "Target confirmed — input will be sent",
            diag_target_not: "Not the target — no input is sent",
            diag_unknown: "Unknown",

            outcome_completed: "completed",
            outcome_cancelled: "cancelled",
            outcome_aborted: "aborted",
            outcome_failed: "error",
            steps_word: "steps",
            busy_note: "Already running. New triggers are ignored, never queued.",

            notes_heading: "Before you use it",
            vacant_colony_sequence_label: "one run",
            vacant_colony_search_step_label: "probe and verify free space",
            vacant_colony_confirm_step_label: "confirm morph start",
            vacant_colony_caveat: "Builds only on verified free space. Search limit: 60 seconds.",
            note_select_drone: "Row build: select 2-12 drones and place the cursor at the first position. F6 uses the game's saved F4 view.",
            note_chat: "Do not use while typing in game chat.",
            note_online: "Check the rules before using it in online matches.",
            note_language_fallback: "No Korean font was found, showing English.",
        }
    }

    pub fn macro_label(&self, macro_id: MacroId) -> &'static str {
        match macro_id {
            MacroId::CreepColony => self.colony_label,
            MacroId::Spire => self.spire_label,
        }
    }

    /// Display name of one build target, including its tile footprint.
    pub fn build_target_name(&self, target: BuildTarget) -> &'static str {
        match target {
            BuildTarget::Colony => self.colony_target_name,
            BuildTarget::Spire => self.spire_target_name,
        }
    }

    /// Display name of one colony row placement order.
    pub fn row_mode_name(&self, mode: RowMode) -> &'static str {
        match mode {
            RowMode::LeftToRight => self.row_mode_left_to_right,
            RowMode::EndsInward => self.row_mode_ends_inward,
            RowMode::Grid6x2 => self.row_mode_grid_6x2,
        }
    }

    pub fn slot_label(&self, slot: HotkeySlot) -> &'static str {
        match slot {
            HotkeySlot::Trigger => self.hotkey_slot_trigger,
            HotkeySlot::SpireAction => self.hotkey_slot_spire_action,
            HotkeySlot::StargateAction => self.hotkey_slot_stargate_action,
            HotkeySlot::Emergency => self.hotkey_slot_emergency,
            HotkeySlot::VacantColony => self.vacant_colony_title(),
        }
    }

    pub fn vacant_colony_title(&self) -> &'static str {
        match self.lang {
            Lang::Korean => "F4 빈자리 크립 콜로니",
            Lang::English => "F4 vacant-space Colonies",
        }
    }

    pub fn vacant_colony_hint(&self) -> &'static str {
        match self.lang {
            Lang::Korean => {
                "게임의 F4에 빈 건설 화면을 저장해 두세요. 선택한 드론 2~12기로 확인된 빈자리만 순서대로 건설합니다."
            }
            Lang::English => {
                "Save a clear build view to in-game F4. Builds verified free positions in order with 2-12 selected drones."
            }
        }
    }

    /// Live counters of a running F4 search: probes, issued orders and
    /// confirmed morph starts. Shown so a long sweep is visibly progressing
    /// instead of looking stuck.
    pub fn vacant_colony_progress(&self, probes: usize, orders: usize, starts: usize) -> String {
        match self.lang {
            Lang::Korean => {
                format!(
                    "검사 {probes}곳 · 보낸 명령 {orders}개 · 변이 시작 확인 {starts}개 (건설 완료 아님)"
                )
            }
            Lang::English => {
                format!(
                    "{probes} probes · {orders} orders · {starts} morphs confirmed started (not completed buildings)"
                )
            }
        }
    }

    pub fn vacant_colony_result(
        &self,
        report: &crate::vacant_colony::VacantColonyReport,
    ) -> (NoticeLevel, String) {
        let (level, state, detail) = match &report.outcome {
            Outcome::Completed => (
                NoticeLevel::Ok,
                match self.lang {
                    Lang::Korean => "명령 전송 완료",
                    Lang::English => "orders sent",
                },
                "",
            ),
            Outcome::Cancelled => (
                NoticeLevel::Info,
                match self.lang {
                    Lang::Korean => "취소",
                    Lang::English => "cancelled",
                },
                "",
            ),
            Outcome::Aborted { detail } | Outcome::Failed { detail } => (
                NoticeLevel::Err,
                match self.lang {
                    Lang::Korean => "중단",
                    Lang::English => "stopped",
                },
                detail.as_str(),
            ),
        };
        let progress = match self.lang {
            Lang::Korean => format!(
                "명령 {}/{} · 변이 시작 확인 {}개 · 검사 {}곳 · 건설 완료 수 아님",
                report.orders.len(),
                report.detected,
                report.starts.len(),
                report.probes
            ),
            Lang::English => format!(
                "{}/{} orders, {} confirmed construction starts, {} probes (not completed buildings)",
                report.orders.len(),
                report.detected,
                report.starts.len(),
                report.probes
            ),
        };
        (
            level,
            format!(
                "{}: {state} · {progress} {}",
                self.vacant_colony_title(),
                detail
            ),
        )
    }

    pub fn steps(&self, done: usize, total: usize) -> String {
        format!("{} {done}/{total}", self.steps_word)
    }

    /// Separator between the compact fields of one status or preview line.
    fn separator(&self) -> &'static str {
        match self.lang {
            Lang::Korean => " · ",
            Lang::English => " | ",
        }
    }

    /// A duration as shown in the preview.
    fn millis(ms: u128) -> String {
        format!("{ms} ms")
    }

    /// The localized word for one Spire action outcome.
    pub fn spire_outcome_word(&self, outcome: &SpireActionOutcome) -> &'static str {
        match outcome {
            SpireActionOutcome::Completed => self.outcome_completed,
            SpireActionOutcome::Cancelled => self.outcome_cancelled,
            SpireActionOutcome::Aborted { .. } => self.outcome_aborted,
            SpireActionOutcome::Failed { .. } => self.outcome_failed,
        }
    }

    /// The localized word for a scan-only pass that produced no result.
    ///
    /// A refused gate or an unusable frame aborts; an OS/injection failure or
    /// an internal panic is an error; a latched F8 stays a cancellation.
    pub fn spire_scan_error_word(&self, error: &SpireScanError) -> &'static str {
        match error {
            SpireScanError::Cancelled => self.outcome_cancelled,
            SpireScanError::Adapter(InputError::Unsafe(_)) | SpireScanError::Unusable { .. } => {
                self.outcome_aborted
            }
            SpireScanError::Adapter(InputError::Injection(_)) | SpireScanError::Internal { .. } => {
                self.outcome_failed
            }
        }
    }

    /// "외 3곳" / "and 3 more" for a truncated position list.
    pub fn spire_more_positions(&self, extra: usize) -> String {
        match self.lang {
            Lang::Korean => format!("외 {extra}곳"),
            Lang::English => format!("and {extra} more"),
        }
    }

    /// Compact list of detection centres: `(790,158) · (1366,373)`.
    pub fn spire_positions(&self, centers: &[Point]) -> String {
        const MAX_SHOWN: usize = 8;
        let shown: Vec<String> = centers
            .iter()
            .take(MAX_SHOWN)
            .map(|center| format!("({},{})", center.x, center.y))
            .collect();
        let mut text = shown.join(self.separator());
        if let Some(extra) = centers
            .len()
            .checked_sub(MAX_SHOWN)
            .filter(|extra| *extra > 0)
        {
            text.push_str(self.separator());
            text.push_str(&self.spire_more_positions(extra));
        }
        text
    }

    /// Preview of one scan-only pass: what was found, where, and how long the
    /// single capture and single search took.
    ///
    /// It can never look like a success (no input was sent), and it never turns
    /// a blank or unsupported frame into "0 spires" — those arrive as a
    /// [`SpireScanError`] instead.
    pub fn spire_scan_preview(&self, report: &SpireScanReport) -> SpirePreview {
        let centers: Vec<Point> = report
            .detections
            .iter()
            .map(|detection| detection.center)
            .collect();
        let headline = format!(
            "{}: {} {}{}{}",
            self.spire_scan_only_tag,
            report.count,
            self.spire_detected_label,
            self.separator(),
            self.spire_scan_only_unsent
        );
        let mut notes = Vec::new();
        if report.count == 0 {
            notes.push(self.spire_zero_note.to_owned());
        }
        SpirePreview {
            level: NoticeLevel::Info,
            headline,
            positions: (!centers.is_empty()).then(|| {
                format!(
                    "{}: {}",
                    self.spire_positions_label,
                    self.spire_positions(&centers)
                )
            }),
            timings: Some(format!(
                "{} {} ({}){}{} {}",
                self.spire_capture_label,
                report.full_captures,
                Self::millis(report.capture_ms),
                self.separator(),
                self.spire_detect_label,
                Self::millis(report.detect_ms)
            )),
            notes,
        }
    }

    /// Preview of a scan-only pass that produced no trustworthy result.
    pub fn spire_scan_error_preview(&self, error: &SpireScanError) -> SpirePreview {
        SpirePreview {
            level: NoticeLevel::Err,
            headline: format!(
                "{} {}: {error}",
                self.spire_scan_label,
                self.spire_scan_error_word(error)
            ),
            positions: None,
            timings: None,
            notes: Vec::new(),
        }
    }

    /// Preview of one full action pass: what was clicked, where `A` was sent,
    /// what was skipped, and the capture/detection timings.
    /// Preview of one full action pass: what was clicked, where `A` was sent,
    /// what was skipped, and the capture/detection timings.
    pub fn spire_action_preview(&self, report: &SpireActionReport) -> SpirePreview {
        self.action_preview(report, self.spire_action_title, self.spire_skipped_note)
    }

    /// Preview of one finished Stargate action. Same report shape as the Spire
    /// action, but titled and worded unambiguously as the Stargate feature.
    pub fn stargate_action_preview(&self, report: &StargateActionReport) -> SpirePreview {
        self.action_preview(
            report,
            self.stargate_action_title,
            self.stargate_skipped_note,
        )
    }

    fn action_preview(
        &self,
        report: &SpireActionReport,
        title: &str,
        skipped_note: &str,
    ) -> SpirePreview {
        let centers: Vec<Point> = report.targets.iter().map(|target| target.center).collect();
        let outcome_word = self.spire_outcome_word(&report.outcome);
        let parts = match self.lang {
            Lang::Korean => [
                format!("{title} {outcome_word}"),
                format!("{} {}회", self.spire_a_sent_label, report.acted),
                format!("{} {}", report.detections, self.spire_detected_label),
                format!("{} {}건", self.spire_skipped_label, report.skipped),
            ],
            Lang::English => [
                format!("{title} {outcome_word}"),
                format!("{} {}", self.spire_a_sent_label, report.acted),
                format!("{} {}", report.detections, self.spire_detected_label),
                format!("{} {}", report.skipped, self.spire_skipped_label),
            ],
        };
        let mut headline = parts.join(self.separator());
        let detail = match &report.outcome {
            SpireActionOutcome::Aborted { detail } | SpireActionOutcome::Failed { detail } => {
                Some(detail.as_str())
            }
            _ => None,
        };
        if let Some(detail) = detail {
            headline += &match self.lang {
                Lang::Korean => format!(" — {detail}"),
                Lang::English => format!(": {detail}"),
            };
        }

        let mut notes = Vec::new();
        if report.skipped > 0 {
            notes.push(skipped_note.to_owned());
        }
        notes.push(self.spire_confirm_scope_note.to_owned());
        notes.push(self.spire_a_not_upgrade.to_owned());

        SpirePreview {
            level: spire_action_notice_level(report),
            headline,
            positions: (!centers.is_empty()).then(|| {
                format!(
                    "{}: {}",
                    self.spire_positions_label,
                    self.spire_positions(&centers)
                )
            }),
            timings: Some(format!(
                "{} {} ({}){}{} {}{}{} {}",
                self.spire_capture_label,
                report.full_captures,
                Self::millis(report.capture_ms),
                self.separator(),
                self.spire_detect_label,
                Self::millis(report.detect_ms),
                self.separator(),
                self.spire_roi_label,
                report.roi_captures
            )),
            notes,
        }
    }

    /// One line describing how a run ended, including the English detail text.
    pub fn outcome(&self, report: &RunReport) -> String {
        let macro_label = self.macro_label(report.macro_id);
        let progress = format!(
            "({})",
            if report.steps_total == 0 {
                report.steps_done.to_string()
            } else {
                self.steps(report.steps_done, report.steps_total)
            }
        );
        let base = match &report.outcome {
            Outcome::Completed => format!("{macro_label} {} {progress}", self.outcome_completed),
            Outcome::Cancelled => format!("{macro_label} {} {progress}", self.outcome_cancelled),
            Outcome::Aborted { detail } => {
                format!(
                    "{macro_label} {} {progress}: {detail}",
                    self.outcome_aborted
                )
            }
            Outcome::Failed { detail } => {
                format!("{macro_label} {} {progress}: {detail}", self.outcome_failed)
            }
        };
        if report.unconfirmed == 0 {
            base
        } else {
            format!(
                "{base} — {} {}",
                report.unconfirmed, self.unconfirmed_suffix
            )
        }
    }

    pub fn config_warning(&self, warning: &ConfigWarning) -> String {
        let (prefix, detail) = match warning {
            ConfigWarning::Read { detail } => (self.warn_read, detail),
            ConfigWarning::Parse { detail } => (self.warn_parse, detail),
            ConfigWarning::Invalid { detail } => (self.warn_invalid, detail),
        };
        format!("{prefix}: {detail}")
    }

    pub fn config_error(&self, error: &ConfigError) -> String {
        format!("{}: {error}", self.save_failed)
    }

    pub fn hotkey_error(&self, error: &HotkeyError) -> String {
        match error {
            HotkeyError::Register { slot, key, detail } => format!(
                "{} — {} {}: {detail}",
                self.hotkey_register_failed,
                self.slot_label(*slot),
                key.label()
            ),
            HotkeyError::Listener { detail } => {
                format!("{}: {detail}", self.hotkey_listener_failed)
            }
        }
    }

    pub fn start_error(&self, error: StartError) -> String {
        match error {
            StartError::Busy => self.busy_note.to_owned(),
            StartError::Thread { detail } => format!("{}: {detail}", self.outcome_failed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Outcome;
    use crate::hotkey::HotkeyKey;
    use crate::vacant_colony::VacantColonyReport;

    fn contains_hangul(text: &str) -> bool {
        text.chars().any(|c| {
            ('\u{AC00}'..='\u{D7A3}').contains(&c) || ('\u{3130}'..='\u{318F}').contains(&c)
        })
    }

    #[test]
    fn korean_labels_are_actually_korean() {
        let labels = Labels::korean();
        assert_eq!(labels.lang, Lang::Korean);
        for text in [
            labels.arm_button,
            labels.disarm_button,
            labels.note_select_drone,
            labels.note_chat,
            labels.note_online,
            labels.hotkey_register_failed,
            labels.status_armed,
            labels.sequence_label,
            labels.trigger_label,
            labels.trigger_single_hint,
            labels.timing_text_hint,
            labels.force_build_checkbox,
            labels.force_build_hint,
            labels.unconfirmed_suffix,
            labels.mouse_click_label,
            labels.advanced_heading,
            labels.row_build_title,
            labels.build_spire_checkbox,
            labels.build_target_label,
            labels.colony_target_name,
            labels.spire_target_name,
            labels.row_build_hint,
            labels.row_mode_label,
            labels.row_mode_left_to_right,
            labels.row_mode_ends_inward,
            labels.spire_action_title,
            labels.spire_action_hotkey_label,
            labels.spire_action_sequence_label,
            labels.spire_search_step_label,
            labels.spire_action_hint,
            labels.spire_confirm_scope_note,
            labels.spire_preview_heading,
            labels.spire_preview_empty,
            labels.spire_positions_label,
            labels.spire_capture_label,
            labels.spire_detect_label,
            labels.spire_scan_label,
            labels.spire_scan_only_unsent,
            labels.spire_a_sent_label,
            labels.spire_skipped_label,
            labels.spire_a_not_upgrade,
            labels.spire_skipped_note,
            labels.spire_zero_note,
            labels.status_scanning,
            labels.arm_hint_invalid,
        ] {
            assert!(contains_hangul(text), "no Hangul in '{text}'");
        }
    }

    #[test]
    fn english_labels_are_plain_ascii() {
        let labels = Labels::english();
        assert_eq!(labels.lang, Lang::English);
        for text in [
            labels.app_title,
            labels.arm_button,
            labels.note_select_drone,
            labels.note_chat,
            labels.note_online,
            labels.busy_note,
            labels.sequence_label,
            labels.trigger_label,
            labels.trigger_single_hint,
            labels.timing_text_hint,
            labels.force_build_checkbox,
            labels.force_build_hint,
            labels.unconfirmed_suffix,
            labels.mouse_click_label,
            labels.advanced_heading,
            labels.row_build_title,
            labels.build_spire_checkbox,
            labels.build_target_label,
            labels.colony_target_name,
            labels.spire_target_name,
            labels.row_build_hint,
            labels.row_mode_label,
            labels.row_mode_left_to_right,
            labels.row_mode_ends_inward,
            labels.spire_action_title,
            labels.spire_action_hotkey_label,
            labels.spire_action_sequence_label,
            labels.spire_search_step_label,
            labels.spire_action_hint,
            labels.spire_confirm_scope_note,
            labels.spire_preview_heading,
            labels.spire_preview_empty,
            labels.spire_detected_label,
            labels.spire_positions_label,
            labels.spire_capture_label,
            labels.spire_detect_label,
            labels.spire_roi_label,
            labels.spire_scan_label,
            labels.spire_scan_only_tag,
            labels.spire_scan_only_unsent,
            labels.spire_a_sent_label,
            labels.spire_skipped_label,
            labels.spire_a_not_upgrade,
            labels.spire_skipped_note,
            labels.spire_zero_note,
            labels.status_scanning,
            labels.arm_hint_invalid,
        ] {
            assert!(text.is_ascii(), "non ASCII in '{text}'");
        }
    }

    #[test]
    fn action_hints_are_short_and_keep_the_verification_contract() {
        for labels in [Labels::korean(), Labels::english()] {
            let hint = labels.spire_action_hint;
            assert!(hint.chars().count() < 120, "{hint}");
            assert!(
                hint.contains("선택 확인") || hint.contains("verifying the selection"),
                "{hint}"
            );
            assert!(hint.contains("1920x1080"), "{hint}");
            assert!(labels.spire_action_title.chars().count() < 24);
            assert!(labels.stargate_action_title.chars().count() < 24);
        }
    }

    #[test]
    fn the_action_preview_warns_type_is_not_ownership_or_upgrade_availability() {
        for labels in [Labels::korean(), Labels::english()] {
            let preview =
                labels.spire_action_preview(&action_report(4, 0, SpireActionOutcome::Completed));
            assert!(
                preview
                    .notes
                    .contains(&labels.spire_confirm_scope_note.to_owned()),
                "{:?}",
                preview.notes
            );
            // The count is still stated as A commands, not finished upgrades.
            assert!(
                preview
                    .notes
                    .contains(&labels.spire_a_not_upgrade.to_owned()),
                "{:?}",
                preview.notes
            );
        }
    }

    #[test]
    fn the_row_build_hint_differs_per_language() {
        assert_ne!(
            Labels::korean().row_build_hint,
            Labels::english().row_build_hint
        );
    }

    #[test]
    fn row_hint_is_short_and_keeps_the_required_constraints() {
        for labels in [Labels::korean(), Labels::english()] {
            let hint = labels.row_build_hint;
            assert!(hint.chars().count() < 150, "{hint}");
            assert!(hint.contains("2~12") || hint.contains("2-12"), "{hint}");
            assert!(hint.contains("1920x1080"), "{hint}");
            assert!(
                hint.contains("그룹 9") || hint.contains("group 9"),
                "{hint}"
            );
            assert!(labels.colony_target_name.contains("2x2"));
            assert!(labels.spire_target_name.contains("2x2"));
            assert_ne!(labels.build_spire_checkbox, labels.colony_target_name);
        }
    }

    #[test]
    fn every_build_target_has_a_distinct_localized_name() {
        for labels in [Labels::korean(), Labels::english()] {
            let names: Vec<&str> = BuildTarget::ALL
                .into_iter()
                .map(|target| labels.build_target_name(target))
                .collect();
            assert_eq!(names.len(), BuildTarget::ALL.len());
            assert_eq!(names[0], labels.colony_target_name);
            assert_eq!(names[1], labels.spire_target_name);
            assert_ne!(names[0], names[1]);
        }
    }

    #[test]
    fn every_row_mode_has_a_distinct_localized_name() {
        for labels in [Labels::korean(), Labels::english()] {
            let names: Vec<&str> = RowMode::ALL
                .into_iter()
                .map(|mode| labels.row_mode_name(mode))
                .collect();
            assert_eq!(names.len(), RowMode::ALL.len());
            assert_eq!(names[0], labels.row_mode_left_to_right);
            assert_eq!(names[1], labels.row_mode_ends_inward);
            assert_ne!(names[0], names[1]);
        }
        assert_ne!(
            Labels::korean().row_mode_label,
            Labels::english().row_mode_label
        );
    }

    #[test]
    fn both_languages_describe_the_same_states() {
        assert_ne!(Labels::korean().arm_button, Labels::english().arm_button);
        assert_eq!(
            Labels::korean().macro_label(MacroId::CreepColony),
            Labels::korean().colony_label
        );
        assert_eq!(
            Labels::english().macro_label(MacroId::Spire),
            Labels::english().spire_label
        );
    }

    #[test]
    fn outcomes_and_errors_are_rendered_with_progress_and_detail() {
        let labels = Labels::korean();
        let report = RunReport {
            macro_id: MacroId::Spire,
            outcome: Outcome::Aborted {
                detail: "foreground window belongs to 'notepad.exe'".to_owned(),
            },
            steps_done: 3,
            steps_total: 6,
            unconfirmed: 1,
        };
        let line = labels.outcome(&report);
        assert!(line.contains(labels.outcome_aborted), "{line}");
        assert!(line.contains("3/6"), "{line}");
        assert!(line.contains("notepad.exe"), "{line}");
        assert!(line.contains(labels.spire_label), "{line}");
    }

    #[test]
    fn hotkey_errors_name_the_binding() {
        let labels = Labels::korean();
        let line = labels.hotkey_error(&HotkeyError::Register {
            slot: HotkeySlot::Trigger,
            key: HotkeyKey::F6,
            detail: "already registered".to_owned(),
        });
        assert!(line.contains("F6"), "{line}");
        assert!(line.contains(labels.hotkey_slot_trigger), "{line}");

        let line = labels.hotkey_error(&HotkeyError::Listener {
            detail: "timed out".to_owned(),
        });
        assert!(line.contains(labels.hotkey_listener_failed), "{line}");
    }

    #[test]
    fn warnings_keep_their_english_detail() {
        let labels = Labels::english();
        let line = labels.config_warning(&ConfigWarning::Parse {
            detail: "unknown field `gapp_ms`".to_owned(),
        });
        assert!(line.contains("unknown field"), "{line}");
        assert!(line.contains(labels.warn_parse), "{line}");
    }

    #[test]
    fn start_errors_are_localized() {
        assert_eq!(
            Labels::korean().start_error(StartError::Busy),
            Labels::korean().busy_note
        );
        assert_eq!(
            Labels::english().start_error(StartError::Busy),
            Labels::english().busy_note
        );
    }

    #[test]
    fn emergency_and_ignored_notes_are_localized() {
        let korean = Labels::korean();
        let english = Labels::english();
        assert_ne!(korean.status_emergency, english.status_emergency);
        assert_ne!(korean.status_not_armed, english.status_not_armed);
        for text in [korean.status_emergency, korean.status_not_armed] {
            assert!(contains_hangul(text), "no Hangul in '{text}'");
        }
        for text in [english.status_emergency, english.status_not_armed] {
            assert!(text.is_ascii(), "non ASCII in '{text}'");
        }
    }

    use crate::spire_action::{SpireTargetReport, TargetDisposition};
    use crate::spire_vision::SpireDetection;

    fn detection(x: i32, y: i32) -> SpireDetection {
        SpireDetection {
            center: Point::new(x, y),
            score: 0.9,
            edge_agreement: 0.8,
            frame_stddev: 30.0,
        }
    }

    fn scan_report(count: usize) -> SpireScanReport {
        SpireScanReport {
            detections: (0..count)
                .map(|index| detection(700 + index as i32, 150))
                .collect(),
            count,
            capture_ms: 28,
            detect_ms: 79,
            full_captures: 1,
        }
    }

    fn action_report(
        acted: usize,
        skipped: usize,
        outcome: SpireActionOutcome,
    ) -> SpireActionReport {
        let targets = (0..acted + skipped)
            .map(|index| SpireTargetReport {
                center: Point::new(700 + index as i32, 160),
                score: 0.9,
                disposition: if index < acted {
                    TargetDisposition::Acted
                } else {
                    TargetDisposition::Skipped {
                        reason: "selection panel did not show the Spire portrait".to_owned(),
                    }
                },
            })
            .collect();
        SpireActionReport {
            label: "Spire",
            outcome,
            detections: acted + skipped,
            targets,
            acted,
            skipped,
            full_captures: 1,
            roi_captures: acted + skipped,
            capture_ms: 28,
            detect_ms: 79,
        }
    }

    #[test]
    fn a_scan_only_preview_reports_what_it_found_without_claiming_input() {
        for labels in [Labels::korean(), Labels::english()] {
            let preview = labels.spire_scan_preview(&scan_report(4));
            assert_eq!(
                preview.level,
                NoticeLevel::Info,
                "a preview can never be a success"
            );
            assert!(preview.headline.contains('4'), "{}", preview.headline);
            assert!(
                preview.headline.contains(labels.spire_scan_only_tag),
                "{}",
                preview.headline
            );
            assert!(
                preview.headline.contains(labels.spire_scan_only_unsent),
                "{}",
                preview.headline
            );
            let timings = preview.timings.expect("capture and detect times");
            assert!(
                timings.contains("28 ms") && timings.contains("79 ms"),
                "{timings}"
            );
            let positions = preview.positions.expect("positions");
            assert!(
                positions.contains(labels.spire_positions_label),
                "{positions}"
            );
            assert!(positions.contains("(700,150)"), "{positions}");
            assert!(preview.notes.is_empty(), "{:?}", preview.notes);
        }
    }

    #[test]
    fn a_zero_detection_scan_says_zero_without_pretending_it_is_an_answer() {
        for labels in [Labels::korean(), Labels::english()] {
            let preview = labels.spire_scan_preview(&scan_report(0));
            assert_eq!(preview.level, NoticeLevel::Info);
            assert!(preview.headline.contains('0'), "{}", preview.headline);
            assert!(preview.positions.is_none(), "nothing to point at");
            assert_eq!(preview.notes, vec![labels.spire_zero_note.to_owned()]);
        }
    }

    #[test]
    fn a_refused_or_unusable_scan_is_an_error_that_keeps_the_english_reason() {
        let labels = Labels::korean();
        let refused = labels.spire_scan_error_preview(&SpireScanError::Adapter(
            InputError::Unsafe("foreground window belongs to 'notepad.exe'".to_owned()),
        ));
        assert_eq!(refused.level, NoticeLevel::Err);
        assert!(
            refused.headline.contains(labels.outcome_aborted),
            "{}",
            refused.headline
        );
        assert!(
            refused.headline.contains("notepad.exe"),
            "{}",
            refused.headline
        );
        assert!(
            refused.headline.contains(labels.spire_scan_label),
            "{}",
            refused.headline
        );
        assert!(refused.positions.is_none() && refused.timings.is_none());

        let blank = labels.spire_scan_error_preview(&SpireScanError::Unusable {
            detail: "the capture is blank".to_owned(),
        });
        assert_eq!(blank.level, NoticeLevel::Err);
        assert!(
            blank.headline.contains(labels.outcome_aborted),
            "{}",
            blank.headline
        );
        assert!(blank.headline.contains("blank"), "{}", blank.headline);
    }

    #[test]
    fn every_scan_error_has_a_localized_word_and_an_error_level() {
        let labels = Labels::korean();
        for (error, word) in [
            (SpireScanError::Cancelled, labels.outcome_cancelled),
            (
                SpireScanError::Adapter(InputError::Injection("SendInput refused".to_owned())),
                labels.outcome_failed,
            ),
            (
                SpireScanError::Unusable {
                    detail: "blank".to_owned(),
                },
                labels.outcome_aborted,
            ),
            (
                SpireScanError::Internal {
                    detail: "internal panic while scanning the screen".to_owned(),
                },
                labels.outcome_failed,
            ),
        ] {
            assert_eq!(labels.spire_scan_error_word(&error), word, "{error:?}");
            assert_eq!(
                labels.spire_scan_error_preview(&error).level,
                NoticeLevel::Err
            );
        }
    }

    #[test]
    fn an_action_preview_counts_a_commands_and_says_they_are_not_buildings() {
        for labels in [Labels::korean(), Labels::english()] {
            let preview =
                labels.spire_action_preview(&action_report(4, 0, SpireActionOutcome::Completed));
            assert_eq!(preview.level, NoticeLevel::Ok);
            assert!(
                preview.headline.contains(labels.spire_action_title),
                "{}",
                preview.headline
            );
            assert!(
                preview.headline.contains(labels.spire_a_sent_label),
                "{}",
                preview.headline
            );
            assert!(
                preview.headline.contains(labels.spire_skipped_label),
                "{}",
                preview.headline
            );
            assert!(
                preview
                    .notes
                    .contains(&labels.spire_a_not_upgrade.to_owned()),
                "{:?}",
                preview.notes
            );
            assert!(
                !preview
                    .notes
                    .contains(&labels.spire_skipped_note.to_owned()),
                "nothing was skipped: {:?}",
                preview.notes
            );
            let timings = preview.timings.expect("capture and detect times");
            assert!(timings.contains(labels.spire_roi_label), "{timings}");
            assert!(
                timings.contains("28 ms") && timings.contains("79 ms"),
                "{timings}"
            );
            assert!(preview.positions.is_some());
        }
    }

    #[test]
    fn a_partial_or_empty_action_is_information_not_success() {
        let labels = Labels::korean();
        let partial =
            labels.spire_action_preview(&action_report(3, 1, SpireActionOutcome::Completed));
        assert_eq!(partial.level, NoticeLevel::Info);
        assert!(
            partial
                .notes
                .contains(&labels.spire_skipped_note.to_owned()),
            "{:?}",
            partial.notes
        );

        let none = labels.spire_action_preview(&action_report(0, 4, SpireActionOutcome::Completed));
        assert_eq!(none.level, NoticeLevel::Info, "no A sent is not a success");
        assert!(
            none.headline.contains(labels.outcome_completed),
            "{}",
            none.headline
        );

        let cancelled =
            labels.spire_action_preview(&action_report(1, 0, SpireActionOutcome::Cancelled));
        assert_eq!(cancelled.level, NoticeLevel::Info);
        assert!(
            cancelled.headline.contains(labels.outcome_cancelled),
            "{}",
            cancelled.headline
        );
    }

    #[test]
    fn an_aborted_action_is_an_error_that_keeps_the_english_detail() {
        let labels = Labels::english();
        let report = action_report(
            0,
            0,
            SpireActionOutcome::Aborted {
                detail: "foreground window belongs to 'notepad.exe'".to_owned(),
            },
        );
        let preview = labels.spire_action_preview(&report);
        assert_eq!(preview.level, NoticeLevel::Err);
        assert!(
            preview.headline.contains(labels.outcome_aborted),
            "{}",
            preview.headline
        );
        assert!(
            preview.headline.contains("notepad.exe"),
            "{}",
            preview.headline
        );
        assert!(preview.positions.is_none(), "nothing was clicked");
    }

    #[test]
    fn a_long_position_list_is_truncated_with_a_localized_suffix() {
        for labels in [Labels::korean(), Labels::english()] {
            let centers: Vec<Point> = (0..10).map(|index| Point::new(700 + index, 150)).collect();
            let text = labels.spire_positions(&centers);
            assert!(text.contains("(700,150)"), "{text}");
            assert!(
                text.contains(labels.spire_more_positions(2).as_str()),
                "{text}"
            );
            assert!(
                !text.contains("(709,150)"),
                "only the first eight are shown: {text}"
            );
            assert_eq!(labels.spire_positions(&[]), "");
        }
    }

    #[test]
    fn the_third_feature_hint_is_short_and_names_the_saved_view() {
        for labels in [Labels::korean(), Labels::english()] {
            let hint = labels.vacant_colony_hint();
            assert!(hint.contains("F4"), "{hint}");
            assert!(hint.contains("2") && hint.contains("12"), "{hint}");
            assert!(hint.chars().count() < 150, "{hint}");
            assert!(!labels.vacant_colony_title().is_empty());
        }
        assert_ne!(
            Labels::korean().vacant_colony_hint(),
            Labels::english().vacant_colony_hint()
        );
    }

    #[test]
    fn the_third_feature_result_never_claims_completed_buildings() {
        let report =
            |outcome: Outcome, orders: usize, starts: usize, detected: u8, probes: usize| {
                VacantColonyReport {
                    outcome,
                    detected,
                    orders: vec![Point::new(900, 400); orders],
                    starts: vec![Point::new(900, 400); starts],
                    probes,
                }
            };

        let labels = Labels::korean();
        let (level, text) = labels.vacant_colony_result(&report(Outcome::Completed, 3, 3, 4, 12));
        assert_eq!(level, NoticeLevel::Ok);
        assert!(text.contains("3/4"), "{text}");
        assert!(text.contains("12"), "{text}");
        assert!(
            text.contains("변이 시작 확인 3개"),
            "the confirmed starts must be visible: {text}"
        );
        assert!(
            text.contains("건설 완료 수 아님"),
            "an issued order is not a finished building: {text}"
        );

        // A partial run: one order issued, but construction was not confirmed.
        let (level, text) = labels.vacant_colony_result(&report(Outcome::Cancelled, 1, 0, 4, 30));
        assert_eq!(level, NoticeLevel::Info, "a partial run is not a success");
        assert!(text.contains("1/4"), "{text}");
        assert!(
            text.contains("변이 시작 확인 0개"),
            "starts are counted separately from orders: {text}"
        );

        let (level, text) = labels.vacant_colony_result(&report(
            Outcome::Aborted {
                detail: "the F4 view changed; refusing stale screen coordinates".to_owned(),
            },
            0,
            0,
            4,
            7,
        ));
        assert_eq!(level, NoticeLevel::Err);
        assert!(
            text.contains("stale screen coordinates"),
            "the English detail must survive: {text}"
        );

        let (english_level, english_text) =
            Labels::english().vacant_colony_result(&report(Outcome::Cancelled, 1, 0, 4, 30));
        assert_eq!(
            english_level,
            NoticeLevel::Info,
            "the same outcome must get the same level in both languages"
        );
        assert!(
            english_text.contains("not completed buildings"),
            "{english_text}"
        );
        assert!(
            english_text.contains("confirmed construction starts"),
            "{english_text}"
        );
    }

    #[test]
    fn the_row_build_notice_level_follows_the_outcome_wording() {
        let mut report = RunReport {
            macro_id: MacroId::CreepColony,
            outcome: Outcome::Completed,
            steps_done: 1,
            steps_total: 1,
            unconfirmed: 0,
        };
        assert_eq!(run_notice_level(&report), NoticeLevel::Ok);
        report.outcome = Outcome::Cancelled;
        assert_eq!(run_notice_level(&report), NoticeLevel::Err);
        report.outcome = Outcome::Failed {
            detail: "stub".to_owned(),
        };
        assert_eq!(run_notice_level(&report), NoticeLevel::Err);
    }
}
