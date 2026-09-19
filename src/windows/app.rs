//! The eframe/egui application.
//!
//! The GUI thread is the only writer of application state. It owns:
//!
//! * the [`MacroRunner`] (one macro at a time, always joined again),
//! * the [`HotkeyListener`] while armed,
//! * all settings and the status the user sees.
//!
//! State is polled in [`eframe::App::logic`], because eframe stops calling
//! [`eframe::App::ui`] while the window is minimized or covered — which is the
//! normal situation here, since the game is in front. The hotkey thread calls
//! `request_repaint`, so hotkeys keep working while the window is hidden.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, CornerRadius, Frame, Margin, Painter, Rect, RichText, Stroke, vec2,
};

use super::adapter::{self, SendInputAdapter};
use super::listener::{HotkeyListener, Wake};
use crate::colony::RowMode;
use crate::config::{Config, ConfigWarning, MAX_INTERVAL_MS, MIN_INTERVAL_MS};
use crate::font::{self, KOREAN_FONT_FILES};
use crate::hotkey::{HotkeyError, HotkeyKey, HotkeySlot};
use crate::macros::{BuildTarget, Key};
use crate::runner::{FinishedReports, HotkeyAction, MacroRunner, action_for};
use crate::ui_text::{Labels, Lang, NoticeLevel, SpirePreview, run_notice_level};
use crate::vacant_colony::VacantProgress;

/// Repaint interval so "running/idle" stays current.
/// Muted label colour shared by the cards' secondary text.
const MUTED: Color32 = Color32::from_rgb(0x64, 0x74, 0x8b);
/// Separator drawn between the keycaps of one sequence row.
const ARROW: &str = "\u{2192}";
/// Amber warning glyph used by the caveat lines.
const CAVEAT: &str = "\u{26a0}";
const HEARTBEAT: Duration = Duration::from_millis(120);
/// How often the foreground process is probed for the safety display.
const DIAGNOSTICS_REFRESH: Duration = Duration::from_millis(500);

/// Starts the GUI. Returns an error string instead of panicking.
pub fn run() -> Result<(), String> {
    // Physical (DPI aware) cursor coordinates and the client capture must share
    // one coordinate space, otherwise the colony-row targeting is off on scaled
    // displays. Harmless if the manifest already set it.
    adapter::make_process_dpi_aware();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("oh-my-macro")
            .with_inner_size([680.0, 640.0])
            .with_min_inner_size([500.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "oh-my-macro",
        options,
        Box::new(|cc| Ok(Box::new(MacroApp::new(cc)))),
    )
    .map_err(|error| error.to_string())
}

struct MacroApp {
    labels: Labels,
    config: Config,
    config_path: PathBuf,
    warning: Option<ConfigWarning>,
    /// The one visible status line. Its level picks the glyph and colour, so a
    /// partial run can never be rendered as a success.
    notice: Option<Notice>,
    hotkey_error: Option<HotkeyError>,
    armed: bool,
    listener: Option<HotkeyListener>,
    runner: MacroRunner,
    /// Which run currently owns the single worker slot.
    running: Option<ActiveRun>,
    /// The newest Spire result, kept for the card's compact preview. Only the
    /// newest one: no history, no activity log.
    spire_preview: Option<SpirePreview>,
    /// The newest Stargate result. Separate from the Spire preview so the two
    /// actions can never cross channels.
    stargate_preview: Option<SpirePreview>,
    /// Live counters of the F4 search, read from the runner while it runs so a
    /// long sweep is visibly progressing instead of looking stuck.
    vacant_progress: Arc<VacantProgress>,
    foreground: Option<Result<String, String>>,
    last_diagnostics: Instant,
    /// Text buffers for the two timing values: the user types ms numbers
    /// instead of dragging sliders, and only a valid value reaches the config.
    press_edit: IntervalEdit,
    gap_edit: IntervalEdit,
}

/// Which run holds the worker slot, so every card's "running" badge is true.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveRun {
    /// The row build for the configured [`BuildTarget`].
    RowBuild,
    /// The Spire action: one full-screen search, then click each detection and
    /// `A` per verified selection. There is no separate read-only pass any
    /// more, so the action key only ever starts this run.
    SpireAction,
    /// The Stargate action: one full-screen search, then click each detection
    /// and `A` per verified selection.
    StargateAction,
    VacantColony,
}

/// One status line plus the level that decides how it is drawn.
#[derive(Clone, Debug)]
struct Notice {
    level: NoticeLevel,
    text: String,
}

/// A numeric millisecond field edited as text.
///
/// The buffer keeps exactly what the user typed; a value outside
/// `MIN_INTERVAL_MS..=MAX_INTERVAL_MS` (or not a number) is flagged and leaves
/// the stored setting untouched.
#[derive(Debug, Default)]
struct IntervalEdit {
    text: String,
    invalid: bool,
}

impl IntervalEdit {
    fn new(value: u32) -> Self {
        Self {
            text: value.to_string(),
            invalid: false,
        }
    }

    /// Re-reads the value from a config that was reloaded or reset.
    fn sync(&mut self, value: u32) {
        self.text = value.to_string();
        self.invalid = false;
    }

    /// Draws the field and writes a valid entry back into `value`.
    fn show(&mut self, ui: &mut egui::Ui, labels: &Labels, value: &mut u32) {
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.text)
                .desired_width(52.0)
                .horizontal_align(egui::Align::RIGHT),
        );
        if response.changed() {
            match self.text.trim().parse::<u32>() {
                Ok(parsed) if (MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&parsed) => {
                    *value = parsed;
                    self.invalid = false;
                }
                _ => self.invalid = true,
            }
        }
        if self.invalid {
            ui.colored_label(
                Color32::from_rgb(0xf8, 0x71, 0x71),
                format!("{} {}", labels.interval_range_hint, self.range()),
            );
        } else {
            ui.label(
                RichText::new("ms")
                    .size(11.0)
                    .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
            );
        }
    }

    fn range(&self) -> String {
        format!("{MIN_INTERVAL_MS}-{MAX_INTERVAL_MS}")
    }
}

