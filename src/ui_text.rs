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
    pub stargate_skipped_note: &'static str,
    /// Label of the retained read-only scan diagnostic (see
    /// [`Labels::spire_scan_preview`]); the action card itself only ever shows
    /// [`Labels::status_running`], because the action key always runs the full
    /// action.
    pub status_scanning: &'static str,
    pub arm_hint_invalid: &'static str,
    pub advanced_heading: &'static str,
    pub app_subtitle: &'static str,

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
            app_title: "oh-my-macro (StarCraft 1 매크로)",
            status_heading: "상태",
            status_armed: "사용 중 — 단축키 등록됨",
            status_disarmed: "중지됨 — 단축키 해제됨",
            status_running: "실행 중",
            status_idle: "대기",
            status_emergency: "F8 응급 정지",
            status_not_armed: "중지 상태라 무시했습니다",
            arm_button: "사용 시작 (무장)",
            disarm_button: "중지 (해제)",
            arm_hint: "무장해야 트리거 단축키와 F8이 동작합니다. 기본값은 중지 상태입니다.",

            hotkeys_heading: "단축키",
            colony_label: "크립 콜로니 (Creep Colony)",
            spire_label: "둥지탑 (Spire)",
            emergency_label: "응급 정지",
            emergency_hint: "F8은 고정이며 변경할 수 없습니다.",
            hotkeys_locked_hint: "단축키는 중지 상태에서만 바꿀 수 있습니다.",

            timing_heading: "타이밍 (두 단축키 공통)",
            press_label: "키/버튼 누름 유지 (ms)",
            gap_label: "동작 사이 간격 (ms)",
            timing_hint: "기본 20ms입니다. 게임이 입력을 놓치면 50ms 전후로 늘려보세요.",
            target_label: "대상 프로세스 (exe 파일명)",
            target_hint: "창 제목이 아니라 실행 파일 이름입니다. 예: StarCraft.exe",
            interval_range_hint: "범위",

            sequence_label: "입력 순서",
            trigger_label: "트리거 단축키",
            trigger_single_hint: "이 단축키 하나가 매크로를 실행합니다. F8은 응급 정지입니다.",
            timing_text_hint: "숫자를 직접 입력하세요(1-2000 ms). 범위를 벗어나면 적용되지 않고 표시만 됩니다.",
            force_build_checkbox: "미리보기 확인 실패 시에도 강행",
            force_build_hint: "체크하면 초록 미리보기를 잠깐만 확인하고, 그래도 안 보이면 클릭해 계속 진행합니다(명령은 미확인으로 표시).",
            unconfirmed_suffix: "건은 미확인(강행)",
            mouse_click_label: "좌클릭",
            row_build_title: "드론 줄짓기 건설",
            build_spire_checkbox: "둥지탑(Spire) 건설 — 끄면 크립 콜로니",
            build_target_label: "건물",
            colony_target_name: "크립 콜로니 (2x2)",
            spire_target_name: "둥지탑 (2x2)",
            row_build_hint: "트리거 단축키 하나가 이 매크로를 실행합니다. 위 체크박스로 지을 건물(크립 콜로니 또는 둥지탑)을 고르고, 표시된 입력 순서와 공유 타이밍을 사용합니다. 선택한 드론 2~12기를 자동으로 세어 커서 오른쪽으로 한 줄로 짓고, 배치 순서는 왼쪽→오른쪽 또는 양끝→가운데 중에서 고를 수 있습니다(두 순서의 줄 범위는 같고 커서가 맨 왼쪽 발자국). 간격은 건물 크기를 따릅니다: 크립 콜로니 2타일(144px), 둥지탑 2타일(144px). 둥지탑 간격은 타일 계산으로 추정한 값이며 아직 실기 검증되지 않았습니다 — 엄격 모드는 미리보기 확인 실패 시 중단하지만 강행 모드는 계속 클릭합니다. 임시 9번 그룹을 사용하고 1920x1080 HUD만 지원합니다. 표시 수는 완성된 건물이 아니라 보낸 명령 수입니다.",
            row_mode_label: "배치 순서",
            row_mode_left_to_right: "왼쪽→오른쪽",
            row_mode_ends_inward: "양끝→가운데",
            row_mode_grid_6x2: "6x2 (1~6 아래 / 7~12 위)",
            spire_action_title: "스파이어 감지 동작",
            spire_action_hotkey_label: "동작 단축키",
            spire_action_sequence_label: "동작 순서",
            spire_search_step_label: "전체 화면 SEARCH 1회",
            spire_action_hint: "무장하고 게임 창이 전면일 때만 동작하며, 동작 단축키를 누르면 바로 실행됩니다(미리보기 전용 모드는 없습니다). 전체 화면 SEARCH 1회에 이어 감지된 위치마다 검증용 캡처를 한 번씩 찍어 선택 패널을 확인합니다(현재 캡처 API는 전체 프레임을 렌더링한 뒤 패널을 잘라내므로 캡처가 1장이라는 뜻은 아니고, 전체 화면 재탐색은 없습니다). 좌표는 저장하지 않으며 1920x1080 리마스터 클라이언트만 지원합니다. 감지기는 캡처 1장으로 보정되어 아직 실기 검증 전입니다. 타이밍과 대상 프로세스는 위 카드와 공유합니다.",
            spire_confirm_scope_note: "감지와 선택 패널 확인은 그 자리의 건물 종류만 알려줍니다 — 이 스파이어가 내 것인지, 업그레이드가 지금 가능한지는 확인하지 않습니다.",
            spire_preview_heading: "마지막 결과",
            spire_preview_empty: "아직 결과가 없습니다 — 게임 창을 전면에 두고 동작 단축키를 누르세요. 입력은 무장 상태에서만 들어갑니다.",
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
            stargate_action_title: "스타게이트 감지 동작",
            stargate_action_hint: "무장하고 게임 창이 전면일 때만 동작하며, 동작 단축키를 누르면 바로 실행됩니다(미리보기 전용 모드는 없습니다). 전체 화면 SEARCH 1회에 이어 감지된 위치마다 검증용 캡처를 찍어 선택 패널을 확인하고, 스타게이트로 확인된 곳에만 A를 한 번 보냅니다. 좌표는 저장하지 않으며 1920x1080 리마스터 클라이언트만 지원합니다. 감지기는 캡처 1장으로 보정되어 아직 실기 검증 전입니다. 타이밍과 대상 프로세스는 위 카드와 공유합니다.",
            stargate_confirm_scope_note: "감지와 선택 패널 확인은 그 자리의 건물 종류만 알려줍니다 — 이 스타게이트가 내 것인지, 지금 업그레이드가 가능한지는 확인하지 않습니다.",
            stargate_skipped_note: "선택 패널이 스타게이트로 확인되지 않은 위치는 A를 보내지 않고 건너뛰었습니다.",
            status_scanning: "스캔 중",
            arm_hint_invalid: "설정이 올바르지 않아 무장할 수 없습니다 — 단축키가 서로 겹치지 않는지 확인하세요.",
            advanced_heading: "고급 설정 및 파일 관리",
            app_subtitle: "스타크래프트 1 빠른 건설 매크로",

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
            hotkey_slot_emergency: "응급 정지",

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
            vacant_colony_caveat: "미리보기가 확인된 곳에만 클릭합니다(강행 없음). 변이 시작 확인은 실제 캡처 1장으로 보정한 패널 판정이며 실기 검증 전입니다. 화면 한 장만 탐색합니다.",
            note_select_drone: "드론 2~12기를 선택한 뒤 트리거 단축키를 누르면 커서 자리부터 오른쪽으로 한 줄로 지어집니다. 실행키(기본 F6)는 게임의 F4 저장 화면으로 이동해 빈자리를 검증하며 지으므로, 먼저 게임에서 F4 화면을 지정해 두세요. 좌표는 저장하지 않습니다.",
            note_chat: "게임 채팅이나 입력 중에는 사용하지 마세요. 채팅 상태를 감지하지 못합니다.",
            note_online: "온라인/랭크/토너먼트 경기에서는 규정 위반이 될 수 있습니다. README를 먼저 확인하세요.",
            note_language_fallback: "한글 글꼴을 찾지 못해 영어로 표시합니다.",
        }
    }

    pub const fn english() -> Self {
        Self {
            lang: Lang::English,
            app_title: "oh-my-macro (StarCraft 1 macros)",
            status_heading: "Status",
            status_armed: "Armed — hotkeys registered",
            status_disarmed: "Disarmed — hotkeys released",
            status_running: "Running",
            status_idle: "Idle",
            status_emergency: "F8 emergency stop",
            status_not_armed: "ignored: not armed",
            arm_button: "Arm",
            disarm_button: "Disarm",
            arm_hint: "The trigger key and F8 only work while armed. The app starts disarmed.",

            hotkeys_heading: "Hotkeys",
            colony_label: "Creep Colony",
            spire_label: "Spire",
            emergency_label: "Emergency stop",
            emergency_hint: "F8 is fixed and cannot be changed.",
            hotkeys_locked_hint: "Hotkeys can only be changed while disarmed.",

            timing_heading: "Timing (both hotkeys)",
            press_label: "Key/button hold time (ms)",
            gap_label: "Delay between actions (ms)",
            timing_hint: "The 20 ms default is fast; increase toward 50 ms if the game misses input.",
            target_label: "Target process (exe basename)",
            target_hint: "Exe file name, not a window title, e.g. StarCraft.exe",
            interval_range_hint: "range",

            sequence_label: "Sequence",
            trigger_label: "Trigger Hotkey",
            trigger_single_hint: "This one key runs the macro. F8 is the emergency stop.",
            timing_text_hint: "Type the value (1-2000 ms). An out-of-range entry is flagged and not applied.",
            force_build_checkbox: "Keep going without a confirmed preview",
            force_build_hint: "When checked, the preview is only confirmed briefly; if it still cannot be confirmed, a click is sent and the run continues (those orders are marked unconfirmed).",
            unconfirmed_suffix: "unconfirmed (forced)",
            mouse_click_label: "Click",
            row_build_title: "Drone row build",
            build_spire_checkbox: "Build a Spire (unchecked: Creep Colony)",
            build_target_label: "Building",
            colony_target_name: "Creep Colony (2x2)",
            spire_target_name: "Spire (2x2)",
            row_build_hint: "The one trigger key runs this macro. The checkbox above picks the building (Creep Colony or Spire) for the shown build sequence and the shared timing. It counts the selected drones (2-12) and builds a row to the right of the cursor, with the row order set to left to right or ends inward (both cover the same span, the cursor staying the leftmost footprint). Spacing follows the building: Creep Colony 2 tiles (144 px), Spire 2 tiles (144 px). The Spire spacing is inferred from tile math and is NOT live-verified yet; with the forced mode off, a failed green preview check stops the run instead of clicking. Uses scratch group 9, 1920x1080 HUD only. The count shown is orders issued, not finished buildings.",
            row_mode_label: "Row order",
            row_mode_left_to_right: "Left to right",
            row_mode_ends_inward: "Ends inward",
            row_mode_grid_6x2: "6x2 (1-6 bottom / 7-12 top)",
            spire_action_title: "Spire detect action",
            spire_action_hotkey_label: "Action hotkey",
            spire_action_sequence_label: "Sequence",
            spire_search_step_label: "full-screen SEARCH x1",
            spire_action_hint: "Runs only while armed and while the game window is in front, and the action hotkey starts it immediately - there is no preview-only mode. One full-screen SEARCH is followed by a verification capture for every detected position to read the selection panel (the current capture API renders the full frame before cropping the panel, so this is a capture per target, not a single capture in total, and there is no second full-screen search). No coordinate is stored, and only the 1920x1080 Remastered client is supported. The detector is calibrated on one screenshot and is not live-verified yet. Timing and the target process are shared with the row-build card above.",
            spire_confirm_scope_note: "A detection and a confirmed selection panel only tell you the building type at that spot - they do not check that the Spire is yours or that the upgrade is available right now.",
            spire_preview_heading: "Last result",
            spire_preview_empty: "No result yet - put the game window in front and press the action hotkey. Input is only injected while armed.",
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
            stargate_action_title: "Stargate detect action",
            stargate_action_hint: "Runs only while armed and while the game window is in front, and the action hotkey starts it immediately - there is no preview-only mode. One full-screen SEARCH is followed by a verification capture per detected position to read the selection panel, and A is sent once only where the panel is confirmed as a Stargate. No coordinate is stored, and only the 1920x1080 Remastered client is supported. The detector is calibrated on one screenshot and is not live-verified yet. Timing and the target process are shared with the row-build card above.",
            stargate_confirm_scope_note: "A detection and a confirmed selection panel only tell you the building type at that spot - they do not check that the Stargate is yours or that an upgrade is available right now.",
            stargate_skipped_note: "Positions whose selection panel was not confirmed as the Stargate were skipped without A.",
            status_scanning: "Scanning",
            arm_hint_invalid: "Cannot arm: the settings are invalid - the hotkeys must not collide.",
            advanced_heading: "Advanced Settings & Diagnostics",
            app_subtitle: "StarCraft 1 Quick Build Helper",

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
            hotkey_slot_emergency: "emergency stop",

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
            vacant_colony_caveat: "Clicks only where a fresh preview is confirmed (no forced clicks). Morph-start confirmation is a panel classifier calibrated from one real capture; not live-validated. It searches a single screen.",
            note_select_drone: "Select 2-12 drones, then press the trigger key to build a row to the right of the cursor. The third key (default F6) recalls the game's saved F4 view and only builds on verified free space, so save that view in game first. No coordinate is stored.",
            note_chat: "Do not use while typing in game chat: chat state is not detected.",
            note_online: "May violate online/ranked/tournament rules. Read the README first.",
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
                "크립과 여유 공간이 있는 화면을 게임의 F4에 미리 저장하세요. 드론 2~12기 선택 → 실행키 → F4 이동 → 왼쪽 아래부터 오른쪽으로, 다음 줄은 위로 탐색 → 초록 미리보기 두 번 확인 → 건설 명령 → 그 드론의 변이 시작(콜로니 패널) 확인 후 다음 드론. 그룹 9 사용 · 강행 없음 · 같은 타이밍 사용. F4는 실행키로 지정할 수 없습니다. 탐색 최대 60초 · 드론당 변이 대기 최대 30초(캡처가 끝날 때까지 멈출 수 없어 초과할 수 있음) · 실기 검증 전입니다."
            }
            Lang::English => {
                "Save a view with creep and free space to in-game F4 first. Select 2–12 drones, then trigger: F4 → search from the lower left to the right, then up → confirm a stable green preview twice → Colony order → wait for that drone's morph panel before the next drone. Uses group 9 and shared timing; never forces placement. F4 stays reserved for the game. Search budget 60s, per-drone morph wait 30s; blocking captures cannot be interrupted, so waits can overshoot. Not live-validated yet."
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
            labels.app_title,
            labels.app_subtitle,
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
            labels.app_subtitle,
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
    fn the_action_hint_states_one_search_and_per_target_verification_captures() {
        // The card must not promise "one capture": the current adapter renders
        // the full frame and then crops the panel, once per detected target.
        let korean = Labels::korean().spire_action_hint;
        assert!(korean.contains("SEARCH"), "{korean}");
        assert!(korean.contains("검증"), "{korean}");
        assert!(korean.contains("캡처"), "{korean}");
        assert!(korean.contains("위치마다"), "{korean}");

        let english = Labels::english().spire_action_hint;
        let lower = english.to_lowercase();
        assert!(lower.contains("search"), "{english}");
        assert!(lower.contains("verification capture"), "{english}");
        assert!(lower.contains("detected position"), "{english}");

        for labels in [Labels::korean(), Labels::english()] {
            // The sequence row itself names the read-only step first.
            let step = labels.spire_search_step_label;
            assert!(step.contains("SEARCH") && step.contains('1'), "{step}");
            let hint = labels.spire_action_hint;
            // No leftover offer of a scan-only/preview mode or of a checkbox.
            assert!(!hint.contains("스캔만"), "{hint}");
            assert!(!hint.contains("scan only"), "{hint}");
            assert!(!hint.contains("체크"), "{hint}");
            assert!(
                !hint.contains("checkbox") && !hint.contains("box"),
                "{hint}"
            );
            assert!(
                !hint.contains("미리보기 전용") || hint.contains("없습니다"),
                "{hint}"
            );
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
    fn the_hint_names_the_trigger_the_checkbox_and_the_spacing() {
        for labels in [Labels::korean(), Labels::english()] {
            let hint = labels.row_build_hint;
            assert!(
                hint.contains("트리거") || hint.contains("trigger key"),
                "{hint}"
            );
            assert!(hint.contains("144"), "{hint}");
            assert!(
                !hint.contains("216"),
                "both buildings share the 144 px pitch now: {hint}"
            );
            // Both buildings are described as 2x2 now that they share the pitch.
            assert!(
                labels.colony_target_name.contains("2x2"),
                "{}",
                labels.colony_target_name
            );
            assert!(
                labels.spire_target_name.contains("2x2"),
                "{}",
                labels.spire_target_name
            );
            assert_ne!(labels.build_spire_checkbox, labels.colony_target_name);
        }
        assert_ne!(
            Labels::english().build_spire_checkbox,
            Labels::english().row_build_title
        );
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
    fn the_third_feature_hint_names_the_recall_key_and_the_limits() {
        for labels in [Labels::korean(), Labels::english()] {
            let hint = labels.vacant_colony_hint();
            assert!(hint.contains("F4"), "{hint}");
            assert!(hint.contains("2") && hint.contains("12"), "{hint}");
            assert!(!labels.vacant_colony_title().is_empty());
        }
        let korean = Labels::korean();
        let english = Labels::english();
        assert_ne!(korean.vacant_colony_hint(), english.vacant_colony_hint());
        assert_ne!(korean.vacant_colony_title(), english.vacant_colony_title());
        // The key the feature presses must be named, and the fact that the
        // search is not live-validated must survive in both languages.
        assert!(
            korean
                .vacant_colony_hint()
                .contains("F4는 실행키로 지정할 수 없습니다")
        );
        assert!(
            english
                .vacant_colony_hint()
                .contains("F4 stays reserved for the game")
        );
        assert!(english.vacant_colony_hint().contains("Not live-validated"));
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