impl MacroApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let korean_font = install_korean_font(&cc.egui_ctx);
        setup_custom_theme(&cc.egui_ctx);

        let labels = if korean_font {
            Labels::korean()
        } else {
            Labels::english()
        };

        let config_path = config_path();
        let loaded = Config::load(&config_path);
        let press_edit = IntervalEdit::new(loaded.config.press_ms);
        let gap_edit = IntervalEdit::new(loaded.config.gap_ms);
        let runner = MacroRunner::new();
        let vacant_progress = runner.vacant_progress();

        Self {
            labels,
            config: loaded.config,
            config_path,
            warning: loaded.warning,
            notice: None,
            hotkey_error: None,
            armed: false,
            listener: None,
            runner,
            running: None,
            spire_preview: None,
            stargate_preview: None,
            vacant_progress,
            foreground: None,
            last_diagnostics: Instant::now()
                .checked_sub(DIAGNOSTICS_REFRESH)
                .unwrap_or_else(Instant::now),
            press_edit,
            gap_edit,
        }
    }

    /// Replaces the one visible status line.
    fn set_notice(&mut self, level: NoticeLevel, text: impl Into<String>) {
        self.notice = Some(Notice {
            level,
            text: text.into(),
        });
    }

    /// Arms: validates settings, then registers all hotkeys or none.
    fn arm(&mut self, ctx: &egui::Context) {
        if self.armed {
            return;
        }
        if let Err(error) = self.config.validate() {
            self.warning = Some(ConfigWarning::Invalid {
                detail: error.to_string(),
            });
            return;
        }
        self.warning = None;
        self.runner.rearm();

        let wake: Wake = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        match HotkeyListener::start(self.config.bindings(), wake, self.runner.cancel_flag()) {
            Ok(listener) => {
                self.listener = Some(listener);
                self.armed = true;
                self.hotkey_error = None;
                self.notice = None;
            }
            Err(error) => {
                // A failed arm leaves no bindings and no running macro behind.
                self.listener = None;
                self.armed = false;
                self.hotkey_error = Some(error);
                self.runner.cancel_and_join();
            }
        }
    }

    /// Cancels immediately, then releases hotkeys and joins the worker.
    ///
    /// `note` becomes the visible status line, so an emergency stop is not
    /// silent; a manual disarm passes `None` because the state indicator
    /// already shows "disarmed".
    fn disarm(&mut self, note: Option<&str>) {
        self.runner.request_cancel();
        if let Some(mut listener) = self.listener.take()
            && let Err(error) = listener.stop()
        {
            self.hotkey_error = Some(error);
        }
        self.runner.cancel_and_join();
        self.running = None;
        self.armed = false;
        self.notice = note.map(|text| Notice {
            level: NoticeLevel::Info,
            text: text.to_owned(),
        });
    }

    /// Routes one hotkey. The row trigger runs the row macro, the Spire action
    /// key runs the full action (search, click each detection, `A` per verified
    /// selection), and the emergency key stops both.
    fn on_hotkey(&mut self, slot: HotkeySlot) {
        match action_for(slot) {
            HotkeyAction::StartRowBuild => self.start_row_build(),
            HotkeyAction::StartSpireAction => self.start_spire_action(),
            HotkeyAction::StartStargateAction => self.start_stargate_action(),
            HotkeyAction::StartVacantColony => self.start_vacant_colony(),
            HotkeyAction::EmergencyStop => self.disarm(Some(self.labels.status_emergency)),
        }
    }

    /// Starts the unified row-build macro: the configured build target decides
    /// the build keys, and the row macro and the Spire action share the one
    /// press/gap pair.
    fn start_row_build(&mut self) {
        if !self.armed {
            self.set_notice(NoticeLevel::Info, self.labels.status_not_armed);
            return;
        }
        let adapter = SendInputAdapter::new(&self.config.target_process);
        let target = self.config.build_target;
        let started = self.runner.try_start_row_build(
            self.config.timing(),
            self.config.colony_row_mode,
            target,
            self.config.force_build,
            Box::new(adapter),
        );
        match started {
            Ok(()) => {
                self.running = Some(ActiveRun::RowBuild);
                self.notice = None;
            }
            // Busy triggers are reported, never queued.
            Err(error) => self.set_notice(NoticeLevel::Info, self.labels.start_error(error)),
        }
    }

    /// Starts the Spire action while armed.
    ///
    /// The action key always runs the full action: one fresh full-screen scan,
    /// a click per detection and `A` once per verified selection. There is no
    /// preview-only mode and nothing to enable first — the scan happens on the
    /// worker thread (the GUI never runs image matching) and no coordinate from
    /// an earlier run is reused. It shares the single worker slot with the row
    /// build, so F8, disarm and window close cancel it the same way and the two
    /// can never overlap.
    fn start_spire_action(&mut self) {
        if !self.armed {
            self.set_notice(NoticeLevel::Info, self.labels.status_not_armed);
            return;
        }
        let adapter = SendInputAdapter::new(&self.config.target_process);
        let started = self
            .runner
            .try_start_spire_action(self.config.timing(), Box::new(adapter));
        match started {
            Ok(()) => {
                // The badge is set before the worker has reported anything, so
                // the search stage of the action is never shown as idle.
                self.running = Some(ActiveRun::SpireAction);
                self.notice = None;
            }
            Err(error) => self.set_notice(NoticeLevel::Info, self.labels.start_error(error)),
        }
    }

    /// Starts the Stargate action while armed.
    ///
    /// The action key always runs the full action: one fresh full-screen scan,
    /// a click per detection and `A` once per verified selection. It shares the
    /// single worker slot with the other runs, so F8, disarm and window close
    /// cancel it the same way and two runs can never overlap.
    fn start_stargate_action(&mut self) {
        if !self.armed {
            self.set_notice(NoticeLevel::Info, self.labels.status_not_armed);
            return;
        }
        let adapter = SendInputAdapter::new(&self.config.target_process);
        let started = self.runner.try_start_stargate_action(
            self.config.timing(),
            self.config.stargate_recall_f2,
            Box::new(adapter),
        );
        match started {
            Ok(()) => {
                self.running = Some(ActiveRun::StargateAction);
                self.notice = None;
            }
            Err(error) => self.set_notice(NoticeLevel::Info, self.labels.start_error(error)),
        }
    }

    fn start_vacant_colony(&mut self) {
        if !self.armed {
            self.set_notice(NoticeLevel::Info, self.labels.status_not_armed);
            return;
        }
        let adapter = SendInputAdapter::new(&self.config.target_process);
        match self
            .runner
            .try_start_vacant_colony(self.config.timing(), Box::new(adapter))
        {
            Ok(()) => {
                self.running = Some(ActiveRun::VacantColony);
                self.notice = None;
            }
            Err(error) => self.set_notice(NoticeLevel::Info, self.labels.start_error(error)),
        }
    }

    fn save_config(&mut self) {
        match self.config.save(&self.config_path) {
            Ok(()) => {
                self.set_notice(NoticeLevel::Ok, self.labels.saved_ok);
                self.warning = None;
            }
            Err(error) => {
                self.set_notice(NoticeLevel::Err, self.labels.config_error(&error));
            }
        }
    }

    fn reload_config(&mut self) {
        let loaded = Config::load(&self.config_path);
        self.config = loaded.config;
        self.warning = loaded.warning;
        self.notice = None;
        // A result from the replaced settings must not be shown as current.
        self.spire_preview = None;
        self.stargate_preview = None;
        self.sync_interval_edits();
    }

    /// Keeps the text fields in step with a config that was just replaced.
    fn sync_interval_edits(&mut self) {
        self.press_edit.sync(self.config.press_ms);
        self.gap_edit.sync(self.config.gap_ms);
    }

    fn drain_hotkeys(&mut self) {
        let mut pending = Vec::new();
        if let Some(listener) = &self.listener {
            while let Some(slot) = listener.try_recv() {
                pending.push(slot);
            }
        }
        if pending.contains(&HotkeySlot::Emergency) {
            self.on_hotkey(HotkeySlot::Emergency);
        } else {
            for slot in pending {
                self.on_hotkey(slot);
            }
        }
    }

    /// Applies everything that finished since the last frame.
    ///
    /// Draining happens **before** hotkeys are routed (see [`Self::logic`]):
    /// the worker slot stays occupied until its report is consumed, so a rapid
    /// repeat trigger would be rejected as busy while a finished report is
    /// still queued — and the stale message would then be overwritten anyway.
    fn apply_finished(&mut self, finished: FinishedReports) {
        if finished.is_empty() {
            return;
        }
        if let Some(report) = &finished.vacant_colony {
            let (level, text) = self.labels.vacant_colony_result(report);
            self.set_notice(level, text);
        }
        if let Some(report) = &finished.row {
            self.set_notice(run_notice_level(report), self.labels.outcome(report));
        }
        // The Spire action reports separately, so its diagnostics (capture and
        // detection timings, per-target dispositions) are not squeezed into the
        // row-build report.
        if let Some(report) = &finished.spire_action {
            let preview = self.labels.spire_action_preview(report);
            self.set_notice(preview.level, preview.headline.clone());
            self.spire_preview = Some(preview);
        }
        // The Stargate action has its own report channel and preview, so a
        // Stargate result is never rendered as a Spire result.
        if let Some(report) = &finished.stargate_action {
            let preview = self.labels.stargate_action_preview(report);
            self.set_notice(preview.level, preview.headline.clone());
            self.stargate_preview = Some(preview);
        }
        // A read-only scan result is still rendered if one ever arrives: the
        // GUI no longer starts that pass (the action key always runs the full
        // action), but the runner keeps it as a backend diagnostic, so its
        // report must keep its place in the card instead of silently vanishing.
        // Only a refused or unusable scan interrupts with a status line.
        if let Some(result) = &finished.spire_scan {
            let preview = match result {
                Ok(report) => self.labels.spire_scan_preview(report),
                Err(error) => self.labels.spire_scan_error_preview(error),
            };
            self.notice = (preview.level != NoticeLevel::Info).then(|| Notice {
                level: preview.level,
                text: preview.headline.clone(),
            });
            self.spire_preview = Some(preview);
        }
        // A finished report means the single worker slot is free again.
        self.running = None;
    }

    fn refresh_diagnostics(&mut self) {
        if self.last_diagnostics.elapsed() < DIAGNOSTICS_REFRESH {
            return;
        }
        self.last_diagnostics = Instant::now();
        self.foreground = Some(adapter::foreground_process_name());
    }
}

impl eframe::App for MacroApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Finished reports first, then hotkeys: the single worker slot is freed
        // by consuming a report, so draining first keeps a rapid repeat trigger
        // working instead of rejecting it as busy and then overwriting that
        // message with the stale report. Draining changes nothing else — an
        // emergency still wins over the other pending slots, the cancel latch
        // stays until rearming, and triggers are still never queued.
        let finished = self.runner.drain_finished();
        self.apply_finished(finished);
        self.drain_hotkeys();
        self.refresh_diagnostics();
        // Keeps the status display alive even when only `logic` runs.
        ctx.request_repaint_after(HEARTBEAT);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let labels = self.labels;
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(0x13, 0x14, 0x17))
                    .inner_margin(Margin::symmetric(14, 12)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.render_header(ui, &labels);
                        ui.add_space(6.0);
                        self.render_control_bar(ui, &labels);
                        ui.add_space(6.0);
                        self.render_status_notifications(ui, &labels);
                        ui.add_space(6.0);
                        self.render_spire_action_card(ui, &labels);
                        ui.add_space(8.0);
                        self.render_stargate_action_card(ui, &labels);
                        ui.add_space(8.0);
                        self.render_row_build_card(ui, &labels);
                        ui.add_space(8.0);
                        self.render_vacant_colony_card(ui, &labels);
                        ui.add_space(8.0);
                        self.render_advanced_section(ui, &labels);
                    });
            });
    }
}

impl MacroApp {
    fn render_header(&self, ui: &mut egui::Ui, labels: &Labels) {
        ui.horizontal(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(labels.app_title)
                        .strong()
                        .size(18.0)
                        .color(Color32::from_rgb(0xf1, 0xf5, 0xf9)),
                );
                ui.label(
                    RichText::new("v0.1.0")
                        .monospace()
                        .size(10.5)
                        .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                render_emergency_badge(ui, labels);
            });
        });
    }

    fn render_control_bar(&mut self, ui: &mut egui::Ui, labels: &Labels) {
        Frame::new()
            .fill(Color32::from_rgb(0x1a, 0x1d, 0x24))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x2c, 0x31, 0x3d)))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Left: State badge. Every run that owns the worker slot
                    // reports "running", including the search stage of the
                    // Spire action: an in-progress pass is never shown as idle.
                    match self.running {
                        Some(_) => render_pill(
                            ui,
                            "⚡",
                            labels.status_running,
                            Color32::from_rgb(0x0c, 0x2c, 0x4d),
                            Color32::from_rgb(0x25, 0x63, 0xeb),
                            Color32::from_rgb(0x38, 0xbd, 0xf8),
                        ),
                        None if self.armed => render_pill(
                            ui,
                            "●",
                            labels.status_armed,
                            Color32::from_rgb(0x0e, 0x2d, 0x20),
                            Color32::from_rgb(0x16, 0x65, 0x47),
                            Color32::from_rgb(0x34, 0xd3, 0x99),
                        ),
                        None => render_pill(
                            ui,
                            "○",
                            labels.status_disarmed,
                            Color32::from_rgb(0x21, 0x25, 0x2d),
                            Color32::from_rgb(0x3d, 0x44, 0x54),
                            Color32::from_rgb(0x94, 0xa3, 0xb8),
                        ),
                    }

                    // Right: Obvious Start / Stop Button
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.armed {
                            let btn = egui::Button::new(
                                RichText::new(format!("⏹ {}", labels.disarm_button))
                                    .strong()
                                    .size(13.0)
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(0x7f, 0x1d, 0x1d))
                            .stroke(Stroke::new(1.0, Color32::from_rgb(0xb9, 0x38, 0x43)))
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(130.0, 32.0));

                            if ui.add(btn).clicked() {
                                self.disarm(None);
                            }
                        } else {
                            let ctx = ui.ctx().clone();
                            let btn = egui::Button::new(
                                RichText::new(format!("▶ {}", labels.arm_button))
                                    .strong()
                                    .size(13.0)
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(0x0d, 0x76, 0x4e))
                            .stroke(Stroke::new(1.0, Color32::from_rgb(0x1e, 0xb3, 0x78)))
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(130.0, 32.0));

                            // Arming is only possible with settings that can
                            // actually be registered.
                            let settings_ok = self.config.validate().is_ok();
                            if ui.add_enabled(settings_ok, btn).clicked() {
                                self.arm(&ctx);
                            }
                        }
                    });
                });

                ui.add_space(3.0);
                ui.label(
                    RichText::new(labels.arm_hint)
                        .size(11.0)
                        .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
                );
                if !self.armed && self.config.validate().is_err() {
                    ui.label(
                        RichText::new(labels.arm_hint_invalid)
                            .size(11.0)
                            .color(Color32::from_rgb(0xfb, 0xbf, 0x24)),
                    );
                }
            });
    }

    fn render_status_notifications(&self, ui: &mut egui::Ui, labels: &Labels) {
        if self.hotkey_error.is_none() && self.warning.is_none() && self.notice.is_none() {
            return;
        }

        if let Some(error) = &self.hotkey_error {
            render_notification(
                ui,
                "⚠",
                &labels.hotkey_error(error),
                Color32::from_rgb(0x38, 0x14, 0x16),
                Color32::from_rgb(0x8f, 0x28, 0x2e),
                Color32::from_rgb(0xfc, 0xa5, 0xa5),
            );
        }

        if let Some(warning) = &self.warning {
            render_notification(
                ui,
                "⚠",
                &labels.config_warning(warning),
                Color32::from_rgb(0x38, 0x2a, 0x10),
                Color32::from_rgb(0x8f, 0x65, 0x1c),
                Color32::from_rgb(0xfd, 0xe0, 0x47),
            );
        }

        if let Some(notice) = &self.notice {
            // The level is decided when the result is interpreted, never by
            // matching words in the text: a scan preview stays information and
            // a pass that sent no `A` never renders as a success.
            let (icon, bg, stroke, text_color) = match notice.level {
                NoticeLevel::Err => (
                    "✕",
                    Color32::from_rgb(0x38, 0x14, 0x16),
                    Color32::from_rgb(0x8f, 0x28, 0x2e),
                    Color32::from_rgb(0xfc, 0xa5, 0xa5),
                ),
                NoticeLevel::Ok => (
                    "✓",
                    Color32::from_rgb(0x0e, 0x2d, 0x20),
                    Color32::from_rgb(0x16, 0x65, 0x47),
                    Color32::from_rgb(0x6e, 0xe7, 0xb7),
                ),
                NoticeLevel::Info => (
                    "ℹ",
                    Color32::from_rgb(0x1c, 0x22, 0x2d),
                    Color32::from_rgb(0x36, 0x42, 0x58),
                    Color32::from_rgb(0x93, 0xc5, 0xfd),
                ),
            };

            render_notification(ui, icon, &notice.text, bg, stroke, text_color);
        }
    }

    /// One card for the one row-build macro, driven by the one trigger key.
    fn render_row_build_card(&mut self, ui: &mut egui::Ui, labels: &Labels) {
        row_build_card(
            ui,
            labels,
            &mut self.config,
            self.armed,
            self.running == Some(ActiveRun::RowBuild),
            &mut self.press_edit,
            &mut self.gap_edit,
        );
    }

    /// One compact card for the Spire detect action: its own key, what one run
    /// sends, the caveats, and the newest result.
    fn render_spire_action_card(&mut self, ui: &mut egui::Ui, labels: &Labels) {
        let running =
            (self.running == Some(ActiveRun::SpireAction)).then_some(labels.status_running);
        // The other action keys and F8 are not offered here.
        let conflicts = [
            self.config.trigger_hotkey,
            self.config.vacant_colony_hotkey,
            self.config.stargate_action_hotkey,
            HotkeyKey::F4,
            HotkeyKey::EMERGENCY,
        ];
        building_action_card(
            ui,
            labels,
            ActionCardSpec {
                title: labels.spire_action_title,
                hint: labels.spire_action_hint,
                confirm_note: labels.spire_confirm_scope_note,
                id_salt: "spire_action_hotkey",
                accent: Color32::from_rgb(0x22, 0xd3, 0xee),
                fill: Color32::from_rgb(0x0f, 0x1e, 0x23),
                stroke: Color32::from_rgb(0x1c, 0x44, 0x4e),
            },
            self.armed,
            running,
            self.spire_preview.as_ref(),
            &conflicts,
            &mut self.config.spire_action_hotkey,
            None,
        );
    }

    /// One compact card for the Stargate detect action. Same action pattern as
    /// the Spire card, but titled and worded as the Stargate feature and wired
    /// to its own config key, so the two can never be confused.
    fn render_stargate_action_card(&mut self, ui: &mut egui::Ui, labels: &Labels) {
        let running =
            (self.running == Some(ActiveRun::StargateAction)).then_some(labels.status_running);
        let conflicts = [
            self.config.trigger_hotkey,
            self.config.vacant_colony_hotkey,
            self.config.spire_action_hotkey,
            HotkeyKey::F4,
            HotkeyKey::EMERGENCY,
        ];
        building_action_card(
            ui,
            labels,
            ActionCardSpec {
                title: labels.stargate_action_title,
                hint: labels.stargate_action_hint,
                confirm_note: labels.stargate_confirm_scope_note,
                id_salt: "stargate_action_hotkey",
                accent: Color32::from_rgb(0xf6, 0xc4, 0x53),
                fill: Color32::from_rgb(0x22, 0x1c, 0x0f),
                stroke: Color32::from_rgb(0x65, 0x4b, 0x1c),
            },
            self.armed,
            running,
            self.stargate_preview.as_ref(),
            &conflicts,
            &mut self.config.stargate_action_hotkey,
            Some((
                &mut self.config.stargate_recall_f2,
                labels.stargate_recall_f2_checkbox,
                labels.stargate_recall_f2_hint,
            )),
        );
    }

    fn render_vacant_colony_card(&mut self, ui: &mut egui::Ui, labels: &Labels) {
        Frame::new()
            .fill(Color32::from_rgb(0x22, 0x1c, 0x12))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x65, 0x4b, 0x22)))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(labels.vacant_colony_title())
                            .strong()
                            .size(14.0)
                            .color(Color32::from_rgb(0xfb, 0xbf, 0x24)),
                    );
                    if self.running == Some(ActiveRun::VacantColony) {
                        ui.spinner();
                        ui.label(labels.status_running);
                        // Live counters: a sweep that is still working must not
                        // look like a frozen window.
                        ui.label(
                            RichText::new(labels.vacant_colony_progress(
                                self.vacant_progress.probes(),
                                self.vacant_progress.orders(),
                                self.vacant_progress.starts(),
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                });
                // What one run sends, so the card cannot be mistaken for the
                // row build: recall the saved view, probe, order, then confirm
                // that the ordered drone actually started morphing before the
                // next one is touched.
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(labels.vacant_colony_sequence_label)
                            .size(11.0)
                            .color(Color32::from_rgb(0x94, 0xa3, 0xb8)),
                    );
                    render_keycap(ui, Key::F4.name(), false);
                    ui.label(RichText::new(ARROW).color(MUTED));
                    ui.label(
                        RichText::new(labels.vacant_colony_search_step_label)
                            .size(11.0)
                            .color(Color32::from_rgb(0xfb, 0xbf, 0x24)),
                    );
                    ui.label(RichText::new(ARROW).color(MUTED));
                    render_keycap(ui, Key::B.name(), false);
                    render_keycap(ui, Key::C.name(), false);
                    render_keycap(ui, labels.mouse_click_label, true);
                    ui.label(RichText::new(ARROW).color(MUTED));
                    ui.label(
                        RichText::new(labels.vacant_colony_confirm_step_label)
                            .size(11.0)
                            .color(Color32::from_rgb(0xfb, 0xbf, 0x24)),
                    );
                });
                ui.label(RichText::new(labels.vacant_colony_hint()).size(11.0));
                // Amber: this path never forces a click, and it is not
                // live-validated in game yet.
                ui.label(
                    RichText::new(format!("{} {}", CAVEAT, labels.vacant_colony_caveat))
                        .size(10.0)
                        .color(Color32::from_rgb(0xfb, 0xbf, 0x24)),
                );
                ui.horizontal(|ui| {
                    ui.label(labels.trigger_label);
                    let conflicts = [
                        self.config.trigger_hotkey,
                        self.config.spire_action_hotkey,
                        HotkeyKey::F4,
                        HotkeyKey::EMERGENCY,
                    ];
                    ui.add_enabled_ui(!self.armed, |ui| {
                        hotkey_combo(
                            ui,
                            "vacant_colony_hotkey",
                            self.armed,
                            &conflicts,
                            &mut self.config.vacant_colony_hotkey,
                        );
                    });
                    render_emergency_badge(ui, labels);
                });
                if self.armed {
                    ui.add_space(3.0);
                    ui.label(
                        RichText::new(labels.hotkeys_locked_hint)
                            .size(10.0)
                            .color(MUTED),
                    );
                }
            });
    }

    fn render_advanced_section(&mut self, ui: &mut egui::Ui, labels: &Labels) {
        Frame::new()
            .fill(Color32::from_rgb(0x18, 0x1a, 0x20))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x28, 0x2c, 0x37)))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                egui::CollapsingHeader::new(
                    RichText::new(format!("⚙ {}", labels.advanced_heading))
                        .strong()
                        .size(12.0)
                        .color(Color32::from_rgb(0xd1, 0xd5, 0xdb)),
                )
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_space(4.0);

                    // Target Process & Safety Check
                    ui.label(
                        RichText::new(labels.diag_heading)
                            .strong()
                            .size(12.0)
                            .color(Color32::from_rgb(0x94, 0xa3, 0xb8)),
                    );
                    ui.horizontal(|ui| {
                        ui.label(labels.target_label);
                        ui.add_enabled(
                            !self.armed,
                            egui::TextEdit::singleline(&mut self.config.target_process)
                                .desired_width(150.0),
                        );
                    });
                    ui.label(
                        RichText::new(labels.target_hint)
                            .size(11.0)
                            .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
                    );

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", labels.diag_foreground));
                        match self.foreground.as_ref() {
                            Some(Ok(name)) => {
                                ui.monospace(name);
                                if name.eq_ignore_ascii_case(self.config.target_process.trim()) {
                                    ui.colored_label(
                                        Color32::from_rgb(0x10, 0xb9, 0x81),
                                        format!("✓ {}", labels.diag_target_ok),
                                    );
                                } else {
                                    ui.colored_label(
                                        Color32::from_rgb(0xf8, 0x71, 0x71),
                                        format!("✕ {}", labels.diag_target_not),
                                    );
                                }
                            }
                            Some(Err(detail)) => {
                                ui.colored_label(
                                    Color32::from_rgb(0x94, 0xa3, 0xb8),
                                    labels.diag_unknown,
                                );
                                ui.label(detail.clone());
                            }
                            None => {
                                ui.label(labels.diag_unknown);
                            }
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Config File Operations
                    ui.label(
                        RichText::new(labels.config_heading)
                            .strong()
                            .size(12.0)
                            .color(Color32::from_rgb(0x94, 0xa3, 0xb8)),
                    );
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", labels.config_path_label));
                        ui.monospace(self.config_path.display().to_string());
                    });
                    ui.horizontal(|ui| {
                        if ui.button(format!("💾 {}", labels.save_button)).clicked() {
                            self.save_config();
                        }
                        if ui
                            .add_enabled(
                                !self.armed,
                                egui::Button::new(format!("🔄 {}", labels.reload_button)),
                            )
                            .clicked()
                        {
                            self.reload_config();
                        }
                        if ui
                            .add_enabled(
                                !self.armed,
                                egui::Button::new(format!("↺ {}", labels.reset_button)),
                            )
                            .clicked()
                        {
                            self.config = Config::default();
                            self.spire_preview = None;
                            self.sync_interval_edits();
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Notes Section
                    ui.label(
                        RichText::new(labels.notes_heading)
                            .strong()
                            .size(12.0)
                            .color(Color32::from_rgb(0x94, 0xa3, 0xb8)),
                    );
                    for note in [
                        labels.note_select_drone,
                        labels.note_chat,
                        labels.note_online,
                    ] {
                        ui.label(
                            RichText::new(format!("• {note}"))
                                .size(11.0)
                                .color(Color32::from_rgb(0xc4, 0xcc, 0xd8)),
                        );
                    }
                    if labels.lang == Lang::English {
                        ui.label(
                            RichText::new(format!("• {}", labels.note_language_fallback))
                                .size(11.0)
                                .color(Color32::from_rgb(0xfb, 0xbf, 0x24)),
                        );
                    }
                });
            });
    }
}

/// One card for the unified row-build macro: the trigger key, the
/// build-target checkbox, the row order, the forced mode and the timing.
fn row_build_card(
    ui: &mut egui::Ui,
    labels: &Labels,
    config: &mut Config,
    armed: bool,
    is_running: bool,
    press_edit: &mut IntervalEdit,
    gap_edit: &mut IntervalEdit,
) {
    // The accent follows the selected building.
    let (accent_color, fill_color, stroke_color) = match config.build_target {
        BuildTarget::Colony => (
            Color32::from_rgb(0x10, 0xb9, 0x81), // Teal/Green
            Color32::from_rgb(0x15, 0x1e, 0x1c),
            Color32::from_rgb(0x23, 0x47, 0x3d),
        ),
        BuildTarget::Spire => (
            Color32::from_rgb(0xa7, 0x8b, 0xfa), // Violet
            Color32::from_rgb(0x1a, 0x17, 0x26),
            Color32::from_rgb(0x3e, 0x31, 0x5e),
        ),
    };
    let target = config.build_target;
    let keys = target.build_keys();
    let editable = !armed;

    Frame::new()
        .fill(fill_color)
        .stroke(Stroke::new(1.0, stroke_color))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            // Header Row: Icon + Title + Running badge
            ui.horizontal(|ui| {
                let (rect, _response) =
                    ui.allocate_exact_size(vec2(24.0, 24.0), egui::Sense::hover());
                match target {
                    BuildTarget::Colony => draw_colony_icon(ui.painter(), rect, accent_color),
                    BuildTarget::Spire => draw_spire_icon(ui.painter(), rect, accent_color),
                }

                ui.add_space(4.0);
                ui.label(
                    RichText::new(labels.row_build_title)
                        .strong()
                        .size(14.0)
                        .color(accent_color),
                );

                if is_running {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        Frame::new()
                            .fill(Color32::from_rgb(0x0c, 0x2c, 0x4d))
                            .stroke(Stroke::new(1.0, Color32::from_rgb(0x25, 0x63, 0xeb)))
                            .corner_radius(CornerRadius::same(4))
                            .inner_margin(Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("⚡ {}", labels.status_running))
                                        .size(11.0)
                                        .color(Color32::from_rgb(0x38, 0xbd, 0xf8)),
                                );
                            });
                    });
                }
            });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            // Build target: unchanged = Creep Colony, checked = Spire.
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(labels.build_target_label)
                        .size(11.0)
                        .color(Color32::from_rgb(0x94, 0xa3, 0xb8)),
                );
                ui.add_enabled_ui(editable, |ui| {
                    let mut is_spire = target == BuildTarget::Spire;
                    if ui
                        .add(egui::Checkbox::new(
                            &mut is_spire,
                            labels.build_spire_checkbox,
                        ))
                        .changed()
                    {
                        config.build_target = if is_spire {
                            BuildTarget::Spire
                        } else {
                            BuildTarget::Colony
                        };
                    }
                });
                ui.label(
                    RichText::new(labels.build_target_name(target))
                        .strong()
                        .size(11.5)
                        .color(accent_color),
                );
            });

            ui.add_space(6.0);

            // Sequence row: Keycaps of the selected building.
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(labels.sequence_label)
                        .size(11.0)
                        .color(Color32::from_rgb(0x94, 0xa3, 0xb8)),
                );
                render_keycap(ui, keys[0].name(), false);
                ui.label(RichText::new("→").color(Color32::from_rgb(0x64, 0x74, 0x8b)));
                render_keycap(ui, keys[1].name(), false);
                ui.label(RichText::new("→").color(Color32::from_rgb(0x64, 0x74, 0x8b)));
                render_keycap(ui, labels.mouse_click_label, true);
            });

            ui.add_space(8.0);

            // One trigger key starts the row build.
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(labels.trigger_label)
                        .size(11.0)
                        .color(Color32::from_rgb(0x94, 0xa3, 0xb8)),
                );
                ui.add_enabled_ui(editable, |ui| {
                    // The action key and F8 are not offered here, so a
                    // conflicting trigger cannot be created in the GUI.
                    let conflicts = [
                        config.spire_action_hotkey,
                        config.vacant_colony_hotkey,
                        HotkeyKey::F4,
                        HotkeyKey::EMERGENCY,
                    ];
                    hotkey_combo(
                        ui,
                        "row_build_hotkey",
                        armed,
                        &conflicts,
                        &mut config.trigger_hotkey,
                    );
                });
                ui.label(
                    RichText::new(labels.emergency_label)
                        .size(10.5)
                        .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
                );
                ui.label(
                    RichText::new("F8")
                        .monospace()
                        .size(11.0)
                        .color(Color32::from_rgb(0xf8, 0x71, 0x71)),
                );
            });
            ui.label(
                RichText::new(labels.trigger_single_hint)
                    .size(10.0)
                    .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
            );

            ui.add_space(8.0);

            // Timing Controls: one shared press/gap pair.
            ui.add_enabled_ui(editable, |ui| {
                ui.label(
                    RichText::new(labels.timing_heading)
                        .size(11.0)
                        .strong()
                        .color(Color32::from_rgb(0x94, 0xa3, 0xb8)),
                );
                ui.add_space(2.0);

                let (press_ms, gap_ms) = config.intervals_mut();

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(labels.press_label)
                            .size(11.0)
                            .color(Color32::from_rgb(0xc4, 0xcc, 0xd8)),
                    );
                    press_edit.show(ui, labels, press_ms);
                });

                ui.add_space(2.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(labels.gap_label)
                            .size(11.0)
                            .color(Color32::from_rgb(0xc4, 0xcc, 0xd8)),
                    );
                    gap_edit.show(ui, labels, gap_ms);
                });
                ui.label(
                    RichText::new(labels.timing_text_hint)
                        .size(10.0)
                        .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
                );
            });

            if armed {
                ui.add_space(3.0);
                ui.label(
                    RichText::new(labels.hotkeys_locked_hint)
                        .size(10.0)
                        .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
                );
            }

            ui.add_space(8.0);
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(labels.row_mode_label)
                            .size(11.0)
                            .color(Color32::from_rgb(0x94, 0xa3, 0xb8)),
                    );
                    for mode in RowMode::ALL {
                        ui.selectable_value(
                            &mut config.colony_row_mode,
                            mode,
                            labels.row_mode_name(mode),
                        );
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.add(egui::Checkbox::new(
                            &mut config.force_build,
                            labels.force_build_checkbox,
                        ));
                    });
                });
                ui.label(
                    RichText::new(labels.force_build_hint)
                        .size(10.0)
                        .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
                );
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(labels.row_build_hint)
                    .size(10.0)
                    .color(Color32::from_rgb(0x64, 0x74, 0x8b)),
            );
        });
}

/// The static wording and colors one action card is drawn with. The two
/// actions share the card layout; only this data differs.
struct ActionCardSpec {
    title: &'static str,
    hint: &'static str,
    confirm_note: &'static str,
    id_salt: &'static str,
    accent: Color32,
    fill: Color32,
    stroke: Color32,
}

/// One compact card for a detect action (Spire or Stargate).
///
/// Deliberately distinct from the row-build card and its checkbox: this is not
/// the `BuildTarget::Spire` row macro. Nothing here starts input — the
/// registered action key does, and only while the game window is foreground —
/// and pressing it always runs the action: there is no preview-only switch and
/// no stored mode, so the card has no control that could turn one on.
#[allow(clippy::too_many_arguments)]
fn building_action_card(
    ui: &mut egui::Ui,
    labels: &Labels,
    spec: ActionCardSpec,
    armed: bool,
    running: Option<&str>,
    preview: Option<&SpirePreview>,
    conflicts: &[HotkeyKey],
    key: &mut HotkeyKey,
    extra_toggle: Option<(&mut bool, &str, &str)>,
) {
    // A card-specific accent, so the two actions cannot be mistaken for each
    // other or for the green Creep Colony / violet Spire row build.
    let accent_color = spec.accent;
    let muted = Color32::from_rgb(0x64, 0x74, 0x8b);
    let label_color = Color32::from_rgb(0x94, 0xa3, 0xb8);
    let editable = !armed;

    Frame::new()
        .fill(spec.fill)
        .stroke(Stroke::new(1.0, spec.stroke))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            // Header Row: Icon + Title + Running badge
            ui.horizontal(|ui| {
                let (rect, _response) =
                    ui.allocate_exact_size(vec2(24.0, 24.0), egui::Sense::hover());
                draw_scan_icon(ui.painter(), rect, accent_color);

                ui.add_space(4.0);
                ui.label(
                    RichText::new(spec.title)
                        .strong()
                        .size(14.0)
                        .color(accent_color),
                );

                if let Some(badge) = running {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        Frame::new()
                            .fill(Color32::from_rgb(0x0c, 0x2c, 0x4d))
                            .stroke(Stroke::new(1.0, Color32::from_rgb(0x25, 0x63, 0xeb)))
                            .corner_radius(CornerRadius::same(4))
                            .inner_margin(Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("⚡ {badge}"))
                                        .size(11.0)
                                        .color(Color32::from_rgb(0x38, 0xbd, 0xf8)),
                                );
                            });
                    });
                }
            });

            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(labels.spire_action_hotkey_label)
                        .size(11.0)
                        .color(label_color),
                );
                ui.add_enabled_ui(editable, |ui| {
                    hotkey_combo(ui, spec.id_salt, armed, conflicts, key);
                });
                ui.separator();
                ui.label(
                    RichText::new(labels.spire_search_step_label)
                        .size(11.0)
                        .color(accent_color),
                );
                ui.label(RichText::new("→").color(muted));
                render_keycap(ui, labels.mouse_click_label, true);
                ui.label(RichText::new("→").color(muted));
                render_keycap(ui, Key::A.name(), false);
                ui.separator();
                ui.label(
                    RichText::new(labels.emergency_label)
                        .size(10.5)
                        .color(Color32::from_rgb(0xf8, 0x71, 0x71)),
                );
                render_keycap(ui, HotkeyKey::EMERGENCY.label(), false);
            });
            ui.add_space(4.0);
            ui.label(RichText::new(spec.hint).size(10.5).color(muted));
            ui.label(
                RichText::new(format!("⚠ {}", spec.confirm_note))
                    .size(10.0)
                    .color(Color32::from_rgb(0xfb, 0xbf, 0x24)),
            );
            if let Some((value, label, hint)) = extra_toggle {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_enabled(editable, egui::Checkbox::new(value, label));
                    ui.label(RichText::new(hint).size(10.0).color(muted));
                });
            }

            if armed {
                ui.add_space(3.0);
                ui.label(
                    RichText::new(labels.hotkeys_locked_hint)
                        .size(10.0)
                        .color(muted),
                );
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // The newest result only: no history, no activity log.
            ui.label(
                RichText::new(labels.spire_preview_heading)
                    .strong()
                    .size(11.0)
                    .color(label_color),
            );
            match preview {
                Some(preview) => {
                    let headline_color = match preview.level {
                        NoticeLevel::Ok => Color32::from_rgb(0x6e, 0xe7, 0xb7),
                        NoticeLevel::Err => Color32::from_rgb(0xfc, 0xa5, 0xa5),
                        NoticeLevel::Info => accent_color,
                    };
                    ui.label(
                        RichText::new(&preview.headline)
                            .size(11.0)
                            .color(headline_color),
                    );
                    ui.add_space(2.0);
                    if let Some(positions) = &preview.positions {
                        ui.label(
                            RichText::new(positions)
                                .monospace()
                                .size(10.5)
                                .color(Color32::from_rgb(0xc4, 0xcc, 0xd8)),
                        );
                    }
                }
                None => {
                    ui.label(
                        RichText::new(labels.spire_preview_empty)
                            .size(10.5)
                            .color(muted),
                    );
                }
            }
        });
}

/// One selectable hotkey. Only keys that are not already used by another
/// binding (or reserved for F8) are offered, so a conflicting setting cannot be
/// created in the GUI.
fn hotkey_combo(
    ui: &mut egui::Ui,
    id_salt: &str,
    armed: bool,
    conflicts: &[HotkeyKey],
    key: &mut HotkeyKey,
) {
    let mut selected = *key;
    let display_text = if armed {
        format!("🔒 {}", selected.label())
    } else {
        selected.label().to_string()
    };
    egui::ComboBox::from_id_salt(id_salt)
        .width(76.0)
        .selected_text(RichText::new(display_text).monospace().strong())
        .show_ui(ui, |ui| {
            for option in HotkeyKey::ALL {
                let taken = conflicts.contains(&option);
                ui.add_enabled_ui(!taken, |ui| {
                    ui.selectable_value(&mut selected, option, option.label());
                });
            }
        });
    // Only a key that was actually offered may replace the stored one.
    if !conflicts.contains(&selected) {
        *key = selected;
    }
}

fn render_pill(
    ui: &mut egui::Ui,
    icon: &str,
    text: &str,
    bg: Color32,
    stroke_color: Color32,
    text_color: Color32,
) {
    Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, stroke_color))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(icon).size(13.0).color(text_color));
            ui.label(RichText::new(text).strong().size(12.0).color(text_color));
        });
}

fn render_notification(
    ui: &mut egui::Ui,
    icon: &str,
    text: &str,
    bg: Color32,
    stroke_color: Color32,
    text_color: Color32,
) {
    Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, stroke_color))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(icon).strong().color(text_color));
            ui.label(RichText::new(text).size(11.5).color(text_color));
        });
}

fn render_keycap(ui: &mut egui::Ui, text: &str, is_mouse: bool) {
    let padding_x = if is_mouse { 6 } else { 8 };
    Frame::new()
        .fill(Color32::from_rgb(0x23, 0x26, 0x2e))
        .stroke(Stroke::new(1.0, Color32::from_rgb(0x42, 0x48, 0x56)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(padding_x, 3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if is_mouse {
                ui.label(RichText::new("🖱").size(11.0));
            }
            ui.label(
                RichText::new(text)
                    .monospace()
                    .strong()
                    .size(11.5)
                    .color(Color32::from_rgb(0xe2, 0xe8, 0xf0)),
            );
        });
}

fn render_emergency_badge(ui: &mut egui::Ui, labels: &Labels) {
    Frame::new()
        .fill(Color32::from_rgb(0x38, 0x14, 0x16))
        .stroke(Stroke::new(1.0, Color32::from_rgb(0x8f, 0x28, 0x2e)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new("🛑").size(12.0));
            ui.label(
                RichText::new(labels.emergency_label)
                    .size(11.0)
                    .color(Color32::from_rgb(0xfc, 0xa5, 0xa5)),
            );
            ui.label(
                RichText::new(HotkeyKey::EMERGENCY.label())
                    .strong()
                    .size(12.0)
                    .color(Color32::from_rgb(0xf8, 0x71, 0x71)),
            );
        })
        .response
        .on_hover_text(labels.emergency_hint);
}

fn draw_colony_icon(painter: &Painter, rect: Rect, accent: Color32) {
    let center = rect.center();
    let r = rect.width().min(rect.height()) * 0.44;

    painter.circle_filled(center, r, accent.gamma_multiply(0.2));

    for i in 0..3 {
        let angle = (i as f32) * std::f32::consts::TAU / 3.0 - std::f32::consts::FRAC_PI_2;
        let offset = vec2(angle.cos(), angle.sin()) * (r * 0.72);
        let node_center = center + offset;
        painter.line_segment(
            [center, node_center],
            Stroke::new(1.8, accent.gamma_multiply(0.7)),
        );
        painter.circle_filled(node_center, r * 0.28, accent);
        painter.circle_filled(node_center, r * 0.12, Color32::WHITE);
    }

    painter.circle_filled(center, r * 0.52, accent);
    painter.circle_filled(center, r * 0.22, Color32::from_rgb(0xe6, 0xff, 0xfa));
}

fn draw_spire_icon(painter: &Painter, rect: Rect, accent: Color32) {
    let center = rect.center();
    let w = rect.width() * 0.44;
    let h = rect.height() * 0.46;

    painter.circle_filled(center, h, accent.gamma_multiply(0.15));

    let top = center - vec2(0.0, h);
    let bottom = center + vec2(0.0, h * 0.9);
    let left = center - vec2(w * 0.35, -h * 0.1);
    let right = center + vec2(w * 0.35, h * 0.1);

    painter.add(egui::Shape::convex_polygon(
        vec![top, right, bottom, left],
        accent,
        Stroke::NONE,
    ));

    let wing_l_tip = center + vec2(-w * 0.85, -h * 0.2);
    let wing_l_base = center + vec2(-w * 0.25, h * 0.6);
    painter.line_segment(
        [left, wing_l_tip],
        Stroke::new(2.0, accent.gamma_multiply(0.8)),
    );
    painter.line_segment(
        [wing_l_tip, wing_l_base],
        Stroke::new(1.5, accent.gamma_multiply(0.5)),
    );

    let wing_r_tip = center + vec2(w * 0.85, -h * 0.2);
    let wing_r_base = center + vec2(w * 0.25, h * 0.6);
    painter.line_segment(
        [right, wing_r_tip],
        Stroke::new(2.0, accent.gamma_multiply(0.8)),
    );
    painter.line_segment(
        [wing_r_tip, wing_r_base],
        Stroke::new(1.5, accent.gamma_multiply(0.5)),
    );

    painter.circle_filled(center + vec2(0.0, -h * 0.1), 2.5, Color32::WHITE);
}

/// A small magnifier: this card looks for Spires, it does not build one.
fn draw_scan_icon(painter: &Painter, rect: Rect, accent: Color32) {
    let stroke = Stroke::new(1.6, accent);
    let radius = rect.width().min(rect.height()) * 0.3;
    let center = rect.center() - vec2(radius * 0.3, radius * 0.3);
    painter.circle_stroke(center, radius, stroke);
    painter.line_segment(
        [
            center + vec2(radius * 0.72, radius * 0.72),
            center + vec2(radius * 1.6, radius * 1.6),
        ],
        stroke,
    );
}

fn setup_custom_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(0x13, 0x14, 0x17);
    visuals.window_fill = Color32::from_rgb(0x13, 0x14, 0x17);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(0x18, 0x1a, 0x1f);
    visuals.widgets.noninteractive.fg_stroke =
        Stroke::new(1.0, Color32::from_rgb(0xd1, 0xd5, 0xdb));
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(0x20, 0x23, 0x2a);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(5);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x2a, 0x2f, 0x38);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(5);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0x33, 0x3a, 0x47);
    visuals.widgets.active.corner_radius = CornerRadius::same(5);
    ctx.set_visuals(visuals);
}

/// Where settings live: `%APPDATA%\oh-my-macro\config.toml`, or next to the exe
/// if `APPDATA` is unavailable.
fn config_path() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata)
            .join("oh-my-macro")
            .join("config.toml");
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("oh-my-macro.toml")
}

/// Installs a Hangul capable system font; returns whether Korean labels can be
/// used. Falling back to English is intentional: egui would otherwise draw boxes.
fn install_korean_font(ctx: &egui::Context) -> bool {
    let Some(bytes) = load_korean_font() else {
        return false;
    };
    let mut fonts = egui::FontDefinitions::default();
    const NAME: &str = "oh-my-macro-korean";
    fonts
        .font_data
        .insert(NAME.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, NAME.to_owned());
    }
    ctx.set_fonts(fonts);
    true
}

/// First usable Korean font file found in the system font directories.
fn load_korean_font() -> Option<Vec<u8>> {
    for directory in font_directories() {
        for file in KOREAN_FONT_FILES {
            let path = directory.join(file);
            if let Ok(bytes) = std::fs::read(&path)
                && font::looks_like_sfnt(&bytes)
            {
                return Some(bytes);
            }
        }
    }
    None
}

fn font_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    match std::env::var_os("WINDIR") {
        Some(windir) => directories.push(PathBuf::from(windir).join("Fonts")),
        None => directories.push(PathBuf::from(r"C:\Windows\Fonts")),
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        directories.push(
            PathBuf::from(local)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts"),
        );
    }
    directories
}
